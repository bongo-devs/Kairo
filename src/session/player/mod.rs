//! One guild's player.
//!
//! Wraps the engine's [`player::AudioPlayer`], an optional [`voice::VoiceConnection`] for the
//! Discord send loop, and an [`AudioLossCounter`]. Engine [`AudioEvent`]s are forwarded to the
//! owning [`SocketContext`] as WebSocket events, and the PATCH operations the REST layer applies
//! (play, stop, seek, volume, pause, filters, endTime, voice) all land here.
//!
//! Engine events carry only [`player::AudioTrackInfo`], never the encodable track, so the protocol
//! [`Track`] for the current track and for the one it replaced is kept in [`PlayerShared`]. A
//! `TrackEnd(Replaced)` resolves to the previous track and every other event to the current one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use bytes::Bytes;
use serde_json::{Map, Value};
use tokio::task::AbortHandle;

use player::player::event::{AudioEvent, AudioEventListener};
use player::track::marker::{MarkerId, MarkerState, TrackMarker};
use player::track::state::AudioTrackEndReason as EngineEndReason;
use player::{AudioPlayer, AudioTrack, CrossfadeOptions, TapeOptions};

use voice::{
    ConnectionState, EventDispatcher, OpusFrameProvider, VoiceConnection, VoiceEvent,
    VoiceEventListener, VoiceServerInfo,
};

use crate::protocol::filters::Filters;
use crate::protocol::load_result::Exception;
use crate::protocol::message::{EmittedEvent, Message, TrackEndReason};
use crate::protocol::player::{CrossfadeSettings, Player, PlayerState, VoiceState};
use crate::protocol::track::Track;
use crate::rest::error::{now_millis, RestError};
use crate::session::context::SocketContext;
use crate::session::loss_counter::AudioLossCounter;
use crate::session::lyrics::{query_from_info, PlayerLyrics};
use crate::sponsorblock::{self, TrackMarkable};

use lyrics::LyricsService;

const GAPLESS_LOOKAHEAD_MS: u64 = 500;

static UPDATE_INTERVAL_SECS: OnceLock<u64> = OnceLock::new();

/// Record the configured `playerUpdateInterval`. Called once from `AppState::new`.
pub fn set_update_interval(secs: u64) {
    let _ = UPDATE_INTERVAL_SECS.set(secs.max(1));
}

// The current track and the one it replaced, as protocol tracks.
#[derive(Default)]
struct TrackSlots {
    current: Option<Track>,
    previous: Option<Track>,
}

/// State shared between a [`LavalinkPlayer`] and the engine event listener it registers.
///
/// The owning [`SocketContext`] is held weakly, since it owns the player in turn.
pub struct PlayerShared {
    guild_id: u64,
    context: Weak<SocketContext>,
    player: Weak<AudioPlayer>,
    tracks: Mutex<TrackSlots>,
    // Set just before an `endTime` marker stops the track, so the engine `Stopped` end that follows
    // is reported to clients as `Finished`.
    end_marker_hit: AtomicBool,
    transition_outgoing: Mutex<Option<Track>>,
    transition_incoming: Mutex<Option<Track>>,
    // Present only when the lyrics feature is enabled.
    lyrics: Mutex<Option<Arc<PlayerLyrics>>>,
    // True while a crossfade or gapless successor is in flight. Lyrics need it to tell the two
    // starts apart: a normal play seeds on `TrackStart`, but a transition successor has to wait for
    // `TrackPromoted`, because at the overlap-start `TrackStart` it is still a pending mixer layer
    // on the wrong executor.
    in_transition: AtomicBool,
    // The same counter the player holds; one listener drives both.
    loss: Arc<AudioLossCounter>,
    // Captured at construction, so events arriving on the decode thread can hand blocking work to
    // the runtime's bounded pool instead of spawning threads of their own.
    runtime: tokio::runtime::Handle,
}

impl PlayerShared {
    fn current_track(&self) -> Option<Track> {
        self.tracks.lock().unwrap().current.clone()
    }

    fn lyrics(&self) -> Option<Arc<PlayerLyrics>> {
        self.lyrics.lock().unwrap().clone()
    }

    fn previous_track(&self) -> Option<Track> {
        self.tracks.lock().unwrap().previous.clone()
    }

    // Make `track` current, moving the old current into `previous` for the `Replaced` event.
    fn replace_current(&self, track: Track) {
        let mut slots = self.tracks.lock().unwrap();
        slots.previous = slots.current.take();
        slots.current = Some(track);
    }

    // Move the outgoing track aside and install the successor before the engine emits its
    // transition events, so `TrackStart` for the successor is already resolvable even though the
    // overlap begins before the old executor reaches its terminator.
    fn begin_transition(&self, track: Track) -> Option<Track> {
        let outgoing = self.current_track()?;
        *self.transition_outgoing.lock().unwrap() = Some(outgoing.clone());
        *self.transition_incoming.lock().unwrap() = Some(track);
        self.in_transition.store(true, Ordering::Release);
        Some(outgoing)
    }

    fn abort_transition(&self) {
        self.transition_outgoing.lock().unwrap().take();
        self.transition_incoming.lock().unwrap().take();
        self.in_transition.store(false, Ordering::Release);
    }

    fn clear_current(&self) {
        self.tracks.lock().unwrap().current = None;
    }

    fn set_user_data(&self, user_data: Map<String, Value>) {
        if let Some(track) = self.tracks.lock().unwrap().current.as_mut() {
            track.user_data = user_data;
        }
    }

    // Drive the loss counter's playback window: a pause or an end closes it, a start or a resume
    // opens it, and only a gap longer than the acceptable switch time resets the usable-data clock.
    //
    // Transition events are skipped on purpose. Those successors overlap the outgoing track, so the
    // successor's start arrives before the outgoing end and the elapsed-gap comparison would see a
    // large gap where there is none, suppressing frame stats for a minute after every track.
    fn track_loss_window(&self, event: &AudioEvent) {
        match event {
            AudioEvent::TrackStart(_) if self.in_transition.load(Ordering::Acquire) => {}
            AudioEvent::TrackEnd(_, EngineEndReason::Crossfade | EngineEndReason::Gapless) => {}
            AudioEvent::TrackStart(_) | AudioEvent::PlayerResume => self.loss.on_playback_started(),
            AudioEvent::TrackEnd(..) | AudioEvent::PlayerPause => self.loss.on_playback_stopped(),
            _ => {}
        }
    }
}

// Feeds the voice send loop from the engine player while recording frame stats.
struct CountingProvider {
    player: Arc<AudioPlayer>,
    loss: Arc<AudioLossCounter>,
}

impl OpusFrameProvider for CountingProvider {
    fn provide(&mut self) -> Option<Bytes> {
        let frame = self.player.try_provide();
        // Only count while a track is playing: idle silence is expected, not loss.
        if self.player.playing_track().is_some() {
            match &frame {
                Some(_) => self.loss.record_success(),
                None => self.loss.record_loss(),
            }
        }
        frame.map(|frame| frame.data)
    }
}

// Forwards voice gateway lifecycle to the WebSocket: a close becomes a `WebSocketClosedEvent`, and
// both a close and a ready are followed by a player update, so the client learns the new `connected`
// and `ping` right away instead of up to `playerUpdateInterval` seconds later.
struct VoiceCloseForwarder {
    guild_id: u64,
    context: Weak<SocketContext>,
}

impl VoiceEventListener for VoiceCloseForwarder {
    fn on_event(&self, event: &VoiceEvent) {
        let Some(context) = self.context.upgrade() else {
            return;
        };
        match event {
            VoiceEvent::GatewayClosed {
                code,
                reason,
                by_remote,
            } => {
                context.send_message(Message::event(EmittedEvent::WebSocketClosed {
                    guild_id: self.guild_id.to_string(),
                    code: *code as i32,
                    reason: reason.clone(),
                    by_remote: *by_remote,
                }));
            }
            VoiceEvent::GatewayReady { .. } => {}
            // Every other voice event is internal to the connection.
            _ => return,
        }
        // The player may already be gone, with a destroy racing the close.
        if let Some(player) = context.get_player(self.guild_id) {
            context.send_message(player.player_update_message());
        }
    }
}

/// Releases a guild's handshake claim when dropped unless the claim moved on.
///
/// The guard is built before the background task is spawned and moved into it,
/// so every exit releases the claim: success, handshake error, timeout, panic
/// unwind, even an abort before the first poll (dropping the task future drops
/// the guard). A retry with the same target therefore always runs a new
/// handshake instead of seeing a stale claim.
pub struct VoiceClaimGuard {
    player: Arc<LavalinkPlayer>,
    voice: VoiceState,
}

impl VoiceClaimGuard {
    pub fn new(player: Arc<LavalinkPlayer>, voice: VoiceState) -> Self {
        Self { player, voice }
    }
}

impl Drop for VoiceClaimGuard {
    fn drop(&mut self) {
        self.player.clear_voice_claim(&self.voice);
    }
}

/// A guild's player: the engine player, the voice connection, and protocol state.
pub struct LavalinkPlayer {
    guild_id: u64,
    user_id: u64,
    player: Arc<AudioPlayer>,
    shared: Arc<PlayerShared>,
    loss: Arc<AudioLossCounter>,
    voice: Mutex<Option<VoiceConnection>>,
    voice_state: Mutex<Option<VoiceState>>,
    // Target of the handshake currently running in the background, if any. Duplicate
    // VOICE_SERVER_UPDATE PATCHes for the same target skip the gate instead of each
    // running a handshake.
    voice_claim: Mutex<Option<VoiceState>>,
    // Serializes a whole `PATCH .../players/{guildId}` body.
    patch_lock: tokio::sync::Mutex<()>,
    filters: Mutex<Filters>,
    crossfade_defaults: Option<CrossfadeOptions>,
    crossfade: Mutex<Option<CrossfadeOptions>>,
    next_track: Mutex<Option<(Box<dyn AudioTrack>, Track)>>,
    transition_marker: Mutex<Option<MarkerId>>,
    manual_transition_marker: Mutex<Option<MarkerId>>,
    end_marker: Mutex<Option<MarkerId>>,
    // Present only when the lyrics feature is enabled.
    lyrics: Option<Arc<PlayerLyrics>>,
    // The periodic player-update timer, armed on a track start and cancelled on a track end.
    update_task: Mutex<Option<AbortHandle>>,
}

impl LavalinkPlayer {
    /// Create a player for `guild_id` owned by `context`.
    pub fn new(
        guild_id: u64,
        user_id: u64,
        player: AudioPlayer,
        crossfade_defaults: Option<CrossfadeOptions>,
        lyrics_service: Option<Arc<LyricsService>>,
        context: &Arc<SocketContext>,
    ) -> Arc<Self> {
        let player = Arc::new(player);
        let loss = Arc::new(AudioLossCounter::new());
        let shared = Arc::new(PlayerShared {
            guild_id,
            context: Arc::downgrade(context),
            player: Arc::downgrade(&player),
            tracks: Mutex::new(TrackSlots::default()),
            end_marker_hit: AtomicBool::new(false),
            transition_outgoing: Mutex::new(None),
            transition_incoming: Mutex::new(None),
            lyrics: Mutex::new(None),
            in_transition: AtomicBool::new(false),
            loss: Arc::clone(&loss),
            runtime: tokio::runtime::Handle::current(),
        });
        player.add_listener(shared.clone() as Arc<dyn AudioEventListener>);

        // Build the live-lyrics handle and its coordinator task only when the feature is on. It
        // needs the runtime, which holds because players are only created from a request handler.
        let lyrics = lyrics_service.map(|service| {
            Arc::new(PlayerLyrics::new(
                service,
                Arc::downgrade(&player),
                Arc::downgrade(context),
                guild_id,
            ))
        });
        if let Some(lyrics) = &lyrics {
            *shared.lyrics.lock().unwrap() = Some(Arc::clone(lyrics));
        }

        Arc::new(Self {
            guild_id,
            user_id,
            player,
            shared,
            loss,
            voice: Mutex::new(None),
            voice_state: Mutex::new(None),
            patch_lock: tokio::sync::Mutex::new(()),
            filters: Mutex::new(Filters::default()),
            crossfade_defaults,
            crossfade: Mutex::new(crossfade_defaults),
            next_track: Mutex::new(None),
            transition_marker: Mutex::new(None),
            manual_transition_marker: Mutex::new(None),
            end_marker: Mutex::new(None),
            lyrics,
            update_task: Mutex::new(None),
            voice_claim: Mutex::new(None),
        })
    }

    pub fn guild_id(&self) -> u64 {
        self.guild_id
    }

    /// This player's share of the node's frame stats.
    pub fn loss_counter(&self) -> &Arc<AudioLossCounter> {
        &self.loss
    }

    pub fn has_track(&self) -> bool {
        self.shared.current_track().is_some()
    }

    pub fn current_track(&self) -> Option<Track> {
        self.shared.current_track()
    }

    /// Whether the player holds a track and is not paused.
    pub fn is_playing(&self) -> bool {
        self.has_track() && !self.player.is_paused()
    }

    pub fn is_connected(&self) -> bool {
        matches!(
            self.voice.lock().unwrap().as_ref().map(|c| c.state()),
            Some(ConnectionState::Connected)
        )
    }

    /// End the current track with the `cleanup` reason if nothing has pulled a frame from it within
    /// `threshold_ms`. A no-op unless a track is loaded.
    ///
    /// The engine records a request on every frame handed out, so this only fires when no voice send
    /// loop is draining the player, for instance after its connection died without a close event the
    /// client acted on. Without it the track stays active forever: a decode thread and an HTTP
    /// connection leak, no track end is emitted, and the client's queue never advances.
    pub fn check_cleanup(&self, threshold_ms: u64) {
        self.player.check_cleanup(threshold_ms);
    }

    fn position(&self) -> i64 {
        self.player.position().unwrap_or(0) as i64
    }

    /// Start playing `track`, already encoded as `proto`, replacing any current track.
    pub fn play(&self, track: Box<dyn AudioTrack>, proto: Track) {
        self.play_at(track, proto, 0);
    }

    /// Start playing `track` at `position_ms`, replacing any current track.
    ///
    /// Decoding begins at `position_ms`, so no audio from the start of the track leaks out before
    /// the seek takes effect.
    pub fn play_at(&self, track: Box<dyn AudioTrack>, proto: Track, position_ms: u64) {
        self.clear_all_transition_markers();
        self.shared.abort_transition();
        // Set the protocol slots first, since `play_track_at` fires Replaced and Start synchronously.
        self.shared.replace_current(proto);
        self.player.play_track_at(track, position_ms);
        self.arm_transition();
        // Clients expect an update the moment a track starts, then one per interval.
        self.send_player_update();
    }

    /// Replace or clear the successor supplied by the queue-owning client.
    pub fn set_next_track(&self, next: Option<(Box<dyn AudioTrack>, Track)>) {
        let transition_pending = self.player.has_pending_next();
        self.clear_scheduled_transition_marker();
        if !transition_pending {
            self.clear_manual_transition_marker();
        }
        *self.next_track.lock().unwrap() = next;
        self.arm_transition();
    }

    /// Apply a complete per-player override. `None` explicitly disables both transition modes.
    pub fn set_crossfade(&self, options: Option<CrossfadeOptions>) {
        self.clear_all_transition_markers();
        *self.crossfade.lock().unwrap() = options;
        self.arm_transition();
    }

    /// Clear the per-player override and restore the server defaults.
    pub fn reset_crossfade(&self) {
        self.clear_all_transition_markers();
        *self.crossfade.lock().unwrap() = self.crossfade_defaults;
        self.arm_transition();
    }

    /// Set the client-driven tape effect. `None`, the default, makes pause and resume instant.
    pub fn set_tape(&self, options: Option<TapeOptions>) {
        self.player.set_tape(options);
    }

    pub fn stop(&self) {
        self.clear_all_transition_markers();
        self.next_track.lock().unwrap().take();
        self.shared.abort_transition();
        self.player.stop_track();
    }

    /// Seek to `position_ms` and emit a player update.
    pub fn seek(&self, position_ms: i64) {
        let position = position_ms.max(0) as u64;
        self.player.set_position(position);
        // Re-anchor live lyrics, in either direction.
        if let Some(lyrics) = &self.lyrics {
            lyrics.on_seek(position);
        }
        self.send_player_update();
    }

    /// Set the volume (`0..=1000`).
    pub fn set_volume(&self, volume: i32) {
        self.player.set_volume(volume);
    }

    /// Pause or resume playback.
    pub fn set_paused(&self, paused: bool) {
        self.player.set_paused(paused);
    }

    /// Replace the filter chain and emit a player update.
    pub fn set_filters(&self, filters: Filters) {
        self.player.options().set_filters(filters.to_engine());
        *self.filters.lock().unwrap() = filters;
        self.send_player_update();
    }

    /// Attach user data to the current track.
    pub fn set_user_data(&self, user_data: Map<String, Value>) {
        self.shared.set_user_data(user_data);
    }

    /// Set (or clear, with `None`/`0`) the millisecond position at which to stop the track.
    pub fn set_end_time(&self, end_time: Option<i64>) {
        if let Some(id) = self.end_marker.lock().unwrap().take() {
            self.player.remove_marker(id);
        }
        match end_time {
            Some(ms) if ms > 0 => {
                let player = Arc::downgrade(&self.player);
                let shared = Arc::downgrade(&self.shared);
                let handler: player::track::marker::TrackMarkerHandler =
                    Box::new(move |state: MarkerState| {
                        // `Late` and `Bypassed` count too: an `endTime` already behind the play
                        // position still means stop here.
                        if matches!(
                            state,
                            MarkerState::Reached | MarkerState::Late | MarkerState::Bypassed
                        ) {
                            if let (Some(player), Some(shared)) =
                                (player.upgrade(), shared.upgrade())
                            {
                                shared.end_marker_hit.store(true, Ordering::Release);
                                player.stop_track();
                            }
                        }
                    });
                let marker = TrackMarker::new(ms as u64, handler);
                let id = marker.id();
                *self.end_marker.lock().unwrap() = Some(id);
                self.player.add_marker(marker);
            }
            _ => {}
        }
    }

    /// Serialize a player PATCH's mutations against every other PATCH for the same guild.
    ///
    /// Discord repeats `VOICE_SERVER_UPDATE` on a region change and clients PATCH once per event, so
    /// without this two voice handshakes race and whichever finishes last wins the slot: a stale
    /// connection displaces a fresh one and the guild goes silent. The same holds for a play racing
    /// a stop or a seek.
    ///
    /// The caller resolves tracks before taking this, so it is never held across a network load, and
    /// nothing called while the guard is held may re-acquire it.
    pub async fn lock_patch(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.patch_lock.lock().await
    }

    /// Whether `voice` differs from the current state or the connection dropped.
    ///
    /// Cheap synchronous check used before paying for a handshake and again when
    /// committing one. Both checks dedupe repeated `VOICE_SERVER_UPDATE` PATCHes
    /// so an older handshake cannot displace a newer connection.
    pub fn voice_reconnect(&self, voice: &VoiceState) -> bool {
        let current = self.voice_state.lock().unwrap();
        match current.as_ref() {
            Some(existing) => {
                existing.token != voice.token
                    || existing.endpoint != voice.endpoint
                    || existing.session_id != voice.session_id
                    || existing.channel_id != voice.channel_id
                    || !self.is_connected()
            }
            None => true,
        }
    }

    /// Claim this guild's background handshake slot for `voice`.
    ///
    /// Returns `false` when a handshake for the same target is already running:
    /// the duplicate PATCH skips the gate instead of running a second handshake.
    /// A PATCH for a different target overwrites the claim; the older task sees
    /// the mismatch on commit and drops its connection (see `commit_voice`).
    pub fn try_claim_voice(&self, voice: &VoiceState) -> bool {
        let mut claim = self.voice_claim.lock().unwrap_or_else(|e| e.into_inner());
        if claim.as_ref().is_some_and(|current| current == voice) {
            return false;
        }
        *claim = Some(voice.clone());
        true
    }

    /// Whether `voice` still owns this guild's handshake claim.
    pub fn claim_is(&self, voice: &VoiceState) -> bool {
        self.voice_claim
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            == Some(voice)
    }

    /// Release the handshake claim when it still belongs to `voice`.
    pub fn clear_voice_claim(&self, voice: &VoiceState) {
        let mut claim = self.voice_claim.lock().unwrap_or_else(|e| e.into_inner());
        if claim.as_ref() == Some(voice) {
            *claim = None;
        }
    }

    /// Whether a finished handshake for `voice` may be stored: the claim still
    /// belongs to it and the guild still needs it. Split out so the decision
    /// matrix is unit-testable without a live voice connection.
    fn commit_allowed(&self, voice: &VoiceState) -> bool {
        self.claim_is(voice) && self.voice_reconnect(voice)
    }

    /// Run the Discord gateway + UDP handshake. No player locks are held across the await.
    ///
    /// The caller must hold a `voice_gate` permit while calling this. DAVE/MLS setup,
    /// TLS and UDP discovery all happen here; the returned connection is not yet stored.
    pub async fn handshake_voice(&self, voice: &VoiceState) -> Result<VoiceConnection, RestError> {
        let channel_id = voice
            .channel_id
            .as_deref()
            .and_then(|id| id.parse::<u64>().ok())
            .unwrap_or(0);
        let info = VoiceServerInfo {
            guild_id: self.guild_id,
            user_id: self.user_id,
            channel_id,
            session_id: voice.session_id.clone(),
            token: voice.token.clone(),
            endpoint: voice.endpoint.clone(),
        };

        let dispatcher = EventDispatcher::new();
        dispatcher.register(Arc::new(VoiceCloseForwarder {
            guild_id: self.guild_id,
            context: self.shared.context.clone(),
        }) as Arc<dyn VoiceEventListener>);

        let provider = CountingProvider {
            player: Arc::clone(&self.player),
            loss: Arc::clone(&self.loss),
        };

        let started = std::time::Instant::now();
        let result = VoiceConnection::connect_with_dispatcher(info, provider, dispatcher).await;
        let elapsed_ms = started.elapsed().as_millis();
        match result {
            Ok(connection) => {
                tracing::debug!(
                    guild_id = self.guild_id,
                    elapsed_ms,
                    "voice handshake succeeded"
                );
                Ok(connection)
            }
            Err(err) => {
                tracing::warn!(
                    guild_id = self.guild_id,
                    elapsed_ms,
                    error = %err,
                    "voice handshake failed"
                );
                Err(RestError::internal(format!(
                    "voice connection failed: {err}"
                )))
            }
        }
    }

    /// Tell the client a background connect gave up: a `WebSocketClosedEvent`
    /// with abnormal-closure code 1006, so existing clients observe the failure
    /// without any protocol change. The claim releases via the task guard.
    fn emit_connect_failed(&self, attempts: u32, cause: &str) {
        tracing::warn!(
            guild_id = self.guild_id,
            attempts,
            cause,
            "voice background connect failed; emitting close event"
        );
        if let Some(context) = self.shared.context.upgrade() {
            context.send_message(Message::event(EmittedEvent::WebSocketClosed {
                guild_id: self.guild_id.to_string(),
                code: 1006,
                reason: format!("voice handshake failed after {attempts} attempts ({cause})"),
                by_remote: true,
            }));
        }
        // The task guard releases the claim on return.
    }

    /// Store a finished handshake. The caller must hold `lock_patch`.
    ///
    /// Drops `connection` and returns `false` when the claim moved on (a newer PATCH
    /// claimed a different target) or the guild no longer needs this target, so a
    /// stale handshake never displaces a fresh connection.
    pub fn commit_voice(&self, voice: VoiceState, connection: VoiceConnection) -> bool {
        if !self.commit_allowed(&voice) {
            connection.disconnect();
            return false;
        }
        if let Some(previous) = self.voice.lock().unwrap().take() {
            previous.disconnect();
        }
        connection.set_speaking(true);
        *self.voice.lock().unwrap() = Some(connection);
        *self.voice_state.lock().unwrap() = Some(voice);
        true
    }

    /// Maximum handshake attempts per background connect, then a final failure.
    /// Three tries ride out a single blip (refused reset, TLS hiccup) without
    /// turning every flap into a reconnect storm of its own.
    const MAX_HANDSHAKE_ATTEMPTS: u32 = 3;

    /// Drive one background voice connect to completion: gate, up to three
    /// handshakes with exponential backoff, commit under the guild lock, then a
    /// player update on success.
    ///
    /// Takes ownership of a [`VoiceClaimGuard`] built by the spawner: the claim
    /// releases on every exit path, including an abort before the first poll.
    /// The caller must have claimed the guild via [`try_claim_voice`](Self::try_claim_voice)
    /// first. Handshakes come from the `make_handshake` factory (one fresh future
    /// per attempt) so tests can run this without a network connection.
    pub async fn background_connect<Fut>(
        guard: VoiceClaimGuard,
        handshake_timeout: Duration,
        mut make_handshake: impl FnMut() -> Fut + Send,
    ) where
        Fut: std::future::Future<Output = Result<VoiceConnection, RestError>> + Send,
    {
        let player = Arc::clone(&guard.player);
        let voice = guard.voice.clone();
        // Held to the end of the task: every `return` below, a panic unwind and a
        // task abort all drop it, releasing the claim exactly when it is still ours.
        let _guard = guard;
        let mut attempt = 0u32;
        let connection = loop {
            attempt += 1;
            // Scoped so the permit drops before any backoff sleep below: a
            // sleeping retry must not hold a gate slot.
            let result = {
                let _permit = match super::voice_gate::acquire_background().await {
                    Ok(permit) => permit,
                    Err(err) => {
                        tracing::warn!(
                            guild_id = player.guild_id,
                            error = %err.message,
                            "voice background acquire failed"
                        );
                        return;
                    }
                };
                tokio::time::timeout(handshake_timeout, make_handshake()).await
            };
            match result {
                Ok(Ok(connection)) => break connection,
                // `handshake_voice` already warned with the elapsed time.
                Ok(Err(_)) if attempt < Self::MAX_HANDSHAKE_ATTEMPTS => {
                    super::voice_gate::record_retry();
                    backoff_sleep(attempt).await;
                }
                Err(_) if attempt < Self::MAX_HANDSHAKE_ATTEMPTS => {
                    tracing::warn!(
                        guild_id = player.guild_id,
                        timeout_ms = handshake_timeout.as_millis(),
                        attempt,
                        "voice handshake timed out; backing off"
                    );
                    super::voice_gate::record_retry();
                    backoff_sleep(attempt).await;
                }
                Ok(Err(_)) => {
                    super::voice_gate::record_outcome(super::voice_gate::HandshakeOutcome::Error);
                    player.emit_connect_failed(attempt, "handshake error");
                    return;
                }
                Err(_) => {
                    super::voice_gate::record_outcome(super::voice_gate::HandshakeOutcome::Timeout);
                    player.emit_connect_failed(attempt, "handshake timeout");
                    return;
                }
            }
        };
        let _serialized = player.lock_patch().await;
        if player.commit_voice(voice, connection) {
            super::voice_gate::record_outcome(super::voice_gate::HandshakeOutcome::Success);
            player.send_player_update();
        } else {
            super::voice_gate::record_outcome(super::voice_gate::HandshakeOutcome::StaleDropped);
        }
    }

    /// Synchronous fallback for `backgroundConnect: false`: one gated handshake,
    /// committed inline, errors returned to the PATCH caller as before.
    pub async fn connect_voice_sync<Fut>(
        guard: VoiceClaimGuard,
        queue_wait: Duration,
        handshake_timeout: Duration,
        make_handshake: impl FnOnce() -> Fut + Send,
    ) -> Result<bool, RestError>
    where
        Fut: std::future::Future<Output = Result<VoiceConnection, RestError>> + Send,
    {
        let player = Arc::clone(&guard.player);
        let voice = guard.voice.clone();
        let _guard = guard;
        let _permit = super::voice_gate::acquire(queue_wait).await?;
        let connection = match tokio::time::timeout(handshake_timeout, make_handshake()).await {
            Ok(Ok(connection)) => connection,
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                return Err(RestError::internal(format!(
                    "voice handshake timed out after {} ms",
                    handshake_timeout.as_millis()
                )));
            }
        };
        let _serialized = player.lock_patch().await;
        Ok(player.commit_voice(voice, connection))
    }

    pub fn player_update_message(&self) -> Message {
        Message::PlayerUpdate {
            guild_id: self.guild_id.to_string(),
            state: self.current_state(),
        }
    }

    /// Send a player update to the owning context, if it is still alive.
    pub fn send_player_update(&self) {
        if let Some(context) = self.shared.context.upgrade() {
            context.send_message(self.player_update_message());
        }
    }

    // Arm this player's periodic update timer, phased to now.
    //
    // One timer per player, rather than a node-wide ticker: a shared ticker phases every guild to
    // process start, so a track beginning just after a tick waits out the rest of the interval
    // before its first periodic update. The first tick here fires immediately.
    fn start_update_task(self: &Arc<Self>) {
        let interval = Duration::from_secs(UPDATE_INTERVAL_SECS.get().copied().unwrap_or(5));
        // Weak, so a destroyed player is not kept alive by its own timer.
        let weak = Arc::downgrade(self);
        let task = self.shared.runtime.spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match weak.upgrade() {
                    Some(player) => player.send_player_update(),
                    None => return,
                }
            }
        });
        self.swap_update_task(Some(task.abort_handle()));
    }

    fn stop_update_task(&self) {
        self.swap_update_task(None);
    }

    fn swap_update_task(&self, task: Option<AbortHandle>) {
        let previous = std::mem::replace(&mut *self.update_task.lock().unwrap(), task);
        if let Some(previous) = previous {
            previous.abort();
        }
    }

    // The live state, reading connection status and heartbeat ping from the voice connection.
    fn current_state(&self) -> PlayerState {
        let position = self.position();
        let guard = self.voice.lock().unwrap();
        let connected = matches!(
            guard.as_ref().map(|c| c.state()),
            Some(ConnectionState::Connected)
        );
        // `ping` is `-1` while disconnected and before the first heartbeat ack.
        let ping = if connected {
            guard
                .as_ref()
                .and_then(|c| c.ping())
                .map(|ping| ping as i64)
                .unwrap_or(-1)
        } else {
            -1
        };
        PlayerState {
            time: now_millis(),
            position,
            connected,
            ping,
        }
    }

    fn voice_state(&self) -> VoiceState {
        self.voice_state.lock().unwrap().clone().unwrap_or_default()
    }

    /// A full protocol snapshot of this player, as returned by the REST API.
    pub fn snapshot(&self) -> Player {
        let state = self.current_state();
        let track = self.shared.current_track().map(|mut track| {
            track.info.position = state.position;
            track
        });
        Player {
            guild_id: self.guild_id.to_string(),
            track,
            volume: self.player.volume(),
            paused: self.player.is_paused(),
            state,
            voice: self.voice_state(),
            filters: self.filters.lock().unwrap().clone(),
            crossfade: self
                .crossfade
                .lock()
                .unwrap()
                .as_ref()
                .map(CrossfadeSettings::from_engine),
        }
    }

    /// The live-lyrics handle, if the lyrics feature is enabled.
    pub fn lyrics(&self) -> Option<&Arc<PlayerLyrics>> {
        self.lyrics.as_ref()
    }

    /// Subscribe to live lyrics with the given `skipTrackSource` mode, seeding whatever is playing
    /// now. Returns `false` when lyrics are disabled.
    pub fn subscribe_lyrics(&self, skip_track_source: bool) -> bool {
        let Some(lyrics) = &self.lyrics else {
            return false;
        };
        lyrics.subscribe(skip_track_source);
        if let Some(track) = self.shared.current_track() {
            lyrics.seed(query_from_info(&track.info));
        }
        true
    }

    /// Unsubscribe from live lyrics. Returns `false` when lyrics are disabled.
    pub fn unsubscribe_lyrics(&self) -> bool {
        let Some(lyrics) = &self.lyrics else {
            return false;
        };
        lyrics.unsubscribe();
        true
    }

    /// Stop playback and tear the voice connection down.
    pub fn destroy(&self) {
        self.stop_update_task();
        if let Some(lyrics) = &self.lyrics {
            lyrics.unsubscribe();
        }
        self.player.stop_track();
        if let Some(connection) = self.voice.lock().unwrap().take() {
            connection.disconnect();
        }
        let mut slots = self.shared.tracks.lock().unwrap();
        slots.current = None;
        slots.previous = None;
    }
}

/// Sleep between handshake retries: 500 ms doubling per attempt, capped at 5 s.
/// No gate permit is held across the sleep (see `background_connect`).
async fn backoff_sleep(failed_attempt: u32) {
    let ms = 500u64
        .saturating_mul(1u64 << failed_attempt.min(4).saturating_sub(1))
        .min(5_000);
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

mod events;
mod markers;
mod transition;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Barrier;

    // `tokio::time::interval` panics on a zero period, so a `playerUpdateInterval: 0` in the config
    // has to be clamped before it reaches the update task.
    #[test]
    fn update_interval_is_clamped_and_set_once() {
        set_update_interval(0);
        assert_eq!(UPDATE_INTERVAL_SECS.get().copied(), Some(1));
        // A `OnceLock` keeps the first value, so a second call must be a no-op rather than a panic.
        set_update_interval(9);
        assert_eq!(UPDATE_INTERVAL_SECS.get().copied(), Some(1));
    }

    fn test_voice(token: &str) -> VoiceState {
        VoiceState {
            token: token.to_string(),
            endpoint: "e".to_string(),
            session_id: "s".to_string(),
            channel_id: Some("c".to_string()),
        }
    }

    // Needs a runtime: `LavalinkPlayer::new` captures `Handle::current`.
    async fn test_player() -> Arc<LavalinkPlayer> {
        test_player_with_channel().await.0
    }

    async fn test_player_with_channel() -> (
        Arc<LavalinkPlayer>,
        tokio::sync::mpsc::UnboundedReceiver<Message>,
        // Kept alive: the player holds only a `Weak` context, and dropping this
        // would close the channel the failure event is asserted on.
        Arc<SocketContext>,
    ) {
        let manager = player::AudioPlayerManager::new();
        let (sender, rx) = tokio::sync::mpsc::unbounded_channel();
        let context = SocketContext::new(
            "test-session".to_string(),
            1,
            manager.clone(),
            None,
            None,
            sender,
        );
        let engine = manager.create_player();
        (
            LavalinkPlayer::new(7, 1, engine, None, None, &context),
            rx,
            context,
        )
    }

    fn boom() -> Result<VoiceConnection, RestError> {
        Err(RestError::internal("boom"))
    }

    #[tokio::test]
    async fn duplicate_claim_for_same_target_returns_false() {
        let player = test_player().await;
        let voice = test_voice("a");
        assert!(player.try_claim_voice(&voice));
        // Same target while in flight: duplicate, skip the gate.
        assert!(!player.try_claim_voice(&voice));
        // After release a retry with the same target claims anew: a new handshake runs.
        player.clear_voice_claim(&voice);
        assert!(player.try_claim_voice(&voice));
        player.clear_voice_claim(&voice);
    }

    #[tokio::test]
    async fn different_target_overwrites_and_release_keeps_newer() {
        let player = test_player().await;
        let old = test_voice("a");
        let new = test_voice("b");
        assert!(player.try_claim_voice(&old));
        assert!(player.try_claim_voice(&new));
        assert!(player.claim_is(&new));
        // Releasing the stale target must not clear the newer claim.
        player.clear_voice_claim(&old);
        assert!(player.claim_is(&new));
        player.clear_voice_claim(&new);
    }

    #[tokio::test]
    async fn commit_allowed_matrix() {
        let player = test_player().await;
        let voice = test_voice("a");
        let other = test_voice("b");
        // Fresh guild, claim mine: allowed.
        assert!(player.try_claim_voice(&voice));
        assert!(player.commit_allowed(&voice));
        // Claim moved on: stale, not allowed.
        assert!(player.try_claim_voice(&other));
        assert!(!player.commit_allowed(&voice));
        assert!(player.commit_allowed(&other));
        player.clear_voice_claim(&other);
    }

    #[tokio::test]
    async fn claim_guard_clears_on_drop_and_survives_panic() {
        let player = test_player().await;
        let voice = test_voice("a");
        assert!(player.try_claim_voice(&voice));
        {
            let _guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
        }
        assert!(!player.claim_is(&voice));
        // A panic unwind drops the guard the same way.
        assert!(player.try_claim_voice(&voice));
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
            panic!("boom");
        }));
        assert!(caught.is_err());
        assert!(!player.claim_is(&voice));
    }

    #[tokio::test]
    async fn background_connect_clears_claim_on_handshake_error() {
        let player = test_player().await;
        let voice = test_voice("a");
        assert!(player.try_claim_voice(&voice));
        let guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
        LavalinkPlayer::background_connect(guard, Duration::from_secs(5), || async { boom() })
            .await;
        assert!(!player.claim_is(&voice));
        // Retry with the same target claims anew.
        assert!(player.try_claim_voice(&voice));
        player.clear_voice_claim(&voice);
    }

    #[tokio::test]
    async fn background_connect_clears_claim_on_timeout() {
        let player = test_player().await;
        let voice = test_voice("a");
        assert!(player.try_claim_voice(&voice));
        let guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
        LavalinkPlayer::background_connect(guard, Duration::from_millis(50), || {
            std::future::pending()
        })
        .await;
        assert!(!player.claim_is(&voice));
    }

    #[tokio::test]
    async fn background_connect_clears_claim_on_abort() {
        let player = test_player().await;
        let voice = test_voice("a");
        assert!(player.try_claim_voice(&voice));
        // The guard moves into the task future, so even an abort before the
        // first poll drops it and releases the claim.
        let guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
        let handle = tokio::spawn(LavalinkPlayer::background_connect(
            guard,
            Duration::from_secs(30),
            || async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                boom()
            },
        ));
        handle.abort();
        let _ = handle.await;
        assert!(!player.claim_is(&voice));
    }

    // Ten racers, one barrier, gated stub: exactly one handshake may start for
    // the same target. The stub blocks on a watch channel (not a one-shot
    // notify, which a retry would miss) so the winner is still in flight while
    // the losers attempt their claims; it then fails to also prove the single
    // winner retries to three attempts.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn duplicate_patch_racers_run_one_handshake() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::sync::watch;
        static WINNERS: AtomicUsize = AtomicUsize::new(0);
        static CALLS: AtomicUsize = AtomicUsize::new(0);

        let player = test_player().await;
        let voice = test_voice("a");
        let barrier = Arc::new(Barrier::new(10));
        let (release_tx, release_rx) = watch::channel(false);
        let tasks: Vec<_> = (0..10)
            .map(|_| {
                let player = Arc::clone(&player);
                let barrier = Arc::clone(&barrier);
                let release_rx = release_rx.clone();
                let voice = voice.clone();
                tokio::spawn(async move {
                    barrier.wait().await;
                    if player.try_claim_voice(&voice) {
                        WINNERS.fetch_add(1, Ordering::AcqRel);
                        let guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
                        let stub = || {
                            let mut release_rx = release_rx.clone();
                            async move {
                                CALLS.fetch_add(1, Ordering::AcqRel);
                                let _ = release_rx.wait_for(|open| *open).await;
                                boom()
                            }
                        };
                        LavalinkPlayer::background_connect(guard, Duration::from_secs(30), stub)
                            .await;
                    }
                })
            })
            .collect();
        // Wait for the single winner to start, let the losers attempt, then release.
        for _ in 0..500 {
            if WINNERS.load(Ordering::Relaxed) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = release_tx.send(true);
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(WINNERS.load(Ordering::Relaxed), 1);
        // One winner, three attempts (initial plus two retries).
        assert_eq!(CALLS.load(Ordering::Relaxed), 3);
        assert!(!player.claim_is(&voice));
    }

    // Failing stub, three attempts then give up: the factory ran exactly three
    // times, a close event reached the session channel, and the claim cleared so
    // the same target can retry afterwards.
    #[tokio::test]
    async fn background_connect_retries_then_reports_failure() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (player, mut rx, _context) = test_player_with_channel().await;
        let voice = test_voice("a");
        static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

        assert!(player.try_claim_voice(&voice));
        let guard = VoiceClaimGuard::new(Arc::clone(&player), voice.clone());
        LavalinkPlayer::background_connect(guard, Duration::from_secs(5), || async {
            ATTEMPTS.fetch_add(1, Ordering::AcqRel);
            boom()
        })
        .await;

        assert_eq!(ATTEMPTS.load(Ordering::Relaxed), 3);
        assert!(!player.claim_is(&voice));
        // Same target can retry afterwards.
        assert!(player.try_claim_voice(&voice));
        player.clear_voice_claim(&voice);

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("close event arrives")
            .expect("channel open");
        match event {
            Message::Event(boxed) => match *boxed {
                EmittedEvent::WebSocketClosed {
                    code,
                    by_remote,
                    guild_id,
                    ..
                } => {
                    assert_eq!(code, 1006);
                    assert!(by_remote);
                    assert_eq!(guild_id, "7");
                }
                other => panic!("expected WebSocketClosed, got {other:?}"),
            },
            other => panic!("expected event message, got {other:?}"),
        }
    }
}

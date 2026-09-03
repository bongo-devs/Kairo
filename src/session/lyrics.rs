//! Live lyrics for one player: fetch them when a track starts, then emit an event per synced line. All state
//! lives in a per-player coordinator task reachable only by command; two epoch counters invalidate stale work.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use tokio::sync::mpsc::{self, UnboundedSender};

use ::lyrics::{LyricsData, LyricsQuery, LyricsService};
use player::track::marker::{MarkerState, TrackMarker};
use player::AudioPlayer;

use crate::protocol::lyrics::{Line, Lyrics};
use crate::protocol::message::{EmittedEvent, Message};
use crate::protocol::track::TrackInfo;
use crate::session::context::SocketContext;

pub fn query_from_info(info: &TrackInfo) -> LyricsQuery {
    LyricsQuery {
        title: info.title.clone(),
        author: info.author.clone(),
        identifier: info.identifier.clone(),
        source_name: info.source_name.clone(),
        uri: info.uri.clone(),
    }
}

/// The two counters that invalidate stale asynchronous work.
#[derive(Debug, Default)]
pub struct LyricsEpochs {
    // Bumped on every seed. A fetch stamps the value at spawn and its result is dropped if the
    // counter has since moved, meaning the track changed out from under it.
    track: AtomicU64,
    // Bumped on every seek and clear. A line marker stamps the value when armed and its fire is
    // dropped if the counter has since moved, meaning a seek re-anchored the line chain.
    sync: AtomicU64,
}

impl LyricsEpochs {
    fn track(&self) -> u64 {
        self.track.load(Ordering::Acquire)
    }

    fn sync(&self) -> u64 {
        self.sync.load(Ordering::Acquire)
    }

    fn bump_track(&self) -> u64 {
        self.track.fetch_add(1, Ordering::AcqRel) + 1
    }

    fn bump_sync(&self) -> u64 {
        self.sync.fetch_add(1, Ordering::AcqRel) + 1
    }
}

// A command for the per-player coordinator task, which owns all of the lyrics state.
enum LyricsCmd {
    // A fetch finished: store the data when `track_epoch` is still current, emit found or
    // not-found, then anchor the line chain at the current position.
    //
    // Boxed because this is by far the largest variant, and tokio's unbounded channel preallocates
    // a 32-slot block sized to it per player, whether or not anything ever subscribes.
    Loaded {
        track_epoch: u64,
        query_source: String,
        data: Option<Box<LyricsData>>,
    },
    LineFired {
        sync_epoch: u64,
        line_idx: usize,
        skipped: bool,
    },
    // Anchor the line chain to `position_ms` after a seek or a transition promotion.
    Reanchor {
        sync_epoch: u64,
        position_ms: u64,
    },
    Clear,
}

/// Per-player lyrics handle, owned by the [`LavalinkPlayer`](super::player::LavalinkPlayer).
pub struct PlayerLyrics {
    service: Arc<LyricsService>,
    runtime: tokio::runtime::Handle,
    tx: UnboundedSender<LyricsCmd>,
    subscribed: AtomicBool,
    skip_track_source: AtomicBool,
    epochs: Arc<LyricsEpochs>,
}

impl PlayerLyrics {
    /// Create the handle and spawn the coordinator task for `guild_id`.
    ///
    /// Must be called from inside the tokio runtime: the captured handle is what lets a later
    /// decode-thread event spawn the fetch.
    pub fn new(
        service: Arc<LyricsService>,
        player: Weak<AudioPlayer>,
        context: Weak<SocketContext>,
        guild_id: u64,
    ) -> Self {
        let runtime = tokio::runtime::Handle::current();
        let epochs = Arc::new(LyricsEpochs::default());
        let (tx, rx) = mpsc::unbounded_channel();
        runtime.spawn(coordinator(
            rx,
            tx.clone(),
            player,
            context,
            guild_id,
            Arc::clone(&epochs),
        ));
        Self {
            service,
            runtime,
            tx,
            subscribed: AtomicBool::new(false),
            skip_track_source: AtomicBool::new(false),
            epochs,
        }
    }

    pub fn is_subscribed(&self) -> bool {
        self.subscribed.load(Ordering::Acquire)
    }

    /// Subscribe to live lyrics with the given `skipTrackSource` mode. Idempotent.
    pub fn subscribe(&self, skip_track_source: bool) {
        self.skip_track_source
            .store(skip_track_source, Ordering::Release);
        self.subscribed.store(true, Ordering::Release);
    }

    /// Stop emitting, clear the coordinator state, and invalidate armed markers.
    pub fn unsubscribe(&self) {
        self.subscribed.store(false, Ordering::Release);
        self.epochs.bump_sync();
        let _ = self.tx.send(LyricsCmd::Clear);
    }

    /// Clear the state for a real track end while staying subscribed, so the next track seeds again.
    pub fn unsubscribe_keep_flag(&self) {
        self.epochs.bump_track();
        self.epochs.bump_sync();
        let _ = self.tx.send(LyricsCmd::Clear);
    }

    /// Seed lyrics for a freshly started or promoted track. No-op unless subscribed.
    ///
    /// No position is passed in: the anchor is read from the live player once the fetch completes,
    /// since playback moves on during it.
    pub fn seed(&self, query: LyricsQuery) {
        if !self.is_subscribed() {
            return;
        }
        let track_epoch = self.epochs.bump_track();
        self.epochs.bump_sync();
        // A new context: drop any previously anchored lines while the fetch is in flight.
        let _ = self.tx.send(LyricsCmd::Clear);

        let service = Arc::clone(&self.service);
        let skip_source = self.skip_track_source.load(Ordering::Acquire);
        let tx = self.tx.clone();
        self.runtime.spawn(async move {
            let data = if skip_source {
                service.load_lyrics_skip_source(&query).await
            } else {
                service.load_lyrics(&query).await
            };
            let _ = tx.send(LyricsCmd::Loaded {
                track_epoch,
                query_source: query.source_name,
                data: data.map(Box::new),
            });
        });
    }

    /// Re-anchor the line chain after a seek to `position_ms`. No-op unless subscribed.
    pub fn on_seek(&self, position_ms: u64) {
        if !self.is_subscribed() {
            return;
        }
        let sync_epoch = self.epochs.bump_sync();
        let _ = self.tx.send(LyricsCmd::Reanchor {
            sync_epoch,
            position_ms,
        });
    }
}

// State owned by the coordinator task alone, so it needs no synchronization.
struct CoordinatorState {
    data: Option<LyricsData>,
    // The source name the lyrics were resolved for, echoed in the found event.
    source_name: String,
    next_marker: Option<player::track::marker::MarkerId>,
    // The last line index emitted, `-1` for none yet, so re-anchoring does not re-emit it.
    last_line_idx: i64,
}

impl CoordinatorState {
    fn new() -> Self {
        Self {
            data: None,
            source_name: String::new(),
            next_marker: None,
            last_line_idx: -1,
        }
    }

    fn lines(&self) -> Option<&Vec<::lyrics::LyricsLine>> {
        self.data.as_ref().and_then(|d| d.lines.as_ref())
    }
}

// The state machine for one player's live lyrics, running until the player or the context is gone.
async fn coordinator(
    mut rx: mpsc::UnboundedReceiver<LyricsCmd>,
    tx: UnboundedSender<LyricsCmd>,
    player: Weak<AudioPlayer>,
    context: Weak<SocketContext>,
    guild_id: u64,
    epochs: Arc<LyricsEpochs>,
) {
    let mut state = CoordinatorState::new();
    let guild = guild_id.to_string();

    while let Some(cmd) = rx.recv().await {
        let Some(context) = context.upgrade() else {
            break;
        };

        match cmd {
            LyricsCmd::Loaded {
                track_epoch,
                query_source,
                data,
            } => {
                // A newer seed superseded this fetch.
                if track_epoch != epochs.track() {
                    continue;
                }
                remove_marker(&player, state.next_marker.take());
                state.last_line_idx = -1;
                state.source_name = query_source;

                match data {
                    Some(data) => {
                        let has_lines = data.lines.as_ref().is_some_and(|l| !l.is_empty());
                        // Build the payload from the borrow, then move `data` into state without a
                        // clone; `anchor` reads the stored lines back through `state.lines()`.
                        let lyrics = Lyrics::from_data(&data);
                        state.data = Some(*data);
                        context.send_message(Message::event(EmittedEvent::LyricsFound {
                            guild_id: guild.clone(),
                            lyrics,
                        }));
                        if has_lines {
                            let position = player_position(&player);
                            anchor(
                                &mut state, &player, &context, &guild, &tx, &epochs, position,
                            );
                        }
                    }
                    None => {
                        state.data = None;
                        context.send_message(Message::event(EmittedEvent::LyricsNotFound {
                            guild_id: guild.clone(),
                        }));
                    }
                }
            }
            LyricsCmd::LineFired {
                sync_epoch,
                line_idx,
                skipped,
            } => {
                // A seek re-anchored the chain after this marker was armed, so its fire is stale.
                if sync_epoch != epochs.sync() {
                    continue;
                }
                emit_line(&mut state, &context, &guild, line_idx, skipped);
                arm_next(&mut state, &player, &tx, &epochs, line_idx + 1);
            }
            LyricsCmd::Reanchor {
                sync_epoch,
                position_ms,
            } => {
                // Superseded by a newer seek.
                if sync_epoch != epochs.sync() {
                    continue;
                }
                if state.lines().is_some() {
                    anchor(
                        &mut state,
                        &player,
                        &context,
                        &guild,
                        &tx,
                        &epochs,
                        position_ms,
                    );
                }
            }
            LyricsCmd::Clear => {
                remove_marker(&player, state.next_marker.take());
                state.data = None;
                state.last_line_idx = -1;
            }
        }
    }
}

fn player_position(player: &Weak<AudioPlayer>) -> u64 {
    player.upgrade().and_then(|p| p.position()).unwrap_or(0)
}

fn remove_marker(player: &Weak<AudioPlayer>, id: Option<player::track::marker::MarkerId>) {
    if let (Some(id), Some(player)) = (id, player.upgrade()) {
        player.remove_marker(id);
    }
}

fn emit_line(
    state: &mut CoordinatorState,
    context: &Arc<SocketContext>,
    guild: &str,
    line_idx: usize,
    skipped: bool,
) {
    let Some(lines) = state.lines() else {
        return;
    };
    let Some(line) = lines.get(line_idx) else {
        return;
    };
    context.send_message(Message::event(EmittedEvent::LyricsLine {
        guild_id: guild.to_string(),
        line_index: line_idx as i32,
        line: Line::from_engine(line),
        skipped,
    }));
    state.last_line_idx = line_idx as i64;
}

// Arm a marker for the line at `next_idx`, stamped with the current sync epoch.
fn arm_next(
    state: &mut CoordinatorState,
    player: &Weak<AudioPlayer>,
    tx: &UnboundedSender<LyricsCmd>,
    epochs: &Arc<LyricsEpochs>,
    next_idx: usize,
) {
    let Some(lines) = state.lines() else {
        state.next_marker = None;
        return;
    };
    let Some(line) = lines.get(next_idx) else {
        state.next_marker = None;
        return;
    };
    let Some(player) = player.upgrade() else {
        state.next_marker = None;
        return;
    };

    let sync_epoch = epochs.sync();
    let timecode = line.timestamp;
    let tx = tx.clone();
    let handler: player::track::marker::TrackMarkerHandler = Box::new(move |marker_state| {
        // A reach, a seek bypass and a late add all mean the line is current now; every other state
        // is a cancellation. This runs under the engine's marker lock, so it may only send.
        let skipped = match marker_state {
            MarkerState::Reached => false,
            MarkerState::Bypassed | MarkerState::Late => true,
            _ => return,
        };
        let _ = tx.send(LyricsCmd::LineFired {
            sync_epoch,
            line_idx: next_idx,
            skipped,
        });
    });

    // `add_marker` may fire the handler synchronously with `Late` when the line is already past;
    // that only sends on the channel, so there is no re-entrant lock on the marker list.
    state.next_marker = player.add_marker(TrackMarker::new(timecode, handler));
}

// Anchor the line chain at `position_ms`: emit the last line at or before it when that line changed,
// then arm the marker for the one after it.
fn anchor(
    state: &mut CoordinatorState,
    player: &Weak<AudioPlayer>,
    context: &Arc<SocketContext>,
    guild: &str,
    tx: &UnboundedSender<LyricsCmd>,
    epochs: &Arc<LyricsEpochs>,
    position_ms: u64,
) {
    remove_marker(player, state.next_marker.take());

    let target = match state.lines() {
        Some(lines) => lines
            .iter()
            .rposition(|line| position_ms >= line.timestamp)
            .map(|i| i as i64)
            .unwrap_or(-1),
        None => return,
    };

    if target >= 0 && target != state.last_line_idx {
        // Emit in both directions, so a backward seek resurfaces the earlier line.
        emit_line(state, context, guild, target as usize, false);
    } else {
        state.last_line_idx = target;
    }

    arm_next(state, player, tx, epochs, (target + 1) as usize);
}

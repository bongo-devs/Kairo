//! The state behind one WebSocket connection.
//!
//! A context owns the connection's players, its outbound message channel and its resume state.
//! While the socket is live, messages go straight to the write task; while the session is paused,
//! meaning the socket dropped but is resumable, they are queued and replayed on resume.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;

use lyrics::LyricsService;
use player::AudioPlayerManager;
use player::CrossfadeOptions;

use crate::protocol::message::Message;
use crate::session::player::LavalinkPlayer;

// Lock a session mutex, recovering from poison instead of propagating the panic. One player thread
// panicking while holding a lock must not take the whole session's event delivery down with it.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// Where outbound messages currently go.
enum Outbound {
    // A live socket: send straight to the write task.
    Live(UnboundedSender<Message>),
    // Resumable: queue until the client reconnects. Unbounded like upstream:
    // playback continues while parked, so any cap drops real track events on a
    // long gap exactly when the client needs them to catch up.
    Queued {
        // The pending replay, oldest first.
        queue: VecDeque<Message>,
    },
    // Permanently closed: drop messages.
    Closed,
}

/// Per-connection state for one session.
pub struct SocketContext {
    /// The session id, as used in REST paths and the `ready` message.
    pub session_id: String,
    /// The bot user id from the `User-Id` handshake header.
    pub user_id: u64,
    manager: AudioPlayerManager,
    crossfade_defaults: Option<CrossfadeOptions>,
    // Shared with every player for live-lyrics subscriptions, `None` when lyrics are disabled.
    lyrics_service: Option<Arc<LyricsService>>,
    players: Mutex<HashMap<u64, Arc<LavalinkPlayer>>>,
    outbound: Mutex<Outbound>,
    resuming: AtomicBool,
    resume_timeout_secs: AtomicU64,
    session_paused: AtomicBool,
    // The pending resume-expiry timer. Without cancelling it on resume, a timer armed by an earlier
    // disconnect outlives its resume and expires the next resume window early.
    resume_timeout_task: Mutex<Option<AbortHandle>>,
    // Bumped whenever a socket takes over this session's outbound channel. A connection whose socket
    // died while the client already reconnected sees a stale epoch and must not park or destroy the
    // session the live socket now owns.
    connection_epoch: AtomicU64,
    sponsorblock: Mutex<HashMap<u64, HashSet<String>>>,
}

impl SocketContext {
    pub fn new(
        session_id: String,
        user_id: u64,
        manager: AudioPlayerManager,
        crossfade_defaults: Option<CrossfadeOptions>,
        lyrics_service: Option<Arc<LyricsService>>,
        sender: UnboundedSender<Message>,
    ) -> Arc<Self> {
        Arc::new(Self {
            session_id,
            user_id,
            manager,
            crossfade_defaults,
            lyrics_service,
            players: Mutex::new(HashMap::new()),
            outbound: Mutex::new(Outbound::Live(sender)),
            resuming: AtomicBool::new(false),
            resume_timeout_secs: AtomicU64::new(60),
            session_paused: AtomicBool::new(false),
            resume_timeout_task: Mutex::new(None),
            connection_epoch: AtomicU64::new(0),
            sponsorblock: Mutex::new(HashMap::new()),
        })
    }

    /// The epoch of the socket that currently owns this session's outbound channel.
    pub fn connection_epoch(&self) -> u64 {
        self.connection_epoch.load(Ordering::Acquire)
    }

    /// Send a message to the client, or queue it while the session is paused.
    pub fn send_message(&self, message: Message) {
        let mut outbound = locked(&self.outbound);
        match &mut *outbound {
            Outbound::Live(sender) => {
                // A send error means the write task is gone; the socket close handler will follow.
                let _ = sender.send(message);
            }

            Outbound::Queued { queue } => queue.push_back(message),
            Outbound::Closed => {}
        }
    }

    /// Get the player for `guild_id`, creating it on the first request for that guild.
    pub fn get_or_create_player(self: &Arc<Self>, guild_id: u64) -> Arc<LavalinkPlayer> {
        let mut players = locked(&self.players);
        if let Some(player) = players.get(&guild_id) {
            return Arc::clone(player);
        }
        let engine = self.manager.create_player();
        let player = LavalinkPlayer::new(
            guild_id,
            self.user_id,
            engine,
            self.crossfade_defaults,
            self.lyrics_service.clone(),
            self,
        );
        players.insert(guild_id, Arc::clone(&player));
        player
    }

    pub fn get_player(&self, guild_id: u64) -> Option<Arc<LavalinkPlayer>> {
        locked(&self.players).get(&guild_id).cloned()
    }

    /// Remove and destroy the player for `guild_id`. Returns whether one existed.
    pub fn remove_player(&self, guild_id: u64) -> bool {
        locked(&self.sponsorblock).remove(&guild_id);
        let player = locked(&self.players).remove(&guild_id);
        if let Some(player) = player {
            player.destroy();
            true
        } else {
            false
        }
    }

    pub fn players(&self) -> Vec<Arc<LavalinkPlayer>> {
        locked(&self.players).values().cloned().collect()
    }

    pub fn player_count(&self) -> usize {
        locked(&self.players).len()
    }

    /// The number of players that hold a track and are not paused.
    pub fn playing_player_count(&self) -> usize {
        self.players
            .lock()
            .unwrap()
            .values()
            .filter(|p| p.is_playing())
            .count()
    }

    /// Whether the client asked for this session to survive a dropped socket.
    pub fn is_resuming(&self) -> bool {
        self.resuming.load(Ordering::Acquire)
    }

    /// How long a dropped socket may stay resumable, in seconds.
    pub fn resume_timeout_secs(&self) -> u64 {
        self.resume_timeout_secs.load(Ordering::Acquire)
    }

    pub fn set_resuming(&self, resuming: bool) {
        self.resuming.store(resuming, Ordering::Release);
    }

    pub fn set_resume_timeout_secs(&self, timeout: u64) {
        self.resume_timeout_secs.store(timeout, Ordering::Release);
    }

    /// Whether the socket dropped and the session is waiting to be resumed.
    pub fn is_paused(&self) -> bool {
        self.session_paused.load(Ordering::Acquire)
    }

    /// Pause the session: queue outbound messages until a resume (or timeout).
    pub fn pause(&self) {
        self.session_paused.store(true, Ordering::Release);
        let mut outbound = locked(&self.outbound);
        *outbound = Outbound::Queued {
            queue: VecDeque::new(),
        };
    }

    /// Arm the resume-expiry timer, cancelling any timer left over from an earlier disconnect.
    pub fn arm_resume_timeout(&self, handle: AbortHandle) {
        if let Some(previous) = locked(&self.resume_timeout_task).replace(handle) {
            previous.abort();
        }
    }

    /// Cancel the pending resume-expiry timer.
    pub fn stop_resume_timeout(&self) {
        if let Some(handle) = locked(&self.resume_timeout_task).take() {
            handle.abort();
        }
    }

    /// Resume the session onto a fresh outbound channel, replaying any queued messages.
    ///
    /// Returns the new connection epoch. The caller is expected to have already pushed `ready` into
    /// `sender`, which the client has to see before the replay.
    pub fn resume_with(&self, sender: UnboundedSender<Message>) -> u64 {
        self.stop_resume_timeout();
        let mut outbound = locked(&self.outbound);
        let queued = match std::mem::replace(&mut *outbound, Outbound::Live(sender.clone())) {
            Outbound::Queued { queue } => queue,
            _ => VecDeque::new(),
        };
        for message in queued {
            let _ = sender.send(message);
        }
        drop(outbound);
        self.session_paused.store(false, Ordering::Release);
        self.connection_epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Replace the outbound channel for a still-live session (a reconnect without resume state).
    ///
    /// Returns the new connection epoch, which invalidates the previous socket's teardown.
    pub fn attach_sender(&self, sender: UnboundedSender<Message>) -> u64 {
        self.stop_resume_timeout();
        *locked(&self.outbound) = Outbound::Live(sender);
        self.session_paused.store(false, Ordering::Release);
        self.connection_epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// The SponsorBlock categories to skip for a guild.
    pub fn get_sponsorblock_categories(&self, guild_id: u64) -> Option<HashSet<String>> {
        locked(&self.sponsorblock).get(&guild_id).cloned()
    }

    pub fn set_sponsorblock_categories(&self, guild_id: u64, categories: HashSet<String>) {
        self.sponsorblock
            .lock()
            .unwrap()
            .insert(guild_id, categories);
    }

    pub fn remove_sponsorblock_categories(&self, guild_id: u64) {
        locked(&self.sponsorblock).remove(&guild_id);
    }

    /// Permanently shut down: destroy all players and stop accepting messages.
    pub fn shutdown(&self) {
        self.stop_resume_timeout();
        crate::node::tasks::TASKS.remove(&crate::node::tasks::session_stats(&self.session_id));
        *locked(&self.outbound) = Outbound::Closed;
        let players: Vec<_> = self
            .players
            .lock()
            .unwrap()
            .drain()
            .map(|(_, p)| p)
            .collect();
        for player in players {
            player.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn context(sender: UnboundedSender<Message>) -> Arc<SocketContext> {
        SocketContext::new(
            "session".to_string(),
            1,
            AudioPlayerManager::new(),
            None,
            None,
            sender,
        )
    }

    // A parked session keeps every event like upstream: no cap, no dropping, so a
    // long gap replays in full order instead of losing the oldest track events.
    #[test]
    fn resume_queue_keeps_everything_in_order() {
        let (dead, mut dead_rx) = mpsc::unbounded_channel();
        let context = context(dead);
        context.pause();
        for n in 0..5_000 {
            // `session_id` is just a cheap sequence marker here.
            context.send_message(Message::Ready {
                resumed: false,
                session_id: n.to_string(),
            });
        }

        let (fresh, mut fresh_rx) = mpsc::unbounded_channel();
        context.resume_with(fresh);

        let mut replayed = Vec::new();
        while let Ok(message) = fresh_rx.try_recv() {
            match message {
                Message::Ready { session_id, .. } => replayed.push(session_id),
                other => panic!("unexpected replay: {other:?}"),
            }
        }
        assert_eq!(replayed.len(), 5_000);
        assert_eq!(replayed[0], "0");
        assert_eq!(replayed[4_999], "4999");
        assert!(
            dead_rx.try_recv().is_err(),
            "nothing reaches the dead socket"
        );
    }

    // A paused session queues player updates like upstream instead of dropping
    // them: the replay carries the whole backlog, then fresh state follows.
    #[test]
    fn paused_session_queues_player_updates_with_events() {
        let (dead, _dead_rx) = mpsc::unbounded_channel();
        let context = context(dead);
        context.pause();
        context.send_message(Message::PlayerUpdate {
            guild_id: "1".to_string(),
            state: crate::protocol::player::PlayerState {
                time: 1,
                position: 2,
                connected: true,
                ping: 3,
            },
        });
        context.send_message(Message::Ready {
            resumed: false,
            session_id: "kept".to_string(),
        });

        let (fresh, mut fresh_rx) = mpsc::unbounded_channel();
        context.resume_with(fresh);

        match fresh_rx.try_recv() {
            Ok(Message::PlayerUpdate { guild_id, .. }) => assert_eq!(guild_id, "1"),
            other => panic!("expected the queued update first, got {other:?}"),
        }
        match fresh_rx.try_recv() {
            Ok(Message::Ready { session_id, .. }) => assert_eq!(session_id, "kept"),
            other => panic!("expected the queued event second, got {other:?}"),
        }
        assert!(fresh_rx.try_recv().is_err(), "nothing else was queued");
    }
}

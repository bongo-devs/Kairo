//! The registry of active sessions and of sessions waiting to be resumed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rand::Rng;

use crate::session::context::SocketContext;

const SESSION_ID_LEN: usize = 16;
const SESSION_ID_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";

/// Lock order: never hold both maps at once. Every method takes one guard, drops it, then takes the
/// other if it still needs to.
#[derive(Default)]
pub struct SocketServer {
    sessions: Mutex<HashMap<String, Arc<SocketContext>>>,
    resumable: Mutex<HashMap<String, Arc<SocketContext>>>,
}

impl SocketServer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn generate_session_id(&self) -> String {
        let mut rng = rand::thread_rng();
        loop {
            let id: String = (0..SESSION_ID_LEN)
                .map(|_| SESSION_ID_CHARS[rng.gen_range(0..SESSION_ID_CHARS.len())] as char)
                .collect();
            // The id is not reserved atomically, so two generators could in principle settle on the
            // same one. Reserving it would mean holding both maps at once.
            if !self.is_attachable(&id) {
                return id;
            }
        }
    }

    pub fn insert(&self, context: Arc<SocketContext>) {
        self.sessions
            .lock()
            .unwrap()
            .insert(context.session_id.clone(), context);
    }

    pub fn get(&self, session_id: &str) -> Option<Arc<SocketContext>> {
        self.sessions.lock().unwrap().get(session_id).cloned()
    }

    pub fn remove(&self, session_id: &str) -> Option<Arc<SocketContext>> {
        self.sessions.lock().unwrap().remove(session_id)
    }

    pub fn move_to_resumable(&self, session_id: &str) -> Option<Arc<SocketContext>> {
        let context = self.sessions.lock().unwrap().remove(session_id)?;
        self.resumable
            .lock()
            .unwrap()
            .insert(session_id.to_string(), Arc::clone(&context));
        Some(context)
    }

    pub fn take_resumable(&self, session_id: &str) -> Option<Arc<SocketContext>> {
        let context = self.resumable.lock().unwrap().remove(session_id)?;
        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.to_string(), Arc::clone(&context));
        Some(context)
    }

    pub fn drop_resumable(&self, session_id: &str) -> Option<Arc<SocketContext>> {
        self.resumable.lock().unwrap().remove(session_id)
    }

    /// Whether a handshake may claim `session_id`, live or resumable.
    pub fn is_attachable(&self, session_id: &str) -> bool {
        let resumable = self.resumable.lock().unwrap().contains_key(session_id);
        if resumable {
            return true;
        }
        self.sessions.lock().unwrap().contains_key(session_id)
    }

    pub fn active_contexts(&self) -> Vec<Arc<SocketContext>> {
        self.sessions.lock().unwrap().values().cloned().collect()
    }

    pub fn total_players(&self) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .map(|c| c.player_count())
            .sum()
    }

    /// Of those, how many hold a track and are not paused.
    pub fn total_playing_players(&self) -> usize {
        self.sessions
            .lock()
            .unwrap()
            .values()
            .map(|c| c.playing_player_count())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_stay_in_the_alphabet() {
        let server = SocketServer::new();
        for _ in 0..64 {
            let id = server.generate_session_id();
            assert_eq!(id.len(), SESSION_ID_LEN);
            assert!(
                id.bytes().all(|b| SESSION_ID_CHARS.contains(&b)),
                "{id} escapes a-z0-9"
            );
        }
    }

    // Generating ids while another thread probes for attachability must not deadlock: the two paths
    // touch both maps, so they have to agree on never holding them together.
    #[test]
    fn concurrent_generate_and_probe_do_not_deadlock() {
        let server = Arc::new(SocketServer::new());
        let probe = {
            let server = Arc::clone(&server);
            std::thread::spawn(move || {
                for _ in 0..5_000 {
                    server.is_attachable("whoever");
                }
            })
        };
        for _ in 0..5_000 {
            server.generate_session_id();
        }
        probe.join().expect("probe thread panicked");
    }
}

//! Client sessions: the socket a client holds, the players it owns, and their events.

pub mod context;
pub mod loss_counter;
pub mod lyrics;
pub mod manager;
pub mod player;

pub use context::SocketContext;
pub use loss_counter::AudioLossCounter;
pub use manager::SocketServer;
pub use player::LavalinkPlayer;

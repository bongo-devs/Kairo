//! The node's own moving parts: shared state, the stats it reports, and its background tasks.

pub mod state;
pub mod stats;
pub mod tasks;

pub use state::AppState;

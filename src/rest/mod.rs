//! The HTTP API: the router, its auth middleware, and one module per group of endpoints.

pub mod error;
pub mod info;
pub mod lyrics;
pub mod metrics;
pub mod players;
pub mod request_log;
pub mod routeplanner;
pub mod sessions;
pub mod sponsorblock;
pub mod track_loading;

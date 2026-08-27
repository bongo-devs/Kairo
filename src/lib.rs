//! A Lavalink v4 compatible audio node.
//!
//! The crate is the node's control plane: [`rest`] serves the HTTP API and the WebSocket, [`session`]
//! owns the players behind it, [`node`] holds the state they share, and [`config`] reads the
//! `application.yml` all three are configured from. Audio decoding and voice transport live in the
//! `player` and `voice` crates.

pub mod config;
pub mod node;
pub mod protocol;
pub mod rest;
pub mod routeplanner;
pub mod session;
pub mod sponsorblock;
pub mod telemetry;

use std::sync::LazyLock;

use mimalloc::MiMalloc;

pub use config::Config;

#[global_allocator]
static ALLOCATOR: MiMalloc = MiMalloc;

/// The configuration, read from disk on first use.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::new);

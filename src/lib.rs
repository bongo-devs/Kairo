//! A Lavalink v4 compatible audio node: this crate is the control plane (HTTP API, WebSocket,
//! players, config), while audio decoding and voice transport live in the `player`/`voice` crates.

pub mod config;
pub mod node;
pub mod protocol;
pub mod rest;
pub mod routeplanner;
pub mod session;
pub mod sponsorblock;
pub mod utils;

use std::sync::LazyLock;

use mimalloc::MiMalloc;

pub use config::Config;

#[global_allocator]
static ALLOCATOR: MiMalloc = MiMalloc;

/// The configuration, read from disk on first use.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::new);

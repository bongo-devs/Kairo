pub mod config;
pub mod node;
pub mod protocol;
pub mod rest;
pub mod routeplanner;
pub mod sponsorblock;
pub mod telemetry;

use std::sync::LazyLock;

use mimalloc::MiMalloc;

pub use config::Config;

#[global_allocator]
static ALLOCATOR: MiMalloc = MiMalloc;

/// The configuration, read from disk on first use.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::new);

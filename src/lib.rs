pub mod config;
pub mod protocol;
pub mod sponsorblock;

use std::sync::LazyLock;

use mimalloc::MiMalloc;

pub use config::Config;

#[global_allocator]
static ALLOCATOR: MiMalloc = MiMalloc;

/// The configuration, read from disk on first use.
pub static CONFIG: LazyLock<Config> = LazyLock::new(Config::new);

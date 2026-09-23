//! Startup odds and ends: logging, the console banner, and the update check.

pub(crate) mod ansi;
pub mod banner;
mod logging;
pub mod update;

pub use logging::init;

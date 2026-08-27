//! Logging and error reporting, both installed once at startup.

mod logging;
mod sentry;

pub use logging::init;
pub use sentry::init_sentry;

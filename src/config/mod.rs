//! Configuration parsed from `application.yml` at startup.

mod filters;
mod lavalink;
mod logging;
mod metrics;
mod sentry;
mod server;

pub use filters::FiltersToggleConfig;
pub use lavalink::{
    HttpConfig, LavalinkConfig, LavalinkServerConfig, RatelimitConfig, RatelimitStrategy,
    ResamplingQuality,
};
pub use logging::{LogFileConfig, LogFormat, LogRotation, LoggingConfig, RequestLoggingConfig};
pub use metrics::{MetricsConfig, PrometheusConfig};
pub use sentry::SentryConfig;
pub use server::{Http2Config, ServerConfig};

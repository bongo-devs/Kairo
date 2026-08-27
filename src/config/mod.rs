//! Configuration parsed from `application.yml` at startup.

mod crossfade;
mod filters;
mod lavalink;
mod logging;
mod lyrics;
mod metrics;
mod sentry;
mod server;

pub use crossfade::{CrossfadeConfig, CrossfadeCurve};
pub use filters::FiltersToggleConfig;
pub use lavalink::{
    HttpConfig, LavalinkConfig, LavalinkServerConfig, RatelimitConfig, RatelimitStrategy,
    ResamplingQuality,
};
pub use logging::{LogFileConfig, LogFormat, LogRotation, LoggingConfig, RequestLoggingConfig};
pub use lyrics::LyricsServerConfig;
pub use metrics::{MetricsConfig, PrometheusConfig};
pub use sentry::SentryConfig;
pub use server::{Http2Config, ServerConfig};

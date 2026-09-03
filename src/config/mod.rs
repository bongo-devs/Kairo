//! Configuration parsed from `application.yml` at startup.

mod crossfade;
mod filters;
mod lavalink;
mod logging;
mod lyrics;
mod metrics;
mod server;

pub use ::sources::SourcesConfig;
pub use crossfade::{CrossfadeConfig, CrossfadeCurve};
pub use filters::FiltersToggleConfig;
pub use lavalink::{
    HttpConfig, LavalinkConfig, LavalinkServerConfig, RatelimitConfig, RatelimitStrategy,
    ResamplingQuality,
};
pub use logging::{LogFileConfig, LogFormat, LogRotation, LoggingConfig, RequestLoggingConfig};
pub use lyrics::LyricsServerConfig;
pub use metrics::{MetricsConfig, PrometheusConfig};
pub use server::{Http2Config, ServerConfig};

use serde::Deserialize;

use player::AudioConfiguration;

/// The whole of `application.yml`. Every block is optional and falls back to its own defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub server: ServerConfig,
    pub lavalink: LavalinkConfig,
    pub logging: LoggingConfig,
    pub sources: SourcesConfig,
    pub crossfade: CrossfadeConfig,
    pub lyrics: LyricsServerConfig,
    pub metrics: MetricsConfig,
}

impl Config {
    /// Read the file named by the first argument, then `KAIRO_CONFIG`, then `application.yml`.
    ///
    /// Panics if the file cannot be read or parsed, since there is nothing to serve without it.
    pub fn new() -> Self {
        let path = std::env::args()
            .nth(1)
            .or_else(|| std::env::var("KAIRO_CONFIG").ok())
            .unwrap_or_else(|| "application.yml".to_string());

        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read config '{}': {}", path, e));

        serde_yaml::from_str::<Config>(&content)
            .unwrap_or_else(|e| panic!("Failed to parse config '{}': {}", path, e))
    }

    pub fn audio_configuration(&self) -> AudioConfiguration {
        let server = &self.lavalink.server;
        let mut config = AudioConfiguration {
            resampling_quality: server.resampling_quality.to_engine(),
            track_stuck_threshold_ms: server.track_stuck_threshold_ms,
            ..AudioConfiguration::default()
        };
        config.set_opus_encoding_quality(server.opus_encoding_quality);
        config.set_opus_bitrate(server.opus_bitrate);
        config
    }
}

//! Configuration parsed from `config.toml` at startup.

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
    ResamplingQuality, VoiceConfig,
};
pub use logging::{LogFileConfig, LogFormat, LogRotation, LoggingConfig, RequestLoggingConfig};
pub use lyrics::LyricsServerConfig;
pub use metrics::{MetricsConfig, PrometheusConfig};
pub use server::{Http2Config, ServerConfig};

use serde::Deserialize;

use player::AudioConfiguration;

const DEFAULT_CONFIG: &str = "config.toml";
// Top-level keys the config recognises; anything else is a likely typo. Kept in step with the
// fields of `Config` below.
const KNOWN_SECTIONS: &[&str] = &[
    "server",
    "lavalink",
    "logging",
    "sources",
    "crossfade",
    "lyrics",
    "metrics",
];

// Warn, before logging is up, about unrecognised top-level sections. Serde drops unknown keys
// rather than failing, so this is the only hint a typo'd section was ignored.
fn warn_unknown_sections<'a>(keys: impl Iterator<Item = &'a str>) {
    for key in keys {
        if !KNOWN_SECTIONS.contains(&key) {
            eprintln!("warning: unknown config section '{key}' ignored");
        }
    }
}

/// The whole config file. Every block is optional and falls back to its own defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The HTTP listener, `server.*`.
    pub server: ServerConfig,
    /// Node settings, `lavalink.*`.
    pub lavalink: LavalinkConfig,
    /// This server's own logging, `logging.*`.
    pub logging: LoggingConfig,
    /// Every source toggle and per-source setting, `sources.*`.
    pub sources: SourcesConfig,
    /// Track transition defaults, `crossfade.*`.
    pub crossfade: CrossfadeConfig,
    /// Lyrics providers, `lyrics.*`.
    pub lyrics: LyricsServerConfig,
    /// Prometheus metrics, `metrics.*`.
    pub metrics: MetricsConfig,
}

impl Config {
    /// Read the file named by the first argument, then `KAIRO_CONFIG`, then `config.toml`.
    ///
    /// Panics if the file cannot be read or parsed, since there is nothing to serve without it.
    pub fn new() -> Self {
        let path = std::env::args()
            .nth(1)
            .or_else(|| std::env::var("KAIRO_CONFIG").ok())
            .unwrap_or_else(|| DEFAULT_CONFIG.to_string());
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read config '{}': {}", path, e));
        Self::from_toml(&content)
            .unwrap_or_else(|e| panic!("Failed to parse config '{}': {}", path, e))
    }

    fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        if let Ok(table) = toml::from_str::<toml::Table>(content) {
            warn_unknown_sections(table.keys().map(String::as_str));
        }
        toml::from_str(content)
    }

    /// Build the engine-wide [`AudioConfiguration`] from these settings.
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

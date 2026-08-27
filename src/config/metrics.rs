//! `metrics.*`, Prometheus metrics settings.

use serde::Deserialize;

/// The `metrics` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    /// The `metrics.prometheus` block.
    pub prometheus: PrometheusConfig,
}

/// `metrics.prometheus.*`, the scrape endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PrometheusConfig {
    /// Whether the metrics endpoint is served at all.
    pub enabled: bool,
    /// The endpoint path, `/metrics` by default.
    pub endpoint: String,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "/metrics".to_string(),
        }
    }
}

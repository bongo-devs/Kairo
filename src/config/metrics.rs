//! `metrics.*`, Prometheus metrics settings.

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct MetricsConfig {
    pub prometheus: PrometheusConfig,
}

/// `metrics.prometheus.*`, the scrape endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PrometheusConfig {
    pub enabled: bool,
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

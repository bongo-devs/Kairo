//! `sentry.*`, error tracking.
//!
//! With a DSN set, [`crate::telemetry::init_sentry`] starts the SDK and its `tracing` integration
//! forwards `WARN` and `ERROR` logs as events and `INFO` as breadcrumbs, tagging the release with
//! the build's git commit and applying the configured environment and tags.

use std::collections::HashMap;

use serde::Deserialize;

/// The `sentry` block. An empty `dsn` leaves Sentry off, which is the default.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SentryConfig {
    /// The Sentry DSN.
    pub dsn: String,
    /// The environment name reported to Sentry, such as `production`.
    pub environment: String,
    /// Extra tags attached to every Sentry event.
    pub tags: HashMap<String, String>,
}

impl SentryConfig {
    /// Whether a DSN is set.
    pub fn is_enabled(&self) -> bool {
        !self.dsn.is_empty()
    }
}

//! `logging.*`, the log output settings, consumed by [`crate::utils`].

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LoggingConfig {
    /// Global default level, one of `trace`, `debug`, `info`, `warn` or `error`.
    pub level: String,
    /// Per-module level overrides, keyed by target such as `kairo`, `voice` or `player`.
    pub levels: HashMap<String, String>,
    pub format: LogFormat,
    /// Colourise console output. Ignored for the file sink.
    pub color: bool,
    pub timestamps: bool,
    pub show_target: bool,
    /// Optional rolling-file output alongside the console. Omit for console only.
    pub file: Option<LogFileConfig>,
    /// REST request logging, `logging.request.*`.
    pub request: RequestLoggingConfig,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            levels: HashMap::new(),
            format: LogFormat::Compact,
            color: true,
            timestamps: true,
            show_target: true,
            file: None,
            request: RequestLoggingConfig::default(),
        }
    }
}

/// `logging.request.*`, read by the request-log middleware in [`crate::rest`].
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RequestLoggingConfig {
    pub enabled: bool,
    /// Append `client=<addr>`, the remote socket address.
    pub include_client_info: bool,
    /// Append `headers=[...]`, with sensitive ones such as `authorization` redacted.
    pub include_headers: bool,
    pub include_query_string: bool,
    /// Append `payload=<body>`, truncated to `max_payload_length`.
    pub include_payload: bool,
    pub max_payload_length: usize,
    /// Also log a `>> ` line before the request runs, not only after it.
    pub before_request: bool,
}

impl Default for RequestLoggingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_client_info: false,
            include_headers: false,
            include_query_string: true,
            include_payload: true,
            max_payload_length: 10000,
            before_request: false,
        }
    }
}

/// `logging.format`, the log line layout.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Short single line: `HH:MM:SS LEVEL target: message`.
    #[default]
    Compact,
    /// Multiple lines with expanded fields.
    Pretty,
    /// One JSON object per line, for log shippers.
    Json,
}

/// `logging.file.*`, the optional rolling-file output.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LogFileConfig {
    /// File path. The parent directory is created if missing, and a date suffix is appended when
    /// rotating, as in `./logs/kairo.log.2026-06-22`.
    pub path: String,
    pub rotation: LogRotation,
}

impl Default for LogFileConfig {
    fn default() -> Self {
        Self {
            path: "./logs/kairo.log".to_string(),
            rotation: LogRotation::Daily,
        }
    }
}

/// `logging.file.rotation`, the roll-over cadence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogRotation {
    #[default]
    Daily,
    Hourly,
    /// Never roll over, leaving one growing file.
    Never,
}

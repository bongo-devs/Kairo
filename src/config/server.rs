//! `server.*`, the HTTP listener.

use serde::Deserialize;

/// The HTTP listener settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// The port to listen on, `2333` by default.
    pub port: u16,
    /// The bind address, all interfaces by default.
    pub address: String,
    /// Cleartext HTTP/2 support, `server.http2.*`.
    pub http2: Http2Config,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 2333,
            address: "0.0.0.0".to_string(),
            http2: Http2Config::default(),
        }
    }
}

/// `server.http2.*`, the HTTP/2 toggle.
///
/// Nothing terminates TLS in front of the listener, so this is h2c: a client that opens with the
/// HTTP/2 connection preface is served over HTTP/2 and everyone else stays on HTTP/1.1. Off by
/// default.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Http2Config {
    /// Whether h2c connections are accepted.
    pub enabled: bool,
}

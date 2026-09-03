//! `server.*`, the HTTP listener.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub port: u16,
    pub address: String,
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

/// Nothing terminates TLS in front of the listener, so this is h2c: a client that opens with the
/// HTTP/2 connection preface is served over HTTP/2 and everyone else stays on HTTP/1.1.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Http2Config {
    pub enabled: bool,
}

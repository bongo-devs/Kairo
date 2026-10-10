//! `lavalink.*`, the audio engine and protocol settings.

use serde::Deserialize;

use player::ResamplingQuality as EngineResamplingQuality;

use super::filters::FiltersToggleConfig;

/// The `lavalink` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LavalinkConfig {
    /// The `lavalink.server` block.
    pub server: LavalinkServerConfig,
}

/// The audio engine and protocol settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LavalinkServerConfig {
    /// REST and WebSocket authorization password.
    pub password: String,
    /// Send buffer length in milliseconds. Accepted for compatibility, unused.
    pub buffer_duration_ms: i32,
    /// Non-allocating frame buffer toggle. Accepted for compatibility, unused.
    pub non_allocating_frame_buffer: bool,
    /// Audio frame buffer duration in milliseconds.
    pub frame_buffer_duration_ms: u64,
    /// Opus encoder complexity, `0..=10`.
    pub opus_encoding_quality: u8,
    /// Opus target bitrate in bits per second.
    pub opus_bitrate: i32,
    /// Resampling quality.
    pub resampling_quality: ResamplingQuality,
    /// Milliseconds without a frame before a `TrackStuck` event fires.
    pub track_stuck_threshold_ms: u64,
    /// Whether to keep old frames across a seek, known as seek ghosting.
    pub use_seek_ghosting: bool,
    /// Seconds between `playerUpdate` messages.
    pub player_update_interval: u64,
    /// Which filters are enabled. A disabled one is rejected on a player update.
    pub filters: FiltersToggleConfig,
    /// Outbound IP rotation, `lavalink.server.ratelimit.*`.
    pub ratelimit: RatelimitConfig,
    /// Outbound HTTP proxy, `lavalink.server.httpConfig.*`.
    pub http_config: HttpConfig,
    /// Outbound HTTP timeouts, `lavalink.server.timeouts.*`.
    pub timeouts: TimeoutsConfig,
    /// Discord voice handshake throttling, `lavalink.server.voice.*`.
    pub voice: VoiceConfig,
}

impl Default for LavalinkServerConfig {
    fn default() -> Self {
        Self {
            password: "youshallnotpass".to_string(),
            buffer_duration_ms: 400,
            non_allocating_frame_buffer: false,
            frame_buffer_duration_ms: 5000,
            opus_encoding_quality: 10,
            opus_bitrate: 96_000,
            resampling_quality: ResamplingQuality::Low,
            track_stuck_threshold_ms: 10_000,
            use_seek_ghosting: true,
            player_update_interval: 5,
            filters: FiltersToggleConfig::default(),
            ratelimit: RatelimitConfig::default(),
            http_config: HttpConfig::default(),
            timeouts: TimeoutsConfig::default(),
            voice: VoiceConfig::default(),
        }
    }
}

/// `lavalink.server.ratelimit.*`, outbound IP rotation.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RatelimitConfig {
    /// CIDR blocks to rotate through, such as `1.0.0.0/8` or `2001:db8::/64`.
    pub ip_blocks: Vec<String>,
    /// Addresses within those blocks to never use.
    pub excluded_ips: Vec<String>,
    /// The rotation strategy.
    pub strategy: RatelimitStrategy,
    /// Whether a search `429` marks the address as failing. Searches are rate limited far more
    /// often than playback requests, so leaving this off keeps them from burning through the block.
    /// The request is retried either way.
    pub search_triggers_fail: bool,
    /// Retries before a rate-limited request gives up. A negative value takes the rotator default
    /// of 10, and `0` means unlimited.
    pub retry_limit: i32,
}

impl Default for RatelimitConfig {
    fn default() -> Self {
        Self {
            ip_blocks: Vec::new(),
            excluded_ips: Vec::new(),
            strategy: RatelimitStrategy::default(),
            // `#[serde(default)]` on the container fills any missing key from here.
            search_triggers_fail: true,
            retry_limit: -1,
        }
    }
}

impl RatelimitConfig {
    /// Whether at least one IP block is configured, so a route planner is worth building.
    pub fn is_enabled(&self) -> bool {
        !self.ip_blocks.is_empty()
    }

    /// Attempts per request, one more than `retryLimit`, which counts retries. A negative limit
    /// takes the rotator default of 10 retries and `0` means unlimited.
    pub fn retry_attempts(&self) -> u32 {
        match self.retry_limit {
            n if n < 0 => player::tools::http_config::DEFAULT_RETRY_ATTEMPTS,
            0 => u32::MAX,
            n => n as u32 + 1,
        }
    }
}

/// The IP rotation strategy, `lavalink.server.ratelimit.strategy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RatelimitStrategy {
    /// Rotate to the next address only when the current one is rate limited.
    #[default]
    RotateOnBan,
    /// Pick a random address per request, spreading load across the block.
    LoadBalance,
    /// A fresh address per request, taken from one `/64`.
    NanoSwitch,
    /// A fresh address per request, rotating the `/64` on a ban.
    RotatingNanoSwitch,
}

// Matched case-insensitively, and an unknown value fails the config load rather than silently
// rotating some other way.
impl<'de> Deserialize<'de> for RatelimitStrategy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.trim().to_lowercase().as_str() {
            "rotateonban" => Ok(Self::RotateOnBan),
            "loadbalance" => Ok(Self::LoadBalance),
            "nanoswitch" => Ok(Self::NanoSwitch),
            "rotatingnanoswitch" => Ok(Self::RotatingNanoSwitch),
            _ => Err(serde::de::Error::custom(format!(
                "unknown ratelimit strategy {raw:?} \
                 (expected RotateOnBan, LoadBalance, NanoSwitch or RotatingNanoSwitch)"
            ))),
        }
    }
}

/// `lavalink.server.httpConfig.*`, an outbound HTTP proxy.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HttpConfig {
    /// Proxy host, empty to send requests directly.
    pub proxy_host: String,
    /// Proxy port.
    pub proxy_port: u16,
    /// Proxy basic-auth username, if the proxy needs one.
    pub proxy_user: String,
    /// Proxy basic-auth password, if the proxy needs one.
    pub proxy_password: String,
}

impl HttpConfig {
    /// Whether a proxy host is set.
    pub fn is_enabled(&self) -> bool {
        !self.proxy_host.is_empty()
    }

    /// Build a [`player::tools::http_config::HttpProxyConfig`] from these settings, if enabled.
    pub fn to_proxy(&self) -> Option<player::tools::http_config::HttpProxyConfig> {
        if !self.is_enabled() {
            return None;
        }
        // Assume http:// when the host omits a scheme.
        let url = if self.proxy_host.contains("://") {
            format!("{}:{}", self.proxy_host, self.proxy_port)
        } else {
            format!("http://{}:{}", self.proxy_host, self.proxy_port)
        };
        Some(player::tools::http_config::HttpProxyConfig {
            url,
            username: (!self.proxy_user.is_empty()).then(|| self.proxy_user.clone()),
            password: (!self.proxy_password.is_empty()).then(|| self.proxy_password.clone()),
        })
    }
}

/// `lavalink.server.timeouts.*`, outbound HTTP timeouts.
///
/// Only `connectTimeoutMs` is accepted. The other two keys of that block have no equivalent in this
/// HTTP client, which never waits to lease a pooled connection and has no per-read deadline, so
/// they are rejected rather than silently ignored.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TimeoutsConfig {
    /// Milliseconds to wait for a TCP and TLS connection before the request fails, `0` for the OS
    /// default. A short timeout lets a blackholed source host fail fast so the load falls through
    /// to the remaining sources.
    pub connect_timeout_ms: u64,
}

impl Default for TimeoutsConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: player::tools::http_config::DEFAULT_CONNECT_TIMEOUT.as_millis()
                as u64,
        }
    }
}

impl TimeoutsConfig {
    /// The connect timeout as a [`Duration`](std::time::Duration), or `None` for the OS default.
    pub fn connect_timeout(&self) -> Option<std::time::Duration> {
        (self.connect_timeout_ms > 0)
            .then(|| std::time::Duration::from_millis(self.connect_timeout_ms))
    }
}

/// `lavalink.server.voice.*`, Discord voice handshake throttling.
///
/// A large bot reconnecting 1000+ guilds at once would otherwise open that many
/// simultaneous TLS + UDP handshakes. Every value here has a documented default;
/// there are no other tuning constants for this path.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoiceConfig {
    /// Max concurrent gateway + UDP handshakes. Past this, background connects
    /// park on the gate (no worker, almost no memory) until a slot frees.
    pub max_concurrent_handshakes: usize,
    /// Queue wait that triggers a warn log on acquire. Background waits are
    /// unbounded by design (a parked wait holds no worker); this is the warn
    /// threshold in milliseconds, not a failure timeout.
    pub queue_warn_ms: u64,
    /// Outer bound on one handshake. The voice crate aborts its inner exchange
    /// after 30 s, so this backstops at the same point rather than killing a
    /// handshake the crate would have completed (DAVE keygen can run long under
    /// a reconnect storm).
    pub handshake_timeout_ms: u64,
    /// Connect in the background and answer PATCH immediately (`true`), or block
    /// the PATCH on the handshake as Lavalink historically does (`false`, with
    /// the pre-change failure mapping: handshake errors return `500`).
    pub background_connect: bool,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            max_concurrent_handshakes: 32,
            queue_warn_ms: 15_000,
            handshake_timeout_ms: 30_000,
            background_connect: false,
        }
    }
}

impl VoiceConfig {
    /// The handshake timeout as a [`Duration`](std::time::Duration).
    pub fn handshake_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.handshake_timeout_ms.max(1))
    }

    /// The queue-warn interval as a [`Duration`](std::time::Duration).
    pub fn queue_warn(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.queue_warn_ms.max(1_000))
    }
}

impl LavalinkServerConfig {
    /// The names of the filters disabled in config.
    pub fn disabled_filters(&self) -> Vec<String> {
        self.filters.disabled()
    }
}

/// Resampling quality, written `LOW`, `MEDIUM` or `HIGH` in YAML.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ResamplingQuality {
    /// Highest quality.
    High,
    /// Balanced.
    Medium,
    /// Linear interpolation, the default.
    #[default]
    Low,
}

impl ResamplingQuality {
    /// Map to the engine's resampling quality.
    pub fn to_engine(self) -> EngineResamplingQuality {
        match self {
            ResamplingQuality::High => EngineResamplingQuality::High,
            ResamplingQuality::Medium => EngineResamplingQuality::Medium,
            ResamplingQuality::Low => EngineResamplingQuality::Low,
        }
    }
}

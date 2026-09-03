use serde::Serialize;

/// Node statistics, sent over the WebSocket (`op: "stats"`) and via `GET /v4/stats`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Frame statistics; `null` when the node has no players or via `GET /v4/stats`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_stats: Option<FrameStats>,
    pub players: i32,
    pub playing_players: i32,
    /// Node uptime in milliseconds.
    pub uptime: i64,
    pub memory: Memory,
    pub cpu: Cpu,
}

/// Per-minute frame statistics.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FrameStats {
    pub sent: i32,
    /// Frames nulled, meaning silence went out because no audio was ready.
    pub nulled: i32,
    /// Expected frames less the sent and nulled ones.
    pub deficit: i32,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Memory {
    pub free: i64,
    pub used: i64,
    pub allocated: i64,
    pub reservable: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cpu {
    pub cores: i32,
    /// System-wide CPU load, `0.0..=1.0`.
    pub system_load: f64,
    /// This process's CPU load, `0.0..=1.0`.
    pub lavalink_load: f64,
}

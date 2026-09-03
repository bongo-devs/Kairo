//! Route planner status for `/v4/routeplanner/*`, served only when `lavalink.server.ratelimit` is
//! set; without it status answers `204` and the free-address endpoints `500`.

use serde::Serialize;

/// The status of the configured IP route planner, one variant per `ratelimit.strategy`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "class", content = "details")]
pub enum RoutePlannerStatus {
    /// A rotating-IP route planner (`strategy: RotateOnBan`).
    #[serde(rename = "RotatingIpRoutePlanner")]
    RotatingIp(RotatingIpDetails),
    /// A nano-switch route planner (`strategy: NanoSwitch`).
    #[serde(rename = "NanoIpRoutePlanner")]
    NanoIp(NanoIpDetails),
    /// A rotating nano-switch route planner (`strategy: RotatingNanoSwitch`).
    #[serde(rename = "RotatingNanoIpRoutePlanner")]
    RotatingNanoIp(RotatingNanoIpDetails),
    /// A balancing-IP route planner (`strategy: LoadBalance`).
    #[serde(rename = "BalancingIpRoutePlanner")]
    BalancingIp(BalancingIpDetails),
}

/// Details for a rotating-IP planner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatingIpDetails {
    /// The IP block being rotated through.
    pub ip_block: IpBlockStatus,
    pub failing_addresses: Vec<FailingAddress>,
    /// The rotation index.
    pub rotate_index: String,
    /// The current IP index within the block.
    pub ip_index: String,
    /// The current outbound address.
    pub current_address: String,
}

/// Details for a nano-switch planner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NanoIpDetails {
    /// The IP block being switched through.
    pub ip_block: IpBlockStatus,
    pub failing_addresses: Vec<FailingAddress>,
    /// The current address index, nanoseconds elapsed since the planner started, so it is a
    /// timestamp rather than a small counter.
    pub current_address_index: String,
}

/// Details for a rotating nano-switch planner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotatingNanoIpDetails {
    /// The IP block being switched through.
    pub ip_block: IpBlockStatus,
    pub failing_addresses: Vec<FailingAddress>,
    /// Which `/64` inside the block is in use, advanced on a ban.
    pub block_index: String,
    /// The current address index within that `/64`, nanoseconds elapsed since the block was
    /// entered.
    pub current_address_index: String,
}

/// Details for a balancing-IP planner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BalancingIpDetails {
    /// The IP block being balanced across.
    pub ip_block: IpBlockStatus,
    pub failing_addresses: Vec<FailingAddress>,
}

/// An IP block's type and size.
#[derive(Debug, Clone, Serialize)]
pub struct IpBlockStatus {
    /// `"Inet4Address"` or `"Inet6Address"`.
    #[serde(rename = "type")]
    pub block_type: String,
    /// The block size as a string.
    pub size: String,
}

/// A failing outbound address.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailingAddress {
    /// The failing address.
    pub failing_address: String,
    /// Unix millisecond timestamp of the failure.
    pub failing_timestamp: i64,
    /// Human-readable failure time.
    pub failing_time: String,
}

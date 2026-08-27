//! IP rotation for outbound HTTP requests, configured under `lavalink.server.ratelimit`.
//!
//! Each strategy hands the HTTP stack a source address to bind, and an address that comes back
//! rate-limited is marked failing and skipped until it expires.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{TimeZone, Utc};
use rand::Rng;

use player::tools::http_config::RoutePlanner;

use crate::config::{RatelimitConfig, RatelimitStrategy};
use crate::protocol::routeplanner::{
    BalancingIpDetails, FailingAddress, IpBlockStatus, NanoIpDetails, RotatingIpDetails,
    RotatingNanoIpDetails, RoutePlannerStatus,
};
use crate::rest::error::now_millis;

// How many addresses to scan past excluded or failing ones before giving up.
const MAX_SCAN: u128 = 1024;

// Addresses in an IPv6 `/64`, the unit the nano strategies work in. Their clock is a 64-bit
// nanosecond count, so it indexes exactly one of these.
const BLOCK64_IPS: u128 = 1u128 << 64;

// How long an address stays marked failing before it is retried anyway. Without this the pool only
// ever shrinks.
const FAILING_TIME_MILLIS: i64 = 7 * 24 * 60 * 60 * 1000;

// A parsed CIDR block.
#[derive(Debug, Clone)]
struct IpBlock {
    // Network base address as an integer, IPv4 mapped into the low 32 bits.
    base: u128,
    is_ipv6: bool,
    // `2^host_bits`, saturated at `u128::MAX`.
    size: u128,
}

impl IpBlock {
    // Parse `a.b.c.d/n` or `v6::/n`. A bare address is its own single-address block. `None` for
    // malformed input.
    fn parse(cidr: &str) -> Option<Self> {
        let (addr_str, prefix_str) = match cidr.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (cidr, None),
        };
        match addr_str.trim().parse::<IpAddr>().ok()? {
            IpAddr::V4(v4) => {
                let prefix: u32 = match prefix_str {
                    Some(prefix) => prefix.trim().parse().ok()?,
                    None => 32,
                };
                if prefix > 32 {
                    return None;
                }
                let host_bits = 32 - prefix;
                let size = 1u128 << host_bits;
                let mask = size - 1;
                Some(Self {
                    base: (u32::from(v4) as u128) & !mask,
                    is_ipv6: false,
                    size,
                })
            }
            IpAddr::V6(v6) => {
                let prefix: u32 = match prefix_str {
                    Some(prefix) => prefix.trim().parse().ok()?,
                    None => 128,
                };
                if prefix > 128 {
                    return None;
                }
                let host_bits = 128 - prefix;
                let (size, mask) = if host_bits >= 128 {
                    (u128::MAX, u128::MAX)
                } else {
                    let size = 1u128 << host_bits;
                    (size, size - 1)
                };
                Some(Self {
                    base: u128::from(v6) & !mask,
                    is_ipv6: true,
                    size,
                })
            }
        }
    }

    // The address at `index`, wrapping by the block size.
    fn address_at(&self, index: u128) -> IpAddr {
        let offset = index % self.size;
        let value = self.base.wrapping_add(offset);
        if self.is_ipv6 {
            IpAddr::V6(Ipv6Addr::from(value))
        } else {
            IpAddr::V4(Ipv4Addr::from((value & 0xFFFF_FFFF) as u32))
        }
    }

    // The address type as the status payload names it.
    fn type_name(&self) -> &'static str {
        if self.is_ipv6 {
            "Inet6Address"
        } else {
            "Inet4Address"
        }
    }
}

/// An IP route planner over one or more CIDR blocks.
pub struct IpRoutePlanner {
    blocks: Vec<IpBlock>,
    total_size: u128,
    excluded: Vec<IpAddr>,
    strategy: RatelimitStrategy,
    // How many times a ban has moved the planner off an address, reported as `rotateIndex`.
    rotate_index: AtomicU64,
    // One past the offset `RotateOnBan` currently sits on, reported as `ipIndex`.
    ip_index: AtomicU64,
    // Set by a ban so the next `RotateOnBan` request picks again instead of reusing `current`.
    rotate_pending: AtomicBool,
    // Origin of the nano strategies' clock.
    created: Instant,
    // Which `/64` `RotatingNanoSwitch` is in, advanced on a ban.
    nano_block: AtomicU64,
    // Nanoseconds since `created` at which that `/64` was entered.
    block_start_nanos: AtomicU64,
    // Addresses currently marked failing, each with the unix-ms timestamp of the failure.
    failing: Mutex<HashMap<IpAddr, i64>>,
    // The most recently handed-out address, for the status payload and `RotateOnBan`'s sticky pick.
    current: Mutex<Option<IpAddr>>,
    // The address the last ban was for, so repeated bans of one address rotate once.
    last_failing: Mutex<Option<IpAddr>>,
}

impl IpRoutePlanner {
    /// Build a planner from `lavalink.server.ratelimit`, or `None` when it is off or no block
    /// parses. Every block that fails to parse is logged.
    pub fn from_config(config: &RatelimitConfig) -> Option<Arc<Self>> {
        if !config.is_enabled() {
            return None;
        }
        let mut blocks = Vec::new();
        for cidr in &config.ip_blocks {
            match IpBlock::parse(cidr) {
                Some(block) => blocks.push(block),
                None => tracing::warn!(block = %cidr, "Ignoring invalid ratelimit ipBlock"),
            }
        }
        if blocks.is_empty() {
            tracing::warn!(
                "ratelimit.ipBlocks contained no valid CIDR blocks; route planner disabled"
            );
            return None;
        }
        let total_size = blocks
            .iter()
            .fold(0u128, |acc, b| acc.saturating_add(b.size));
        // Both nano strategies index a `/64` with a 64-bit nanosecond clock, so an IPv4 or smaller
        // pool has nothing to rotate through.
        if is_nano(config.strategy) && (!blocks[0].is_ipv6 || total_size < BLOCK64_IPS) {
            tracing::warn!(
                strategy = ?config.strategy,
                "ratelimit.strategy needs an IPv6 block of at least a /64; route planner disabled"
            );
            return None;
        }
        let excluded = config
            .excluded_ips
            .iter()
            .filter_map(|ip| match ip.trim().parse::<IpAddr>() {
                Ok(address) => Some(address),
                Err(_) => {
                    tracing::warn!(address = %ip, "Ignoring unparseable ratelimit excludedIp");
                    None
                }
            })
            .collect();

        Some(Arc::new(Self {
            blocks,
            total_size,
            excluded,
            strategy: config.strategy,
            rotate_index: AtomicU64::new(0),
            ip_index: AtomicU64::new(0),
            rotate_pending: AtomicBool::new(false),
            created: Instant::now(),
            nano_block: AtomicU64::new(0),
            block_start_nanos: AtomicU64::new(0),
            failing: Mutex::new(HashMap::new()),
            current: Mutex::new(None),
            last_failing: Mutex::new(None),
        }))
    }

    // The address at a global index spanning every block, wrapping by `total_size`.
    fn address_at_global(&self, index: u128) -> IpAddr {
        let mut offset = index % self.total_size;
        for block in &self.blocks {
            if offset < block.size {
                return block.address_at(offset);
            }
            offset -= block.size;
        }
        // Unreachable while `offset < total_size`, but fall back to the first block's base.
        self.blocks[0].address_at(0)
    }

    fn is_excluded(&self, address: &IpAddr) -> bool {
        self.excluded.contains(address)
    }

    fn is_failing(&self, address: &IpAddr) -> bool {
        let mut failing = self.failing.lock().unwrap();
        match failing.get(address) {
            // An expired entry is dropped and the address retried. Only the address asked about:
            // this runs up to `MAX_SCAN` times per request, so walking the whole map here would
            // make every scan step O(n).
            Some(&at) if at < expiry_cutoff() => {
                failing.remove(address);
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    // The first usable address at or after `start`, with the global offset it was found at so
    // `RotateOnBan` can resume from there.
    fn usable_from(&self, start: u128) -> (IpAddr, u128) {
        let scan = self.total_size.min(MAX_SCAN);
        for i in 0..scan {
            let offset = start.wrapping_add(i) % self.total_size;
            let address = self.address_at_global(offset);
            if !self.is_excluded(&address) && !self.is_failing(&address) {
                return (address, offset);
            }
        }
        // Everything in range is excluded or failing, so hand back the start address anyway.
        let offset = start % self.total_size;
        (self.address_at_global(offset), offset)
    }

    // The `RotateOnBan` address. The pick is sticky until a ban sets `rotate_pending`; a fresh pick
    // jumps a random 10..19 addresses on a block wider than 128 so a ban does not land on the
    // neighbouring, most likely equally rate-limited address, then scans forward from there.
    fn rotate_on_ban_address(&self) -> IpAddr {
        // `current` before `failing` (through `is_failing`), the lock order used everywhere here.
        let mut current = self.current.lock().unwrap();
        if !self.rotate_pending.swap(false, Ordering::AcqRel) {
            if let Some(address) = *current {
                return address;
            }
        }
        let step: u128 = if self.total_size > 128 {
            rand::thread_rng().gen_range(10..20)
        } else {
            1
        };
        let index = (self.ip_index.load(Ordering::Acquire) as u128).wrapping_add(step);
        // `ip_index` counts addresses handed out and the pick sits one below it, so the first pick
        // is the block's base rather than the address after it.
        let (address, offset) = self.usable_from(index.wrapping_sub(1));
        self.ip_index
            .store(offset.wrapping_add(1) as u64, Ordering::Release);
        *current = Some(address);
        address
    }

    // Nanoseconds elapsed since the current `/64` was entered.
    fn nanos_in_block(&self) -> u128 {
        let elapsed = self.created.elapsed().as_nanos();
        elapsed.saturating_sub(self.block_start_nanos.load(Ordering::Acquire) as u128)
    }

    // The nano strategies index by the nanosecond clock itself, so consecutive requests land on
    // consecutive addresses.
    fn nano_index(&self) -> u128 {
        let offset = self.nanos_in_block();
        if self.strategy == RatelimitStrategy::RotatingNanoSwitch {
            let block = self.nano_block.load(Ordering::Acquire) as u128;
            return block.wrapping_mul(BLOCK64_IPS).wrapping_add(offset);
        }
        // The clock only spans one `/64`, so a wider block picks one at random per request.
        let blocks64 = self.total_size / BLOCK64_IPS;
        if blocks64 > 1 {
            let chosen = rand::thread_rng().gen_range(0..blocks64);
            return chosen.wrapping_mul(BLOCK64_IPS).wrapping_add(offset);
        }
        offset
    }

    /// The payload behind `GET /v4/routeplanner/status`.
    pub fn status(&self) -> RoutePlannerStatus {
        // Every configured block is reported as one combined block: the size is the total and the
        // type comes from the first.
        let ip_block = IpBlockStatus {
            block_type: self.blocks[0].type_name().to_string(),
            size: self.total_size.to_string(),
        };
        let failing_addresses = self.failing_addresses();

        match self.strategy {
            RatelimitStrategy::RotateOnBan => {
                let current = self
                    .current
                    .lock()
                    .unwrap()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                RoutePlannerStatus::RotatingIp(RotatingIpDetails {
                    ip_block,
                    failing_addresses,
                    rotate_index: self.rotate_index.load(Ordering::Acquire).to_string(),
                    ip_index: self.ip_index.load(Ordering::Acquire).to_string(),
                    current_address: current,
                })
            }
            RatelimitStrategy::NanoSwitch => RoutePlannerStatus::NanoIp(NanoIpDetails {
                ip_block,
                failing_addresses,
                current_address_index: self.nanos_in_block().to_string(),
            }),
            RatelimitStrategy::RotatingNanoSwitch => {
                RoutePlannerStatus::RotatingNanoIp(RotatingNanoIpDetails {
                    ip_block,
                    failing_addresses,
                    block_index: self.nano_block.load(Ordering::Acquire).to_string(),
                    current_address_index: self.nanos_in_block().to_string(),
                })
            }
            RatelimitStrategy::LoadBalance => RoutePlannerStatus::BalancingIp(BalancingIpDetails {
                ip_block,
                failing_addresses,
            }),
        }
    }

    fn failing_addresses(&self) -> Vec<FailingAddress> {
        let mut failing = self.failing.lock().unwrap();
        prune_expired(&mut failing);
        failing
            .iter()
            .map(|(address, &timestamp)| FailingAddress {
                failing_address: address.to_string(),
                failing_timestamp: timestamp,
                failing_time: format_millis(timestamp),
            })
            .collect()
    }

    /// Stop treating `address` as failing. `true` if it was.
    pub fn free_address(&self, address: &IpAddr) -> bool {
        self.failing.lock().unwrap().remove(address).is_some()
    }

    /// Stop treating every address as failing.
    pub fn free_all(&self) {
        self.failing.lock().unwrap().clear();
    }
}

impl RoutePlanner for IpRoutePlanner {
    fn next_address(&self) -> Option<IpAddr> {
        if self.total_size == 0 {
            return None;
        }
        let address = match self.strategy {
            // Sticky: the same address until it is banned.
            RatelimitStrategy::RotateOnBan => return Some(self.rotate_on_ban_address()),
            // The clock is the index, so every request gets a fresh address on its own.
            RatelimitStrategy::NanoSwitch | RatelimitStrategy::RotatingNanoSwitch => {
                self.usable_from(self.nano_index()).0
            }
            // A random address per request, skipping excluded and failing ones.
            RatelimitStrategy::LoadBalance => {
                let mut rng = rand::thread_rng();
                let mut chosen = None;
                for _ in 0..16 {
                    let address = self.address_at_global(rng.gen_range(0..self.total_size));
                    if !self.is_excluded(&address) && !self.is_failing(&address) {
                        chosen = Some(address);
                        break;
                    }
                }
                chosen.unwrap_or_else(|| self.address_at_global(rng.gen_range(0..self.total_size)))
            }
        };
        *self.current.lock().unwrap() = Some(address);
        Some(address)
    }

    fn mark_failing(&self, address: IpAddr) {
        {
            let mut failing = self.failing.lock().unwrap();
            // Prune here rather than only when the status endpoint is polled: an unpolled node on a
            // large block would otherwise keep an entry per banned address forever, and a ban is
            // the only moment the map grows.
            prune_expired(&mut failing);
            failing.insert(address, now_millis());
        }
        tracing::warn!(%address, "Route planner marked source address as failing (rate-limited)");
        // A rate-limited address usually fails on several in-flight requests at once, and those
        // duplicates must not each cost a rotation step or the block drains N addresses per ban.
        let repeat = {
            let mut last = self.last_failing.lock().unwrap();
            let repeat = *last == Some(address);
            *last = Some(address);
            repeat
        };
        if repeat {
            return;
        }
        match self.strategy {
            // Step past the banned address on the next request.
            RatelimitStrategy::RotateOnBan => {
                self.rotate_index.fetch_add(1, Ordering::AcqRel);
                self.rotate_pending.store(true, Ordering::Release);
            }
            // Abandon the whole `/64` and restart the clock inside the next one.
            RatelimitStrategy::RotatingNanoSwitch => {
                self.nano_block.fetch_add(1, Ordering::AcqRel);
                self.block_start_nanos
                    .store(self.created.elapsed().as_nanos() as u64, Ordering::Release);
            }
            // Nothing to advance, the next request picks a different address regardless.
            RatelimitStrategy::NanoSwitch | RatelimitStrategy::LoadBalance => {}
        }
    }
}

// The two nano strategies, which need an IPv6 `/64` or wider.
fn is_nano(strategy: RatelimitStrategy) -> bool {
    matches!(
        strategy,
        RatelimitStrategy::NanoSwitch | RatelimitStrategy::RotatingNanoSwitch
    )
}

// Unix-ms before which a recorded failure has expired and the address is usable again.
fn expiry_cutoff() -> i64 {
    now_millis().saturating_sub(FAILING_TIME_MILLIS)
}

fn prune_expired(failing: &mut HashMap<IpAddr, i64>) {
    let cutoff = expiry_cutoff();
    failing.retain(|_, at| *at >= cutoff);
}

// The human-readable form of a timestamp, as `failingTime` reports it.
fn format_millis(millis: i64) -> String {
    match Utc.timestamp_millis_opt(millis).single() {
        Some(dt) => dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
        None => millis.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn config(blocks: &[&str], excluded: &[&str], strategy: RatelimitStrategy) -> RatelimitConfig {
        RatelimitConfig {
            ip_blocks: blocks.iter().map(|s| s.to_string()).collect(),
            excluded_ips: excluded.iter().map(|s| s.to_string()).collect(),
            strategy,
            ..Default::default()
        }
    }

    #[test]
    fn ipv4_block_sizes_and_wraps() {
        let block = IpBlock::parse("10.0.0.0/30").unwrap();
        assert_eq!(block.size, 4);
        assert_eq!(block.address_at(0), ip("10.0.0.0"));
        assert_eq!(block.address_at(3), ip("10.0.0.3"));
        assert_eq!(block.address_at(5), ip("10.0.0.1")); // wraps by size
    }

    #[test]
    fn ipv4_block_masks_host_bits() {
        // A host address with a /24 prefix is masked down to the network address.
        let block = IpBlock::parse("192.168.1.55/24").unwrap();
        assert_eq!(block.size, 256);
        assert_eq!(block.address_at(0), ip("192.168.1.0"));
    }

    #[test]
    fn ipv6_block_parses() {
        let block = IpBlock::parse("2001:db8::/126").unwrap();
        assert!(block.is_ipv6);
        assert_eq!(block.size, 4);
        assert_eq!(block.address_at(1), ip("2001:db8::1"));
    }

    #[test]
    fn invalid_cidr_is_rejected() {
        assert!(IpBlock::parse("not-an-ip/24").is_none());
        assert!(IpBlock::parse("10.0.0.0/40").is_none());
        assert!(IpBlock::parse("10.0.0.0/x").is_none());
    }

    // A bare address is a one-address block rather than a config error.
    #[test]
    fn bare_address_is_a_single_address_block() {
        let v4 = IpBlock::parse("1.2.3.4").unwrap();
        assert_eq!(v4.size, 1);
        assert_eq!(v4.address_at(0), ip("1.2.3.4"));

        let v6 = IpBlock::parse("2001:db8::1").unwrap();
        assert!(v6.is_ipv6);
        assert_eq!(v6.size, 1);
        assert_eq!(v6.address_at(0), ip("2001:db8::1"));
    }

    #[test]
    fn rotate_on_ban_advances_only_when_banned() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();

        // Sticky: the same address until it's banned.
        assert_eq!(planner.next_address(), Some(ip("10.0.0.0")));
        assert_eq!(planner.next_address(), Some(ip("10.0.0.0")));

        planner.mark_failing(ip("10.0.0.0"));
        assert_eq!(planner.next_address(), Some(ip("10.0.0.1")));
    }

    #[test]
    fn rotate_on_ban_skips_excluded() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &["10.0.0.0"],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        assert_eq!(planner.next_address(), Some(ip("10.0.0.1")));
    }

    #[test]
    fn load_balance_returns_addresses_in_block() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &[],
            RatelimitStrategy::LoadBalance,
        ))
        .unwrap();
        let block = IpBlock::parse("10.0.0.0/30").unwrap();
        let valid: Vec<IpAddr> = (0..4).map(|i| block.address_at(i)).collect();
        for _ in 0..32 {
            let address = planner.next_address().unwrap();
            assert!(valid.contains(&address), "{address} not in block");
        }
    }

    #[test]
    fn free_address_unmarks_failing() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        planner.mark_failing(ip("10.0.0.0"));
        assert!(planner.free_address(&ip("10.0.0.0")));
        assert!(!planner.free_address(&ip("10.0.0.0"))); // already cleared
        assert!(planner.failing.lock().unwrap().is_empty());
    }

    // A rate-limited address fails on every request already in flight, and those duplicates must
    // not each cost a rotation step.
    #[test]
    fn repeated_bans_of_one_address_rotate_once() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/24"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        let first = planner.next_address().unwrap();

        planner.mark_failing(first);
        planner.mark_failing(first);
        planner.mark_failing(first);
        assert_eq!(planner.rotate_index.load(Ordering::Acquire), 1);

        let second = planner.next_address().unwrap();
        assert_ne!(second, first);
        planner.mark_failing(second);
        assert_eq!(planner.rotate_index.load(Ordering::Acquire), 2);
    }

    // `rotateIndex` counts bans while `ipIndex` tracks the address offset, and on a block wider than
    // 128 addresses a ban steps 10..19 addresses ahead rather than one.
    #[test]
    fn rotate_on_ban_reports_ban_count_and_address_offset_separately() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/24"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        let first = planner.next_address().unwrap();
        // A second request re-uses the cached pick instead of rescanning.
        assert_eq!(planner.next_address(), Some(first));
        let picked = planner.ip_index.load(Ordering::Acquire);
        assert!((10..=20).contains(&picked), "first pick at {picked}");

        planner.mark_failing(first);
        planner.next_address().unwrap();
        let ip_index = planner.ip_index.load(Ordering::Acquire);
        assert!(ip_index >= picked + 10, "{picked} -> {ip_index}");

        match planner.status() {
            RoutePlannerStatus::RotatingIp(details) => {
                assert_eq!(details.rotate_index, "1");
                assert_eq!(details.ip_index, ip_index.to_string());
            }
            other => panic!("expected RotatingIp, got {other:?}"),
        }
    }

    #[test]
    fn status_reports_rotating_for_rotate_on_ban() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/24"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        match planner.status() {
            RoutePlannerStatus::RotatingIp(details) => {
                assert_eq!(details.ip_block.block_type, "Inet4Address");
                assert_eq!(details.ip_block.size, "256");
            }
            other => panic!("expected RotatingIp, got {other:?}"),
        }
    }

    #[test]
    fn disabled_when_no_blocks() {
        assert!(
            IpRoutePlanner::from_config(&config(&[], &[], RatelimitStrategy::RotateOnBan))
                .is_none()
        );
    }

    #[test]
    fn status_class_matches_strategy() {
        let cases: [(&[&str], RatelimitStrategy, &str); 4] = [
            (
                &["10.0.0.0/24"],
                RatelimitStrategy::RotateOnBan,
                "RotatingIpRoutePlanner",
            ),
            (
                &["10.0.0.0/24"],
                RatelimitStrategy::LoadBalance,
                "BalancingIpRoutePlanner",
            ),
            (
                &["2001:db8::/48"],
                RatelimitStrategy::NanoSwitch,
                "NanoIpRoutePlanner",
            ),
            (
                &["2001:db8::/48"],
                RatelimitStrategy::RotatingNanoSwitch,
                "RotatingNanoIpRoutePlanner",
            ),
        ];
        for (blocks, strategy, class) in cases {
            let planner = IpRoutePlanner::from_config(&config(blocks, &[], strategy)).unwrap();
            let status = serde_json::to_value(planner.status()).unwrap();
            assert_eq!(status["class"], class, "{strategy:?}");
            assert!(status["details"]["ipBlock"]["size"].is_string(), "{class}");
        }
    }

    #[test]
    fn nano_switch_needs_an_ipv6_64_or_bigger() {
        for blocks in [&["10.0.0.0/8"], &["2001:db8::/96"]] {
            for strategy in [
                RatelimitStrategy::NanoSwitch,
                RatelimitStrategy::RotatingNanoSwitch,
            ] {
                assert!(
                    IpRoutePlanner::from_config(&config(blocks, &[], strategy)).is_none(),
                    "{blocks:?} accepted for {strategy:?}"
                );
            }
        }
    }

    #[test]
    fn nano_switch_hands_out_a_fresh_address_per_request() {
        let planner = IpRoutePlanner::from_config(&config(
            &["2001:db8::/64"],
            &[],
            RatelimitStrategy::NanoSwitch,
        ))
        .unwrap();
        // The clock is the index, so no two requests share an address without a ban.
        let first = planner.next_address().unwrap();
        assert_ne!(first, planner.next_address().unwrap());
    }

    #[test]
    fn rotating_nano_moves_to_the_next_block_on_ban() {
        let planner = IpRoutePlanner::from_config(&config(
            &["2001:db8::/48"],
            &[],
            RatelimitStrategy::RotatingNanoSwitch,
        ))
        .unwrap();

        let block_of = |address: IpAddr| match address {
            IpAddr::V6(v6) => v6.segments()[3],
            other => panic!("expected IPv6, got {other}"),
        };
        let first = planner.next_address().unwrap();
        assert_eq!(block_of(first), 0);

        planner.mark_failing(first);
        assert_eq!(block_of(planner.next_address().unwrap()), 1);
        match planner.status() {
            RoutePlannerStatus::RotatingNanoIp(details) => {
                assert_eq!(details.block_index, "1");
                assert_eq!(details.failing_addresses.len(), 1);
            }
            other => panic!("expected RotatingNanoIp, got {other:?}"),
        }
    }

    #[test]
    fn failing_addresses_expire() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        // Backdate the failure past the cache duration: the address is usable again.
        planner
            .failing
            .lock()
            .unwrap()
            .insert(ip("10.0.0.0"), now_millis() - FAILING_TIME_MILLIS - 1);
        assert!(!planner.is_failing(&ip("10.0.0.0")));
        assert!(planner.failing.lock().unwrap().is_empty());
    }

    // A node nobody polls for status still has to forget expired bans, and a ban is the only moment
    // the map grows, so that is where the pruning happens.
    #[test]
    fn marking_a_ban_drops_the_expired_ones() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        planner
            .failing
            .lock()
            .unwrap()
            .insert(ip("10.0.0.3"), now_millis() - FAILING_TIME_MILLIS - 1);

        planner.mark_failing(ip("10.0.0.0"));

        let failing = planner.failing.lock().unwrap();
        assert_eq!(failing.len(), 1);
        assert!(failing.contains_key(&ip("10.0.0.0")));
    }

    // Nothing looks up an address that has stopped failing, so the status walk is what prunes it.
    #[test]
    fn listing_failing_addresses_drops_the_expired_ones() {
        let planner = IpRoutePlanner::from_config(&config(
            &["10.0.0.0/30"],
            &[],
            RatelimitStrategy::RotateOnBan,
        ))
        .unwrap();
        {
            let mut failing = planner.failing.lock().unwrap();
            failing.insert(ip("10.0.0.0"), now_millis() - FAILING_TIME_MILLIS - 1);
            failing.insert(ip("10.0.0.1"), now_millis());
        }
        let listed = planner.failing_addresses();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].failing_address, "10.0.0.1");
        assert_eq!(planner.failing.lock().unwrap().len(), 1);
    }
}

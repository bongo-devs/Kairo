//! Global gate for Discord voice handshakes.
//!
//! A large bot reconnecting 1000+ guilds at once would otherwise open that many
//! simultaneous TLS + UDP handshakes. That exhausts file descriptors and starves the
//! runtime so search and playback HTTP starts failing while voice alone looks
//! connected. Handshakes past the limit queue here.
//!
//! Tuning lives in `lavalink.server.voice.*` (`VoiceConfig`), applied once at
//! startup via [`configure`]. There are no other constants on this path.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::VoiceConfig;
use crate::rest::error::RestError;

/// Defaults matching `VoiceConfig::default`, used until [`configure`] runs and by
/// tests that never configure the gate.
const DEFAULT_MAX_CONCURRENT: usize = 32;
const DEFAULT_QUEUE_WARN_MS: u64 = 15_000;

/// Wait-duration buckets (ms) for the Prometheus histogram.
const WAIT_BUCKETS_MS: [u64; 9] = [10, 50, 100, 250, 500, 1000, 2500, 5000, 15000];
/// Handshake-duration buckets (ms) for the Prometheus histogram.
const HANDSHAKE_BUCKETS_MS: [u64; 7] = [100, 250, 500, 1000, 2000, 5000, 10000];

static GATE: OnceLock<Arc<Semaphore>> = OnceLock::new();
static QUEUE_WARN_MS: AtomicU64 = AtomicU64::new(DEFAULT_QUEUE_WARN_MS);

static IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
static WAITING: AtomicU64 = AtomicU64::new(0);
static TOTAL_ACQUIRED: AtomicU64 = AtomicU64::new(0);
static TOTAL_TIMED_OUT: AtomicU64 = AtomicU64::new(0);
static TOTAL_WAIT_MS: AtomicU64 = AtomicU64::new(0);
static TOTAL_HANDSHAKE_MS: AtomicU64 = AtomicU64::new(0);
static WAIT_COUNTS: OnceLock<Vec<AtomicU64>> = OnceLock::new();
static HANDSHAKE_COUNTS: OnceLock<Vec<AtomicU64>> = OnceLock::new();

fn wait_counts() -> &'static [AtomicU64] {
    WAIT_COUNTS.get_or_init(|| {
        (0..WAIT_BUCKETS_MS.len() + 1)
            .map(|_| AtomicU64::new(0))
            .collect()
    })
}

fn handshake_counts() -> &'static [AtomicU64] {
    HANDSHAKE_COUNTS.get_or_init(|| {
        (0..HANDSHAKE_BUCKETS_MS.len() + 1)
            .map(|_| AtomicU64::new(0))
            .collect()
    })
}

fn gate() -> Arc<Semaphore> {
    Arc::clone(GATE.get_or_init(|| Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT))))
}

/// Apply `lavalink.server.voice.*`. Call once at startup before serving; later
/// calls are ignored because permits may already be checked out.
pub fn configure(config: &VoiceConfig) {
    QUEUE_WARN_MS.store(config.queue_warn_ms.max(1_000), Ordering::Relaxed);
    let _ = GATE.set(Arc::new(Semaphore::new(
        config.max_concurrent_handshakes.max(1),
    )));
}

fn observe(counts: &[AtomicU64], buckets: &[u64], value_ms: u64) {
    let slot = buckets
        .iter()
        .position(|b| value_ms <= *b)
        .unwrap_or(counts.len() - 1);
    counts[slot].fetch_add(1, Ordering::Relaxed);
}

/// A held handshake slot. Records handshake duration on drop.
pub struct HandshakePermit {
    _permit: OwnedSemaphorePermit,
    started: Instant,
}

impl Drop for HandshakePermit {
    fn drop(&mut self) {
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        TOTAL_HANDSHAKE_MS.fetch_add(elapsed_ms, Ordering::Relaxed);
        observe(handshake_counts(), &HANDSHAKE_BUCKETS_MS, elapsed_ms);
        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Snapshot of gate state for logs and debug endpoints.
#[derive(Debug, Clone, Copy)]
pub struct VoiceGateMetrics {
    pub in_flight: u64,
    pub waiting: u64,
    pub available: usize,
    pub total_acquired: u64,
    pub total_timed_out: u64,
    pub avg_wait_ms: u64,
    pub avg_handshake_ms: u64,
}

/// Acquire a handshake slot, failing fast after `queue_wait`.
///
/// For callers that must answer inline. The background storm path uses
/// [`acquire_background`] instead so a 1000-guild tail is queued, not failed.
pub async fn acquire(queue_wait: Duration) -> Result<HandshakePermit, RestError> {
    WAITING.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    let permit = tokio::time::timeout(queue_wait, gate().acquire_owned()).await;
    WAITING.fetch_sub(1, Ordering::Relaxed);
    match permit {
        Ok(Ok(permit)) => Ok(held(permit, started)),
        Ok(Err(_)) => Err(RestError::internal("voice handshake limiter closed")),
        Err(_) => {
            TOTAL_TIMED_OUT.fetch_add(1, Ordering::Relaxed);
            Err(RestError::service_unavailable(
                "voice handshake queue full; retry with backoff",
            ))
        }
    }
}

/// Acquire a handshake slot with no queue timeout, for background connects.
///
/// One fair FIFO wait: cancelling and re-queueing would lose queue position and
/// starve the tail under churn. A parked acquire holds no worker and almost no
/// memory, so a 1000-guild storm drains through the live handshakes instead of
/// failing its tail with 503s that non-retrying clients never recover from.
/// Waits past the configured warn threshold log once on acquire. Permits are
/// released by timeout-bounded handshakes, so the queue cannot stick.
pub async fn acquire_background() -> Result<HandshakePermit, RestError> {
    WAITING.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    let permit = gate().acquire_owned().await;
    WAITING.fetch_sub(1, Ordering::Relaxed);
    match permit {
        Ok(permit) => {
            let wait_ms = started.elapsed().as_millis() as u64;
            if wait_ms >= QUEUE_WARN_MS.load(Ordering::Relaxed) {
                tracing::warn!(
                    wait_ms,
                    waiting = WAITING.load(Ordering::Relaxed),
                    "voice handshake waited past the warn threshold in the background queue"
                );
            }
            Ok(held(permit, started))
        }
        Err(_) => Err(RestError::internal("voice handshake limiter closed")),
    }
}

fn held(permit: OwnedSemaphorePermit, queued_at: Instant) -> HandshakePermit {
    let wait_ms = queued_at.elapsed().as_millis() as u64;
    TOTAL_WAIT_MS.fetch_add(wait_ms, Ordering::Relaxed);
    observe(wait_counts(), &WAIT_BUCKETS_MS, wait_ms);
    TOTAL_ACQUIRED.fetch_add(1, Ordering::Relaxed);
    IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
    HandshakePermit {
        _permit: permit,
        started: Instant::now(),
    }
}

/// Current gate state.
pub fn metrics() -> VoiceGateMetrics {
    let acquired = TOTAL_ACQUIRED.load(Ordering::Relaxed);
    VoiceGateMetrics {
        in_flight: IN_FLIGHT.load(Ordering::Relaxed),
        waiting: WAITING.load(Ordering::Relaxed),
        available: gate().available_permits(),
        total_acquired: acquired,
        total_timed_out: TOTAL_TIMED_OUT.load(Ordering::Relaxed),
        avg_wait_ms: TOTAL_WAIT_MS
            .load(Ordering::Relaxed)
            .checked_div(acquired)
            .unwrap_or(0),
        avg_handshake_ms: TOTAL_HANDSHAKE_MS
            .load(Ordering::Relaxed)
            .checked_div(acquired)
            .unwrap_or(0),
    }
}

/// Histogram exposition data: bucket upper bounds, per-bucket counts, sum, total.
pub struct HistogramData {
    pub buckets: &'static [u64],
    pub counts: Vec<u64>,
    pub sum_ms: u64,
    pub total: u64,
}

/// Queue-wait duration distribution.
pub fn wait_histogram() -> HistogramData {
    HistogramData {
        buckets: &WAIT_BUCKETS_MS,
        counts: wait_counts()
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .collect(),
        sum_ms: TOTAL_WAIT_MS.load(Ordering::Relaxed),
        total: TOTAL_ACQUIRED.load(Ordering::Relaxed),
    }
}

/// Handshake-duration distribution.
pub fn handshake_histogram() -> HistogramData {
    HistogramData {
        buckets: &HANDSHAKE_BUCKETS_MS,
        counts: handshake_counts()
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .collect(),
        sum_ms: TOTAL_HANDSHAKE_MS.load(Ordering::Relaxed),
        total: TOTAL_ACQUIRED.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    // 100 concurrent acquirers must never hold more than the cap at once, even
    // though every one eventually gets a permit. Proves the gate bounds
    // handshake concurrency under a reconnect storm.
    #[tokio::test]
    async fn concurrent_acquires_never_exceed_the_cap() {
        static ACTIVE: AtomicUsize = AtomicUsize::new(0);
        static PEAK: AtomicUsize = AtomicUsize::new(0);

        let cap = gate().available_permits() + IN_FLIGHT.load(Ordering::Relaxed) as usize;
        let tasks: Vec<_> = (0..100)
            .map(|_| {
                tokio::spawn(async {
                    let _permit = acquire(Duration::from_secs(30))
                        .await
                        .expect("gate stays open");
                    let held = ACTIVE.fetch_add(1, Ordering::AcqRel) + 1;
                    PEAK.fetch_max(held, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    ACTIVE.fetch_sub(1, Ordering::AcqRel);
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        assert!(
            PEAK.load(Ordering::Relaxed) <= cap.max(1),
            "peak {} exceeded the observed cap {cap}",
            PEAK.load(Ordering::Relaxed)
        );
        assert_eq!(ACTIVE.load(Ordering::Relaxed), 0);
    }
}

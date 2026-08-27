//! The node's own resource usage, as reported by `/v4/stats` and the periodic stats event.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use perf_monitor::cpu::ProcessStat;

use crate::protocol::stats::{Cpu, FrameStats, Memory};
use crate::session::loss_counter::EXPECTED_PACKET_COUNT_PER_MIN;
use crate::session::SocketContext;

// Sampling costs a syscall per call and clients ask for stats far more often than the numbers move.
const CPU_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

static CPU: Mutex<Option<CpuState>> = Mutex::new(None);

fn cores() -> i32 {
    perf_monitor::cpu::processor_numbers()
        .map(|count| count as i32)
        .unwrap_or(1)
}

// Linux reports process memory in pages; everywhere else the counts are already bytes.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn memory_scale() -> u64 {
    static PAGE_SIZE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PAGE_SIZE.get_or_init(|| match unsafe { libc::sysconf(libc::_SC_PAGESIZE) } {
        size if size > 0 => size as u64,
        _ => 4096,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn memory_scale() -> u64 {
    1
}

fn process_memory() -> (u64, u64) {
    perf_monitor::mem::get_process_memory_info()
        .map(|info| {
            let scale = memory_scale();
            (
                info.resident_set_size * scale,
                info.virtual_memory_size * scale,
            )
        })
        .unwrap_or((0, 0))
}

// The host's RAM, so a container with a memory limit advertises a ceiling it never gets.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn total_ram() -> u64 {
    match unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) } {
        pages if pages > 0 => pages as u64 * memory_scale(),
        _ => 0,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn total_ram() -> u64 {
    0
}

/// Process memory: `used` is the resident set rather than the Rust heap, `allocated` the address
/// space held, `free` the part of that which is not resident, `reservable` the physical ceiling.
pub fn memory() -> Memory {
    let (resident, virtual_size) = process_memory();
    let allocated = virtual_size.max(resident);
    Memory {
        free: (allocated - resident) as i64,
        used: resident as i64,
        allocated: allocated as i64,
        reservable: total_ram().max(allocated) as i64,
    }
}

#[cfg(unix)]
fn system_load() -> Option<f64> {
    let mut avg = [0.0f64; 1];
    if unsafe { libc::getloadavg(avg.as_mut_ptr(), 1) } != 1 {
        return None;
    }
    Some(avg[0] / cores() as f64)
}

#[cfg(not(unix))]
fn system_load() -> Option<f64> {
    None
}

fn ratio(load: f64) -> f64 {
    if load.is_finite() {
        load.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

struct CpuState {
    process: Option<ProcessStat>,
    cached: Cpu,
    refreshed: Instant,
}

impl CpuState {
    fn new() -> Self {
        Self {
            process: ProcessStat::cur().ok(),
            cached: Cpu {
                cores: cores(),
                // Host load reads correctly from the first sample; ours needs a window to elapse.
                system_load: ratio(system_load().unwrap_or(0.0)),
                lavalink_load: 0.0,
            },
            refreshed: Instant::now(),
        }
    }

    fn refresh(&mut self) {
        let lavalink_load = self
            .process
            .as_mut()
            .and_then(|stat| stat.cpu().ok())
            .map(|usage| usage / cores() as f64)
            .unwrap_or(0.0);
        self.cached = Cpu {
            cores: cores(),
            system_load: ratio(system_load().unwrap_or(lavalink_load)),
            lavalink_load: ratio(lavalink_load),
        };
        self.refreshed = Instant::now();
    }
}

/// Host and process CPU load, resampled at most once per refresh interval.
pub fn cpu() -> Cpu {
    let mut guard = CPU.lock().unwrap();
    let state = guard.get_or_insert_with(CpuState::new);
    if state.refreshed.elapsed() >= CPU_REFRESH_INTERVAL {
        state.refresh();
    }
    state.cached
}

/// Frame stats averaged over a session's playing players, or `None` when none has a usable window.
pub fn aggregate_frame_stats(context: &Arc<SocketContext>) -> Option<FrameStats> {
    let mut players: i64 = 0;
    let mut sent: i64 = 0;
    let mut nulled: i64 = 0;
    for player in context.players() {
        if !player.is_playing() || !player.loss_counter().is_data_usable() {
            continue;
        }
        let (success, loss) = player.loss_counter().last_minute();
        players += 1;
        sent += success;
        nulled += loss;
    }

    if players == 0 {
        return None;
    }
    let deficit = players * EXPECTED_PACKET_COUNT_PER_MIN - (sent + nulled);
    Some(FrameStats {
        sent: (sent / players) as i32,
        nulled: (nulled / players) as i32,
        deficit: (deficit / players) as i32,
    })
}

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

fn resident_memory() -> u64 {
    perf_monitor::mem::get_process_memory_info()
        .map(|info| info.resident_set_size * memory_scale())
        .unwrap_or(0)
}

// What the allocator holds committed from the OS.
//
// Not the address space: mimalloc reserves arenas a gibibyte at a time and never unmaps them, so the
// virtual size ratchets up with peak concurrency and says nothing about memory in use.
fn committed_memory() -> u64 {
    let mut fields = [0usize; 8];
    let [elapsed, user, system, rss, peak_rss, commit, peak_commit, faults] = &mut fields;
    unsafe {
        libmimalloc_sys::mi_process_info(
            elapsed,
            user,
            system,
            rss,
            peak_rss,
            commit,
            peak_commit,
            faults,
        );
    }
    fields[5] as u64
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

/// Process memory: `used` is the resident set rather than the Rust heap, `allocated` what the
/// allocator holds committed, `free` the committed part that is not resident, `reservable` the
/// physical ceiling.
pub fn memory() -> Memory {
    let resident = resident_memory();
    // A commit smaller than the resident set means the resident pages the allocator does not own
    // (the binary, thread stacks) outweigh its own; clamp so `free` cannot go negative.
    let allocated = committed_memory().max(resident);
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

#[cfg(test)]
mod tests {
    use super::*;

    // The commit the allocator reports has to be real memory, not the reserved address space: the
    // arenas alone are gibibytes wide, so anything near the virtual size means `memory` went back to
    // reporting it.
    #[test]
    fn memory_reports_commit_rather_than_address_space() {
        let memory = memory();
        assert!(memory.used > 0, "a running process has a resident set");
        assert!(memory.allocated >= memory.used);
        assert_eq!(memory.free, memory.allocated - memory.used);
        assert!(memory.reservable >= memory.allocated);
        // A test binary commits single-digit MB. Two gibibytes is one arena reservation.
        assert!(
            memory.allocated < 2 * 1024 * 1024 * 1024,
            "allocated looks like address space: {}",
            memory.allocated
        );
    }
}

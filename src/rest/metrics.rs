//! The Prometheus metrics endpoint.
//!
//! Exposes the node's `lavalink_*` gauges in the Prometheus text exposition format, version 0.0.4.
//! Gated on `metrics.prometheus.enabled`, registered at `metrics.prometheus.endpoint`, and exempt
//! from auth so a scraper needs no credentials.

use std::fmt::Write;

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::node::AppState;
use crate::protocol::stats::Stats;

const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// `GET {metrics.prometheus.endpoint}`, render the `lavalink_*` gauges.
pub async fn metrics(State(state): State<AppState>) -> Response {
    let body = render(&state.build_stats(None));
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response()
}
fn render(stats: &Stats) -> String {
    let mut out = String::new();
    let memory_help = "Memory statistics in bytes.";
    let cpu_help = "CPU statistics.";

    gauge(
        &mut out,
        "lavalink_players_total",
        "Total number of players connected.",
        stats.players as f64,
    );
    gauge(
        &mut out,
        "lavalink_playing_players_total",
        "Number of players currently playing audio.",
        stats.playing_players as f64,
    );
    gauge(
        &mut out,
        "lavalink_uptime_milliseconds",
        "Uptime of the node in milliseconds.",
        stats.uptime as f64,
    );
    gauge(
        &mut out,
        "lavalink_memory_free_bytes",
        &format!("{memory_help} (Free)"),
        stats.memory.free as f64,
    );
    gauge(
        &mut out,
        "lavalink_memory_used_bytes",
        &format!("{memory_help} (Used)"),
        stats.memory.used as f64,
    );
    gauge(
        &mut out,
        "lavalink_memory_allocated_bytes",
        &format!("{memory_help} (Allocated)"),
        stats.memory.allocated as f64,
    );
    gauge(
        &mut out,
        "lavalink_memory_reservable_bytes",
        &format!("{memory_help} (Reservable)"),
        stats.memory.reservable as f64,
    );
    gauge(
        &mut out,
        "lavalink_cpu_cores",
        &format!("{cpu_help} (Cores)"),
        stats.cpu.cores as f64,
    );
    gauge(
        &mut out,
        "lavalink_cpu_system_load_percentage",
        &format!("{cpu_help} (System Load)"),
        stats.cpu.system_load,
    );
    gauge(
        &mut out,
        "lavalink_cpu_lavalink_load_percentage",
        &format!("{cpu_help} (LL Load)"),
        stats.cpu.lavalink_load,
    );
    voice_gate_metrics(&mut out);
    runtime_metrics(&mut out);
    out
}

// Voice-handshake gate families (`kairo_voice_*`). Counters are monotonic;
// the wait/handshake distributions are histograms so `histogram_quantile`
// works without the lossy lifetime averages in logs.
fn voice_gate_metrics(out: &mut String) {
    let gate = crate::session::voice_gate::metrics();
    counter(
        out,
        "kairo_voice_handshakes_total",
        "Total voice handshakes admitted by the gate.",
        gate.total_acquired as f64,
    );
    counter(
        out,
        "kairo_voice_handshake_queue_timeouts_total",
        "Foreground gate acquires that hit the queue wait (background storm path waits unbounded instead).",
        gate.total_timed_out as f64,
    );
    counter(
        out,
        "kairo_voice_handshake_success_total",
        "Background connects that committed a live connection.",
        gate.total_success as f64,
    );
    counter(
        out,
        "kairo_voice_handshake_timeout_total",
        "Background connects that exhausted retries on handshake timeout.",
        gate.total_handshake_timeout as f64,
    );
    counter(
        out,
        "kairo_voice_handshake_error_total",
        "Background connects that exhausted retries on handshake error.",
        gate.total_handshake_error as f64,
    );
    counter(
        out,
        "kairo_voice_handshake_stale_dropped_total",
        "Finished handshakes dropped because a newer PATCH claimed the guild.",
        gate.total_stale_dropped as f64,
    );
    counter(
        out,
        "kairo_voice_handshake_retries_total",
        "Retry attempts past the first handshake attempt.",
        gate.total_retries as f64,
    );
    gauge(
        out,
        "kairo_voice_handshakes_in_flight",
        "Voice handshakes currently holding a gate permit.",
        gate.in_flight as f64,
    );
    gauge(
        out,
        "kairo_voice_handshake_queue_waiting",
        "Voice connects currently parked in the gate queue.",
        gate.waiting as f64,
    );
    histogram_ms(
        out,
        "kairo_voice_queue_wait",
        "Gate queue wait distribution in milliseconds.",
        &crate::session::voice_gate::wait_histogram(),
    );
    histogram_ms(
        out,
        "kairo_voice_handshake_duration",
        "Handshake permit-hold distribution in milliseconds.",
        &crate::session::voice_gate::handshake_histogram(),
    );
}

// Tokio runtime families (`kairo_runtime_*`), stable metrics only: per-worker
// cumulative busy seconds (use `rate()` for busy %), alive tasks, global queue.
// Blocking-pool gauges need `--cfg tokio_unstable` (see `runtime_metrics`) and
// are deliberately not exported: a global rustflag would silently drop out from
// under any user with their own RUSTFLAGS set.
fn runtime_metrics(out: &mut String) {
    let m = tokio::runtime::Handle::current().metrics();
    gauge(
        out,
        "kairo_runtime_workers",
        "Tokio worker threads.",
        m.num_workers() as f64,
    );
    gauge(
        out,
        "kairo_runtime_alive_tasks",
        "Tasks currently alive in the runtime.",
        m.num_alive_tasks() as f64,
    );
    gauge(
        out,
        "kairo_runtime_global_queue_depth",
        "Tasks pending in the runtime global queue.",
        m.global_queue_depth() as f64,
    );
    // Cumulative per-worker busy time; `rate(kairo_runtime_worker_busy_seconds_total[1m])`
    // is the worker busy fraction.
    let _ = writeln!(
        out,
        "# HELP kairo_runtime_worker_busy_seconds_total Cumulative worker busy time in seconds."
    );
    let _ = writeln!(
        out,
        "# TYPE kairo_runtime_worker_busy_seconds_total counter"
    );
    for worker in 0..m.num_workers() {
        let secs = m.worker_total_busy_duration(worker).as_secs_f64();
        let _ = writeln!(
            out,
            "kairo_runtime_worker_busy_seconds_total{{worker=\"{worker}\"}} {secs}"
        );
    }
}

// Append one gauge family to `out`: its `# HELP` line, its `# TYPE` line, then the value.
fn gauge(out: &mut String, name: &str, help: &str, value: f64) {
    // `writeln!` to a String is infallible.
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} gauge");
    let _ = writeln!(out, "{name} {value}");
}

// Append one counter family.
fn counter(out: &mut String, name: &str, help: &str, value: f64) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} counter");
    let _ = writeln!(out, "{name} {value}");
}

// Append one histogram family (cumulative buckets, `+Inf`, count, sum) from
// `voice_gate` bucket data. Bucket bounds and observations are milliseconds.
fn histogram_ms(
    out: &mut String,
    name: &str,
    help: &str,
    data: &crate::session::voice_gate::HistogramData,
) {
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} histogram");
    let mut cumulative = 0u64;
    for (bound, count) in data.buckets.iter().zip(data.counts.iter()) {
        cumulative += count;
        let _ = writeln!(out, "{name}_bucket{{le=\"{bound}\"}} {cumulative}");
    }
    // Any observation past the last bound plus the explicit trailing slot.
    cumulative += data.counts.get(data.buckets.len()).copied().unwrap_or(0);
    let _ = writeln!(out, "{name}_bucket{{le=\"+Inf\"}} {cumulative}");
    let _ = writeln!(out, "{name}_count {}", data.total);
    let _ = writeln!(out, "{name}_sum {}", data.sum_ms);
}

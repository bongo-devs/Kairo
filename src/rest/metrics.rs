//! The node's `lavalink_*` gauges in the Prometheus text exposition format 0.0.4. Gated on
//! `metrics.prometheus.enabled` and exempt from auth so a scraper needs no credentials.

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
    out
}

fn gauge(out: &mut String, name: &str, help: &str, value: f64) {
    // `writeln!` to a String is infallible.
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} gauge");
    let _ = writeln!(out, "{name} {value}");
}

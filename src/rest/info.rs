//! Server info, version, and stats endpoints.

use axum::extract::State;
use axum::Json;

use crate::node::AppState;
use crate::protocol::info::{Git, Info, Version};
use crate::protocol::stats::Stats;

// The audio engine crate version, which is not readable from here at compile time.
const PLAYER_VERSION: &str = "0.1.0";

/// `GET /version`, the plain-text server version. No auth required.
pub async fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// `GET /v4/info`, server capabilities and build information.
pub async fn info(State(state): State<AppState>) -> Json<Info> {
    let filters = state.config().lavalink.server.filters.enabled();
    let rust_version = option_env!("CARGO_PKG_RUST_VERSION")
        .filter(|version| !version.is_empty())
        .unwrap_or("stable");
    Json(Info {
        version: Version::from_semver(env!("CARGO_PKG_VERSION")),
        build_time: env!("KAIRO_BUILD_TIME").parse().unwrap_or(0),
        git: Git {
            branch: env!("KAIRO_GIT_BRANCH").to_string(),
            commit: env!("KAIRO_GIT_COMMIT").to_string(),
            commit_time: env!("KAIRO_GIT_COMMIT_TIME").parse().unwrap_or(0),
        },
        jvm: format!("rust {rust_version}"),
        lavaplayer: format!("player {PLAYER_VERSION}"),
        source_managers: state.source_names(),
        filters,
        plugins: Vec::new(),
    })
}

/// `GET /v4/stats`, aggregate node statistics. Frame stats are per-session, so they are omitted.
pub async fn stats(State(state): State<AppState>) -> Json<Stats> {
    Json(state.build_stats(None))
}

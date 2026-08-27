//! The HTTP API: the router, its auth middleware, and one module per group of endpoints.
//!
//! Every route except the Prometheus endpoint requires the configured password in the
//! `Authorization` header. Failures are rendered as the v4 JSON error body by
//! [`error_body_middleware`](error::error_body_middleware).

pub mod error;
pub mod info;
pub mod lyrics;
pub mod metrics;
pub mod players;
pub mod request_log;
pub mod routeplanner;
pub mod sessions;
pub mod sponsorblock;
pub mod track_loading;
pub mod ws;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Router;

use crate::node::AppState;
use crate::rest::error::{error_body_middleware, RestError};

// Shared by the route and by `auth`, which answers a wrong password differently there.
const WEBSOCKET_PATH: &str = "/v4/websocket";

/// Build the REST and WebSocket application.
pub fn app(state: AppState) -> Router {
    let mut router = Router::new()
        .route("/version", get(info::version))
        .route("/v4/info", get(info::info))
        .route("/v4/stats", get(info::stats))
        .route("/v4/loadtracks", get(track_loading::load_tracks))
        .route("/v4/loadsearch", get(track_loading::load_search))
        .route("/v4/decodetrack", get(track_loading::decode_track))
        .route("/v4/decodetracks", post(track_loading::decode_tracks))
        .route("/v4/sessions/{sessionId}", patch(sessions::patch_session))
        .route(
            "/v4/sessions/{sessionId}/players",
            get(players::get_players),
        )
        .route(
            "/v4/sessions/{sessionId}/players/{guildId}",
            get(players::get_player)
                .patch(players::patch_player)
                .delete(players::delete_player),
        )
        .route("/v4/routeplanner/status", get(routeplanner::status))
        .route(
            "/v4/routeplanner/free/address",
            post(routeplanner::free_address),
        )
        .route("/v4/routeplanner/free/all", post(routeplanner::free_all))
        .route(
            "/v4/sessions/{sessionId}/players/{guildId}/sponsorblock/categories",
            get(sponsorblock::get_categories)
                .put(sponsorblock::set_categories)
                .delete(sponsorblock::delete_categories),
        )
        .route(WEBSOCKET_PATH, get(ws::websocket_handler));

    // Mounted only when lyrics are enabled, so a node without them answers `404` here instead of an
    // auth-gated `503`.
    if state.lyrics_service().is_some() {
        router = router
            .route("/v4/lyrics", get(lyrics::get_lyrics))
            .route(
                "/v4/sessions/{sessionId}/players/{guildId}/track/lyrics",
                get(lyrics::get_player_lyrics),
            )
            .route(
                "/v4/sessions/{sessionId}/players/{guildId}/lyrics/subscribe",
                post(lyrics::subscribe).delete(lyrics::unsubscribe),
            );
    }

    let prometheus = &state.config().metrics.prometheus;
    if prometheus.enabled {
        router = router.route(&prometheus.endpoint, get(metrics::metrics));
    }

    router
        // Innermost is auth, then the error body, so it also formats the `401`, `403`, `404` and
        // `405` the layers below produce, then the version header. Request logging goes outermost so
        // it sees every request, including the ones auth rejects.
        .layer(middleware::from_fn_with_state(state.clone(), auth))
        .layer(middleware::from_fn(error_body_middleware))
        .layer(middleware::from_fn(api_version_header))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            request_log::request_logging,
        ))
        .with_state(state)
}

// Tag every response with the protocol version the client is talking to.
async fn api_version_header(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("Lavalink-Api-Version", HeaderValue::from_static("4"));
    response
}

// Require the configured password on every route: `401` when the header is absent, `403` when it is
// wrong. The WebSocket upgrade answers `401` for both, since a client that cannot authenticate has no
// session to be told about.
async fn auth(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    // Scraping is anonymous so a metrics collector needs no credentials. Nothing else is exempt,
    // `/version` included.
    let metrics = &state.config().metrics.prometheus;
    if metrics.enabled && path == metrics.endpoint {
        return next.run(request).await;
    }
    let wrong_password_status = if path == WEBSOCKET_PATH {
        StatusCode::UNAUTHORIZED
    } else {
        StatusCode::FORBIDDEN
    };

    let password = &state.config().lavalink.server.password;
    match request
        .headers()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
    {
        None => RestError::unauthorized("Authorization header is missing").into_response(),
        Some(provided) if provided == password => next.run(request).await,
        Some(_) => {
            RestError::new(wrong_password_status, "Authorization header is invalid").into_response()
        }
    }
}

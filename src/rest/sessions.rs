//! Session configuration endpoint (`PATCH /v4/sessions/{sessionId}`).

use axum::extract::{Path, State};
use axum::Json;

use crate::node::AppState;
use crate::protocol::omissible::Omissible;
use crate::protocol::session::{Session, SessionUpdate};
use crate::rest::error::{RestError, RestResult};

// Ceiling for `timeout`, in seconds. Clients are loose about the unit and some send milliseconds, so
// an unclamped `180e3` would park a session, its players, voice connections and queued events, for
// about 50 hours. The protocol has no key for the ceiling itself.
const MAX_RESUME_TIMEOUT_SECS: i64 = 3_600;

/// `PATCH /v4/sessions/{sessionId}`, turn resuming on or off and set the resume timeout.
pub async fn patch_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(update): Json<SessionUpdate>,
) -> RestResult<Json<Session>> {
    let context = state
        .sockets()
        .get(&session_id)
        .ok_or_else(|| RestError::not_found(format!("Session {session_id} not found")))?;

    if let Omissible::Present(resuming) = update.resuming {
        context.set_resuming(resuming);
    }
    if let Omissible::Present(timeout) = update.timeout {
        let clamped = timeout.clamp(0, MAX_RESUME_TIMEOUT_SECS);
        if clamped != timeout {
            tracing::warn!(
                session = %session_id,
                requested = timeout,
                clamped,
                "resume timeout clamped"
            );
        }
        context.set_resume_timeout_secs(clamped as u64);
    }

    Ok(Json(Session {
        resuming: context.is_resuming(),
        timeout: context.resume_timeout_secs() as i64,
    }))
}

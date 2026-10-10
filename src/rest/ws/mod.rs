//! The `/v4/websocket` endpoint: the handshake and the per-session read/write halves.
//!
//! A client opens one socket per session and receives every event on it. The socket carries no
//! inbound commands, those go over REST; a session id in the handshake asks to resume an earlier one.

mod receiver;
mod sender;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::node::AppState;
use crate::protocol::message::Message;
use crate::rest::error::RestError;
use crate::session::SocketContext;

/// Outbound channel capacity. Past this the client is not draining, so player updates and stats are
/// dropped and events parked for resume (see `SocketContext::send_message`), bounding memory.
const OUTBOUND_CAPACITY: usize = 1024;

/// `GET /v4/websocket`, upgrade the connection once the handshake headers check out.
pub async fn websocket_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let user_id = headers
        .get("user-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|user_id| *user_id != 0);
    let Some(user_id) = user_id else {
        return RestError::bad_request("Missing or invalid User-Id header").into_response();
    };

    let session_id = header_string(&headers, "session-id");
    let client_name = header_string(&headers, "client-name");
    let user_agent = header_string(&headers, "user-agent");

    // `Session-Resumed` rides on the 101 response, so probe before the upgrade; the session itself
    // is only attached once the socket exists.
    let resumable = session_id
        .as_deref()
        .is_some_and(|id| state.sockets().is_attachable(id));

    let mut response = ws.on_upgrade(move |socket| {
        handle_socket(socket, state, user_id, session_id, client_name, user_agent)
    });
    response.headers_mut().insert(
        "Session-Resumed",
        if resumable {
            HeaderValue::from_static("true")
        } else {
            HeaderValue::from_static("false")
        },
    );
    response
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    user_id: u64,
    requested_session: Option<String>,
    client_name: Option<String>,
    user_agent: Option<String>,
) {
    let (sink, stream) = socket.split();
    let (sender, receiver) = mpsc::channel::<Message>(OUTBOUND_CAPACITY);

    let (context, resumed, epoch, backlog) =
        attach(&state, requested_session.as_deref(), user_id, sender);
    tracing::info!(session = %context.session_id, resumed, "websocket ready");
    if !resumed {
        log_new_connection(client_name, user_agent);
    }

    // Couple the halves: whichever ends first aborts the other, so a dead write task tears the
    // session down instead of leaving the read loop spinning while events pile up unread.
    let write_task = tokio::spawn(sender::write_loop(sink, receiver, backlog));
    let read_task = tokio::spawn(receiver::read_loop(stream, context.session_id.clone()));
    let reason = supervise(read_task, write_task).await;

    tracing::debug!(
        session = %context.session_id,
        ?reason,
        dropped_updates = context.dropped_updates(),
        "websocket closing"
    );
    on_disconnect(&state, context, epoch);
}

fn log_new_connection(client_name: Option<String>, user_agent: Option<String>) {
    match (client_name, user_agent) {
        (Some(name), _) => tracing::info!("Connection successfully established from {name}"),
        (None, agent) => {
            tracing::info!("Connection successfully established");
            match agent {
                Some(agent) => tracing::warn!(
                    "Library developers: Please specify a 'Client-Name' header. User agent: {agent}"
                ),
                None => {
                    tracing::warn!("Library developers: Please specify a 'Client-Name' header.")
                }
            }
        }
    }
}

/// Which half ended first, for the teardown log.
#[derive(Debug)]
enum Ended {
    Read,
    Write,
}

/// Run both halves until either ends, abort the other, and return which ended so teardown runs once.
async fn supervise(mut read_task: JoinHandle<()>, mut write_task: JoinHandle<()>) -> Ended {
    tokio::select! {
        _ = &mut read_task => {
            write_task.abort();
            Ended::Read
        }
        _ = &mut write_task => {
            read_task.abort();
            Ended::Write
        }
    }
}

fn attach(
    state: &AppState,
    requested_session: Option<&str>,
    user_id: u64,
    sender: mpsc::Sender<Message>,
) -> (Arc<SocketContext>, bool, u64, Vec<Message>) {
    if let Some(id) = requested_session {
        // `ready` leads the backlog so the write task flushes it ahead of the resume queue and the
        // player updates, the order a client expects on resume.
        if let Some(context) = state.sockets().take_resumable(id) {
            let (epoch, queued) = context.resume_with(sender);
            let mut backlog = Vec::with_capacity(queued.len() + 1);
            backlog.push(ready(&context.session_id, true));
            backlog.extend(queued);
            for player in context.players() {
                player.send_player_update();
            }
            return (context, true, epoch, backlog);
        }
        if let Some(context) = state.sockets().get(id) {
            let epoch = context.attach_sender(sender);
            for player in context.players() {
                player.send_player_update();
            }
            tracing::info!(session = %context.session_id, "reattached to a live session");
            let backlog = vec![ready(&context.session_id, true)];
            return (context, true, epoch, backlog);
        }
    }

    let session_id = state.sockets().generate_session_id();
    let context = SocketContext::new(
        session_id.clone(),
        user_id,
        state.manager().clone(),
        state.config().crossfade.to_engine(),
        state.lyrics_service().cloned(),
        sender,
    );
    state.sockets().insert(Arc::clone(&context));
    state.arm_session_stats(&context);
    let epoch = context.connection_epoch();
    (context, false, epoch, vec![ready(&session_id, false)])
}

fn ready(session_id: &str, resumed: bool) -> Message {
    Message::Ready {
        resumed,
        session_id: session_id.to_string(),
    }
}

fn on_disconnect(state: &AppState, context: Arc<SocketContext>, epoch: u64) {
    let session_id = context.session_id.clone();

    // A client that reconnects before its old socket finishes closing bumps the epoch; without this
    // guard the late close would park or destroy the session the new socket is already using.
    if context.connection_epoch() != epoch {
        tracing::debug!(session = %session_id, "stale socket closed; session already reattached");
        return;
    }

    if context.is_resuming() {
        let timeout = context.resume_timeout_secs();
        state.sockets().move_to_resumable(&session_id);
        context.pause();
        tracing::info!(session = %session_id, timeout, "session parked for resume");

        let state = state.clone();
        let timer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(timeout)).await;
            if let Some(context) = state.sockets().drop_resumable(&session_id) {
                tracing::info!(session = %session_id, "resume window expired; destroying session");
                context.shutdown();
            }
        });
        context.arm_resume_timeout(timer.abort_handle());
    } else {
        state.sockets().remove(&session_id);
        context.shutdown();
        tracing::info!(session = %session_id, "session closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future;

    // A dead write half used to leave the session live with events draining into nothing. Coupling
    // the halves means a finished write task ends the supervisor even while the read half blocks.
    #[tokio::test]
    async fn dead_write_half_tears_down_the_session() {
        let read = tokio::spawn(future::pending::<()>());
        let write = tokio::spawn(async {});
        let ended = tokio::time::timeout(Duration::from_secs(1), supervise(read, write))
            .await
            .expect("supervisor must return when the write half ends");
        assert!(matches!(ended, Ended::Write));
    }

    #[tokio::test]
    async fn closed_read_half_tears_down_the_session() {
        let read = tokio::spawn(async {});
        let write = tokio::spawn(future::pending::<()>());
        let ended = tokio::time::timeout(Duration::from_secs(1), supervise(read, write))
            .await
            .expect("supervisor must return when the read half ends");
        assert!(matches!(ended, Ended::Read));
    }
}

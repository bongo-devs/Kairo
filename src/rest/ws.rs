//! The `/v4/websocket` endpoint: the handshake, and the socket's read and write loops.
//!
//! A client opens one socket per session and receives every event on it. The socket carries no
//! inbound commands, those go over REST; a session id in the handshake asks to resume an earlier one.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::node::AppState;
use crate::protocol::message::Message;
use crate::rest::error::RestError;
use crate::session::SocketContext;

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

    // Probed before the upgrade: `Session-Resumed` rides on the 101 response, which is built here,
    // while the session itself is only attached once the socket exists.
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
    let (mut sink, mut stream) = socket.split();
    let (sender, mut receiver) = mpsc::unbounded_channel::<Message>();

    let (context, resumed, epoch) = attach(&state, requested_session.as_deref(), user_id, sender);
    tracing::info!(session = %context.session_id, resumed, "websocket ready");
    if !resumed {
        match (client_name, user_agent) {
            (Some(name), _) => {
                tracing::info!("Connection successfully established from {name}")
            }
            (None, agent) => {
                tracing::info!("Connection successfully established");
                match agent {
                    Some(agent) => tracing::warn!(
                        "Library developers: Please specify a 'Client-Name' header. User agent: {agent}"
                    ),
                    None => tracing::warn!(
                        "Library developers: Please specify a 'Client-Name' header."
                    ),
                }
            }
        }
    }

    let write_task = tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            let text = match serde_json::to_string(&message) {
                Ok(text) => text,
                Err(_) => continue,
            };
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(message)) = stream.next().await {
        match message {
            WsMessage::Close(_) => break,
            WsMessage::Text(_) => tracing::warn!(
                session = %context.session_id,
                "Lavalink v4 does not support websocket messages. Please use the REST api."
            ),
            _ => {}
        }
    }

    write_task.abort();
    on_disconnect(&state, context, epoch);
}

fn attach(
    state: &AppState,
    requested_session: Option<&str>,
    user_id: u64,
    sender: mpsc::UnboundedSender<Message>,
) -> (Arc<SocketContext>, bool, u64) {
    if let Some(id) = requested_session {
        // `ready` is queued before the sender is attached, so it precedes the player updates and any
        // event the session had already buffered.
        if let Some(context) = state.sockets().take_resumable(id) {
            send_ready(&sender, &context.session_id, true);
            let epoch = context.resume_with(sender);
            for player in context.players() {
                player.send_player_update();
            }
            return (context, true, epoch);
        }
        if let Some(context) = state.sockets().get(id) {
            send_ready(&sender, &context.session_id, true);
            let epoch = context.attach_sender(sender);
            for player in context.players() {
                player.send_player_update();
            }
            tracing::info!(session = %context.session_id, "reattached to a live session");
            return (context, true, epoch);
        }
    }

    let session_id = state.sockets().generate_session_id();
    send_ready(&sender, &session_id, false);
    let context = SocketContext::new(
        session_id,
        user_id,
        state.manager().clone(),
        state.config().crossfade.to_engine(),
        state.lyrics_service().cloned(),
        sender,
    );
    state.sockets().insert(Arc::clone(&context));
    state.arm_session_stats(&context);
    let epoch = context.connection_epoch();
    (context, false, epoch)
}

fn send_ready(sender: &mpsc::UnboundedSender<Message>, session_id: &str, resumed: bool) {
    let _ = sender.send(Message::Ready {
        resumed,
        session_id: session_id.to_string(),
    });
}

fn on_disconnect(state: &AppState, context: Arc<SocketContext>, epoch: u64) {
    let session_id = context.session_id.clone();

    // A client that reconnects before its old socket finishes closing bumps the epoch. Without this
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

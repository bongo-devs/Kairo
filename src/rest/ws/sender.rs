//! The socket's write half: drains the outbound channel to the client.

use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::protocol::message::Message;

const SEND_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(20);

type Sink = SplitSink<WebSocket, WsMessage>;

/// Flush the resume `backlog` in order, then stream live messages, pinging an idle client to
/// surface a half-open socket. Returns when the channel closes or a send stalls past the timeout,
/// which tears the session down.
pub(super) async fn write_loop(
    mut sink: Sink,
    mut receiver: mpsc::Receiver<Message>,
    backlog: Vec<Message>,
    session_id: String,
) {
    let mut next_ping = Instant::now() + PING_INTERVAL;
    for message in backlog {
        if !send_text(&mut sink, &message).await {
            tracing::warn!(session = %session_id, "websocket event write failed or timed out");
            return;
        }
        // A resumed session may have a large event backlog. Keep emitting heartbeats while it is
        // flushed; otherwise the read half's idle timeout can kill the socket before replay ends.
        if Instant::now() >= next_ping {
            if !send_ping(&mut sink, &session_id).await {
                return;
            }
            next_ping = Instant::now() + PING_INTERVAL;
        }
    }

    loop {
        tokio::select! {
            message = receiver.recv() => {
                let Some(message) = message else {
                    tracing::debug!(session = %session_id, "websocket outbound channel closed");
                    return;
                };
                if !send_text(&mut sink, &message).await {
                    tracing::warn!(session = %session_id, "websocket event write failed or timed out");
                    return;
                }
            }
            _ = tokio::time::sleep_until(next_ping) => {
                if !send_ping(&mut sink, &session_id).await {
                    return;
                }
                next_ping = Instant::now() + PING_INTERVAL;
            }
        }
    }
}

async fn send_ping(sink: &mut Sink, session_id: &str) -> bool {
    let ping = sink.send(WsMessage::Ping(Vec::new().into()));
    if matches!(tokio::time::timeout(SEND_TIMEOUT, ping).await, Ok(Ok(()))) {
        true
    } else {
        tracing::warn!(session = %session_id, "websocket ping write failed or timed out");
        false
    }
}

async fn send_text(sink: &mut Sink, message: &Message) -> bool {
    let Ok(text) = serde_json::to_string(message) else {
        return true; // skip an unserializable message, keep the socket
    };
    let send = sink.send(WsMessage::Text(text.into()));
    matches!(tokio::time::timeout(SEND_TIMEOUT, send).await, Ok(Ok(())))
}

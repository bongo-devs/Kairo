//! The socket's write half: drains the outbound channel to the client.

use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::stream::SplitSink;
use futures_util::SinkExt;
use tokio::sync::mpsc;

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
) {
    for message in backlog {
        if !send_text(&mut sink, &message).await {
            return;
        }
    }

    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ping.tick().await; // skip the immediate first tick

    loop {
        tokio::select! {
            message = receiver.recv() => {
                let Some(message) = message else { return };
                if !send_text(&mut sink, &message).await {
                    return;
                }
            }
            _ = ping.tick() => {
                let ping = sink.send(WsMessage::Ping(Vec::new().into()));
                if !matches!(tokio::time::timeout(SEND_TIMEOUT, ping).await, Ok(Ok(()))) {
                    return;
                }
            }
        }
    }
}

async fn send_text(sink: &mut Sink, message: &Message) -> bool {
    let Ok(text) = serde_json::to_string(message) else {
        return true; // skip an unserializable message, keep the socket
    };
    let send = sink.send(WsMessage::Text(text.into()));
    matches!(tokio::time::timeout(SEND_TIMEOUT, send).await, Ok(Ok(())))
}

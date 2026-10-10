//! The socket's read half: watches for the client closing or going silent.

use std::time::Duration;

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::stream::SplitStream;
use futures_util::StreamExt;

const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(70);

/// Read until the client closes, errors, or goes silent past the keepalive window. The v4 protocol
/// carries no inbound commands, so a healthy client's only traffic is its pong replies.
pub(super) async fn read_loop(mut stream: SplitStream<WebSocket>, session_id: String) {
    loop {
        match tokio::time::timeout(READ_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => break,
            Ok(Some(Ok(WsMessage::Text(_)))) => tracing::warn!(
                session = %session_id,
                "Kairo does not support websocket messages. Please use the REST api."
            ),
            Ok(Some(Ok(_))) => {} // ping, pong, binary: liveness only
            Err(_) => {
                tracing::warn!(session = %session_id, "websocket idle past keepalive; assuming half-open");
                break;
            }
        }
    }
}

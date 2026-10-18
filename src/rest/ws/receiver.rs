//! The socket's read half: watches for the client closing or erroring.

use axum::extract::ws::{Message as WsMessage, WebSocket};
use futures_util::stream::SplitStream;
use futures_util::StreamExt;

/// Read until the client closes or errors. No read deadline: a quiet client is healthy.
pub(super) async fn read_loop(mut stream: SplitStream<WebSocket>, session_id: String) {
    loop {
        match stream.next().await {
            Some(Ok(WsMessage::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(WsMessage::Text(_))) => tracing::warn!(
                session = %session_id,
                "Kairo does not support websocket messages. Please use the REST api."
            ),
            Some(Ok(_)) => {} // ping, pong, binary: liveness only
        }
    }
}

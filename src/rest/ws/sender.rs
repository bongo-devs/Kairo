//! The socket's write half: drains the outbound channel to the client.

use axum::extract::ws::Message as WsMessage;
use futures_util::Sink;
use futures_util::SinkExt;
use tokio::sync::mpsc;

use crate::protocol::message::Message;

// Flush the resume `backlog` in order, then stream live messages until the channel closes; a slow
// socket waits in the channel, it is never timed out.
pub(super) async fn write_loop<S>(
    mut sink: S,
    mut receiver: mpsc::UnboundedReceiver<Message>,
    backlog: Vec<Message>,
    session_id: String,
) where
    S: Sink<WsMessage> + Unpin,
{
    for message in backlog {
        if !send_text(&mut sink, &message).await {
            tracing::warn!(session = %session_id, "websocket write failed, closing the socket");
            return;
        }
    }

    while let Some(message) = receiver.recv().await {
        if !send_text(&mut sink, &message).await {
            tracing::warn!(session = %session_id, "websocket write failed, closing the socket");
            return;
        }
    }
    tracing::debug!(session = %session_id, "websocket outbound channel closed");
}

async fn send_text<S>(sink: &mut S, message: &Message) -> bool
where
    S: Sink<WsMessage> + Unpin,
{
    let Ok(text) = serde_json::to_string(message) else {
        return true; // skip an unserializable message, keep the socket
    };
    sink.send(WsMessage::Text(text.into())).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::time::Duration;

    /// A sink that records everything written, standing in for the socket.
    #[derive(Clone, Default)]
    struct RecordingSink {
        sent: Arc<Mutex<Vec<WsMessage>>>,
    }

    impl Sink<WsMessage> for RecordingSink {
        type Error = Infallible;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: WsMessage) -> Result<(), Self::Error> {
            self.sent.lock().unwrap().push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    /// A sink that refuses writes until its gate opens, standing in for a stalled TCP window.
    #[derive(Clone, Default)]
    struct StallingSink {
        sent: Arc<Mutex<Vec<WsMessage>>>,
        open: Arc<AtomicBool>,
        waker: Arc<Mutex<Option<Waker>>>,
    }

    impl StallingSink {
        fn open(&self) {
            self.open.store(true, Ordering::SeqCst);
            if let Some(waker) = self.waker.lock().unwrap().take() {
                waker.wake();
            }
        }
    }

    impl Sink<WsMessage> for StallingSink {
        type Error = Infallible;

        fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            if self.open.load(Ordering::SeqCst) {
                return Poll::Ready(Ok(()));
            }
            *self.waker.lock().unwrap() = Some(cx.waker().clone());
            Poll::Pending
        }

        fn start_send(self: Pin<&mut Self>, item: WsMessage) -> Result<(), Self::Error> {
            self.sent.lock().unwrap().push(item);
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    fn ready(session_id: &str) -> Message {
        Message::Ready {
            resumed: false,
            session_id: session_id.to_string(),
        }
    }

    fn sent_ids(sent: &Arc<Mutex<Vec<WsMessage>>>) -> Vec<String> {
        sent.lock()
            .unwrap()
            .iter()
            .map(|message| match message {
                WsMessage::Text(text) => {
                    let value: serde_json::Value = serde_json::from_str(text).unwrap();
                    value["sessionId"].as_str().unwrap().to_string()
                }
                other => panic!("unexpected frame: {other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn backlog_flushes_before_live_messages() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink = RecordingSink::default();
        let sent = sink.sent.clone();

        tx.send(ready("live")).unwrap();
        drop(tx);

        write_loop(sink, rx, vec![ready("one"), ready("two")], "s".to_string()).await;

        assert_eq!(sent_ids(&sent), ["one", "two", "live"]);
    }

    // A stalled socket must be waited on, not timed out: the message still arrives on release.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_socket_is_waited_on_not_timed_out() {
        let (tx, rx) = mpsc::unbounded_channel();
        let sink = StallingSink::default();
        let sent = sink.sent.clone();
        let gate = sink.clone();

        let write = tokio::spawn(write_loop(sink, rx, Vec::new(), "s".to_string()));
        tx.send(ready("queued")).unwrap();
        drop(tx);

        // Let the writer reach the stalled sink, then idle far past any send deadline.
        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::time::sleep(Duration::from_secs(31)).await;
        assert!(
            !write.is_finished(),
            "a slow socket must be waited on, not cut off"
        );

        gate.open();
        tokio::time::timeout(Duration::from_secs(1), write)
            .await
            .expect("the writer must deliver once the socket drains")
            .unwrap();
        assert_eq!(sent_ids(&sent), ["queued"]);
    }
}

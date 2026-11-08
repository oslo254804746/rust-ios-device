//! CLI diagnostics admit reviewed project targets only.

use tracing::{Level, Metadata};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

pub(crate) fn init(level: Level) {
    init_with_writer(level, std::io::stderr);
}

fn project_event(metadata: &Metadata<'_>, level: Level) -> bool {
    let target = metadata.target();
    let owned = ["ios", "ios_cli", "ios_core"].iter().any(|namespace| {
        target
            .strip_prefix(*namespace)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
    });
    owned && metadata.level() <= &level
}

fn init_with_writer<W>(level: Level, writer: W)
where
    W: for<'writer> tracing_subscriber::fmt::MakeWriter<'writer> + Send + Sync + 'static,
{
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
                    project_event(metadata, level)
                })),
        )
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::{
        client::IntoClientRequest, protocol::CloseFrame, Message,
    };

    const MAX_CAPTURE: usize = 256 * 1024;
    const SECRET: &str = "synthetic-private-UDID-token-device-content";

    #[derive(Default)]
    struct Capture {
        bytes: Vec<u8>,
        overflow: bool,
    }

    struct Writer(Arc<Mutex<Capture>>);

    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let mut capture = self.0.lock().unwrap();
            let available = MAX_CAPTURE - capture.bytes.len();
            let end = available.min(bytes.len());
            capture.bytes.extend_from_slice(&bytes[..end]);
            if end != bytes.len() {
                capture.overflow = true;
                return Err(io::Error::from(io::ErrorKind::OutOfMemory));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn actual_websocket_dependency_logs_cannot_leak_through_cli_subscriber() {
        const CHILD_ENV: &str = "IOS_WS_LOGGING_TEST_CHILD";
        const TEST_NAME: &str =
            "logging::tests::actual_websocket_dependency_logs_cannot_leak_through_cli_subscriber";
        let mode = std::env::var(CHILD_ENV)
            .ok()
            .filter(|_| std::env::args().any(|argument| argument == "--exact"));
        let Some(mode) = mode else {
            // Subscribers and the log facade are global. Two owned, bounded
            // child fixtures compare raw dependency logging with the actual
            // production subscriber without affecting the parallel harness.
            for mode in ["raw", "safe"] {
                let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", TEST_NAME, "--test-threads=1"])
                    .env(CHILD_ENV, mode)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .expect("spawn isolated WebSocket logging fixture");
                let result = tokio::time::timeout(Duration::from_secs(10), child.wait()).await;
                if result.is_err() {
                    let _ = child.start_kill();
                    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
                    panic!("isolated WebSocket logging fixture timed out: {mode}");
                }
                assert!(
                    result.unwrap().unwrap().success(),
                    "isolated WebSocket logging fixture failed: {mode}"
                );
            }
            return;
        };
        let safe = match mode.as_str() {
            "raw" => false,
            "safe" => true,
            _ => panic!("invalid logging fixture mode"),
        };
        let capture = Arc::new(Mutex::new(Capture::default()));
        let output = capture.clone();
        let writer = move || Writer(output.clone());
        if safe {
            init_with_writer(Level::TRACE, writer);
        } else {
            tracing_subscriber::fmt()
                .with_max_level(Level::TRACE)
                .with_writer(writer)
                .init();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut websocket = tokio_tungstenite::accept_async(stream).await.unwrap();
                assert!(matches!(websocket.next().await, Some(Ok(Message::Text(text))) if text == SECRET));
                websocket.send(Message::Text(SECRET.into())).await.unwrap();
                websocket.close(Some(CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal,
                    reason: SECRET.into(),
                })).await.unwrap();
            });
            let mut request = format!("ws://{address}/connect?udid={SECRET}")
                .into_client_request().unwrap();
            let mut authorization = format!("Bearer {SECRET}").parse::<
                tokio_tungstenite::tungstenite::http::HeaderValue
            >().unwrap();
            authorization.set_sensitive(true);
            request.headers_mut().insert("authorization", authorization);
            let (mut websocket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
            websocket.send(Message::Text(SECRET.into())).await.unwrap();
            assert!(matches!(websocket.next().await, Some(Ok(Message::Text(text))) if text == SECRET));
            assert!(matches!(websocket.next().await, Some(Ok(Message::Close(Some(close)))) if close.reason == SECRET));
            server.await.unwrap();
            // Own diagnostic phases remain visible; admission matches whole
            // namespace components, so similarly named third-party targets
            // cannot enter the formatter.
            tracing::trace!("reviewed WebSocket fixture completed");
            tracing::error!(target: "ios_core::synthetic", "reviewed core error phase");
            tracing::error!(target: "ios_core_unreviewed", secret = SECRET, "unreviewed namespace");
        }).await.expect("bounded actual WebSocket fixture");
        let capture = capture.lock().unwrap();
        assert!(!capture.overflow);
        let original = std::str::from_utf8(&capture.bytes).unwrap();
        assert!(original.contains("reviewed WebSocket fixture completed"));
        assert!(original.contains("reviewed core error phase"));
        if safe {
            assert!(!original.contains(SECRET));
            assert!(!original.contains(&hex::encode(SECRET)));
            assert!(!original.contains("unreviewed namespace"));
            assert!(!original.contains("tungstenite"));
        } else {
            assert!(original.contains(SECRET));
            assert!(original.contains("Request:"));
            assert!(original.contains(&format!("Bearer {SECRET}")));
            assert!(original.contains("Received message"));
            assert!(original.contains("Received close frame"));
        }
    }
}

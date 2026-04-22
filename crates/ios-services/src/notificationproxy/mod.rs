//! Minimal notification proxy client.
//!
//! Service: `com.apple.mobile.notification_proxy`
//! Reference: go-ios/ios/notificationproxy/notificationproxy.go

use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

pub const SERVICE_NAME: &str = "com.apple.mobile.notification_proxy";
pub const SPRINGBOARD_FINISHED_STARTUP: &str = "com.apple.springboard.finishedstartup";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    Notification(String),
    ProxyDeath,
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationProxyError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("proxy closed before notification arrived")]
    ProxyDeath,
    #[error("timed out waiting for notification")]
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProxyEvent {
    Notification(String),
    ProxyDeath,
}

#[derive(Debug)]
pub struct NotificationProxyClient<S> {
    stream: S,
    observing: HashSet<String>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> NotificationProxyClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            observing: HashSet::new(),
        }
    }

    pub async fn observe(&mut self, notification: &str) -> Result<(), NotificationProxyError> {
        if self.observing.contains(notification) {
            return Ok(());
        }

        self.send_request(NotificationProxyRequest {
            command: "ObserveNotification",
            name: Some(notification),
        })
        .await?;
        self.observing.insert(notification.to_string());
        Ok(())
    }

    pub async fn post(&mut self, notification: &str) -> Result<(), NotificationProxyError> {
        self.send_request(NotificationProxyRequest {
            command: "PostNotification",
            name: Some(notification),
        })
        .await
    }

    pub async fn wait_for(
        &mut self,
        notification: &str,
        timeout: Duration,
    ) -> Result<(), NotificationProxyError> {
        self.observe(notification).await?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(NotificationProxyError::Timeout);
            }

            let event = tokio::time::timeout(remaining, self.recv_event())
                .await
                .map_err(|_| NotificationProxyError::Timeout)??;

            match event {
                NotificationEvent::Notification(name) if name == notification => return Ok(()),
                NotificationEvent::ProxyDeath => return Err(NotificationProxyError::ProxyDeath),
                NotificationEvent::Notification(_) => {}
            }
        }
    }

    pub async fn wait_for_springboard(
        &mut self,
        timeout: Duration,
    ) -> Result<(), NotificationProxyError> {
        self.wait_for(SPRINGBOARD_FINISHED_STARTUP, timeout).await
    }

    pub async fn next_event(
        &mut self,
        timeout: Duration,
    ) -> Result<NotificationProxyEvent, NotificationProxyError> {
        let message = tokio::time::timeout(timeout, self.recv_message())
            .await
            .map_err(|_| NotificationProxyError::Timeout)??;

        match message.command.as_deref() {
            Some("RelayNotification") => message
                .name
                .map(NotificationProxyEvent::Notification)
                .ok_or_else(|| {
                    NotificationProxyError::Protocol("RelayNotification missing Name field".into())
                }),
            Some("ProxyDeath") => Ok(NotificationProxyEvent::ProxyDeath),
            other => Err(NotificationProxyError::Protocol(format!(
                "unexpected notification proxy command: {}",
                other.unwrap_or("<missing>")
            ))),
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), NotificationProxyError> {
        self.send_request(NotificationProxyRequest {
            command: "Shutdown",
            name: None,
        })
        .await
    }

    pub async fn recv_event(&mut self) -> Result<NotificationEvent, NotificationProxyError> {
        let message = self.recv_message().await?;
        match message.command.as_deref() {
            Some("RelayNotification") => Ok(NotificationEvent::Notification(
                message.name.ok_or_else(|| {
                    NotificationProxyError::Protocol("RelayNotification missing Name".to_string())
                })?,
            )),
            Some("ProxyDeath") => Ok(NotificationEvent::ProxyDeath),
            Some(other) => Err(NotificationProxyError::Protocol(format!(
                "unexpected notification proxy command: {other}"
            ))),
            None => Err(NotificationProxyError::Protocol(
                "notification proxy message missing Command".to_string(),
            )),
        }
    }

    async fn send_request(
        &mut self,
        request: NotificationProxyRequest<'_>,
    ) -> Result<(), NotificationProxyError> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &request)
            .map_err(|e| NotificationProxyError::Plist(e.to_string()))?;
        self.stream
            .write_all(&(buf.len() as u32).to_be_bytes())
            .await?;
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv_message(&mut self) -> Result<NotificationProxyMessage, NotificationProxyError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
        if len > MAX_PLIST_SIZE {
            return Err(NotificationProxyError::Protocol(format!(
                "plist length {len} exceeds max {MAX_PLIST_SIZE}"
            )));
        }
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        plist::from_bytes(&buf).map_err(|e| NotificationProxyError::Plist(e.to_string()))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct NotificationProxyRequest<'a> {
    command: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NotificationProxyMessage {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    #[derive(Default)]
    struct MockStream {
        read_buf: Vec<u8>,
        written: Vec<u8>,
        read_pos: usize,
    }

    impl MockStream {
        fn with_frames(frames: Vec<Vec<u8>>) -> Self {
            let mut read_buf = Vec::new();
            for frame in frames {
                read_buf.extend_from_slice(&(frame.len() as u32).to_be_bytes());
                read_buf.extend_from_slice(&frame);
            }
            Self {
                read_buf,
                written: Vec::new(),
                read_pos: 0,
            }
        }
    }

    impl AsyncRead for MockStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let remaining = self.read_buf.len().saturating_sub(self.read_pos);
            if remaining == 0 {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "no more test data",
                )));
            }

            let to_copy = remaining.min(buf.remaining());
            let start = self.read_pos;
            let end = start + to_copy;
            buf.put_slice(&self.read_buf[start..end]);
            self.read_pos = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    fn plist_frame(value: plist::Value) -> Vec<u8> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &value).unwrap();
        buf
    }

    #[tokio::test]
    async fn observe_encodes_notification_request() {
        let mut stream = MockStream::default();
        let mut client = NotificationProxyClient::new(&mut stream);
        client.observe("com.apple.example.ready").await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("ObserveNotification"));
        assert_eq!(dict["Name"].as_string(), Some("com.apple.example.ready"));
    }

    #[tokio::test]
    async fn post_encodes_notification_request() {
        let mut stream = MockStream::default();
        let mut client = NotificationProxyClient::new(&mut stream);
        client.post("com.apple.example.trigger").await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("PostNotification"));
        assert_eq!(dict["Name"].as_string(), Some("com.apple.example.trigger"));
    }

    #[tokio::test]
    async fn wait_for_matches_relay_notification() {
        let frame = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("RelayNotification".into()),
            ),
            (
                "Name".to_string(),
                plist::Value::String("com.apple.example.ready".into()),
            ),
        ])));
        let mut stream = MockStream::with_frames(vec![frame]);
        let mut client = NotificationProxyClient::new(&mut stream);

        client
            .wait_for("com.apple.example.ready", Duration::from_millis(100))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn recv_event_decodes_relay_notification() {
        let frame = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("RelayNotification".into()),
            ),
            (
                "Name".to_string(),
                plist::Value::String("com.apple.example.ready".into()),
            ),
        ])));
        let mut stream = MockStream::with_frames(vec![frame]);
        let mut client = NotificationProxyClient::new(&mut stream);

        let event = client.recv_event().await.unwrap();
        assert_eq!(
            event,
            NotificationEvent::Notification("com.apple.example.ready".into())
        );
    }

    #[tokio::test]
    async fn recv_event_decodes_proxy_death() {
        let frame = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Command".to_string(),
            plist::Value::String("ProxyDeath".into()),
        )])));
        let mut stream = MockStream::with_frames(vec![frame]);
        let mut client = NotificationProxyClient::new(&mut stream);

        let event = client.recv_event().await.unwrap();
        assert_eq!(event, NotificationEvent::ProxyDeath);
    }

    #[tokio::test]
    async fn wait_for_springboard_uses_expected_name() {
        let frame = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("RelayNotification".into()),
            ),
            (
                "Name".to_string(),
                plist::Value::String(SPRINGBOARD_FINISHED_STARTUP.into()),
            ),
        ])));
        let mut stream = MockStream::with_frames(vec![frame]);
        let mut client = NotificationProxyClient::new(&mut stream);

        client
            .wait_for_springboard(Duration::from_millis(100))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn next_event_returns_notification_name() {
        let frame = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("RelayNotification".into()),
            ),
            (
                "Name".to_string(),
                plist::Value::String("com.apple.example.stream".into()),
            ),
        ])));
        let mut stream = MockStream::with_frames(vec![frame]);
        let mut client = NotificationProxyClient::new(&mut stream);

        let event = client.next_event(Duration::from_millis(100)).await.unwrap();
        assert_eq!(
            event,
            NotificationProxyEvent::Notification("com.apple.example.stream".into())
        );
    }

    #[tokio::test]
    async fn next_event_maps_proxy_death() {
        let frame = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Command".to_string(),
            plist::Value::String("ProxyDeath".into()),
        )])));
        let mut stream = MockStream::with_frames(vec![frame]);
        let mut client = NotificationProxyClient::new(&mut stream);

        let event = client.next_event(Duration::from_millis(100)).await.unwrap();
        assert_eq!(event, NotificationProxyEvent::ProxyDeath);
    }

    #[tokio::test]
    async fn wait_for_times_out_when_no_notification_arrives() {
        let (client_side, _server_side) = tokio::io::duplex(1024);
        let mut client = NotificationProxyClient::new(client_side);

        let err = client
            .wait_for("com.apple.example.ready", Duration::from_millis(10))
            .await
            .unwrap_err();
        assert!(matches!(err, NotificationProxyError::Timeout));
    }
}

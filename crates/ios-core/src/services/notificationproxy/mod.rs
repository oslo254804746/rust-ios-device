//! Minimal notification proxy client.
//!
//! Service: `com.apple.mobile.notification_proxy`
//! Reference: go-ios/ios/notificationproxy/notificationproxy.go

use std::collections::HashSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

mod lifecycle;
use lifecycle::{await_notification_operation, NotificationState, SHUTDOWN_TIMEOUT};

#[cfg(test)]
mod lifecycle_tests;

pub const SERVICE_NAME: &str = "com.apple.mobile.notification_proxy";
pub const RSD_SERVICE_NAME: &str = "com.apple.mobile.notification_proxy.shim.remote";
pub const SPRINGBOARD_FINISHED_STARTUP: &str = "com.apple.springboard.finishedstartup";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    Notification(String),
    ProxyDeath,
}

service_error!(
    NotificationProxyError,
    #[error("proxy closed before notification arrived")]
    ProxyDeath,
    #[error("timed out waiting for notification")]
    Timeout,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationProxyEvent {
    Notification(String),
    ProxyDeath,
}

#[derive(Debug)]
pub struct NotificationProxyClient<S> {
    stream: S,
    observing: HashSet<String>,
    last_relay_state: Option<u64>,
    state: NotificationState,
}

impl<S: AsyncRead + AsyncWrite + Unpin> NotificationProxyClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            observing: HashSet::new(),
            last_relay_state: None,
            state: NotificationState::default(),
        }
    }

    /// Returns the 64-bit `notify_get_state()` value carried by the most recent
    /// `RelayNotification` message (sent by the secure service since iOS 27.2), if present.
    pub fn last_relay_state(&self) -> Option<u64> {
        self.last_relay_state
    }

    pub async fn observe(&mut self, notification: &str) -> Result<(), NotificationProxyError> {
        let deadline = self.deadline(None)?;
        await_notification_operation(
            deadline,
            "observe device notification",
            self.observe_inner(notification),
        )
        .await
    }

    async fn observe_inner(&mut self, notification: &str) -> Result<(), NotificationProxyError> {
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
        let deadline = self.deadline(None)?;
        await_notification_operation(
            deadline,
            "post device notification",
            self.send_request(NotificationProxyRequest {
                command: "PostNotification",
                name: Some(notification),
            }),
        )
        .await
    }

    pub async fn wait_for(
        &mut self,
        notification: &str,
        timeout: Duration,
    ) -> Result<(), NotificationProxyError> {
        let deadline = self.deadline(Some(timeout))?;
        await_notification_operation(
            deadline,
            "wait for device notification",
            self.wait_for_inner(notification, deadline),
        )
        .await
    }

    async fn wait_for_inner(
        &mut self,
        notification: &str,
        deadline: Option<Instant>,
    ) -> Result<(), NotificationProxyError> {
        self.observe_inner(notification).await?;
        loop {
            if let Some(deadline) = deadline.filter(|deadline| Instant::now() >= *deadline) {
                // Do not consume a ready unrelated frame after the total budget.
                // Poll the already due timer to retain its actual Elapsed.
                return await_notification_operation(
                    Some(deadline),
                    "wait for device notification",
                    std::future::pending(),
                )
                .await;
            }

            let event = self.recv_event_inner().await?;

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
        let deadline = self.deadline(Some(timeout))?;
        await_notification_operation(
            deadline,
            "receive device notification",
            self.next_event_inner(),
        )
        .await
    }

    async fn next_event_inner(&mut self) -> Result<NotificationProxyEvent, NotificationProxyError> {
        let message = self.recv_message().await?;

        match message.command.as_deref() {
            Some("RelayNotification") => {
                let name = message.name.ok_or_else(|| {
                    NotificationProxyError::Protocol("RelayNotification missing Name field".into())
                })?;
                self.last_relay_state = message.state;
                Ok(NotificationProxyEvent::Notification(name))
            }
            Some("ProxyDeath") => Ok(NotificationProxyEvent::ProxyDeath),
            other => Err(NotificationProxyError::Protocol(format!(
                "unexpected notification proxy command: {}",
                other.unwrap_or("<missing>")
            ))),
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), NotificationProxyError> {
        let deadline = self.deadline(Some(SHUTDOWN_TIMEOUT))?;
        await_notification_operation(
            deadline,
            "shutdown notification proxy",
            self.send_request(NotificationProxyRequest {
                command: "Shutdown",
                name: None,
            }),
        )
        .await?;
        self.invalidate();
        Ok(())
    }

    pub async fn recv_event(&mut self) -> Result<NotificationEvent, NotificationProxyError> {
        let deadline = self.deadline(None)?;
        await_notification_operation(
            deadline,
            "receive device notification",
            self.recv_event_inner(),
        )
        .await
    }

    async fn recv_event_inner(&mut self) -> Result<NotificationEvent, NotificationProxyError> {
        let message = self.recv_message().await?;
        match message.command.as_deref() {
            Some("RelayNotification") => {
                let name = message.name.ok_or_else(|| {
                    NotificationProxyError::Protocol("RelayNotification missing Name".to_string())
                })?;
                self.last_relay_state = message.state;
                Ok(NotificationEvent::Notification(name))
            }
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
        let mut guard = self.begin_frame()?;
        let result = async {
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &request)?;
            let len = u32::try_from(buf.len()).map_err(|error| {
                NotificationProxyError::Io(crate::error::context::retained_io(
                    std::io::ErrorKind::InvalidInput,
                    "notification request length",
                    error,
                ))
            })?;
            let mut stream = guard.io();
            stream.write_all(&len.to_be_bytes()).await?;
            stream.write_all(&buf).await?;
            stream.flush().await?;
            Ok(())
        }
        .await;
        guard.finish(result)
    }

    async fn recv_message(&mut self) -> Result<NotificationProxyMessage, NotificationProxyError> {
        let mut guard = self.begin_frame()?;
        let result = async {
            let mut stream = guard.io();
            let mut len_buf = [0u8; 4];
            stream.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;
            const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
            if len > MAX_PLIST_SIZE {
                return Err(NotificationProxyError::Protocol(format!(
                    "plist length {len} exceeds max {MAX_PLIST_SIZE}"
                )));
            }
            let mut buf = vec![0u8; len];
            stream.read_exact(&mut buf).await?;
            Ok(plist::from_bytes(&buf)?)
        }
        .await;
        let message: NotificationProxyMessage = guard.finish(result)?;
        if message.command.as_deref() == Some("ProxyDeath") {
            self.invalidate();
        }
        Ok(message)
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
    #[serde(default, deserialize_with = "deserialize_optional_state")]
    state: Option<u64>,
}

fn deserialize_optional_state<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<plist::Integer> = Option::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(int) => int
            .as_unsigned()
            .or_else(|| int.as_signed().map(|v| v as u64))
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid notification State integer")),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::MockStream;

    use super::*;

    #[tokio::test]
    async fn native_discovery_notification_deadlines_keep_elapsed_and_observations() {
        for wait in [false, true] {
            let (stream, _peer) = tokio::io::duplex(16 * 1024);
            let mut client = NotificationProxyClient::new(stream);
            let duration = Duration::from_millis(5);
            let error = if wait {
                client
                    .wait_for("owned-private", duration)
                    .await
                    .unwrap_err()
            } else {
                client.next_event(duration).await.unwrap_err()
            };
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
            assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
            if wait {
                assert!(client.observing.contains("owned-private"));
                let error = client
                    .wait_for("owned-private", Duration::ZERO)
                    .await
                    .unwrap_err();
                assert!(matches!(error, NotificationProxyError::Timeout));
                assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
            }
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
        assert_eq!(client.last_relay_state(), None);
    }

    #[tokio::test]
    async fn relay_notification_with_state_is_accepted_and_recorded() {
        let make_state_frame = || {
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Command".to_string(),
                    plist::Value::String("RelayNotification".into()),
                ),
                (
                    "Name".to_string(),
                    plist::Value::String("com.apple.springboard.lockstate".into()),
                ),
                ("State".to_string(), plist::Value::from(42u64)),
            ])))
        };

        let mut stream = MockStream::with_frames(vec![make_state_frame()]);
        let mut client = NotificationProxyClient::new(&mut stream);
        let event = client.next_event(Duration::from_millis(100)).await.unwrap();
        assert_eq!(
            event,
            NotificationProxyEvent::Notification("com.apple.springboard.lockstate".into())
        );
        assert_eq!(client.last_relay_state(), Some(42));

        let mut stream = MockStream::with_frames(vec![make_state_frame()]);
        let mut client = NotificationProxyClient::new(&mut stream);
        client
            .wait_for(
                "com.apple.springboard.lockstate",
                Duration::from_millis(100),
            )
            .await
            .unwrap();
        assert_eq!(client.last_relay_state(), Some(42));
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
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&err));
    }
}

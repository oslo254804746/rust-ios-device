//! Darwin notifications avoid AUTONOTIFY's eager resolution of rich content.

use std::future::Future;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::notificationproxy::{
    NotificationEvent, NotificationProxyClient, NotificationProxyError,
};

use super::{
    DataInclusionPolicy, PasteboardError, PasteboardSnapshot, XpcValue,
    PASTEBOARD_CHANGED_NOTIFICATION, REQUEST_TIMEOUT,
};

/// Reads snapshots after Darwin pasteboard notifications. `read` must open a
/// fresh pasteboard connection for every call: the daemon supports one reply
/// per connection. Dropping the monitor closes its notification connection.
///
/// The initial snapshot establishes a baseline and is not emitted as a change.
/// Idle waits have no deadline; registration and each snapshot read are bounded.
/// After a read/transport error or cancellation, discard and recreate the monitor.
pub struct PasteboardMonitor<S, F> {
    notifications: NotificationProxyClient<S>,
    read: F,
    policy: DataInclusionPolicy,
    last_change: Option<(XpcValue, i64)>,
    ready: bool,
}

impl<S, F, Fut> PasteboardMonitor<S, F>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut(DataInclusionPolicy) -> Fut,
    Fut: Future<Output = Result<PasteboardSnapshot, PasteboardError>>,
{
    /// Register before taking the baseline so a concurrent copy is not lost.
    /// `PromiseSecondary` is recommended; resolving every representation can
    /// stall the device daemon for rich copies from applications such as Notes.
    pub async fn start(
        stream: S,
        read: F,
        policy: DataInclusionPolicy,
    ) -> Result<Self, PasteboardError> {
        policy.validate()?;
        let mut monitor = Self {
            notifications: NotificationProxyClient::new(stream),
            read,
            policy,
            last_change: None,
            ready: false,
        };
        tokio::time::timeout(REQUEST_TIMEOUT, async {
            monitor
                .notifications
                .observe(PASTEBOARD_CHANGED_NOTIFICATION)
                .await?;
            let baseline = (monitor.read)(DataInclusionPolicy::AllPromised).await?;
            monitor.remember(&baseline);
            Ok::<_, PasteboardError>(())
        })
        .await
        .map_err(|_| PasteboardError::Timeout {
            seconds: REQUEST_TIMEOUT.as_secs(),
        })??;
        monitor.ready = true;
        Ok(monitor)
    }

    /// Wait for a new `(nonce, changeCount)` and return the bounded snapshot.
    /// Unrelated notifications and repeated counters within one nonce are skipped.
    pub async fn next_change(&mut self) -> Result<PasteboardSnapshot, PasteboardError> {
        if !self.ready {
            return Err(PasteboardError::Closed);
        }
        // Fail closed if this future is cancelled partway through a plist frame.
        self.ready = false;
        loop {
            match self.notifications.recv_event().await? {
                NotificationEvent::Notification(name)
                    if name == PASTEBOARD_CHANGED_NOTIFICATION => {}
                NotificationEvent::Notification(_) => continue,
                NotificationEvent::ProxyDeath => {
                    return Err(NotificationProxyError::ProxyDeath.into())
                }
            }
            let snapshot = tokio::time::timeout(REQUEST_TIMEOUT, (self.read)(self.policy))
                .await
                .map_err(|_| PasteboardError::Timeout {
                    seconds: REQUEST_TIMEOUT.as_secs(),
                })??;
            let duplicate = snapshot.change_id().is_some_and(|(nonce, count)| {
                self.last_change
                    .as_ref()
                    .is_some_and(|(old_nonce, old_count)| nonce == old_nonce && count == *old_count)
            });
            if duplicate {
                continue;
            }
            self.remember(&snapshot);
            self.ready = true;
            return Ok(snapshot);
        }
    }

    fn remember(&mut self, snapshot: &PasteboardSnapshot) {
        self.last_change = snapshot
            .change_id()
            .map(|(nonce, count)| (nonce.clone(), count));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use indexmap::IndexMap;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn snapshot(nonce: &str, count: i64) -> PasteboardSnapshot {
        PasteboardSnapshot::from_xpc(&XpcValue::Dictionary(IndexMap::from([
            ("command".into(), XpcValue::String("PULL_REPLY".into())),
            (
                "metadata".into(),
                XpcValue::Dictionary(IndexMap::from([
                    ("nonce".into(), XpcValue::String(nonce.into())),
                    ("changeCount".into(), XpcValue::Int64(count)),
                ])),
            ),
        ])))
        .unwrap()
    }

    async fn notify(stream: &mut tokio::io::DuplexStream, command: &str, name: &str) {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Command".to_owned(), plist::Value::String(command.into())),
            ("Name".to_owned(), plist::Value::String(name.into())),
        ]));
        let mut bytes = Vec::new();
        value.to_writer_xml(&mut bytes).unwrap();
        stream.write_u32(bytes.len() as u32).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
    }

    #[tokio::test]
    async fn monitor_registers_before_baseline_and_deduplicates_nonce_and_count() {
        let (client, mut server) = duplex(4096);
        let mut snapshots = VecDeque::from([
            snapshot("one", 4),
            snapshot("one", 4),
            snapshot("one", 5),
            snapshot("two", 5),
        ]);
        let policies = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&policies);
        let mut monitor = PasteboardMonitor::start(
            client,
            move |policy| {
                observed.lock().unwrap().push(policy);
                std::future::ready(Ok(snapshots.pop_front().unwrap()))
            },
            DataInclusionPolicy::PromiseSecondary,
        )
        .await
        .unwrap();

        let size = server.read_u32().await.unwrap();
        let mut request = vec![0; size as usize];
        server.read_exact(&mut request).await.unwrap();
        let request: plist::Dictionary = plist::from_bytes(&request).unwrap();
        assert_eq!(request["Command"].as_string(), Some("ObserveNotification"));
        assert_eq!(
            request["Name"].as_string(),
            Some(PASTEBOARD_CHANGED_NOTIFICATION)
        );

        notify(&mut server, "RelayNotification", "unrelated").await;
        for _ in 0..3 {
            notify(
                &mut server,
                "RelayNotification",
                PASTEBOARD_CHANGED_NOTIFICATION,
            )
            .await;
        }
        assert_eq!(monitor.next_change().await.unwrap().change_count, Some(5));
        assert_eq!(
            monitor.next_change().await.unwrap().change_id(),
            snapshot("two", 5).change_id()
        );
        assert_eq!(
            *policies.lock().unwrap(),
            [
                DataInclusionPolicy::AllPromised,
                DataInclusionPolicy::PromiseSecondary,
                DataInclusionPolicy::PromiseSecondary,
                DataInclusionPolicy::PromiseSecondary,
            ]
        );
        notify(&mut server, "ProxyDeath", "").await;
        assert!(matches!(
            monitor.next_change().await,
            Err(PasteboardError::Notification(
                NotificationProxyError::ProxyDeath
            ))
        ));
        assert!(matches!(
            monitor.next_change().await,
            Err(PasteboardError::Closed)
        ));
    }

    #[tokio::test]
    async fn idle_monitor_waits_and_cancellation_prevents_partial_frame_reuse() {
        let (client, mut server) = duplex(4096);
        let mut monitor = PasteboardMonitor::start(
            client,
            |_| std::future::ready(Ok(snapshot("one", 1))),
            DataInclusionPolicy::PromiseSecondary,
        )
        .await
        .unwrap();
        // Deliver only part of the next frame header, then cancel the read.
        server.write_all(&[0, 0]).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), monitor.next_change())
                .await
                .is_err()
        );
        assert!(matches!(
            monitor.next_change().await,
            Err(PasteboardError::Closed)
        ));
    }

    #[tokio::test]
    async fn snapshot_failure_is_not_reported_as_a_change() {
        let (client, mut server) = duplex(4096);
        let mut first = true;
        let mut monitor = PasteboardMonitor::start(
            client,
            move |_| {
                let result = if first {
                    first = false;
                    Ok(snapshot("one", 1))
                } else {
                    Err(PasteboardError::Protocol("fixture failure".into()))
                };
                std::future::ready(result)
            },
            DataInclusionPolicy::PromiseSecondary,
        )
        .await
        .unwrap();
        notify(
            &mut server,
            "RelayNotification",
            PASTEBOARD_CHANGED_NOTIFICATION,
        )
        .await;
        assert!(matches!(
            monitor.next_change().await,
            Err(PasteboardError::Protocol(_))
        ));
        assert!(matches!(
            monitor.next_change().await,
            Err(PasteboardError::Closed)
        ));
    }
}

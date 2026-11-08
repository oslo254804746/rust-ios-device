use std::time::Duration;

use anyhow::{Context, Result};
use ios_core::notificationproxy::NotificationEvent;
use ios_core::TunMode;
use ios_core::{connect, ConnectOptions};

#[derive(clap::Args)]
pub struct NotificationCmd {
    #[command(subcommand)]
    sub: NotificationSub,
}

#[derive(clap::Subcommand)]
enum NotificationSub {
    /// Post a single notification by name
    Post { notification: String },
    /// Observe, post, and wait for a notification on the same connection
    Roundtrip {
        notification: String,
        #[arg(long, default_value = "5", help = "Timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Observe repeated notifications by name until timeout or count is reached
    Observe {
        notification: String,
        #[arg(long, help = "Stop after receiving this many notifications")]
        count: Option<u64>,
        #[arg(long, default_value = "300", help = "Timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Wait for a single notification by name
    Wait {
        notification: String,
        #[arg(long, default_value = "300", help = "Timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
    /// Stream matching notifications until interrupted or limits are reached
    Stream {
        #[arg(required = true)]
        notifications: Vec<String>,
        #[arg(long, help = "Overall timeout in seconds")]
        timeout: Option<u64>,
        #[arg(long, help = "Stop after receiving this many notifications")]
        count: Option<u64>,
        #[arg(short = 'j', long, help = "Output JSON lines")]
        json: bool,
    },
    /// Wait until SpringBoard finishes startup
    Springboard {
        #[arg(long, default_value = "300", help = "Timeout in seconds")]
        timeout: u64,
        #[arg(short = 'j', long, help = "Output JSON")]
        json: bool,
    },
}

mod control;

impl NotificationCmd {
    pub(crate) fn preflight(&self) -> Result<()> {
        control::preflight(&self.sub)
    }

    pub async fn run(self, udid: Option<String>) -> Result<()> {
        self.preflight()?;
        let mut signal = Box::pin(tokio::signal::ctrl_c());
        let (device, stream) = control::setup(
            control::SETUP_BUDGET,
            async {
                let udid = match udid {
                    Some(udid) => udid,
                    None => {
                        let devices = ios_core::list_devices()
                            .await
                            .context("failed to list connected iOS devices for default --udid")?;
                        crate::default_udid_from_devices(&devices)
                            .ok_or_else(|| anyhow::anyhow!("--udid required for notify"))?
                    }
                };
                let opts = ConnectOptions {
                    tun_mode: TunMode::Userspace,
                    pair_record_path: None,
                    skip_tunnel: true,
                };
                let device = connect(&udid, opts).await?;
                let stream = device
                    .connect_service(ios_core::notificationproxy::SERVICE_NAME)
                    .await?;
                Ok((device, stream))
            },
            signal.as_mut(),
        )
        .await?;
        let mut client = ios_core::notificationproxy::NotificationProxyClient::new(stream);
        let result = control::capture(
            &mut client,
            self.sub,
            |line| {
                crate::output::write_line(format_args!("{line}"))?;
                Ok(())
            },
            signal.as_mut(),
        )
        .await;
        // Keep the device owner through command cleanup as well as service use.
        drop(device);
        result
    }
}

async fn finish_notification_session<S>(
    client: &mut ios_core::notificationproxy::NotificationProxyClient<S>,
    operation: Result<()>,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // A partial/failed frame or ProxyDeath cannot safely receive another wire
    // command. The enclosing command still owns and drops that transport.
    let cleanup = if client.is_usable() {
        client
            .shutdown()
            .await
            .map_err(|error| anyhow::Error::new(error).context("notification proxy cleanup failed"))
    } else {
        Ok(())
    };
    match (operation, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(NotificationCleanupFailure { primary, cleanup }.into()),
    }
}

struct NotificationCleanupFailure {
    primary: anyhow::Error,
    cleanup: anyhow::Error,
}

impl std::fmt::Display for NotificationCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("notification operation failed; cleanup also failed")
    }
}

impl std::fmt::Debug for NotificationCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationCleanupFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field(
                "cleanup_class",
                &ios_core::classify_error(self.cleanup.as_ref()),
            )
            .finish_non_exhaustive()
    }
}

impl std::error::Error for NotificationCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

#[cfg(test)]
mod lifecycle_tests;

async fn next_stream_notification<S>(
    client: &mut ios_core::notificationproxy::NotificationProxyClient<S>,
    remaining: Option<Duration>,
) -> Result<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let event = match remaining {
        Some(remaining) => tokio::time::timeout(remaining, client.recv_event())
            .await
            .map_err(|elapsed| {
                anyhow::Error::new(elapsed).context("timed out waiting for notification")
            })??,
        None => client.recv_event().await?,
    };
    match event {
        NotificationEvent::Notification(name) => Ok(name),
        NotificationEvent::ProxyDeath => Err(anyhow::Error::new(
            ios_core::notificationproxy::NotificationProxyError::ProxyDeath,
        )
        .context("notification proxy closed")),
    }
}

fn render_notification_output(
    notification: &str,
    json: bool,
    index: Option<u64>,
) -> Result<String> {
    if json {
        let mut obj = serde_json::Map::new();
        if let Some(index) = index {
            obj.insert("index".to_string(), serde_json::Value::from(index));
        }
        obj.insert(
            "notification".to_string(),
            serde_json::Value::String(notification.to_string()),
        );
        Ok(serde_json::to_string(&serde_json::Value::Object(obj))?)
    } else if let Some(index) = index {
        Ok(format!("Received notification {index}: {notification}"))
    } else {
        Ok(format!("Received notification: {notification}"))
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[tokio::test]
    async fn stream_contract_notifications_preserve_record_death_and_deadline() {
        use tokio::io::AsyncWriteExt;
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = ios_core::notificationproxy::NotificationProxyClient::new(stream);
        let name = "owned-private-notification\n\"";
        for (command, name) in [("RelayNotification", Some(name)), ("ProxyDeath", None)] {
            let mut reply = plist::Dictionary::new();
            reply.insert("Command".into(), plist::Value::String(command.into()));
            if let Some(name) = name {
                reply.insert("Name".into(), plist::Value::String(name.into()));
            }
            let mut payload = Vec::new();
            plist::to_writer_xml(&mut payload, &reply).unwrap();
            peer.write_all(&(payload.len() as u32).to_be_bytes())
                .await
                .unwrap();
            peer.write_all(&payload).await.unwrap();
        }
        let received = next_stream_notification(&mut client, Some(Duration::from_secs(1)))
            .await
            .unwrap();
        assert_eq!(received, name);
        let record = render_notification_output(&received, true, Some(1)).unwrap();
        assert!(!record.contains('\n'));
        let record: serde_json::Value = serde_json::from_str(&record).unwrap();
        assert_eq!(record["notification"], name);
        let error = next_stream_notification(&mut client, Some(Duration::from_secs(1)))
            .await
            .unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Disconnected
        );
        assert!(matches!(
            error.downcast_ref::<ios_core::notificationproxy::NotificationProxyError>(),
            Some(ios_core::notificationproxy::NotificationProxyError::ProxyDeath)
        ));
        let envelope = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "notify",
            &error,
        )
        .unwrap();
        assert!(!envelope.contains("owned-private"));
        assert!(!client.is_usable());
        let rejected = next_stream_notification(&mut client, None)
            .await
            .unwrap_err();
        assert!(matches!(
            rejected.downcast_ref::<ios_core::notificationproxy::NotificationProxyError>(),
            Some(ios_core::notificationproxy::NotificationProxyError::Protocol(_))
        ));
        drop(client);
        drop(peer);
        // ProxyDeath now closes tracked reuse. Preserve the original issued
        // idle-timeout/native-EOF assertions on a fresh owned connection.
        let (stream, peer) = tokio::io::duplex(4096);
        let mut client = ios_core::notificationproxy::NotificationProxyClient::new(stream);
        let elapsed = next_stream_notification(&mut client, Some(Duration::ZERO))
            .await
            .unwrap_err();
        assert_eq!(
            ios_core::classify_error(elapsed.as_ref()),
            ios_core::ErrorClass::Timeout
        );
        assert!(elapsed
            .chain()
            .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
        drop(peer);
        let eof = next_stream_notification(&mut client, None)
            .await
            .unwrap_err();
        assert_eq!(
            ios_core::classify_error(eof.as_ref()),
            ios_core::ErrorClass::Io
        );
        let io = eof
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: NotificationSub,
    }

    #[test]
    fn parses_post_subcommand() {
        let cmd = TestCli::parse_from(["notify", "post", "com.apple.example.ready"]);
        match cmd.command {
            NotificationSub::Post { notification } => {
                assert_eq!(notification, "com.apple.example.ready");
            }
            NotificationSub::Roundtrip { .. }
            | NotificationSub::Observe { .. }
            | NotificationSub::Wait { .. }
            | NotificationSub::Stream { .. }
            | NotificationSub::Springboard { .. } => panic!("expected post subcommand"),
        }
    }

    #[test]
    fn parses_roundtrip_subcommand() {
        let cmd = TestCli::parse_from([
            "notify",
            "roundtrip",
            "com.apple.example.ready",
            "--timeout",
            "7",
            "--json",
        ]);
        match cmd.command {
            NotificationSub::Roundtrip {
                notification,
                timeout,
                json,
            } => {
                assert_eq!(notification, "com.apple.example.ready");
                assert_eq!(timeout, 7);
                assert!(json);
            }
            NotificationSub::Post { .. }
            | NotificationSub::Observe { .. }
            | NotificationSub::Wait { .. }
            | NotificationSub::Stream { .. }
            | NotificationSub::Springboard { .. } => panic!("expected roundtrip subcommand"),
        }
    }

    #[test]
    fn parses_wait_subcommand() {
        let cmd = TestCli::parse_from([
            "notify",
            "wait",
            "com.apple.example.ready",
            "--timeout",
            "5",
            "--json",
        ]);
        match cmd.command {
            NotificationSub::Wait {
                notification,
                timeout,
                json,
            } => {
                assert_eq!(notification, "com.apple.example.ready");
                assert_eq!(timeout, 5);
                assert!(json);
            }
            _ => panic!("expected wait subcommand"),
        }
    }

    #[test]
    fn parses_stream_subcommand() {
        let cmd = TestCli::parse_from([
            "notify",
            "stream",
            "com.apple.example.ready",
            "com.apple.example.done",
            "--timeout",
            "5",
            "--count",
            "2",
            "--json",
        ]);
        match cmd.command {
            NotificationSub::Stream {
                notifications,
                timeout,
                count,
                json,
            } => {
                assert_eq!(
                    notifications,
                    vec![
                        "com.apple.example.ready".to_string(),
                        "com.apple.example.done".to_string()
                    ]
                );
                assert_eq!(timeout, Some(5));
                assert_eq!(count, Some(2));
                assert!(json);
            }
            _ => panic!("expected stream subcommand"),
        }
    }

    #[test]
    fn parses_observe_subcommand() {
        let cmd = TestCli::parse_from([
            "notify",
            "observe",
            "com.apple.example.ready",
            "--count",
            "2",
            "--timeout",
            "5",
            "--json",
        ]);
        match cmd.command {
            NotificationSub::Observe {
                notification,
                count,
                timeout,
                json,
            } => {
                assert_eq!(notification, "com.apple.example.ready");
                assert_eq!(count, Some(2));
                assert_eq!(timeout, 5);
                assert!(json);
            }
            _ => panic!("expected observe subcommand"),
        }
    }

    #[test]
    fn parses_springboard_subcommand() {
        let cmd = TestCli::parse_from(["notify", "springboard", "--timeout", "9", "--json"]);
        match cmd.command {
            NotificationSub::Springboard { timeout, json } => {
                assert_eq!(timeout, 9);
                assert!(json);
            }
            _ => panic!("expected springboard subcommand"),
        }
    }

    #[test]
    fn formats_notification_json_object() {
        let rendered =
            render_notification_output("com.apple.example.ready", true, Some(2)).unwrap();
        assert_eq!(
            rendered,
            r#"{"index":2,"notification":"com.apple.example.ready"}"#
        );
    }
}

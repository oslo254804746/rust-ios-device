use std::{future::Future, time::Duration};

use ios_core::notificationproxy::{
    NotificationEvent, NotificationProxyClient, NotificationProxyError,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::*;

const BOUND: Duration = Duration::from_secs(1);
const SHORT: Duration = Duration::from_millis(20);

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, future)
        .await
        .expect("owned fixture stalled")
}

fn frame(command: &str, name: Option<&str>) -> Vec<u8> {
    let mut value = plist::Dictionary::new();
    value.insert("Command".into(), plist::Value::String(command.into()));
    if let Some(name) = name {
        value.insert("Name".into(), plist::Value::String(name.into()));
    }
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &value).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

async fn request(peer: &mut DuplexStream) -> plist::Dictionary {
    let len = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(len < 4096);
    let mut body = vec![0; len];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    plist::from_bytes(&body).unwrap()
}

fn permission_primary() -> anyhow::Error {
    anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "owned-private-primary-device-name",
    ))
    .context("owned-private-context")
}

fn has_elapsed(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>())
}

#[tokio::test]
async fn lifecycle_notification_cli_success_attempts_exact_shutdown_and_closes() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    bounded(client.post("owned")).await.unwrap();
    let posted = request(&mut peer).await;
    assert_eq!(posted["Command"].as_string(), Some("PostNotification"));
    assert_eq!(posted["Name"].as_string(), Some("owned"));
    bounded(finish_notification_session(&mut client, Ok(())))
        .await
        .unwrap();
    let shutdown = request(&mut peer).await;
    assert_eq!(shutdown.len(), 1);
    assert_eq!(shutdown["Command"].as_string(), Some("Shutdown"));
    assert!(!client.is_usable());
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_notification_cli_primary_survives_successful_cleanup_unchanged() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    bounded(client.observe("owned")).await.unwrap();
    request(&mut peer).await;
    let primary = permission_primary();
    let error = bounded(finish_notification_session(&mut client, Err(primary)))
        .await
        .unwrap_err();
    assert!(error.downcast_ref::<NotificationCleanupFailure>().is_none());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(error
        .chain()
        .any(|source| source.to_string() == "owned-private-context"));
    assert_eq!(
        request(&mut peer).await["Command"].as_string(),
        Some("Shutdown")
    );
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_cli_success_cleanup_native_failure_is_reported() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    bounded(client.post("owned")).await.unwrap();
    request(&mut peer).await;
    drop(peer);
    let error = bounded(finish_notification_session(&mut client, Ok(())))
        .await
        .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
    assert!(
        matches!(error.downcast_ref::<NotificationProxyError>(), Some(NotificationProxyError::Io(io)) if io.kind() == std::io::ErrorKind::BrokenPipe)
    );
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_cli_dual_native_failure_preserves_primary_and_safe_output() {
    let (stream, peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    drop(peer);
    let error = bounded(finish_notification_session(
        &mut client,
        Err(permission_primary()),
    ))
    .await
    .unwrap_err();
    let failure = error.downcast_ref::<NotificationCleanupFailure>().unwrap();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert_eq!(
        failure
            .primary
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(
        matches!(failure.cleanup.downcast_ref::<NotificationProxyError>(), Some(NotificationProxyError::Io(io)) if io.kind() == std::io::ErrorKind::BrokenPipe)
    );
    let displayed = format!("{failure}");
    let debug = format!("{failure:?}");
    assert!(!displayed.contains("owned-private"));
    assert!(!debug.contains("owned-private"));
    let output = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "notify",
        &error,
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["error"]["class"], "permission_denied");
    assert!(!output.contains("owned-private"));
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_cli_dual_cleanup_expiry_keeps_primary_and_separate_elapsed() {
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut client = NotificationProxyClient::new(stream);
    client.set_operation_timeout(Some(SHORT));
    let error = bounded(finish_notification_session(
        &mut client,
        Err(permission_primary()),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(!has_elapsed(&error));
    let failure = error.downcast_ref::<NotificationCleanupFailure>().unwrap();
    assert!(has_elapsed(&failure.cleanup));
    assert_eq!(
        ios_core::classify_error(failure.cleanup.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(!client.is_usable());
    let mut sent = [0; 64];
    bounded(peer.read_exact(&mut sent)).await.unwrap();
    assert_eq!(sent.as_slice(), &frame("Shutdown", None)[..64]);
}

#[tokio::test]
async fn lifecycle_notification_cli_success_actual_default_cleanup_expiry_is_reported() {
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut client = NotificationProxyClient::new(stream);
    let error = tokio::time::timeout(
        Duration::from_secs(6),
        finish_notification_session(&mut client, Ok(())),
    )
    .await
    .expect("default cleanup cap stalled")
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(has_elapsed(&error));
    assert!(error.downcast_ref::<NotificationCleanupFailure>().is_none());
    assert!(!client.is_usable());
    let mut sent = [0; 64];
    bounded(peer.read_exact(&mut sent)).await.unwrap();
    assert_eq!(sent.as_slice(), &frame("Shutdown", None)[..64]);
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_notification_cli_partial_read_failure_skips_unsafe_late_shutdown() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    let bytes = frame("RelayNotification", Some("owned"));
    bounded(peer.write_all(&bytes[..2])).await.unwrap();
    let primary = bounded(client.next_event(SHORT)).await.unwrap_err();
    assert!(!client.is_usable());
    bounded(peer.write_all(&bytes[2..])).await.unwrap();
    let error = bounded(finish_notification_session(
        &mut client,
        Err(primary.into()),
    ))
    .await
    .unwrap_err();
    assert!(has_elapsed(&error));
    assert!(error.downcast_ref::<NotificationCleanupFailure>().is_none());
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_notification_cli_proxy_death_keeps_original_error_without_cleanup_write() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    bounded(peer.write_all(&frame("ProxyDeath", None)))
        .await
        .unwrap();
    assert_eq!(
        bounded(client.recv_event()).await.unwrap(),
        NotificationEvent::ProxyDeath
    );
    let error = bounded(finish_notification_session(
        &mut client,
        Err(NotificationProxyError::ProxyDeath.into()),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
    assert!(error.downcast_ref::<NotificationCleanupFailure>().is_none());
    assert!(matches!(
        error.downcast_ref::<NotificationProxyError>(),
        Some(NotificationProxyError::ProxyDeath)
    ));
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
}

#[tokio::test]
async fn lifecycle_notification_cli_cancel_drops_owned_transport_without_detached_cleanup() {
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut capture = Box::pin(async move {
        let mut client = NotificationProxyClient::new(stream);
        finish_notification_session(&mut client, Ok(())).await
    });
    let mut sent = [0; 64];
    bounded(async {
        tokio::select! {
            result = &mut capture => panic!("cleanup completed before prefix: {result:?}"),
            result = peer.read_exact(&mut sent) => { result.unwrap(); }
        }
    })
    .await;
    assert_eq!(sent.as_slice(), &frame("Shutdown", None)[..64]);
    drop(capture);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

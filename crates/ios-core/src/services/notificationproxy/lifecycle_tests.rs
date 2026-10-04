use std::{future::Future, time::Duration};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufStream, DuplexStream};

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
    value_frame(&plist::Value::Dictionary(value))
}

fn value_frame(value: &plist::Value) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, value).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

async fn request(peer: &mut DuplexStream) -> plist::Dictionary {
    let size = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(size < 16 * 1024);
    let mut body = vec![0; size];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    plist::from_bytes(&body).unwrap()
}

fn issued_timer(error: &NotificationProxyError) {
    assert_eq!(crate::classify_error(error), crate::ErrorClass::Timeout);
    assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(error));
}

async fn reject_all<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    client: &mut NotificationProxyClient<S>,
) {
    assert!(!client.is_usable());
    assert!(client.observing.is_empty());
    let errors = [
        bounded(client.observe("owned")).await.unwrap_err(),
        bounded(client.post("owned")).await.unwrap_err(),
        bounded(client.wait_for("owned", BOUND)).await.unwrap_err(),
        bounded(client.wait_for_springboard(BOUND))
            .await
            .unwrap_err(),
        bounded(client.next_event(BOUND)).await.unwrap_err(),
        bounded(client.recv_event()).await.unwrap_err(),
        bounded(client.shutdown()).await.unwrap_err(),
    ];
    assert!(errors
        .into_iter()
        .all(|e| matches!(e, NotificationProxyError::Protocol(_))));
}

#[tokio::test]
async fn lifecycle_notification_preflight_zero_overflow_and_unpolled_do_no_io() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    drop(client.observe("owned"));
    drop(client.post("owned"));
    drop(client.wait_for("owned", BOUND));
    drop(client.wait_for_springboard(BOUND));
    drop(client.next_event(BOUND));
    drop(client.recv_event());
    drop(client.shutdown());
    client.set_operation_timeout(Some(Duration::ZERO));
    let errors = [
        client.observe("owned").await.unwrap_err(),
        client.post("owned").await.unwrap_err(),
        client.wait_for("owned", BOUND).await.unwrap_err(),
        client.wait_for_springboard(BOUND).await.unwrap_err(),
        client.next_event(BOUND).await.unwrap_err(),
        client.recv_event().await.unwrap_err(),
        client.shutdown().await.unwrap_err(),
    ];
    for error in errors {
        assert!(matches!(error, NotificationProxyError::Timeout));
        assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
    }
    client.set_operation_timeout(None);
    for error in [
        client.wait_for("owned", Duration::ZERO).await.unwrap_err(),
        client
            .wait_for_springboard(Duration::ZERO)
            .await
            .unwrap_err(),
        client.next_event(Duration::ZERO).await.unwrap_err(),
    ] {
        assert!(matches!(error, NotificationProxyError::Timeout));
    }
    client.set_operation_timeout(Some(Duration::MAX));
    for error in [
        client.observe("owned").await.unwrap_err(),
        client.post("owned").await.unwrap_err(),
        client.recv_event().await.unwrap_err(),
    ] {
        assert!(
            matches!(error, NotificationProxyError::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidInput)
        );
    }
    client.set_operation_timeout(None);
    for error in [
        client.wait_for("owned", Duration::MAX).await.unwrap_err(),
        client
            .wait_for_springboard(Duration::MAX)
            .await
            .unwrap_err(),
        client.next_event(Duration::MAX).await.unwrap_err(),
    ] {
        assert!(
            matches!(error, NotificationProxyError::Io(ref e) if e.kind() == std::io::ErrorKind::InvalidInput)
        );
    }
    assert!(client.is_usable());
    assert!(client.observing.is_empty());
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    bounded(client.observe("owned")).await.unwrap();
    let req = request(&mut peer).await;
    assert_eq!(req["Command"].as_string(), Some("ObserveNotification"));
    assert_eq!(req["Name"].as_string(), Some("owned"));
}

#[tokio::test]
async fn lifecycle_notification_empty_idle_retains_subscription_and_reuses_late_event() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    bounded(client.observe("owned")).await.unwrap();
    request(&mut peer).await;
    {
        let mut pending = Box::pin(client.recv_event());
        assert!(tokio::time::timeout(SHORT, &mut pending).await.is_err());
    }
    issued_timer(&bounded(client.next_event(SHORT)).await.unwrap_err());
    issued_timer(&bounded(client.wait_for("owned", SHORT)).await.unwrap_err());
    assert!(client.is_usable());
    assert!(client.observing.contains("owned"));
    bounded(client.observe("owned")).await.unwrap();
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
        .await
        .unwrap();
    assert_eq!(
        bounded(client.recv_event()).await.unwrap(),
        NotificationEvent::Notification("owned".into())
    );
    assert!(client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_fragmented_cancel_or_expiry_rejects_unread_late_frame() {
    let bytes = frame("RelayNotification", Some("owned"));
    for prefix in [2, 4, 6] {
        for expiry in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = NotificationProxyClient::new(stream);
            bounded(client.observe("owned")).await.unwrap();
            request(&mut peer).await;
            bounded(peer.write_all(&bytes[..prefix])).await.unwrap();
            if expiry {
                issued_timer(&bounded(client.next_event(SHORT)).await.unwrap_err());
            } else {
                let mut pending = Box::pin(client.recv_event());
                assert!(tokio::time::timeout(SHORT, &mut pending).await.is_err());
            }
            bounded(peer.write_all(&bytes[prefix..])).await.unwrap();
            reject_all(&mut client).await;
            let mut late = vec![0; bytes.len() - prefix];
            bounded(client.stream.read_exact(&mut late)).await.unwrap();
            assert_eq!(late, bytes[prefix..]);
            assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
        }
    }
}

#[tokio::test]
async fn lifecycle_notification_buffered_header_commits_partial_receive_ownership() {
    let bytes = frame("RelayNotification", Some("owned"));
    let (stream, mut peer) = tokio::io::duplex(4096);
    bounded(peer.write_all(&bytes[..4])).await.unwrap();
    let mut buffered = BufStream::new(stream);
    assert_eq!(bounded(buffered.fill_buf()).await.unwrap(), &bytes[..4]);
    let mut client = NotificationProxyClient::new(buffered);
    issued_timer(&bounded(client.next_event(SHORT)).await.unwrap_err());
    bounded(peer.write_all(&bytes[4..])).await.unwrap();
    reject_all(&mut client).await;
    let mut late = vec![0; bytes.len() - 4];
    bounded(client.stream.read_exact(&mut late)).await.unwrap();
    assert_eq!(late, bytes[4..]);
}

#[tokio::test]
async fn lifecycle_notification_empty_and_partial_send_have_distinct_reuse_policy() {
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut client = NotificationProxyClient::new(stream);
    bounded(client.stream.write_all(&[0x5a; 64])).await.unwrap();
    client.set_operation_timeout(Some(SHORT));
    issued_timer(&bounded(client.post("owned")).await.unwrap_err());
    assert!(client.is_usable());
    let mut seed = [0; 64];
    bounded(peer.read_exact(&mut seed)).await.unwrap();
    assert_eq!(seed, [0x5a; 64]);
    let name = "owned".repeat(100);
    issued_timer(&bounded(client.post(&name)).await.unwrap_err());
    reject_all(&mut client).await;
    let mut sent = [0; 64];
    bounded(peer.read_exact(&mut sent)).await.unwrap();
    assert_eq!(
        sent.as_slice(),
        &frame("PostNotification", Some(&name))[..64]
    );
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
}

#[tokio::test]
async fn lifecycle_notification_wait_budget_includes_observe_without_abandoning_subscription() {
    let (stream, mut peer) = tokio::io::duplex(512);
    let mut client = NotificationProxyClient::new(stream);
    bounded(client.stream.write_all(&[0x5a; 512]))
        .await
        .unwrap();
    let (result, ()) = tokio::join!(
        client.wait_for("owned", Duration::from_millis(200)),
        async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            let mut seed = [0; 512];
            bounded(peer.read_exact(&mut seed)).await.unwrap();
            assert_eq!(seed, [0x5a; 512]);
            let req = request(&mut peer).await;
            assert_eq!(req["Command"].as_string(), Some("ObserveNotification"));
            assert_eq!(req["Name"].as_string(), Some("owned"));
            tokio::time::sleep(Duration::from_millis(125)).await;
            bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
                .await
                .unwrap();
        }
    );
    issued_timer(&result.unwrap_err());
    assert!(client.is_usable());
    assert!(client.observing.contains("owned"));
    assert_eq!(
        bounded(client.recv_event()).await.unwrap(),
        NotificationEvent::Notification("owned".into())
    );
}

#[tokio::test]
async fn lifecycle_notification_cancel_after_complete_observe_keeps_subscription() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    {
        let mut waiting = Box::pin(client.wait_for("owned", BOUND));
        let req = bounded(async {
            tokio::select! {
                result = &mut waiting => panic!("wait ended before fixture reply: {result:?}"),
                req = request(&mut peer) => req,
            }
        })
        .await;
        assert_eq!(req["Name"].as_string(), Some("owned"));
    }
    assert!(client.is_usable());
    assert!(client.observing.contains("owned"));
    bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
        .await
        .unwrap();
    bounded(client.wait_for("owned", BOUND)).await.unwrap();
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
}

#[tokio::test]
async fn lifecycle_notification_complete_business_errors_reuse_but_proxy_death_closes() {
    for next in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = NotificationProxyClient::new(stream);
        for bytes in [
            frame("Unknown", None),
            value_frame(&plist::Value::Dictionary(plist::Dictionary::new())),
            frame("RelayNotification", None),
        ] {
            bounded(peer.write_all(&bytes)).await.unwrap();
            let error = if next {
                bounded(client.next_event(BOUND)).await.unwrap_err()
            } else {
                bounded(client.recv_event()).await.unwrap_err()
            };
            assert!(matches!(error, NotificationProxyError::Protocol(_)));
            assert!(client.is_usable());
        }
        bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
            .await
            .unwrap();
        assert_eq!(
            bounded(client.recv_event()).await.unwrap(),
            NotificationEvent::Notification("owned".into())
        );
        bounded(peer.write_all(&frame("ProxyDeath", None)))
            .await
            .unwrap();
        if next {
            assert_eq!(
                bounded(client.next_event(BOUND)).await.unwrap(),
                NotificationProxyEvent::ProxyDeath
            );
        } else {
            assert_eq!(
                bounded(client.recv_event()).await.unwrap(),
                NotificationEvent::ProxyDeath
            );
        }
        reject_all(&mut client).await;
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    bounded(peer.write_all(&frame("ProxyDeath", None)))
        .await
        .unwrap();
    assert!(matches!(
        bounded(client.wait_for("owned", BOUND)).await.unwrap_err(),
        NotificationProxyError::ProxyDeath
    ));
    reject_all(&mut client).await;
}

#[tokio::test]
async fn lifecycle_notification_native_eof_write_failure_and_typed_codec_errors_invalidate() {
    let (stream, peer) = tokio::io::duplex(4096);
    drop(peer);
    let mut client = NotificationProxyClient::new(stream);
    let error = bounded(client.recv_event()).await.unwrap_err();
    assert!(
        matches!(error, NotificationProxyError::Io(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    reject_all(&mut client).await;
    let (stream, peer) = tokio::io::duplex(4096);
    drop(peer);
    let mut client = NotificationProxyClient::new(stream);
    let error = bounded(client.observe("owned")).await.unwrap_err();
    assert!(
        matches!(error, NotificationProxyError::Io(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe)
    );
    reject_all(&mut client).await;
    let wrong_name = plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "Command".to_string(),
            plist::Value::String("RelayNotification".into()),
        ),
        ("Name".to_string(), plist::Value::from(1u64)),
    ]));
    for (bytes, codec) in [
        ([4u32.to_be_bytes().as_slice(), b"junk"].concat(), true),
        ((4u32 * 1024 * 1024 + 1).to_be_bytes().to_vec(), false),
        (value_frame(&wrong_name), true),
    ] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = NotificationProxyClient::new(stream);
        bounded(peer.write_all(&bytes)).await.unwrap();
        let error = bounded(client.recv_event()).await.unwrap_err();
        if codec {
            assert!(matches!(error, NotificationProxyError::Plist(_)));
        } else {
            assert!(matches!(error, NotificationProxyError::Protocol(_)));
        }
        reject_all(&mut client).await;
    }
}

#[tokio::test]
async fn lifecycle_notification_shutdown_wire_max_clamp_and_terminal_rejection() {
    for budget in [None, Some(Duration::MAX), Some(BOUND)] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = NotificationProxyClient::new(stream);
        bounded(client.observe("owned")).await.unwrap();
        request(&mut peer).await;
        client.set_operation_timeout(budget);
        bounded(client.shutdown()).await.unwrap();
        let req = request(&mut peer).await;
        assert_eq!(req.len(), 1);
        assert_eq!(req["Command"].as_string(), Some("Shutdown"));
        reject_all(&mut client).await;
        assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_notification_shutdown_empty_partial_cancel_expiry_and_actual_default_cap() {
    for partial in [false, true] {
        for expiry in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(64);
            let mut client = NotificationProxyClient::new(stream);
            if !partial {
                bounded(client.stream.write_all(&[0x5a; 64])).await.unwrap();
            }
            if expiry {
                client.set_operation_timeout(Some(SHORT));
                issued_timer(&bounded(client.shutdown()).await.unwrap_err());
            } else {
                let mut pending = Box::pin(client.shutdown());
                assert!(tokio::time::timeout(SHORT, &mut pending).await.is_err());
            }
            assert_eq!(client.is_usable(), !partial);
            let mut sent = [0; 64];
            bounded(peer.read_exact(&mut sent)).await.unwrap();
            if partial {
                assert_eq!(sent.as_slice(), &frame("Shutdown", None)[..64]);
                reject_all(&mut client).await;
            } else {
                assert_eq!(sent, [0x5a; 64]);
            }
        }
    }
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut client = NotificationProxyClient::new(stream);
    let error = tokio::time::timeout(Duration::from_secs(6), client.shutdown())
        .await
        .expect("default shutdown cap stalled")
        .unwrap_err();
    issued_timer(&error);
    reject_all(&mut client).await;
    let mut sent = [0; 64];
    bounded(peer.read_exact(&mut sent)).await.unwrap();
    assert_eq!(sent.as_slice(), &frame("Shutdown", None)[..64]);
}

#[tokio::test]
async fn lifecycle_notification_unrelated_events_do_not_restart_configured_wait_budget() {
    let (stream, mut peer) = tokio::io::duplex(32 * 1024);
    let mut client = NotificationProxyClient::new(stream);
    client.set_operation_timeout(Some(Duration::from_millis(100)));
    let (result, ()) = tokio::join!(client.wait_for("owned", BOUND), async {
        request(&mut peer).await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            bounded(peer.write_all(&frame("RelayNotification", Some("other"))))
                .await
                .unwrap();
        }
    });
    issued_timer(&result.unwrap_err());
    assert!(client.is_usable());
    assert!(client.observing.contains("owned"));
    client.set_operation_timeout(None);
    assert_eq!(
        bounded(client.recv_event()).await.unwrap(),
        NotificationEvent::Notification("other".into())
    );
}

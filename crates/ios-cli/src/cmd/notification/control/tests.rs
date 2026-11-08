use std::{
    future::Future,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use ios_core::notificationproxy::NotificationProxyClient;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::oneshot,
};

use super::*;

const BOUND: Duration = Duration::from_secs(2);
const SHORT: Duration = Duration::from_millis(20);

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, future)
        .await
        .expect("owned fixture stalled")
}

fn no_signal() -> impl Future<Output = std::io::Result<()>> {
    std::future::pending()
}

fn stream_sub(timeout: Option<u64>, count: Option<u64>) -> NotificationSub {
    NotificationSub::Stream {
        notifications: vec!["owned".into()],
        timeout,
        count,
        json: true,
    }
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

async fn shutdown(peer: &mut DuplexStream) {
    let req = request(peer).await;
    assert_eq!(req.len(), 1);
    assert_eq!(req["Command"].as_string(), Some("Shutdown"));
}

fn issued_timer(error: &anyhow::Error) {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
}

#[test]
fn lifecycle_notification_cli_control_preflight_rejects_zero_and_overflow_with_idle_policy() {
    for timeout in [0, u64::MAX] {
        for sub in [
            NotificationSub::Roundtrip {
                notification: "owned".into(),
                timeout,
                json: false,
            },
            NotificationSub::Observe {
                notification: "owned".into(),
                count: None,
                timeout,
                json: false,
            },
            NotificationSub::Wait {
                notification: "owned".into(),
                timeout,
                json: false,
            },
            NotificationSub::Springboard {
                timeout,
                json: false,
            },
            stream_sub(Some(timeout), None),
        ] {
            let error = preflight(&sub).unwrap_err();
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
            assert!(!error
                .chain()
                .any(|source| source.is::<tokio::time::error::Elapsed>()));
        }
    }
    assert!(checked_deadline(Instant::now(), Duration::MAX).is_err());
    preflight(&NotificationSub::Post {
        notification: "owned".into(),
    })
    .unwrap();
    preflight(&stream_sub(None, None)).unwrap();
    let observe = NotificationSub::Observe {
        notification: "owned".into(),
        count: None,
        timeout: 1,
        json: false,
    };
    preflight(&observe).unwrap();
    assert_eq!(total_budget(&observe), None);
    assert_eq!(total_budget(&stream_sub(None, None)), None);
    assert_eq!(
        total_budget(&stream_sub(Some(2), None)),
        Some(Duration::from_secs(2))
    );
}

#[tokio::test]
async fn lifecycle_notification_cli_control_setup_admission_and_unpolled_do_not_start() {
    let polled = Arc::new(AtomicUsize::new(0));
    for budget in [Duration::ZERO, Duration::MAX] {
        let observed = polled.clone();
        let error = setup(
            budget,
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            no_signal(),
        )
        .await
        .unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::InvalidInput
        );
    }
    let observed = polled.clone();
    drop(setup(
        BOUND,
        async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        no_signal(),
    ));
    assert_eq!(polled.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_notification_cli_control_setup_uses_one_absolute_budget_across_stages() {
    let stages = Arc::new(AtomicUsize::new(0));
    let observed = stages.clone();
    let error = bounded(setup(
        Duration::from_millis(200),
        async move {
            tokio::time::sleep(Duration::from_millis(125)).await;
            observed.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(125)).await;
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        no_signal(),
    ))
    .await
    .unwrap_err();
    issued_timer(&error);
    assert_eq!(stages.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert_eq!(stages.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_notification_cli_control_setup_signal_priority_preserves_native_or_cancelled() {
    let polled = Arc::new(AtomicUsize::new(0));
    for failure in [false, true] {
        let observed = polled.clone();
        let signal = std::future::ready(if failure {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "owned-private-signal",
            ))
        } else {
            Ok(())
        });
        let error = bounded(setup(
            BOUND,
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            signal,
        ))
        .await
        .unwrap_err();
        if failure {
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::PermissionDenied
            );
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        } else {
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Cancelled
            );
            assert!(error
                .chain()
                .any(|source| source.is::<ios_core::OperationCancelled>()));
        }
    }
    assert_eq!(polled.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_notification_cli_control_setup_expiry_or_caller_drop_releases_owned_stream() {
    for expiry in [false, true] {
        let (mut stream, mut peer) = tokio::io::duplex(64);
        let mut pending = Box::pin(setup(
            if expiry { SHORT } else { BOUND },
            async move {
                stream.write_all(&[0x5a; 64]).await?;
                std::future::pending::<Result<()>>().await
            },
            no_signal(),
        ));
        let mut prefix = [0; 64];
        bounded(async {
            tokio::select! {
                result = &mut pending => panic!("setup ended before wire prefix: {result:?}"),
                result = peer.read_exact(&mut prefix) => { result.unwrap(); }
            }
        })
        .await;
        assert_eq!(prefix, [0x5a; 64]);
        if expiry {
            issued_timer(&bounded(&mut pending).await.unwrap_err());
        }
        drop(pending);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_notification_cli_control_send_expiry_distinguishes_empty_and_partial_progress() {
    for partial in [false, true] {
        let (mut stream, mut peer) = tokio::io::duplex(64);
        if !partial {
            bounded(stream.write_all(&[0x5a; 64])).await.unwrap();
        }
        let mut client = NotificationProxyClient::new(stream);
        let error = bounded(send_stage(SHORT, async { Ok(client.post("owned").await?) }))
            .await
            .unwrap_err();
        issued_timer(&error);
        assert_eq!(client.is_usable(), !partial);
        let mut prefix = [0; 64];
        bounded(peer.read_exact(&mut prefix)).await.unwrap();
        if partial {
            assert_eq!(
                prefix.as_slice(),
                &frame("PostNotification", Some("owned"))[..64]
            );
        } else {
            assert_eq!(prefix, [0x5a; 64]);
        }
    }
}

#[tokio::test]
async fn lifecycle_notification_cli_control_roundtrip_total_includes_slow_observe_and_post() {
    let (mut stream, mut peer) = tokio::io::duplex(512);
    bounded(stream.write_all(&[0x5a; 512])).await.unwrap();
    let mut client = NotificationProxyClient::new(stream);
    let mut records = Vec::new();
    let sub = NotificationSub::Roundtrip {
        notification: "owned".into(),
        timeout: 1,
        json: true,
    };
    let (result, ()) = bounded(async {
        tokio::join!(
            capture(
                &mut client,
                sub,
                |line| {
                    records.push(line.to_string());
                    Ok(())
                },
                no_signal()
            ),
            async {
                tokio::time::sleep(Duration::from_millis(650)).await;
                let mut seed = [0; 512];
                bounded(peer.read_exact(&mut seed)).await.unwrap();
                assert_eq!(seed, [0x5a; 512]);
                assert_eq!(
                    request(&mut peer).await["Command"].as_string(),
                    Some("ObserveNotification")
                );
                assert_eq!(
                    request(&mut peer).await["Command"].as_string(),
                    Some("PostNotification")
                );
                tokio::time::sleep(Duration::from_millis(650)).await;
                bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
                    .await
                    .unwrap();
                shutdown(&mut peer).await;
            }
        )
    })
    .await;
    issued_timer(&result.unwrap_err());
    assert!(records.is_empty());
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_cli_control_stream_total_starts_before_all_subscriptions() {
    let (mut stream, mut peer) = tokio::io::duplex(512);
    bounded(stream.write_all(&[0x5a; 512])).await.unwrap();
    let mut client = NotificationProxyClient::new(stream);
    let mut records = Vec::new();
    let sub = NotificationSub::Stream {
        notifications: vec!["owned".into(), "other".into()],
        timeout: Some(1),
        count: Some(1),
        json: true,
    };
    let (result, ()) = bounded(async {
        tokio::join!(
            capture(
                &mut client,
                sub,
                |line| {
                    records.push(line.to_string());
                    Ok(())
                },
                no_signal()
            ),
            async {
                tokio::time::sleep(Duration::from_millis(650)).await;
                let mut seed = [0; 512];
                bounded(peer.read_exact(&mut seed)).await.unwrap();
                assert_eq!(seed, [0x5a; 512]);
                for name in ["owned", "other"] {
                    let req = request(&mut peer).await;
                    assert_eq!(req["Command"].as_string(), Some("ObserveNotification"));
                    assert_eq!(req["Name"].as_string(), Some(name));
                }
                tokio::time::sleep(Duration::from_millis(650)).await;
                bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
                    .await
                    .unwrap();
                shutdown(&mut peer).await;
            }
        )
    })
    .await;
    result.unwrap();
    assert!(records.is_empty());
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_cli_control_observe_keeps_per_event_idle_and_exact_json_bytes() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    let mut output = Vec::new();
    let sub = NotificationSub::Observe {
        notification: "owned".into(),
        count: Some(1),
        timeout: 1,
        json: true,
    };
    let (result, ()) = bounded(async {
        tokio::join!(
            capture(
                &mut client,
                sub,
                |line| {
                    crate::output::write_line_to(&mut output, format_args!("{line}"))?;
                    Ok(())
                },
                no_signal()
            ),
            async {
                assert_eq!(
                    request(&mut peer).await["Command"].as_string(),
                    Some("ObserveNotification")
                );
                for name in ["other", "owned"] {
                    tokio::time::sleep(Duration::from_millis(650)).await;
                    bounded(peer.write_all(&frame("RelayNotification", Some(name))))
                        .await
                        .unwrap();
                }
                shutdown(&mut peer).await;
            }
        )
    })
    .await;
    result.unwrap();
    assert_eq!(output, b"{\"index\":1,\"notification\":\"owned\"}\n");
}

#[tokio::test]
async fn lifecycle_notification_cli_control_infinite_stream_stops_only_on_owned_signal() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    let (tx, rx) = oneshot::channel();
    let mut emitted = 0;
    let (result, ()) = bounded(async {
        tokio::join!(
            capture(
                &mut client,
                stream_sub(None, None),
                |_| {
                    emitted += 1;
                    Ok(())
                },
                async { rx.await.expect("signal dropped") }
            ),
            async {
                request(&mut peer).await;
                tokio::time::sleep(Duration::from_millis(125)).await;
                tx.send(Ok(())).unwrap();
                shutdown(&mut peer).await;
            }
        )
    })
    .await;
    result.unwrap();
    assert_eq!(emitted, 0);
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_notification_cli_control_signal_is_preserved_between_setup_and_capture() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, rx) = oneshot::channel();
    let mut signal = Box::pin(async { rx.await.expect("signal dropped") });
    let mut client = bounded(setup(
        BOUND,
        async { Ok(NotificationProxyClient::new(stream)) },
        signal.as_mut(),
    ))
    .await
    .unwrap();
    tx.send(Ok(())).unwrap();
    let mut emitted = 0;
    bounded(capture(
        &mut client,
        stream_sub(None, None),
        |_| {
            emitted += 1;
            Ok(())
        },
        signal.as_mut(),
    ))
    .await
    .unwrap();
    assert_eq!(emitted, 0);
    // A reset signal future would lose this queued stop and send Observe first.
    shutdown(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_notification_cli_control_due_deadline_or_ready_stop_precedes_ready_wire() {
    for expiry in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
            .await
            .unwrap();
        let mut client = NotificationProxyClient::new(stream);
        let mut emitted = 0;
        let signal = async move {
            if expiry {
                std::future::pending().await
            } else {
                Ok(())
            }
        };
        let deadline = expiry.then(|| Instant::now() - Duration::from_secs(1));
        bounded(controlled_at(
            &mut client,
            stream_sub(None, None),
            |_| {
                emitted += 1;
                Ok(())
            },
            signal,
            deadline,
        ))
        .await
        .unwrap();
        assert_eq!(emitted, 0);
        assert!(client.is_usable());
        assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
        bounded(finish_notification_session(&mut client, Ok(())))
            .await
            .unwrap();
        shutdown(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_notification_cli_control_continuously_ready_events_yield_to_stop() {
    let (stream, mut peer) = tokio::io::duplex(32 * 1024);
    for _ in 0..20 {
        bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
            .await
            .unwrap();
    }
    let mut client = NotificationProxyClient::new(stream);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let signal = std::future::poll_fn(move |_| {
        if observed.load(Ordering::SeqCst) >= 3 {
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Pending
        }
    });
    bounded(capture(
        &mut client,
        stream_sub(None, None),
        |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        signal,
    ))
    .await
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    request(&mut peer).await;
    shutdown(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_notification_cli_control_idle_stop_closes_but_partial_stop_sends_no_late_cleanup(
) {
    for partial in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = NotificationProxyClient::new(stream);
        let (tx, rx) = oneshot::channel();
        let mut emitted = 0;
        let (result, ()) = bounded(async {
            tokio::join!(
                capture(
                    &mut client,
                    stream_sub(None, None),
                    |_| {
                        emitted += 1;
                        Ok(())
                    },
                    async { rx.await.expect("signal dropped") }
                ),
                async {
                    request(&mut peer).await;
                    if partial {
                        bounded(peer.write_all(&frame("RelayNotification", Some("owned"))[..2]))
                            .await
                            .unwrap();
                    }
                    tokio::time::sleep(SHORT).await;
                    tx.send(Ok(())).unwrap();
                    if partial {
                        assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
                    } else {
                        shutdown(&mut peer).await;
                    }
                }
            )
        })
        .await;
        result.unwrap();
        assert_eq!(emitted, 0);
        assert!(!client.is_usable());
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_notification_cli_control_finite_signal_is_typed_cancelled_and_cleans_up() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    let mut emitted = 0;
    let error = bounded(capture(
        &mut client,
        NotificationSub::Post {
            notification: "owned".into(),
        },
        |_| {
            emitted += 1;
            Ok(())
        },
        std::future::ready(Ok(())),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(error
        .chain()
        .any(|source| source.is::<ios_core::OperationCancelled>()));
    assert_eq!(emitted, 0);
    shutdown(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_notification_cli_control_native_signal_fault_retains_primary_with_cleanup_failure(
) {
    let (stream, peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    drop(peer);
    let error = bounded(capture(
        &mut client,
        stream_sub(None, None),
        |_| panic!("unexpected output"),
        std::future::ready(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "owned-private-signal",
        ))),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    let failure = error
        .downcast_ref::<super::super::NotificationCleanupFailure>()
        .unwrap();
    assert_eq!(
        failure
            .primary
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        ios_core::classify_error(failure.cleanup.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
    assert!(!format!("{failure:?}").contains("owned-private"));
}

#[tokio::test]
async fn lifecycle_notification_cli_control_injected_writer_fault_runs_returned_error_cleanup() {
    struct FaultWriter;
    impl std::io::Write for FaultWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "owned-private-writer",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for dual in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
            .await
            .unwrap();
        let mut client = NotificationProxyClient::new(stream);
        let mut peer = Some(peer);
        let error = bounded(capture(
            &mut client,
            NotificationSub::Wait {
                notification: "owned".into(),
                timeout: 1,
                json: true,
            },
            |line| {
                if dual {
                    drop(peer.take());
                }
                crate::output::write_line_to(&mut FaultWriter, format_args!("{line}"))?;
                Ok(())
            },
            no_signal(),
        ))
        .await
        .unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::PermissionDenied
        );
        if dual {
            let failure = error
                .downcast_ref::<super::super::NotificationCleanupFailure>()
                .unwrap();
            assert_eq!(
                failure
                    .primary
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            );
            assert_eq!(
                ios_core::classify_error(failure.cleanup.as_ref()),
                ios_core::ErrorClass::Disconnected
            );
        } else {
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::PermissionDenied
            );
            let peer = peer.as_mut().unwrap();
            request(peer).await;
            shutdown(peer).await;
        }
        assert!(!client.is_usable());
    }
}

#[tokio::test]
async fn lifecycle_notification_cli_control_caller_drop_closes_owned_listener_without_shutdown() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut listening = Box::pin(async move {
        let mut client = NotificationProxyClient::new(stream);
        capture(
            &mut client,
            stream_sub(None, None),
            |_| panic!("unexpected output"),
            no_signal(),
        )
        .await
    });
    bounded(async {
        tokio::select! {
            result = &mut listening => panic!("listener ended before observe: {result:?}"),
            req = request(&mut peer) => { assert_eq!(req["Command"].as_string(), Some("ObserveNotification")); }
        }
    }).await;
    bounded(peer.write_all(&frame("RelayNotification", Some("owned"))[..2]))
        .await
        .unwrap();
    assert!(tokio::time::timeout(SHORT, &mut listening).await.is_err());
    drop(listening);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_notification_cli_control_zero_count_keeps_original_observe_and_stream_policy() {
    for stream_mode in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        bounded(peer.write_all(&frame("RelayNotification", Some("owned"))))
            .await
            .unwrap();
        let mut client = NotificationProxyClient::new(stream);
        let mut records = Vec::new();
        let sub = if stream_mode {
            stream_sub(None, Some(0))
        } else {
            NotificationSub::Observe {
                notification: "owned".into(),
                count: Some(0),
                timeout: 1,
                json: true,
            }
        };
        bounded(capture(
            &mut client,
            sub,
            |line| {
                records.push(line.to_string());
                Ok(())
            },
            no_signal(),
        ))
        .await
        .unwrap();
        assert_eq!(records.len(), usize::from(stream_mode));
        if stream_mode {
            assert_eq!(records[0], "{\"index\":1,\"notification\":\"owned\"}");
        }
        assert_eq!(
            request(&mut peer).await["Command"].as_string(),
            Some("ObserveNotification")
        );
        shutdown(&mut peer).await;
    }
}

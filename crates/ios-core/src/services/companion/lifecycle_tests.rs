use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::*;

const BOUND: Duration = Duration::from_secs(2);
const SHORT: Duration = Duration::from_millis(20);

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned fixture stalled")
}

fn frame(value: plist::Dictionary) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &value).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

fn dictionary(key: &str, value: plist::Value) -> plist::Dictionary {
    let mut result = plist::Dictionary::new();
    result.insert(key.into(), value);
    result
}

async fn request(peer: &mut DuplexStream) -> plist::Dictionary {
    let len = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(len < 4096);
    let mut body = vec![0; len];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    plist::from_bytes(&body).unwrap()
}

async fn reply(peer: &mut DuplexStream, value: plist::Dictionary) {
    bounded(peer.write_all(&frame(value))).await.unwrap();
}

fn forwarding<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    client: &CompanionProxyClient<S>,
) -> ForwardingHandle<S> {
    // An owned synthetic active-forwarding state. No real device/start or
    // forwarding service effect is implied by this private fixture.
    ForwardingHandle {
        stream: Arc::clone(&client.stream),
        usable: Arc::clone(&client.usable),
        timeout: client.timeout,
        remote_port: 42,
        service_port: 43,
        response: plist::Dictionary::new(),
        stop_response: None,
        stopped: false,
    }
}

fn issued(error: &CompanionError, duration: Duration) {
    assert_eq!(crate::classify_error(error), crate::ErrorClass::Timeout);
    assert_eq!(error.timeout_duration(), Some(duration));
    assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(error));
}

#[tokio::test]
async fn lifecycle_companion_admission_and_unpolled_do_not_touch_shared_transport() {
    for timeout in [Duration::ZERO, Duration::MAX] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = CompanionProxyClient::with_timeout(stream, timeout);
        for error in [
            client.list().await.unwrap_err(),
            client
                .send_only(command("StartListeningForDevices"))
                .await
                .unwrap_err(),
        ] {
            assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
            if timeout.is_zero() {
                assert!(matches!(error, CompanionError::Timeout(Duration::ZERO)));
            } else {
                assert!(
                    matches!(error, CompanionError::Io(ref io) if io.kind() == std::io::ErrorKind::InvalidInput)
                );
            }
        }
        assert!(client.is_usable());
        drop(Box::pin(client.list()));
        assert!(client.is_usable());
        assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_request_total_includes_blocked_send_and_late_reply() {
    let timeout = Duration::from_millis(200);
    let (mut stream, mut peer) = tokio::io::duplex(512);
    bounded(stream.write_all(&[0x5a; 512])).await.unwrap();
    let mut client = CompanionProxyClient::with_timeout(stream, timeout);
    let (result, ()) = bounded(async {
        tokio::join!(client.list(), async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            let mut seed = [0; 512];
            bounded(peer.read_exact(&mut seed)).await.unwrap();
            assert_eq!(seed, [0x5a; 512]);
            assert_eq!(
                request(&mut peer).await["Command"].as_string(),
                Some("GetDeviceRegistry")
            );
            tokio::time::sleep(Duration::from_millis(125)).await;
            reply(
                &mut peer,
                dictionary("PairedDevicesArray", plist::Value::Array(vec![])),
            )
            .await;
        })
    })
    .await;
    issued(&result.unwrap_err(), timeout);
    assert!(!client.is_usable());
    assert!(matches!(
        bounded(client.list()).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cancelled_request_empty_or_partial_send_tracks_commitment() {
    for empty in [false, true] {
        let (mut stream, mut peer) = tokio::io::duplex(64);
        if empty {
            bounded(stream.write_all(&[0x5a; 64])).await.unwrap();
        }
        let mut client = CompanionProxyClient::new(stream);
        let mut operation = Box::pin(client.list());
        assert!(tokio::time::timeout(SHORT, &mut operation).await.is_err());
        drop(operation);
        assert_eq!(client.is_usable(), empty);
        let mut prefix = [0; 64];
        bounded(peer.read_exact(&mut prefix)).await.unwrap();
        if empty {
            assert_eq!(prefix, [0x5a; 64]);
        } else {
            assert_eq!(&prefix[..4], &frame(command("GetDeviceRegistry"))[..4]);
        }
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cancelled_committed_request_rejects_late_reply_and_waiting_handle() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::new(stream);
    let mut handle = forwarding(&client);
    let mut primary = Box::pin(client.list());
    bounded(async {
        tokio::select! {
            result = &mut primary => panic!("request ended before its reply: {result:?}"),
            req = request(&mut peer) => assert_eq!(req["Command"].as_string(), Some("GetDeviceRegistry")),
        }
    }).await;
    let mut stop = Box::pin(handle.stop());
    assert!(tokio::time::timeout(SHORT, &mut stop).await.is_err());
    drop(primary);
    assert!(matches!(
        bounded(&mut stop).await,
        Err(CompanionError::Protocol(_))
    ));
    drop(stop);
    assert!(!handle.is_stopped() && !client.is_usable());
    reply(
        &mut peer,
        dictionary("PairedDevicesArray", plist::Value::Array(vec![])),
    )
    .await;
    assert!(matches!(
        bounded(client.list()).await,
        Err(CompanionError::Protocol(_))
    ));
    drop(handle);
    tokio::task::yield_now().await;
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_buffered_partial_reply_expiry_invalidates_shared_state() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::with_timeout(tokio::io::BufStream::new(stream), SHORT);
    let first = frame(dictionary(
        "PairedDevicesArray",
        plist::Value::Array(vec![]),
    ));
    let second = frame(command("late"));
    let mut replies = first;
    replies.extend(&second[..2]);
    bounded(peer.write_all(&replies)).await.unwrap();
    bounded(client.list()).await.unwrap();
    assert!(client.is_usable());
    // The second header can already reside in the BufStream read buffer.
    let error = bounded(client.list()).await.unwrap_err();
    issued(&error, SHORT);
    assert!(!client.is_usable());
    assert!(matches!(
        bounded(client.list()).await,
        Err(CompanionError::Protocol(_))
    ));
    for _ in 0..2 {
        request(&mut peer).await;
    }
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
}

#[tokio::test]
async fn lifecycle_companion_every_client_path_rejects_invalid_shared_transport() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::new(stream);
    let handle = forwarding(&client);
    let mut operation = Box::pin(client.list());
    bounded(async {
        tokio::select! {
            result = &mut operation => panic!("request ended before reply: {result:?}"),
            _ = request(&mut peer) => {}
        }
    })
    .await;
    drop(operation);
    assert!(!client.is_usable());
    assert!(matches!(
        bounded(client.list()).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(matches!(
        bounded(client.get_value("owned", "name")).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(matches!(
        bounded(client.start_forwarding_service_port(42, None, None)).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(matches!(
        bounded(client.start_forwarding_service_port_handle(42, None, None)).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(matches!(
        bounded(client.stop_forwarding_service_port(42)).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(matches!(
        bounded(client.listen_for_devices()).await,
        Err(CompanionError::Protocol(_))
    ));
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    drop(handle);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_zero_stop_rejects_without_write_and_keeps_completed_retry() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let client = CompanionProxyClient::new(stream);
    let mut handle = forwarding(&client);
    let error = bounded(handle.stop_with_timeout(Duration::ZERO))
        .await
        .unwrap_err();
    assert!(matches!(error, CompanionError::Timeout(Duration::ZERO)));
    assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
    assert!(!handle.is_stopped() && client.is_usable());
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    let (result, ()) = bounded(async {
        tokio::join!(handle.stop_with_timeout(Duration::MAX), async {
            assert_eq!(
                request(&mut peer).await["Command"].as_string(),
                Some("StopForwardingServicePort")
            );
            reply(&mut peer, command("ComandSuccess")).await;
        })
    })
    .await;
    result.unwrap();
    assert!(handle.is_stopped() && client.is_usable());
    drop(handle);
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
}

#[tokio::test]
async fn lifecycle_companion_mutex_wait_uses_request_budget_without_progress() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::with_timeout(stream, SHORT);
    let shared = Arc::clone(&client.stream);
    let lock = shared.lock().await;
    issued(&bounded(client.list()).await.unwrap_err(), SHORT);
    assert!(client.is_usable());
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    drop(lock);
    let (result, ()) = bounded(async {
        tokio::join!(client.list(), async {
            request(&mut peer).await;
            reply(
                &mut peer,
                dictionary("PairedDevicesArray", plist::Value::Array(vec![])),
            )
            .await;
        })
    })
    .await;
    result.unwrap();
    assert!(client.is_usable());
}

#[tokio::test]
async fn lifecycle_companion_native_and_frame_failures_preserve_families_and_reject_reuse() {
    for kind in 0..4 {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = CompanionProxyClient::new(stream);
        let (result, ()) = bounded(async {
            tokio::join!(client.list(), async {
                request(&mut peer).await;
                match kind {
                    0 => {
                        bounded(peer.shutdown()).await.unwrap();
                    }
                    1 => {
                        bounded(peer.write_u32((MAX_PLIST_SIZE + 1) as u32))
                            .await
                            .unwrap();
                    }
                    2 => {
                        bounded(peer.write_all(&[0, 0, 0, 2, b'?', b'?']))
                            .await
                            .unwrap();
                    }
                    _ => {
                        let mut body = Vec::new();
                        plist::to_writer_xml(&mut body, &plist::Value::Array(vec![])).unwrap();
                        bounded(peer.write_u32(body.len() as u32)).await.unwrap();
                        bounded(peer.write_all(&body)).await.unwrap();
                    }
                }
            })
        })
        .await;
        let error = result.unwrap_err();
        match kind {
            0 => assert!(
                matches!(error, CompanionError::Io(ref io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
            ),
            1 => assert!(matches!(error, CompanionError::Protocol(_))),
            _ => assert!(matches!(error, CompanionError::Plist(_))),
        }
        assert!(!client.is_usable());
        assert!(matches!(
            bounded(client.list()).await,
            Err(CompanionError::Protocol(_))
        ));
    }
    let (stream, peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::new(stream);
    drop(peer);
    assert!(
        matches!(bounded(client.list()).await, Err(CompanionError::Io(ref io)) if io.kind() == std::io::ErrorKind::BrokenPipe)
    );
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_companion_complete_business_failure_keeps_request_and_stop_reuse() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::new(stream);
    let mut handle = forwarding(&client);
    let ((), ()) = bounded(async {
        tokio::join!(
            async {
                assert!(matches!(
                    client.list().await,
                    Err(CompanionError::Protocol(_))
                ));
                assert!(client.is_usable());
                assert!(matches!(
                    handle.stop().await,
                    Err(CompanionError::Protocol(_))
                ));
                assert!(!handle.is_stopped() && client.is_usable());
                let response = handle.stop().await.unwrap();
                assert!(handle.is_stopped() && client.is_usable());
                assert_eq!(
                    handle.stop_with_timeout(Duration::ZERO).await.unwrap(),
                    response
                );
                assert_eq!(
                    handle.stop_with_timeout(Duration::MAX).await.unwrap(),
                    response
                );
            },
            async {
                for name in ["GetDeviceRegistry", "StopForwardingServicePort"] {
                    assert_eq!(request(&mut peer).await["Command"].as_string(), Some(name));
                    reply(&mut peer, dictionary("Error", "denied".into())).await;
                }
                assert_eq!(
                    request(&mut peer).await["Command"].as_string(),
                    Some("StopForwardingServicePort")
                );
                reply(&mut peer, command("ComandSuccess")).await;
            }
        )
    })
    .await;
    drop(handle);
    assert!(client.is_usable());
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
}

#[tokio::test]
async fn lifecycle_companion_stop_expiry_is_bounded_and_drop_sends_no_second_request() {
    for budget in [SHORT, Duration::MAX] {
        let effective = budget.min(lifecycle::CLEANUP_TIMEOUT);
        let (stream, mut peer) = tokio::io::duplex(4096);
        let client = CompanionProxyClient::with_timeout(stream, Duration::MAX);
        let mut handle = forwarding(&client);
        let operation = async {
            tokio::join!(handle.stop_with_timeout(budget), async {
                let req = request(&mut peer).await;
                assert_eq!(
                    req["Command"].as_string(),
                    Some("StopForwardingServicePort")
                );
                assert_eq!(req["GizmoRemotePortNumber"].as_unsigned_integer(), Some(42));
            })
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(6), operation)
            .await
            .expect("cleanup fixture stalled");
        issued(&result.unwrap_err(), effective);
        assert!(!handle.is_stopped() && !client.is_usable());
        drop(handle);
        tokio::task::yield_now().await;
        assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_existing_drop_cleanup_max_is_capped_and_releases_mutex() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let client = CompanionProxyClient::with_timeout(stream, Duration::MAX);
    drop(forwarding(&client));
    assert_eq!(
        request(&mut peer).await["Command"].as_string(),
        Some("StopForwardingServicePort")
    );
    tokio::time::timeout(Duration::from_secs(6), async {
        while client.is_usable() {
            tokio::time::sleep(SHORT).await;
        }
    })
    .await
    .expect("existing Drop cleanup exceeded cap");
    tokio::task::yield_now().await;
    assert!(client.stream.try_lock().is_ok());
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_listener_cancelled_next_keeps_owned_partial_read() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut listener = bounded(CompanionProxyClient::new(stream).listen_for_devices())
        .await
        .unwrap();
    assert_eq!(
        request(&mut peer).await["Command"].as_string(),
        Some("StartListeningForDevices")
    );
    let event = dictionary("FutureEvent", "owned".into());
    let bytes = frame(event.clone());
    bounded(peer.write_all(&bytes[..2])).await.unwrap();
    assert!(tokio::time::timeout(SHORT, listener.next_event())
        .await
        .is_err());
    assert!(listener.pending.is_some() && !listener.finished);
    assert!(lifecycle::usable(&listener.usable));
    bounded(peer.write_all(&bytes[2..])).await.unwrap();
    assert_eq!(
        bounded(listener.next_event()).await.unwrap().unwrap(),
        event
    );
    assert!(listener.pending.is_none());
    drop(listener);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_listener_terminal_idle_releases_pending_mutex_and_shared_state() {
    for partial in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let client = CompanionProxyClient::new(stream);
        let mut handle = forwarding(&client);
        let shared = Arc::clone(&client.stream);
        let mut listener = bounded(client.listen_for_devices()).await.unwrap();
        request(&mut peer).await;
        if partial {
            bounded(peer.write_all(&[0, 0])).await.unwrap();
        }
        issued(
            &bounded(listener.next_event_with_timeout(SHORT))
                .await
                .unwrap_err(),
            SHORT,
        );
        assert!(listener.finished && listener.pending.is_none());
        assert!(!lifecycle::usable(&listener.usable));
        assert!(shared.try_lock().is_ok());
        assert!(bounded(listener.next_event()).await.is_none());
        assert!(matches!(
            bounded(handle.stop()).await,
            Err(CompanionError::Protocol(_))
        ));
        drop(handle);
        drop(listener);
        drop(shared);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_listener_zero_and_overflow_do_not_begin_or_finish_reads() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut listener = bounded(CompanionProxyClient::new(stream).listen_for_devices())
        .await
        .unwrap();
    request(&mut peer).await;
    for budget in [Duration::ZERO, Duration::MAX] {
        let error = bounded(listener.next_event_with_timeout(budget))
            .await
            .unwrap_err();
        assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        assert!(!listener.finished && listener.pending.is_none());
        assert!(lifecycle::usable(&listener.usable));
    }
    let event = command("FutureEvent");
    reply(&mut peer, event.clone()).await;
    assert_eq!(
        bounded(listener.next_event()).await.unwrap().unwrap(),
        event
    );
}

#[tokio::test]
async fn lifecycle_companion_whole_listener_drop_releases_read_and_prevents_shared_stop() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let client = CompanionProxyClient::new(stream);
    let mut handle = forwarding(&client);
    let mut listener = bounded(client.listen_for_devices()).await.unwrap();
    request(&mut peer).await;
    bounded(peer.write_all(&[0, 0])).await.unwrap();
    assert!(tokio::time::timeout(SHORT, listener.next_event())
        .await
        .is_err());
    drop(listener);
    assert!(handle.stream.try_lock().is_ok());
    assert!(matches!(
        bounded(handle.stop()).await,
        Err(CompanionError::Protocol(_))
    ));
    drop(handle);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

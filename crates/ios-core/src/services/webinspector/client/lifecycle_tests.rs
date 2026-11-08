use super::*;
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadBuf},
    time::Instant,
};

const BOUND: Duration = Duration::from_secs(2);
const SHORT: Duration = Duration::from_millis(20);

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    timeout(BOUND, operation)
        .await
        .expect("owned fixture stalled")
}

fn dictionary(key: &str, value: plist::Value) -> plist::Dictionary {
    plist::Dictionary::from_iter([(key.to_owned(), value)])
}

fn message(selector: &str, arguments: plist::Dictionary) -> plist::Value {
    plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("__selector".to_owned(), selector.into()),
        ("__argument".to_owned(), plist::Value::Dictionary(arguments)),
    ]))
}

fn current_state() -> plist::Value {
    message(
        "_rpc_reportCurrentState:",
        dictionary(
            "WIRAutomationAvailabilityKey",
            "WIRAutomationAvailabilityAvailable".into(),
        ),
    )
}

fn frame(value: &plist::Value) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, value).unwrap();
    let mut result = (body.len() as u32).to_be_bytes().to_vec();
    result.extend(body);
    result
}

async fn request(peer: &mut DuplexStream) -> plist::Dictionary {
    let len = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(len < 8192);
    let mut body = vec![0; len];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    plist::from_bytes(&body).unwrap()
}

fn application() -> Application {
    Application {
        id: "PID:42".into(),
        bundle_identifier: "owned".into(),
        pid: 42,
        name: "owned".into(),
        availability: AutomationAvailability::Available,
        is_active: true,
        is_proxy: false,
        is_ready: true,
        host_application_identifier: None,
    }
}

fn connected_applications() -> plist::Value {
    let app = plist::Dictionary::from_iter([
        (
            "WIRApplicationIdentifierKey".to_owned(),
            plist::Value::String("PID:42".into()),
        ),
        (
            "WIRApplicationBundleIdentifierKey".to_owned(),
            "owned".into(),
        ),
        ("WIRApplicationNameKey".to_owned(), "owned".into()),
        (
            "WIRAutomationAvailabilityKey".to_owned(),
            "WIRAutomationAvailabilityAvailable".into(),
        ),
        ("WIRIsApplicationActiveKey".to_owned(), true.into()),
        ("WIRIsApplicationProxyKey".to_owned(), false.into()),
        ("WIRIsApplicationReadyKey".to_owned(), true.into()),
    ]);
    message(
        "_rpc_reportConnectedApplicationList:",
        dictionary(
            "WIRApplicationDictionaryKey",
            plist::Value::Dictionary(dictionary("PID:42", app.into())),
        ),
    )
}

fn seed<S>(client: &mut WebInspectorClient<S>, pending: bool) {
    client.automation_availability = Some(AutomationAvailability::Available);
    client.applications.insert("PID:42".into(), application());
    let page = Page::from_plist(
        "1",
        &plist::Dictionary::from_iter([
            (
                "WIRPageIdentifierKey".to_owned(),
                plist::Value::Integer(1u64.into()),
            ),
            ("WIRTypeKey".to_owned(), "WIRTypeAutomation".into()),
            ("WIRSessionIdentifierKey".to_owned(), "owned-session".into()),
        ]),
    )
    .unwrap();
    client
        .application_pages
        .insert("PID:42".into(), IndexMap::from_iter([(1, page)]));
    if pending {
        client
            .pending_events
            .push_back(WebInspectorEvent::ConnectedDrivers);
    }
}

fn cleared<S: AsyncRead + AsyncWrite + Unpin>(client: &WebInspectorClient<S>) {
    assert!(!client.is_usable());
    assert!(client.automation_availability().is_none());
    assert!(client.applications().is_empty());
    assert!(client.application_pages("PID:42").is_none());
    assert!(client.application_by_bundle("owned").is_none());
    assert!(client.page("PID:42", 1).is_none());
    assert!(client
        .automation_page_by_session("PID:42", "owned-session")
        .is_none());
    assert!(client.open_pages_snapshot().is_empty());
    assert!(client.pending_events.is_empty());
    assert_eq!(client.connection_id(), "owned-connection");
}

fn issued(error: &WebInspectorError, duration: Duration) {
    assert_eq!(crate::classify_error(error), crate::ErrorClass::Timeout);
    assert_eq!(error.timeout_duration(), Some(duration));
    assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(error));
}

async fn entry<S: AsyncRead + AsyncWrite + Unpin>(
    client: &mut WebInspectorClient<S>,
    kind: u8,
) -> Result<(), WebInspectorError> {
    match kind {
        0 => client.report_identifier().await,
        1 => client.request_connected_applications().await,
        2 => client.request_listing("PID:42").await,
        3 => client.request_application_launch("owned").await,
        4 => {
            client
                .request_automation_session("owned-session", "PID:42")
                .await
        }
        5 => {
            client
                .send_socket_setup("owned-session", "PID:42", 1, false)
                .await
        }
        6 => {
            client
                .send_socket_data("owned-session", "PID:42", 1, &serde_json::json!({"id":1}))
                .await
        }
        7 => {
            client
                .send_socket_teardown("owned-session", "PID:42", 1)
                .await
        }
        8 => client.next_event().await.map(|_| ()),
        9 => client
            .next_event_with_timeout(Duration::MAX)
            .await
            .map(|_| ()),
        10 => client.start_preserving_errors(Duration::MAX).await,
        _ => client.open_application_pages(BOUND).await.map(|_| ()),
    }
}

#[tokio::test]
async fn lifecycle_webinspector_checked_zero_overflow_admission_preserves_ready_cache_and_wire() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
    seed(&mut client, true);
    for budget in [Duration::ZERO, Duration::MAX] {
        client.set_operation_timeout(Some(budget));
        for kind in 0..12 {
            let error = bounded(entry(&mut client, kind)).await.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                if budget.is_zero() {
                    crate::ErrorClass::Timeout
                } else {
                    crate::ErrorClass::InvalidInput
                }
            );
            assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
            assert!(client.is_usable());
            assert_eq!(client.pending_events.len(), 1);
            assert_eq!(client.open_pages_snapshot().len(), 1);
        }
        client.set_operation_timeout(None);
        let error = client.next_event_with_timeout(budget).await.unwrap_err();
        assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        assert_eq!(client.pending_events.len(), 1);
    }
    assert!(timeout(SHORT, peer.read_u8()).await.is_err());
    assert!(matches!(
        client.next_event().await.unwrap(),
        WebInspectorEvent::ConnectedDrivers
    ));
    client.set_operation_timeout(Some(Duration::MAX));
    issued(
        &client.next_event_with_timeout(SHORT).await.unwrap_err(),
        SHORT,
    );
    assert!(client.is_usable());
}

#[tokio::test]
async fn lifecycle_webinspector_nominal_due_timer_suppresses_ready_operation() {
    let count = AtomicUsize::new(0);
    let mut deadline = checked_deadline(SHORT).unwrap();
    deadline.instant = Instant::now();
    let error = bounded(await_operation(Some(deadline), "owned deadline", async {
        count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }))
    .await
    .unwrap_err();
    issued(&error, SHORT);
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_empty_idle_expiry_and_cancel_preserve_subsequent_event() {
    for expired in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
        seed(&mut client, false);
        if expired {
            issued(
                &client.next_event_with_timeout(SHORT).await.unwrap_err(),
                SHORT,
            );
        } else {
            assert!(timeout(SHORT, client.next_event()).await.is_err());
        }
        assert!(client.is_usable());
        assert_eq!(client.open_pages_snapshot().len(), 1);
        peer.write_all(&frame(&current_state())).await.unwrap();
        assert!(matches!(
            bounded(client.next_event()).await.unwrap(),
            WebInspectorEvent::CurrentState { .. }
        ));
    }
}

#[tokio::test]
async fn lifecycle_webinspector_partial_header_body_expiry_or_cancel_clears_all_owned_state() {
    let full = frame(&current_state());
    for length in [2, 7] {
        for expired in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
            seed(&mut client, false);
            peer.write_all(&full[..length]).await.unwrap();
            if expired {
                issued(
                    &bounded(client.next_event_with_timeout(SHORT))
                        .await
                        .unwrap_err(),
                    SHORT,
                );
            } else {
                assert!(timeout(SHORT, client.next_event()).await.is_err());
            }
            cleared(&client);
            client.restore_pending_events_front(vec![WebInspectorEvent::ConnectedDrivers]);
            client.set_operation_timeout(None);
            for kind in 0..12 {
                assert!(matches!(
                    bounded(entry(&mut client, kind)).await,
                    Err(WebInspectorError::Protocol(_))
                ));
            }
            assert!(client.pending_events.is_empty());
            drop(client);
            assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
        }
    }
}

#[tokio::test]
async fn lifecycle_webinspector_buffered_partial_next_header_counts_as_progress() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut bytes = frame(&current_state());
    bytes.extend_from_slice(&[0, 0]);
    peer.write_all(&bytes).await.unwrap();
    let mut client =
        WebInspectorClient::with_connection_id(BufReader::new(stream), "owned-connection");
    bounded(client.next_event()).await.unwrap();
    assert_eq!(client.stream.buffer(), &[0, 0]);
    issued(
        &client.next_event_with_timeout(SHORT).await.unwrap_err(),
        SHORT,
    );
    cleared(&client);
    assert!(matches!(
        client.next_event().await,
        Err(WebInspectorError::Protocol(_))
    ));
}

#[tokio::test]
async fn lifecycle_webinspector_unpolled_and_empty_or_partial_send_cancellation_own_local_state() {
    for partial in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(64);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
        drop(client.report_identifier());
        drop(client.start_preserving_errors(BOUND));
        drop(client.next_event());
        assert!(client.is_usable());
        if !partial {
            client.stream.write_all(&[0x5a; 64]).await.unwrap();
        }
        assert!(timeout(SHORT, client.report_identifier()).await.is_err());
        assert_eq!(client.is_usable(), !partial);
        let mut prefix = [0; 64];
        bounded(peer.read_exact(&mut prefix)).await.unwrap();
        if !partial {
            assert_eq!(prefix, [0x5a; 64]);
            let (result, value) =
                bounded(async { tokio::join!(client.report_identifier(), request(&mut peer)) })
                    .await;
            result.unwrap();
            assert_eq!(
                value["__selector"].as_string(),
                Some("_rpc_reportIdentifier:")
            );
        } else {
            assert_ne!(prefix, [0x5a; 64]);
        }
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_webinspector_native_eof_bad_codec_size_and_routing_preserve_error_then_reject() {
    for kind in 0..8 {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
        seed(&mut client, false);
        let bytes = match kind {
            1 => vec![0, 0, 0, 3, b'b', b'a', b'd'],
            2 => frame(&plist::Value::Array(vec![])),
            3 => (16u32 * 1024 * 1024 + 1).to_be_bytes().to_vec(),
            6 => frame(&message(
                "_rpc_applicationSentData:",
                dictionary(
                    "WIRMessageDataKey",
                    plist::Value::Data(b"owned-private-invalid-json".to_vec()),
                ),
            )),
            7 => frame(&message(
                "_rpc_applicationUpdated:",
                dictionary("WIRApplicationIdentifierKey", "PID:owned-private".into()),
            )),
            _ => frame(&message(
                "_rpc_reportCurrentState:",
                plist::Dictionary::new(),
            )),
        };
        if kind == 0 || kind == 5 {
            drop(peer);
        } else {
            peer.write_all(&bytes).await.unwrap();
        }
        let error = if kind == 5 {
            bounded(client.report_identifier()).await.unwrap_err()
        } else {
            bounded(client.next_event()).await.unwrap_err()
        };
        match kind {
            0 => {
                let WebInspectorError::Io(native) = &error else {
                    panic!("lost native EOF")
                };
                assert_eq!(native.kind(), std::io::ErrorKind::UnexpectedEof);
                assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
            }
            1 | 2 => assert!(matches!(error, WebInspectorError::Plist(_))),
            5 => {
                let WebInspectorError::Io(native) = &error else {
                    panic!("lost native write error")
                };
                assert_eq!(native.kind(), std::io::ErrorKind::BrokenPipe);
                assert_eq!(
                    crate::classify_error(&error),
                    crate::ErrorClass::Disconnected
                );
            }
            6 => {
                assert!(matches!(error, WebInspectorError::Json(_)));
                assert!(crate::test_native::has_source::<serde_json::Error>(&error));
                assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
            }
            7 => {
                assert!(matches!(error, WebInspectorError::Io(_)));
                assert!(crate::test_native::has_source::<std::num::ParseIntError>(
                    &error
                ));
                assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
                assert!(!format!("{error:?}").contains("owned-private"));
            }
            _ => assert!(matches!(error, WebInspectorError::Protocol(_))),
        }
        cleared(&client);
        assert!(matches!(
            client.next_event().await,
            Err(WebInspectorError::Protocol(_))
        ));
    }
}

#[tokio::test]
async fn lifecycle_webinspector_local_xml_failure_before_io_preserves_ready_state() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
    seed(&mut client, true);
    let deadline = client.deadline(None).unwrap();
    let error = client
        .send_message(
            deadline,
            "owned",
            dictionary("UID", plist::Value::Uid(plist::Uid::new(1))),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WebInspectorError::Plist(_)));
    assert!(client.is_usable());
    assert_eq!(client.pending_events.len(), 1);
    assert!(timeout(SHORT, peer.read_u8()).await.is_err());
}

struct FlushStream {
    inner: DuplexStream,
    block: bool,
}
impl AsyncRead for FlushStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for FlushStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.block {
            Poll::Pending
        } else {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "owned-private-flush",
            )))
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn lifecycle_webinspector_committed_flush_cancel_or_native_failure_invalidates() {
    for block in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = WebInspectorClient::with_connection_id(
            FlushStream {
                inner: stream,
                block,
            },
            "owned-connection",
        );
        if block {
            assert!(timeout(SHORT, client.report_identifier()).await.is_err());
        } else {
            let error = client.report_identifier().await.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::PermissionDenied
            );
            let WebInspectorError::Io(native) = error else {
                panic!("lost native flush error")
            };
            assert_eq!(native.kind(), std::io::ErrorKind::PermissionDenied);
        }
        assert_eq!(
            request(&mut peer).await["__selector"].as_string(),
            Some("_rpc_reportIdentifier:")
        );
        cleared(&client);
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_webinspector_startup_committed_cancel_or_expiry_cannot_consume_late_state() {
    for preserve in [false, true] {
        for cancelled in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
            let operation = async {
                let budget = if cancelled { BOUND } else { SHORT };
                if preserve {
                    client.start_preserving_errors(budget).await
                } else {
                    client.start(budget).await
                }
            };
            let result = bounded(async {
                tokio::join!(
                    async {
                        if cancelled {
                            assert!(timeout(SHORT, operation).await.is_err());
                            Ok(())
                        } else {
                            operation.await
                        }
                    },
                    async {
                        assert_eq!(
                            request(&mut peer).await["__selector"].as_string(),
                            Some("_rpc_reportIdentifier:")
                        );
                    }
                )
            })
            .await
            .0;
            if !cancelled {
                let error = result.unwrap_err();
                if preserve {
                    issued(&error, SHORT);
                } else {
                    assert!(matches!(error, WebInspectorError::Disabled));
                }
            }
            cleared(&client);
            peer.write_all(&frame(&current_state())).await.unwrap();
            assert!(matches!(
                client.next_event().await,
                Err(WebInspectorError::Protocol(_))
            ));
        }
    }
}

#[tokio::test]
async fn lifecycle_webinspector_startup_send_and_receive_share_original_total() {
    let (stream, mut peer) = tokio::io::duplex(512);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
    client.stream.write_all(&[0x5a; 512]).await.unwrap();
    let total = Duration::from_millis(200);
    let (result, ()) = bounded(async {
        tokio::join!(client.start_preserving_errors(total), async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            let mut prefix = [0; 512];
            peer.read_exact(&mut prefix).await.unwrap();
            assert_eq!(prefix, [0x5a; 512]);
            request(&mut peer).await;
            tokio::time::sleep(Duration::from_millis(125)).await;
            peer.write_all(&frame(&current_state())).await.unwrap();
        })
    })
    .await;
    issued(&result.unwrap_err(), total);
    cleared(&client);
    assert!(matches!(
        client.next_event().await,
        Err(WebInspectorError::Protocol(_))
    ));
}

#[tokio::test]
async fn lifecycle_webinspector_decoded_application_event_owns_empty_blocked_listing_send() {
    for cancelled in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(512);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
        client.stream.write_all(&[0x5a; 512]).await.unwrap();
        let (result, ()) = bounded(async {
            tokio::join!(
                async {
                    if cancelled {
                        assert!(timeout(SHORT, client.next_event()).await.is_err());
                        Ok(())
                    } else {
                        client.next_event_with_timeout(SHORT).await.map(|_| ())
                    }
                },
                async {
                    peer.write_all(&frame(&connected_applications()))
                        .await
                        .unwrap();
                }
            )
        })
        .await;
        if !cancelled {
            issued(&result.unwrap_err(), SHORT);
        }
        cleared(&client);
        let mut prefix = [0; 512];
        peer.read_exact(&mut prefix).await.unwrap();
        assert_eq!(prefix, [0x5a; 512]);
        assert!(timeout(SHORT, peer.read_u8()).await.is_err());
        drop(client);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_webinspector_event_receive_and_automatic_listing_share_one_total() {
    let (stream, mut peer) = tokio::io::duplex(512);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
    client.stream.write_all(&[0x5a; 512]).await.unwrap();
    let total = Duration::from_millis(200);
    client.set_operation_timeout(Some(total));
    let (result, ()) = bounded(async {
        tokio::join!(client.next_event(), async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            peer.write_all(&frame(&connected_applications()))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(125)).await;
            let mut prefix = [0; 512];
            peer.read_exact(&mut prefix).await.unwrap();
            assert_eq!(prefix, [0x5a; 512]);
            assert!(timeout(SHORT, peer.read_u8()).await.is_err());
        })
    })
    .await;
    issued(&result.unwrap_err(), total);
    cleared(&client);
    drop(client);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_webinspector_discovery_quiescence_requires_empty_usable_idle() {
    for partial in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
        let (result, ()) = bounded(async {
            tokio::join!(client.open_application_pages(SHORT), async {
                assert_eq!(
                    request(&mut peer).await["__selector"].as_string(),
                    Some("_rpc_getConnectedApplications:")
                );
                if partial {
                    peer.write_all(&[0, 0]).await.unwrap();
                }
            })
        })
        .await;
        if partial {
            issued(&result.unwrap_err(), SHORT);
            cleared(&client);
        } else {
            assert!(result.unwrap().is_empty());
            assert!(client.is_usable());
        }
    }
}

#[tokio::test]
async fn lifecycle_webinspector_configured_total_is_not_idle_quiescence_and_shortens_explicit_event(
) {
    for discover in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
        client.set_operation_timeout(Some(SHORT));
        let error = if discover {
            let (result, ()) = bounded(async {
                tokio::join!(client.open_application_pages(BOUND), async {
                    request(&mut peer).await;
                })
            })
            .await;
            result.unwrap_err()
        } else {
            bounded(client.next_event_with_timeout(BOUND))
                .await
                .unwrap_err()
        };
        issued(&error, SHORT);
        assert!(client.is_usable());
    }
}

#[tokio::test]
async fn lifecycle_webinspector_successful_application_event_emits_original_listing_and_retains_cache(
) {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = WebInspectorClient::with_connection_id(stream, "owned-connection");
    peer.write_all(&frame(&connected_applications()))
        .await
        .unwrap();
    let (result, req) =
        bounded(async { tokio::join!(client.next_event_with_timeout(BOUND), request(&mut peer)) })
            .await;
    assert!(
        matches!(result.unwrap(), WebInspectorEvent::ConnectedApplications { applications } if applications == vec![application()])
    );
    assert_eq!(
        req["__selector"].as_string(),
        Some("_rpc_forwardGetListing:")
    );
    let args = req["__argument"].as_dictionary().unwrap();
    assert_eq!(args.len(), 2);
    assert_eq!(
        args["WIRApplicationIdentifierKey"].as_string(),
        Some("PID:42")
    );
    assert_eq!(
        args["WIRConnectionIdentifierKey"].as_string(),
        Some("owned-connection")
    );
    assert!(client.is_usable());
    assert_eq!(client.applications()["PID:42"], application());
}

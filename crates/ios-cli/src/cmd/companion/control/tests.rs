use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::oneshot,
};

const BOUND: Duration = Duration::from_secs(2);
const SHORT: Duration = Duration::from_millis(20);
async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned fixture stalled")
}
fn no_signal() -> impl Future<Output = std::io::Result<()>> {
    std::future::pending()
}
fn dict(key: &str, value: plist::Value) -> plist::Dictionary {
    let mut result = plist::Dictionary::new();
    result.insert(key.into(), value);
    result
}
fn frame(value: plist::Dictionary) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &value).unwrap();
    let mut result = (body.len() as u32).to_be_bytes().to_vec();
    result.extend(body);
    result
}
async fn request(peer: &mut DuplexStream) -> plist::Dictionary {
    let length = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(length < 4096);
    let mut body = vec![0; length];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    plist::from_bytes(&body).unwrap()
}
async fn reply(peer: &mut DuplexStream, value: plist::Dictionary) {
    bounded(peer.write_all(&frame(value))).await.unwrap();
}
fn forward() -> CompanionSub {
    CompanionSub::Forward {
        remote_port: 42,
        service_name: None,
        timeout: None,
    }
}
async fn start(peer: &mut DuplexStream) {
    let req = request(peer).await;
    assert_eq!(
        req["Command"].as_string(),
        Some("StartForwardingServicePort")
    );
    assert_eq!(req["GizmoRemotePortNumber"].as_unsigned_integer(), Some(42));
    reply(peer, dict("CompanionProxyServicePort", 43u64.into())).await;
}
async fn stop(peer: &mut DuplexStream) {
    let req = request(peer).await;
    assert_eq!(req.len(), 2);
    assert_eq!(
        req["Command"].as_string(),
        Some("StopForwardingServicePort")
    );
    assert_eq!(req["GizmoRemotePortNumber"].as_unsigned_integer(), Some(42));
}
fn issued(error: &anyhow::Error) {
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
}

#[test]
fn lifecycle_companion_cli_preflight_checks_every_finite_timeout_before_io() {
    for value in [0, u64::MAX] {
        for sub in [
            CompanionSub::Listen {
                timeout: Some(value),
            },
            CompanionSub::Forward {
                remote_port: 42,
                service_name: None,
                timeout: Some(value),
            },
            CompanionSub::Stop {
                remote_port: 42,
                timeout: Some(value),
            },
        ] {
            let error = preflight(&sub).unwrap_err();
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::InvalidInput
            );
            assert!(!error
                .chain()
                .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
        }
    }
    assert!(preflight(&CompanionSub::List).is_ok());
    assert!(preflight(&CompanionSub::Listen { timeout: None }).is_ok());
}

#[tokio::test]
async fn lifecycle_companion_cli_setup_zero_overflow_and_unpolled_work_do_not_start() {
    let count = Arc::new(AtomicUsize::new(0));
    for budget in [Duration::ZERO, Duration::MAX] {
        let seen = count.clone();
        let error = setup(
            budget,
            None,
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
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
    let seen = count.clone();
    drop(setup(
        BOUND,
        None,
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        no_signal(),
    ));
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_setup_and_later_stages_share_original_total() {
    let count = Arc::new(AtomicUsize::new(0));
    let total = Instant::now() + Duration::from_millis(200);
    let seen = count.clone();
    bounded(setup(
        BOUND,
        Some(total),
        async move {
            tokio::time::sleep(Duration::from_millis(125)).await;
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        no_signal(),
    ))
    .await
    .unwrap();
    let seen = count.clone();
    let error = bounded(drive(
        async move {
            tokio::time::sleep(Duration::from_millis(125)).await;
            seen.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        no_signal(),
        Some(total),
        || Ok(()),
    ))
    .await
    .unwrap_err();
    issued(&error);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    tokio::time::sleep(Duration::from_millis(75)).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_companion_cli_ready_stop_and_deadline_win_before_ready_operation() {
    for expiry in [false, true] {
        let count = AtomicUsize::new(0);
        let signal = async move {
            if expiry {
                std::future::pending().await
            } else {
                Ok(())
            }
        };
        let result = bounded(drive(
            async {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            signal,
            expiry.then(Instant::now),
            || Err(cancelled()),
        ))
        .await;
        let error = result.unwrap_err();
        if expiry {
            issued(&error);
        } else {
            assert!(error
                .chain()
                .any(|cause| cause.is::<ios_core::OperationCancelled>()));
        }
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cli_same_pinned_signal_survives_setup_into_command() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, rx) = oneshot::channel();
    let mut signal = Box::pin(async { rx.await.expect("signal dropped") });
    let client = bounded(setup(
        BOUND,
        None,
        async { Ok(CompanionProxyClient::new(stream)) },
        signal.as_mut(),
    ))
    .await
    .unwrap();
    tx.send(Ok(())).unwrap();
    let error = bounded(run(
        client,
        CompanionSub::List,
        None,
        true,
        |_| panic!("unexpected output"),
        signal.as_mut(),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_setup_caller_drop_releases_owned_transport() {
    let (mut stream, mut peer) = tokio::io::duplex(64);
    let mut operation = Box::pin(setup(
        BOUND,
        None,
        async move {
            stream.write_all(&[0x5a; 64]).await?;
            std::future::pending::<Result<()>>().await
        },
        no_signal(),
    ));
    let mut prefix = [0; 64];
    bounded(async {
        tokio::select! {
            result=&mut operation=>panic!("setup ended: {result:?}"),
            result=peer.read_exact(&mut prefix)=>{result.unwrap();}
        }
    })
    .await;
    drop(operation);
    assert_eq!(prefix, [0x5a; 64]);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_forward_total_expiry_awaits_independent_exact_stop() {
    for dual in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut output = Vec::new();
        let deadline = Instant::now() + SHORT;
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    CompanionProxyClient::new(stream),
                    forward(),
                    Some(deadline),
                    true,
                    |line| {
                        crate::output::write_line_to(&mut output, format_args!("{line}"))?;
                        Ok(())
                    },
                    no_signal()
                ),
                async {
                    start(&mut peer).await;
                    stop(&mut peer).await;
                    assert!(Instant::now() >= deadline);
                    reply(
                        &mut peer,
                        if dual {
                            dict("Error", "owned-private-cleanup".into())
                        } else {
                            dict("Command", "ComandSuccess".into())
                        },
                    )
                    .await;
                }
            )
        })
        .await;
        let error = result.unwrap_err();
        issued(&error);
        if dual {
            let failure = error.downcast_ref::<CompanionCleanupFailure>().unwrap();
            assert_eq!(
                ios_core::classify_error(failure.cleanup.as_ref()),
                ios_core::ErrorClass::Protocol
            );
            assert!(!format!("{failure:?}").contains("owned-private"));
        }
        assert_eq!(
            output,
            b"{\"CompanionProxyServicePort\":43,\"GizmoRemotePortNumber\":42}\n"
        );
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cli_forward_output_fault_retains_primary_after_single_or_dual_cleanup()
{
    struct FaultWriter;
    impl std::io::Write for FaultWriter {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "owned-private-output",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for dual in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    CompanionProxyClient::new(stream),
                    forward(),
                    None,
                    true,
                    |line| {
                        crate::output::write_line_to(&mut FaultWriter, format_args!("{line}"))?;
                        Ok(())
                    },
                    no_signal()
                ),
                async {
                    start(&mut peer).await;
                    stop(&mut peer).await;
                    reply(
                        &mut peer,
                        if dual {
                            dict("Error", "owned-private-cleanup".into())
                        } else {
                            dict("Command", "ComandSuccess".into())
                        },
                    )
                    .await;
                }
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::PermissionDenied
        );
        if dual {
            let failure = error.downcast_ref::<CompanionCleanupFailure>().unwrap();
            assert_eq!(
                ios_core::classify_error(failure.cleanup.as_ref()),
                ios_core::ErrorClass::Protocol
            );
            assert!(!format!("{failure:?}").contains("owned-private"));
        }
        // A complete business-failed close must not cause an extra Drop stop.
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cli_forward_native_signal_fault_preserves_native_cleanup_branch() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, rx) = oneshot::channel();
    let mut tx = Some(tx);
    let (result, ()) = bounded(async {
        tokio::join!(
            run(
                CompanionProxyClient::new(stream),
                forward(),
                None,
                false,
                |_| {
                    tx.take()
                        .unwrap()
                        .send(Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "owned-private-signal",
                        )))
                        .unwrap();
                    Ok(())
                },
                async { rx.await.expect("signal dropped") }
            ),
            async {
                start(&mut peer).await;
                stop(&mut peer).await;
                drop(peer);
            }
        )
    })
    .await;
    let error = result.unwrap_err();
    let failure = error.downcast_ref::<CompanionCleanupFailure>().unwrap();
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
    assert_eq!(
        ios_core::classify_error(failure.cleanup.as_ref()),
        ios_core::ErrorClass::Io
    );
    let CompanionError::Io(native) = failure.cleanup.downcast_ref::<CompanionError>().unwrap()
    else {
        panic!("cleanup lost its native IO error");
    };
    assert_eq!(native.kind(), std::io::ErrorKind::UnexpectedEof);
}

#[tokio::test]
async fn lifecycle_companion_cli_actual_cleanup_five_second_expiry_retains_output_primary() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (result, ()) = tokio::time::timeout(Duration::from_secs(6), async {
        tokio::join!(
            run(
                CompanionProxyClient::new(stream),
                forward(),
                None,
                true,
                |_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "owned-private-output",
                    )
                    .into())
                },
                no_signal()
            ),
            async {
                start(&mut peer).await;
                stop(&mut peer).await;
            }
        )
    })
    .await
    .expect("cleanup fixture stalled");
    let error = result.unwrap_err();
    let failure = error.downcast_ref::<CompanionCleanupFailure>().unwrap();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    issued(&failure.cleanup);
    assert!(!error
        .chain()
        .any(|cause| cause.is::<tokio::time::error::Elapsed>()));
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_infinite_listener_yields_ready_events_to_stop_with_exact_output() {
    let (stream, mut peer) = tokio::io::duplex(32 * 1024);
    for _ in 0..20 {
        reply(&mut peer, dict("Event", "owned".into())).await;
    }
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let mut output = Vec::new();
    let signal = std::future::poll_fn(move |_| {
        if seen.load(Ordering::SeqCst) >= 3 {
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Pending
        }
    });
    bounded(run(
        CompanionProxyClient::new(stream),
        CompanionSub::Listen { timeout: None },
        None,
        true,
        |line| {
            crate::output::write_line_to(&mut output, format_args!("{line}"))?;
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
        signal,
    ))
    .await
    .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(
        output,
        b"{\"Event\":\"owned\"}\n{\"Event\":\"owned\"}\n{\"Event\":\"owned\"}\n"
    );
    assert_eq!(
        request(&mut peer).await["Command"].as_string(),
        Some("StartListeningForDevices")
    );
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_listener_idle_or_partial_operator_stop_closes_owned_stream() {
    for partial in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let (tx, rx) = oneshot::channel();
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    CompanionProxyClient::new(stream),
                    CompanionSub::Listen { timeout: None },
                    None,
                    true,
                    |_| panic!("unexpected output"),
                    async { rx.await.expect("signal dropped") }
                ),
                async {
                    request(&mut peer).await;
                    if partial {
                        bounded(peer.write_all(&[0, 0])).await.unwrap();
                    }
                    tokio::time::sleep(SHORT).await;
                    tx.send(Ok(())).unwrap();
                }
            )
        })
        .await;
        result.unwrap();
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cli_failed_or_cancelled_start_has_no_late_stop() {
    for expiry in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut operation = Box::pin(run(
            CompanionProxyClient::new(stream),
            forward(),
            expiry.then(|| Instant::now() + SHORT),
            true,
            |_| panic!("unexpected output"),
            no_signal(),
        ));
        bounded(async {
            tokio::select! {
                result=&mut operation=>panic!("start ended before request: {result:?}"),
                _=request(&mut peer)=>{}
            }
        })
        .await;
        if expiry {
            issued(&bounded(&mut operation).await.unwrap_err());
        }
        drop(operation);
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cli_completed_forward_caller_drop_never_spawns_stop() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (tx, rx) = oneshot::channel();
    let mut tx = Some(tx);
    let mut operation = Box::pin(run(
        CompanionProxyClient::new(stream),
        forward(),
        None,
        true,
        |_| {
            tx.take().unwrap().send(()).unwrap();
            Ok(())
        },
        no_signal(),
    ));
    bounded(async {
        tokio::select! {
            result=&mut operation=>panic!("forward ended before start: {result:?}"),
            ()=async{start(&mut peer).await;rx.await.unwrap();}=>{}
        }
    })
    .await;
    // A command cancellation after start must not leave the legacy handle
    // Drop task alive. The owned forwarding scope must disarm before waiting.
    drop(operation);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_graceful_forward_stop_returns_cleanup_result() {
    for business_failure in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let (tx, rx) = oneshot::channel();
        let mut tx = Some(tx);
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    CompanionProxyClient::new(stream),
                    forward(),
                    None,
                    false,
                    |line| {
                        assert_eq!(line, "Forwarded companion port 42 through service port 43. Press Ctrl+C to stop.");
                        tx.take().unwrap().send(Ok(())).unwrap();
                        Ok(())
                    },
                    async { rx.await.expect("signal dropped") }
                ),
                async {
                    start(&mut peer).await;
                    stop(&mut peer).await;
                    reply(&mut peer, if business_failure {
                        dict("Error", "owned-private-cleanup".into())
                    } else {
                        dict("Command", "ComandSuccess".into())
                    }).await;
                }
            )
        }).await;
        if business_failure {
            let error = result.unwrap_err();
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Protocol
            );
            assert!(error.downcast_ref::<CompanionCleanupFailure>().is_none());
        } else {
            result.unwrap();
        }
        assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn lifecycle_companion_cli_caller_drop_during_cleanup_has_no_followup_stop() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut operation = Box::pin(run(
        CompanionProxyClient::new(stream),
        forward(),
        None,
        true,
        |_| {
            Err(
                std::io::Error::new(std::io::ErrorKind::PermissionDenied, "owned-private-output")
                    .into(),
            )
        },
        no_signal(),
    ));
    bounded(async {
        tokio::select! {
            result=&mut operation=>panic!("cleanup ended before reply: {result:?}"),
            ()=async{start(&mut peer).await;stop(&mut peer).await;}=>{}
        }
    })
    .await;
    drop(operation);
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

#[tokio::test]
async fn lifecycle_companion_cli_invalid_session_skips_unpolled_cleanup_and_retains_primary() {
    let count = AtomicUsize::new(0);
    let error = finish_forwarding(
        async {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(plist::Dictionary::new())
        },
        false,
        Err(
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "owned-private-output")
                .into(),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(error.downcast_ref::<CompanionCleanupFailure>().is_none());
}

#[tokio::test]
async fn lifecycle_companion_cli_list_get_stop_success_lines_preserve_original_formats() {
    for kind in 0..3 {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut output = Vec::new();
        let sub = match kind {
            0 => CompanionSub::List,
            1 => CompanionSub::Get {
                udid: "owned".into(),
                key: "name".into(),
            },
            _ => CompanionSub::Stop {
                remote_port: 42,
                timeout: None,
            },
        };
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    CompanionProxyClient::new(stream),
                    sub,
                    None,
                    kind == 0,
                    |line| {
                        crate::output::write_line_to(&mut output, format_args!("{line}"))?;
                        Ok(())
                    },
                    no_signal()
                ),
                async {
                    request(&mut peer).await;
                    reply(
                        &mut peer,
                        match kind {
                            0 => dict(
                                "PairedDevicesArray",
                                plist::Value::Array(vec!["owned".into()]),
                            ),
                            1 => dict("RetrievedValueDictionary", "owned".into()),
                            _ => dict("Command", "ComandSuccess".into()),
                        },
                    )
                    .await;
                }
            )
        })
        .await;
        result.unwrap();
        assert_eq!(
            output,
            if kind == 0 {
                b"[\n  \"owned\"\n]\n".as_slice()
            } else if kind == 1 {
                b"owned\n".as_slice()
            } else {
                b"ComandSuccess\n".as_slice()
            }
        );
    }
}

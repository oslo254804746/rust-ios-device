//! Real CLI shell owner over the shared owned plist peer, input and writer sources.
use super::super::{control::CleanupFailure, test_peer::*};
use super::*;
use ios_core::{classify_error, ErrorClass, OperationCancelled};
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context as TaskContext, Poll},
};
use tokio::{
    io::{AsyncWriteExt, ReadBuf},
    sync::Notify,
};

fn options(json: bool) -> Options<'static> {
    Options {
        url: None,
        bundle_id: None,
        page_id: Some("page-7"),
        open_safari: false,
        continue_on_error: true,
        timeout: Duration::from_secs(2),
        json,
    }
}

#[derive(Debug, PartialEq)]
enum Output {
    Out(String),
    Error(String),
    Prompt,
}

fn record(output: &mut Vec<Output>, item: Emission<'_>) -> Result<()> {
    output.push(match item {
        Emission::OutLine(value) => Output::Out(value.to_owned()),
        Emission::ErrorLine(value) => Output::Error(value.to_owned()),
        Emission::Prompt => Output::Prompt,
    });
    Ok(())
}

fn out(item: &Output) -> &str {
    let Output::Out(value) = item else {
        panic!("expected stdout record")
    };
    value
}

#[tokio::test]
async fn lifecycle_shell_normal_formats_default_identity_and_owned_cleanup() {
    for json in [false, true] {
        let (mut source, stream) = fixture(FlushFault::None);
        source.uses_rsd = json;
        let options = options(json);
        let mut output = Vec::new();
        let finished = AtomicUsize::new(0);
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    &options,
                    false,
                    |item| record(&mut output, item),
                    pending_signal(),
                    || Ok(&b" 6*7 \n6*7\n.exit\n"[..]),
                    |_| {
                        finished.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    },
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert_eq!(source.lists.load(Ordering::SeqCst), 1);
        assert_eq!(*source.connects.lock().unwrap(), ["owned-network-first"]);
        assert_eq!(trace.methods, ["Runtime.evaluate", "Runtime.evaluate"]);
        assert_eq!(trace.teardowns, 1);
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if json {
            let metadata: Value = serde_json::from_str(out(&output[0])).unwrap();
            assert_eq!(
                metadata,
                json!({"kind":"session", "bundle_id":BUNDLE, "application_id":APP,
                "page_id":7, "page_key":"page-7", "page_type":"web_page", "url":URL, "title":TITLE,
                "session_id":trace.session, "inspector_connection_id":trace.connection,
                "automation_connection_id":null, "service_name":RSD_SERVICE_NAME})
            );
            for (index, item) in output[1..].iter().enumerate() {
                assert_eq!(
                    serde_json::from_str::<Value>(out(item)).unwrap(),
                    json!({"kind":"result", "expression":"6*7",
                    "result":{"id":index+1,"result":{"result":{"type":"number","value":42}}}})
                );
            }
        } else {
            assert_eq!(
                output,
                [
                    Output::Out(format!("Connected to {BUNDLE} page=7 {URL}")),
                    Output::Out("42".into()),
                    Output::Out("42".into())
                ]
            );
        }
    }
}

#[tokio::test]
async fn lifecycle_shell_exit_blank_prompt_and_eof_keep_existing_channels() {
    for input in ["\n.exit\n", "exit\n", "quit\n", ""] {
        let (source, stream) = fixture(FlushFault::None);
        let mut output = Vec::new();
        let options = options(false);
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    Some("owned-explicit".into()),
                    &options,
                    true,
                    |item| record(&mut output, item),
                    pending_signal(),
                    || Ok(input.as_bytes()),
                    |_| Ok(()),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert!(trace.methods.is_empty());
        assert_eq!(trace.teardowns, 1);
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
        assert_eq!(
            output
                .iter()
                .filter(|item| **item == Output::Prompt)
                .count(),
            if input.starts_with('\n') { 2 } else { 1 }
        );
        if input.is_empty() {
            assert_eq!(output.last().unwrap(), &Output::Out(String::new()));
        }
    }
}

#[tokio::test]
async fn lifecycle_shell_safari_navigation_and_explicit_page_precedence() {
    let (source, stream) = fixture(FlushFault::None);
    let url = "https://example.invalid/路径?quote=\"a\"";
    let expression = format!("window.location = {}", serde_json::to_string(url).unwrap());
    let mut options = options(true);
    options.open_safari = true;
    options.bundle_id = Some("com.example.other");
    options.url = Some(url);
    let mut output = Vec::new();
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                Some("owned-explicit".into()),
                &options,
                false,
                |item| record(&mut output, item),
                pending_signal(),
                || Ok(&b"6*7\n"[..]),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    expressions: vec![expression, "6*7".into()],
                    ..Default::default()
                }
            )
        )
    })
    .await;
    result.unwrap();
    let trace = trace.unwrap();
    assert!(trace
        .selectors
        .iter()
        .any(|selector| selector == "_rpc_requestApplicationLaunch:"));
    assert_eq!(trace.methods, ["Runtime.evaluate", "Runtime.evaluate"]);
    assert_eq!(trace.teardowns, 1);
    assert_eq!(output.len(), 2, "navigation has no extra result record");
}

#[tokio::test]
async fn lifecycle_shell_zero_overflow_and_ready_signal_precede_input_and_discovery() {
    for timeout in [Duration::ZERO, Duration::MAX] {
        let (source, _peer) = fixture(FlushFault::None);
        let mut options = options(true);
        options.timeout = timeout;
        let opens = AtomicUsize::new(0);
        let error = run(
            &source,
            None,
            &options,
            false,
            |_| panic!("no output"),
            pending_signal(),
            || {
                opens.fetch_add(1, Ordering::SeqCst);
                Ok(&b""[..])
            },
            |_| panic!("no acquired input"),
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(
            classify_error(error.as_ref()),
            if timeout.is_zero() {
                ErrorClass::Timeout
            } else {
                ErrorClass::InvalidInput
            }
        );
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
        assert!(source.connects.lock().unwrap().is_empty());
    }
    for failed_signal in [false, true] {
        let (source, _peer) = fixture(FlushFault::None);
        let options = options(false);
        let error = run(
            &source,
            None,
            &options,
            false,
            |_| panic!("no output"),
            async {
                if failed_signal {
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            },
            || -> std::io::Result<&[u8]> { panic!("ready signal must precede input acquisition") },
            |_| panic!("no acquired input"),
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(
            classify_error(error.as_ref()),
            if failed_signal {
                ErrorClass::PermissionDenied
            } else {
                ErrorClass::Cancelled
            }
        );
        assert_eq!(
            error.downcast_ref::<OperationCancelled>().is_some(),
            !failed_signal
        );
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn lifecycle_shell_admission_failure_and_empty_default_restore_owned_input() {
    for empty in [false, true] {
        let (mut source, _peer) = fixture(FlushFault::None);
        if empty {
            source.devices.clear();
        } else {
            source.list_error = true;
        }
        let options = options(false);
        let finished = AtomicUsize::new(0);
        let error = run(
            &source,
            None,
            &options,
            false,
            |_| panic!("no output"),
            pending_signal(),
            || Ok(&b""[..]),
            |_| {
                finished.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert!(source.connects.lock().unwrap().is_empty());
        if empty {
            assert_eq!(
                error.to_string(),
                "--udid required: no connected iOS devices found"
            );
        } else {
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::PermissionDenied
            );
        }
    }
    let (source, _peer) = fixture(FlushFault::None);
    let error = run(
        &source,
        None,
        &options(false),
        false,
        |_| panic!("no output"),
        pending_signal(),
        || -> std::io::Result<&[u8]> {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        },
        |_| Ok(()),
        CLEANUP_BUDGET,
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(source.lists.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_shell_connect_and_startup_share_setup_total() {
    let (mut source, mut stream) = fixture(FlushFault::None);
    source.connect_delay = Duration::from_millis(60);
    let mut options = options(false);
    options.timeout = Duration::from_millis(100);
    let started = tokio::time::Instant::now();
    let (result, ()) = bounded(async {
        tokio::join!(
            run(
                &source,
                Some("owned-explicit".into()),
                &options,
                false,
                |_| panic!("no output"),
                pending_signal(),
                || Ok(&b""[..]),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            async {
                let request = read_frame(&mut stream).await.unwrap().unwrap();
                assert_eq!(
                    request["__selector"].as_string(),
                    Some("_rpc_reportIdentifier:")
                );
                assert!(read_frame(&mut stream).await.unwrap().is_none());
            }
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some());
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "startup must not restart the 100ms total"
    );
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_shell_cancelled_incomplete_attach_is_typed_without_guessed_teardown() {
    let (source, stream) = fixture(FlushFault::None);
    let notify = Arc::new(Notify::new());
    let options = options(true);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |_| panic!("no metadata before attachment"),
                async {
                    notify.notified().await;
                    Ok(())
                },
                || Ok(&b""[..]),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    setup_stall: Some(notify.clone()),
                    ..Default::default()
                }
            )
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Cancelled);
    assert!(error.downcast_ref::<OperationCancelled>().is_some());
    assert_eq!(trace.unwrap().teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_shell_idle_is_unbounded_and_active_cancel_cleans_once() {
    let (source, stream) = fixture(FlushFault::TeardownDelay);
    let (input, _writer) = tokio::io::duplex(64);
    let metadata = Notify::new();
    let mut options = options(true);
    options.timeout = Duration::from_millis(100);
    let finished = AtomicUsize::new(0);
    let start = tokio::time::Instant::now();
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |_| {
                    metadata.notify_one();
                    Ok(())
                },
                async {
                    metadata.notified().await;
                    tokio::time::sleep(Duration::from_millis(160)).await;
                    Ok(())
                },
                || Ok(input),
                |_| {
                    finished.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                Duration::from_millis(80)
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    result.unwrap();
    let trace = trace.unwrap();
    assert_eq!(trace.teardowns, 1);
    assert!(trace.methods.is_empty());
    assert!(start.elapsed() >= Duration::from_millis(160));
    assert_eq!(finished.load(Ordering::SeqCst), 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_shell_evaluations_get_fresh_totals_after_long_idle() {
    let (source, stream) = fixture(FlushFault::None);
    let (input, mut writer) = tokio::io::duplex(64);
    let metadata = Notify::new();
    let mut options = options(false);
    options.timeout = Duration::from_millis(100);
    let mut output = Vec::new();
    let (result, trace, ()) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |item| {
                    metadata.notify_one();
                    record(&mut output, item)
                },
                pending_signal(),
                || Ok(input),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    eval_delay: Duration::from_millis(60),
                    ..Default::default()
                }
            ),
            async {
                metadata.notified().await;
                tokio::time::sleep(Duration::from_millis(140)).await;
                writer.write_all(b"6*7\n6*7\n.exit\n").await.unwrap();
            }
        )
    })
    .await;
    result.unwrap();
    assert_eq!(
        trace.unwrap().methods,
        ["Runtime.evaluate", "Runtime.evaluate"]
    );
    assert_eq!(
        output[1..],
        [Output::Out("42".into()), Output::Out("42".into())]
    );
}

#[tokio::test]
async fn lifecycle_shell_committed_expiry_or_cancel_never_continues_invalid_client() {
    for cancel in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let notify = Arc::new(Notify::new());
        let mut options = options(false);
        options.timeout = Duration::from_millis(100);
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    &options,
                    false,
                    |_| Ok(()),
                    async {
                        notify.notified().await;
                        if cancel {
                            Ok(())
                        } else {
                            std::future::pending().await
                        }
                    },
                    || Ok(&b"6*7\n6*7\n"[..]),
                    |_| Ok(()),
                    CLEANUP_BUDGET
                ),
                peer(
                    stream,
                    PeerPolicy {
                        eval_stall: Some(notify.clone()),
                        ..Default::default()
                    }
                )
            )
        })
        .await;
        if cancel {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
            assert!(error
                .downcast_ref::<tokio::time::error::Elapsed>()
                .is_some());
        }
        let trace = trace.unwrap();
        assert_eq!(trace.methods, ["Runtime.evaluate"]);
        assert_eq!(trace.teardowns, 0);
        assert!(source.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn lifecycle_shell_native_eof_and_codec_failures_stop_with_original_sources() {
    for invalid in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let (result, trace) = bounded(async {
            let config = options(true);
            tokio::join!(
                run(
                    &source,
                    None,
                    &config,
                    false,
                    |_| Ok(()),
                    pending_signal(),
                    || Ok(&b"6*7\n6*7\n"[..]),
                    |_| Ok(()),
                    CLEANUP_BUDGET
                ),
                peer(
                    stream,
                    PeerPolicy {
                        eval_disconnect: !invalid,
                        eval_invalid: invalid,
                        ..Default::default()
                    }
                )
            )
        })
        .await;
        let error = result.unwrap_err();
        if invalid {
            assert!(error.chain().any(|source| source.is::<plist::Error>()));
        } else {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
        }
        let trace = trace.unwrap();
        assert_eq!(trace.methods, ["Runtime.evaluate"]);
        assert_eq!(trace.teardowns, 0);
        assert!(source.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn lifecycle_shell_only_completed_business_errors_may_continue() {
    for json in [false, true] {
        for keep_going in [false, true] {
            let (source, stream) = fixture(FlushFault::None);
            let mut options = options(json);
            options.continue_on_error = keep_going;
            let mut output = Vec::new();
            let (result, trace) = bounded(async {
                tokio::join!(
                    run(
                        &source,
                        None,
                        &options,
                        false,
                        |item| record(&mut output, item),
                        pending_signal(),
                        || Ok(&b"6*7\n6*7\n"[..]),
                        |_| Ok(()),
                        CLEANUP_BUDGET
                    ),
                    peer(
                        stream,
                        PeerPolicy {
                            eval_error: true,
                            ..Default::default()
                        }
                    )
                )
            })
            .await;
            let trace = trace.unwrap();
            assert_eq!(trace.teardowns, 1);
            assert_eq!(trace.methods.len(), if keep_going { 2 } else { 1 });
            if keep_going {
                result.unwrap();
                assert_eq!(output.len(), 3);
                for item in &output[1..] {
                    if json {
                        assert_eq!(
                            serde_json::from_str::<Value>(out(item)).unwrap(),
                            json!({"kind":"error","expression":"6*7","error":"owned completed business failure"})
                        );
                    } else {
                        assert_eq!(
                            item,
                            &Output::Error("owned completed business failure".into())
                        );
                    }
                }
            } else {
                assert!(result.unwrap_err().to_string().contains("evaluate error"));
                assert_eq!(output.len(), 1);
            }
        }
    }
}

#[tokio::test]
async fn lifecycle_shell_writer_failure_keeps_primary_through_native_or_stalled_cleanup() {
    for fault in [
        FlushFault::None,
        FlushFault::TeardownIo,
        FlushFault::TeardownStall,
    ] {
        let (source, stream) = fixture(fault);
        let (result, trace) = bounded(async {
            let config = options(true);
            tokio::join!(
                run(
                    &source,
                    None,
                    &config,
                    false,
                    |_| Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()),
                    pending_signal(),
                    || Ok(&b""[..]),
                    |_| Ok(()),
                    Duration::from_millis(40)
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(
            error
                .chain()
                .find_map(|source| source.downcast_ref::<std::io::Error>())
                .unwrap()
                .kind(),
            std::io::ErrorKind::BrokenPipe
        );
        match fault {
            FlushFault::None => assert!(error.downcast_ref::<CleanupFailure>().is_none()),
            FlushFault::TeardownIo => assert_eq!(
                error
                    .downcast_ref::<CleanupFailure>()
                    .unwrap()
                    .cleanup
                    .chain()
                    .find_map(|source| source.downcast_ref::<std::io::Error>())
                    .unwrap()
                    .kind(),
                std::io::ErrorKind::PermissionDenied
            ),
            FlushFault::TeardownStall => assert_eq!(
                classify_error(
                    error
                        .downcast_ref::<CleanupFailure>()
                        .unwrap()
                        .cleanup
                        .as_ref()
                ),
                ErrorClass::Timeout
            ),
            _ => unreachable!(),
        }
        assert_eq!(trace.unwrap().teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn lifecycle_shell_late_evaluation_output_expires_then_cleanup_uses_fresh_budget() {
    let (source, stream) = fixture(FlushFault::TeardownDelay);
    let mut options = options(false);
    options.timeout = Duration::from_millis(100);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |item| {
                    if matches!(item, Emission::OutLine("42")) {
                        std::thread::sleep(Duration::from_millis(110));
                    }
                    Ok(())
                },
                pending_signal(),
                || Ok(&b"6*7\n6*7\n"[..]),
                |_| Ok(()),
                Duration::from_millis(80)
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some());
    let trace = trace.unwrap();
    assert_eq!(trace.methods, ["Runtime.evaluate"]);
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_shell_active_output_cancel_is_normal_stop_but_cleanup_failure_is_error() {
    for fault in [FlushFault::TeardownDelay, FlushFault::TeardownIo] {
        let (source, stream) = fixture(fault);
        let notify = Notify::new();
        let (result, trace) = bounded(async {
            let config = options(false);
            tokio::join!(
                run(
                    &source,
                    None,
                    &config,
                    false,
                    |item| {
                        if matches!(item, Emission::OutLine("42")) {
                            notify.notify_one();
                        }
                        Ok(())
                    },
                    async {
                        notify.notified().await;
                        Ok(())
                    },
                    || Ok(&b"6*7\n6*7\n"[..]),
                    |_| Ok(()),
                    Duration::from_millis(80)
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        match fault {
            FlushFault::TeardownDelay => result.unwrap(),
            FlushFault::TeardownIo => assert_eq!(
                classify_error(result.unwrap_err().as_ref()),
                ErrorClass::PermissionDenied
            ),
            _ => unreachable!(),
        }
        let trace = trace.unwrap();
        assert_eq!(trace.methods, ["Runtime.evaluate"]);
        assert_eq!(trace.teardowns, 1);
    }
}

struct FailedInput;
impl AsyncRead for FailedInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Err(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        )))
    }
}

struct BlankInput(Arc<AtomicUsize>);
impl AsyncRead for BlankInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if buf.remaining() != 0 {
            buf.put_slice(b"\n");
        }
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn lifecycle_shell_continuously_ready_blank_input_yields_to_cancellation() {
    let (source, stream) = fixture(FlushFault::None);
    let metadata = Notify::new();
    let reads = Arc::new(AtomicUsize::new(0));
    let options = options(true);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |_| {
                    metadata.notify_one();
                    Ok(())
                },
                async {
                    metadata.notified().await;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Ok(())
                },
                || Ok(BlankInput(reads.clone())),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    result.unwrap();
    assert!(reads.load(Ordering::SeqCst) > 1);
    let trace = trace.unwrap();
    assert!(trace.methods.is_empty());
    assert_eq!(trace.teardowns, 1);
}

#[tokio::test]
async fn lifecycle_shell_setup_output_cancel_is_typed_and_retains_independent_cleanup() {
    let (source, stream) = fixture(FlushFault::TeardownDelay);
    let notify = Notify::new();
    let options = options(true);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |_| {
                    notify.notify_one();
                    Ok(())
                },
                async {
                    notify.notified().await;
                    Ok(())
                },
                || Ok(&b""[..]),
                |_| Ok(()),
                Duration::from_millis(80)
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    let error = result.unwrap_err();
    assert!(error.downcast_ref::<OperationCancelled>().is_some());
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Cancelled);
    assert_eq!(trace.unwrap().teardowns, 1);
}

#[tokio::test]
async fn lifecycle_shell_completed_protocol_error_continues_with_safe_error_records() {
    let (source, stream) = fixture(FlushFault::None);
    let options = options(true);
    let mut output = Vec::new();
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |item| record(&mut output, item),
                pending_signal(),
                || Ok(&b"6*7\n6*7\n"[..]),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    eval_protocol_error: true,
                    ..Default::default()
                }
            )
        )
    })
    .await;
    result.unwrap();
    let trace = trace.unwrap();
    assert_eq!(trace.methods, ["Runtime.evaluate", "Runtime.evaluate"]);
    assert_eq!(trace.teardowns, 1);
    for item in &output[1..] {
        let value: Value = serde_json::from_str(out(item)).unwrap();
        assert_eq!(value["kind"], "error");
        assert_eq!(value["expression"], "6*7");
        assert!(value["error"]
            .as_str()
            .unwrap()
            .contains("WebInspector protocol request failed:"));
        assert!(!value.to_string().contains("owned private protocol payload"));
    }
}

#[cfg(target_os = "linux")]
struct ReadStarted<R, F> {
    inner: R,
    ready: Option<F>,
}

#[cfg(target_os = "linux")]
impl<R: AsyncRead + Unpin, F: FnOnce() + Unpin> AsyncRead for ReadStarted<R, F> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if let Some(ready) = self.ready.take() {
            ready();
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

#[cfg(target_os = "linux")]
async fn idle_process<R: AsyncRead + Unpin, F: FnOnce() + Unpin>(
    input: R,
    signal: impl Future<Output = std::io::Result<()>>,
    finish: impl FnOnce(R) -> std::io::Result<()>,
    ready: F,
) {
    let (source, stream) = fixture(FlushFault::None);
    let options = options(true);
    let input = ReadStarted {
        inner: input,
        ready: Some(ready),
    };
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                &options,
                false,
                |_| { Ok(()) },
                signal,
                || Ok(input),
                |input| finish(input.inner),
                CLEANUP_BUDGET
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    result.unwrap();
    let trace = trace.unwrap();
    assert_eq!(trace.teardowns, 1);
    assert!(trace.methods.is_empty());
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn lifecycle_shell_process_child() {
    if std::env::var_os("IOS_TEST_OWNED_SHELL_STDIN").is_some() {
        idle_process(
            Input::open().unwrap(),
            tokio::signal::ctrl_c(),
            Input::finish,
            || {
                let mut stdout = std::io::stdout().lock();
                writeln!(stdout, "owned shell ready").unwrap();
                stdout.flush().unwrap();
            },
        )
        .await;
    } else {
        let (input, _writer) = tokio::io::duplex(64);
        let notify = Notify::new();
        idle_process(
            input,
            async {
                notify.notified().await;
                tokio::time::sleep(Duration::from_millis(20)).await;
                Ok(())
            },
            |_| Ok(()),
            || notify.notify_one(),
        )
        .await;
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn lifecycle_shell_actual_sigint_exits_with_idle_stdin_writer_still_open() {
    use tokio::io::AsyncBufReadExt;
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cmd::webinspector::tasks::interactive::tests::lifecycle_shell_process_child",
            "--nocapture",
        ])
        .env("IOS_TEST_OWNED_SHELL_STDIN", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let writer = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let ready = tokio::time::timeout(Duration::from_secs(2), async {
        for _ in 0..16 {
            let mut line = String::new();
            let count = output.read_line(&mut line).await.unwrap();
            assert!(line.len() < 4096 && count > 0);
            if line == "owned shell ready\n" {
                return;
            }
        }
        panic!("owned child did not reach shell idle input");
    })
    .await;
    if ready.is_err() {
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
    ready.expect("owned shell must reach its input wait");
    let pid = i32::try_from(child.id().unwrap()).unwrap();
    // SAFETY: this live child PID is owned by this test; only it receives SIGINT.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGINT) }, 0);
    let status = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
    if status.is_err() {
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
    assert!(status
        .expect("shell and runtime must exit before stdin writer closes")
        .unwrap()
        .success());
    drop(writer);
}

#[tokio::test]
async fn lifecycle_shell_input_utf8_limit_and_native_io_failures_clean_attached_session() {
    for input in [vec![0xff, b'\n'], vec![b'x'; 1024 * 1024 + 1]] {
        let (source, stream) = fixture(FlushFault::None);
        let (result, trace) = bounded(async {
            let config = options(false);
            tokio::join!(
                run(
                    &source,
                    None,
                    &config,
                    false,
                    |_| Ok(()),
                    pending_signal(),
                    || Ok(input.as_slice()),
                    |_| Ok(()),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        if input[0] == 0xff {
            assert!(error.downcast_ref::<std::string::FromUtf8Error>().is_some());
        } else {
            assert!(error.to_string().contains("input line exceeds"));
        }
        let trace = trace.unwrap();
        assert!(trace.methods.is_empty());
        assert_eq!(trace.teardowns, 1);
    }
    let (source, stream) = fixture(FlushFault::None);
    let (result, trace) = bounded(async {
        let config = options(false);
        tokio::join!(
            run(
                &source,
                None,
                &config,
                false,
                |_| Ok(()),
                pending_signal(),
                || Ok(FailedInput),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    assert_eq!(
        classify_error(result.unwrap_err().as_ref()),
        ErrorClass::PermissionDenied
    );
    assert_eq!(trace.unwrap().teardowns, 1);
}

#[tokio::test]
async fn lifecycle_shell_signal_registration_failure_during_idle_stays_native() {
    let (source, stream) = fixture(FlushFault::None);
    let metadata = Notify::new();
    let (input, _writer) = tokio::io::duplex(64);
    let (result, trace) = bounded(async {
        let config = options(false);
        tokio::join!(
            run(
                &source,
                None,
                &config,
                false,
                |_| {
                    metadata.notify_one();
                    Ok(())
                },
                async {
                    metadata.notified().await;
                    tokio::task::yield_now().await;
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                },
                || Ok(input),
                |_| Ok(()),
                CLEANUP_BUDGET
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
    assert!(error.downcast_ref::<OperationCancelled>().is_none());
    assert_eq!(trace.unwrap().teardowns, 1);
}

#[tokio::test]
async fn lifecycle_shell_input_restoration_failure_preserves_primary_and_is_observable() {
    for fail_primary in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let (result, trace) = bounded(async {
            let config = options(false);
            tokio::join!(
                run(
                    &source,
                    None,
                    &config,
                    false,
                    |_| {
                        if fail_primary {
                            Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
                        } else {
                            Ok(())
                        }
                    },
                    pending_signal(),
                    || Ok(&b""[..]),
                    |_| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(
            error
                .chain()
                .find_map(|source| source.downcast_ref::<std::io::Error>())
                .unwrap()
                .kind(),
            if fail_primary {
                std::io::ErrorKind::BrokenPipe
            } else {
                std::io::ErrorKind::PermissionDenied
            }
        );
        if fail_primary {
            assert!(error.downcast_ref::<CleanupFailure>().is_some());
        }
        assert_eq!(trace.unwrap().teardowns, 1);
    }
}

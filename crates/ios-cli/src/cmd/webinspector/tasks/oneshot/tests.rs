//! Actual finite command orchestration over an owned, bounded plist peer.
//! Device admission and output are private injected sources, never real devices.
use super::*;
use ios_core::{classify_error, ErrorClass, OperationCancelled};
use serde_json::Value as JsonValue;
use std::sync::{atomic::Ordering, Arc};
use tokio::io::AsyncWriteExt;

use super::super::test_peer::*;

fn eval() -> Command<'static> {
    Command::Eval {
        app_id: None,
        bundle_id: BUNDLE.into(),
        page_selector: "page-7",
        expression: "6*7",
    }
}

fn budget(command: &Command<'_>) -> Budget {
    Budget::new(Duration::from_secs(3), command.phase()).unwrap()
}

#[tokio::test]
async fn lifecycle_oneshot_tabs_preserve_rows_human_output_and_first_default() {
    for json in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let command = Command::Tabs;
        let mut output = Vec::new();
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    budget(&Command::Tabs),
                    json,
                    |line| {
                        output.push(line.to_owned());
                        Ok(())
                    },
                    pending_signal(),
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
        assert!(source.dropped.load(Ordering::SeqCst));
        assert_eq!(trace.teardowns, 0);
        assert!(!trace
            .selectors
            .iter()
            .any(|selector| selector == "_rpc_forwardSocketSetup:"));
        if json {
            let rows: JsonValue = serde_json::from_str(&output[0]).unwrap();
            assert_eq!(
                rows,
                json!([{"application_id":APP,"bundle_identifier":BUNDLE,"application_name":"Safari",
                "pid":42,"page_id":7,"page_key":"page-7","page_type":"web_page","title":TITLE,"url":URL}])
            );
        } else {
            assert_eq!(
                output,
                [
                    format!("Safari [{BUNDLE}] page=7 {TITLE} {URL}"),
                    format!("  app_id: {APP}"),
                    "  type: web_page".into()
                ]
            );
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_eval_preserves_value_json_page_key_and_teardown() {
    for json in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let command = eval();
        let total = budget(&command);
        let mut output = Vec::new();
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    Some("owned-explicit".into()),
                    command,
                    total,
                    json,
                    |line| {
                        output.push(line.to_owned());
                        Ok(())
                    },
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
        assert_eq!(*source.connects.lock().unwrap(), ["owned-explicit"]);
        assert_eq!(trace.methods, ["Runtime.evaluate"]);
        assert_eq!(trace.teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if json {
            assert_eq!(
                serde_json::from_str::<JsonValue>(&output[0]).unwrap(),
                json!({"id":1,"result":{"result":{"type":"number","value":42}}})
            );
        } else {
            assert_eq!(output, ["42"]);
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_launch_preserves_metadata_navigation_and_owned_cleanup() {
    for json in [false, true] {
        let (mut source, stream) = fixture(FlushFault::None);
        source.uses_rsd = json;
        let command = Command::Launch {
            bundle_id: BUNDLE,
            url: Some(URL),
        };
        let total = budget(&command);
        let mut output = Vec::new();
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    total,
                    json,
                    |line| {
                        output.push(line.to_owned());
                        Ok(())
                    },
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert_eq!(
            trace.methods,
            [
                "Automation.createBrowsingContext",
                "Automation.navigateBrowsingContext",
                "Automation.getBrowsingContext",
                "Automation.evaluateJavaScriptFunction",
                "Automation.closeBrowsingContext"
            ]
        );
        assert_eq!(trace.teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if json {
            assert_eq!(
                serde_json::from_str::<JsonValue>(&output[0]).unwrap(),
                json!({
                    "bundle_id":BUNDLE,"application_id":APP,"application_name":"Safari","pid":42,
                    "page_id":7,"page_key":"page-7","page_type":"automation","url":URL,"title":TITLE,
                    "session_id":trace.session,"inspector_connection_id":trace.connection,
                    "automation_connection_id":"owned-automation-connection","service_name":RSD_SERVICE_NAME,"uses_rsd":true,
                })
            );
        } else {
            assert_eq!(
                output,
                [
                    format!("Launched {BUNDLE} (pid 42)"),
                    "  page: 7".into(),
                    format!("  url: {URL}"),
                    format!("  title: {TITLE}"),
                    format!("  session: {}", trace.session.unwrap()),
                    "  automation connection: owned-automation-connection".into(),
                    format!("  service: {SERVICE_NAME}")
                ]
            );
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_ready_cancel_or_signal_error_precedes_default_selection() {
    for failed_signal in [false, true] {
        let (source, _peer) = fixture(FlushFault::None);
        let error = run(
            &source,
            None,
            Command::Tabs,
            budget(&Command::Tabs),
            true,
            |_| panic!("no output"),
            async {
                if failed_signal {
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            },
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
        assert!(source.connects.lock().unwrap().is_empty());
        assert_eq!(
            classify_error(error.as_ref()),
            if failed_signal {
                ErrorClass::PermissionDenied
            } else {
                ErrorClass::Cancelled
            }
        );
    }
}

#[tokio::test]
async fn lifecycle_oneshot_default_selection_failures_precede_connection() {
    for permission_error in [false, true] {
        let (mut source, _peer) = fixture(FlushFault::None);
        source.devices.clear();
        source.list_error = permission_error;
        let error = run(
            &source,
            None,
            Command::Tabs,
            budget(&Command::Tabs),
            true,
            |_| panic!("no output"),
            pending_signal(),
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(source.lists.load(Ordering::SeqCst), 1);
        assert!(source.connects.lock().unwrap().is_empty());
        if permission_error {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        } else {
            assert_eq!(
                error.to_string(),
                "--udid required: no connected iOS devices found"
            );
        }
    }
    let (mut source, _peer) = fixture(FlushFault::None);
    source.connect_error = true;
    let error = run(
        &source,
        Some("owned".into()),
        Command::Tabs,
        budget(&Command::Tabs),
        true,
        |_| panic!("no output"),
        pending_signal(),
        CLEANUP_BUDGET,
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(source.lists.load(Ordering::SeqCst), 0);
    assert_eq!(source.connects.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn lifecycle_oneshot_connection_and_startup_share_one_absolute_total() {
    let (mut source, mut stream) = fixture(FlushFault::None);
    source.connect_delay = Duration::from_millis(60);
    let total = Budget::new(Duration::from_millis(100), "opened-tabs").unwrap();
    let (result, ()) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                Command::Tabs,
                total,
                true,
                |_| panic!("no output"),
                pending_signal(),
                CLEANUP_BUDGET
            ),
            async {
                let request = read_frame(&mut stream).await.unwrap().unwrap();
                assert_eq!(
                    request["__selector"].as_string(),
                    Some("_rpc_reportIdentifier:")
                );
                tokio::time::sleep(Duration::from_millis(60)).await;
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
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_startup_native_eof_and_plist_errors_drop_without_teardown() {
    for malformed in [false, true] {
        let (source, mut stream) = fixture(FlushFault::None);
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    Command::Tabs,
                    budget(&Command::Tabs),
                    true,
                    |_| panic!("no output"),
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                async {
                    let _ = read_frame(&mut stream).await.unwrap().unwrap();
                    if malformed {
                        stream.write_all(&5u32.to_be_bytes()).await.unwrap();
                        stream.write_all(b"wrong").await.unwrap();
                        assert!(read_frame(&mut stream).await.unwrap().is_none());
                    } else {
                        stream.write_all(&[0, 0]).await.unwrap();
                        drop(stream);
                    }
                }
            )
        })
        .await;
        let error = result.unwrap_err();
        assert!(source.dropped.load(Ordering::SeqCst));
        if malformed {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
            assert!(error.chain().any(|source| source.is::<plist::Error>()));
            assert!(matches!(
                error.downcast_ref::<ios_core::webinspector::WebInspectorError>(),
                Some(ios_core::webinspector::WebInspectorError::Plist(_))
            ));
        } else {
            assert!(error.chain().any(|source| source.is::<std::io::Error>()));
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
            assert!(error.chain().any(|source| source
                .downcast_ref::<std::io::Error>()
                .is_some_and(|source| source.kind() == std::io::ErrorKind::UnexpectedEof)));
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_cancelled_committed_eval_drops_stream_without_retry() {
    let (source, stream) = fixture(FlushFault::None);
    let notify = Arc::new(tokio::sync::Notify::new());
    let command = eval();
    let total = budget(&command);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| panic!("no output"),
                async {
                    notify.notified().await;
                    Ok(())
                },
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
    let error = result.unwrap_err();
    assert!(error.downcast_ref::<OperationCancelled>().is_some());
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(trace.unwrap().teardowns, 0);
}

#[tokio::test]
async fn lifecycle_oneshot_completed_business_failure_keeps_primary_and_cleans_once() {
    let (source, stream) = fixture(FlushFault::None);
    let command = eval();
    let total = budget(&command);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| panic!("no output"),
                pending_signal(),
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
    assert!(result
        .unwrap_err()
        .to_string()
        .starts_with("webinspector evaluate error:"));
    assert_eq!(trace.unwrap().teardowns, 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_writer_failure_retains_primary_and_secondary_native_sources() {
    for fault in [
        FlushFault::None,
        FlushFault::TeardownIo,
        FlushFault::TeardownStall,
    ] {
        let (source, stream) = fixture(fault);
        let command = eval();
        let total = budget(&command);
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    total,
                    true,
                    |_| Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()),
                    pending_signal(),
                    Duration::from_millis(40)
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(classify_error(error.as_ref()), ErrorClass::Disconnected);
        assert_eq!(trace.unwrap().teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if !matches!(fault, FlushFault::None) {
            let dual = error.downcast_ref::<control::CleanupFailure>().unwrap();
            assert!(dual.primary.downcast_ref::<std::io::Error>().is_some());
            assert_eq!(
                classify_error(dual.cleanup.as_ref()),
                if matches!(fault, FlushFault::TeardownIo) {
                    ErrorClass::PermissionDenied
                } else {
                    ErrorClass::Timeout
                }
            );
            if matches!(fault, FlushFault::TeardownStall) {
                assert!(dual
                    .cleanup
                    .chain()
                    .any(|source| source.is::<tokio::time::error::Elapsed>()));
            }
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_output_expiry_or_cancel_still_gets_independent_cleanup() {
    for cancelled in [false, true] {
        let (source, stream) = fixture(FlushFault::TeardownDelay);
        let notify = tokio::sync::Notify::new();
        let command = eval();
        let total = budget(&command);
        let mut outputs = 0;
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    total,
                    true,
                    |_| {
                        outputs += 1;
                        if cancelled {
                            notify.notify_one();
                        } else {
                            std::thread::sleep(
                                total
                                    .deadline
                                    .saturating_duration_since(tokio::time::Instant::now())
                                    + Duration::from_millis(2),
                            );
                        }
                        Ok(())
                    },
                    async {
                        notify.notified().await;
                        Ok(())
                    },
                    Duration::from_millis(150)
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(outputs, 1);
        assert_eq!(
            classify_error(error.as_ref()),
            if cancelled {
                ErrorClass::Cancelled
            } else {
                ErrorClass::Timeout
            }
        );
        if !cancelled {
            assert!(error
                .downcast_ref::<tokio::time::error::Elapsed>()
                .is_some());
        }
        assert!(error.downcast_ref::<control::CleanupFailure>().is_none());
        assert_eq!(trace.unwrap().teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn lifecycle_oneshot_cleanup_failure_makes_completed_output_fail() {
    let (source, stream) = fixture(FlushFault::TeardownIo);
    let command = eval();
    let total = budget(&command);
    let mut outputs = 0;
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| {
                    outputs += 1;
                    Ok(())
                },
                pending_signal(),
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
    assert_eq!(outputs, 1);
    assert_eq!(trace.unwrap().teardowns, 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_automation_stop_failure_survives_second_cleanup_expiry() {
    let (source, stream) = fixture(FlushFault::TeardownStall);
    let command = Command::Launch {
        bundle_id: BUNDLE,
        url: None,
    };
    let total = budget(&command);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| Ok(()),
                pending_signal(),
                Duration::from_millis(40)
            ),
            peer(
                stream,
                PeerPolicy {
                    stop_error: true,
                    ..Default::default()
                }
            )
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Protocol);
    let dual = error.downcast_ref::<control::CleanupFailure>().unwrap();
    assert!(dual
        .primary
        .downcast_ref::<ios_core::webinspector::WebInspectorError>()
        .is_some());
    assert_eq!(classify_error(dual.cleanup.as_ref()), ErrorClass::Timeout);
    assert!(dual
        .cleanup
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    assert_eq!(trace.unwrap().teardowns, 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_expired_committed_eval_keeps_elapsed_without_retry() {
    let (source, stream) = fixture(FlushFault::None);
    let command = eval();
    let total = Budget::new(Duration::from_millis(700), command.phase()).unwrap();
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| panic!("no output"),
                pending_signal(),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    eval_stall: Some(Arc::new(tokio::sync::Notify::new())),
                    ..Default::default()
                }
            )
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    let trace = trace.unwrap();
    assert_eq!(trace.methods, ["Runtime.evaluate"]);
    assert_eq!(trace.teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
}

//! Actual browser child constructor, local stop and forwarding cleanup.
use super::super::bridge_tests::{read_frame, send_json};
use super::lifecycle_tests::{
    no_more_requests, setup_before_target, source_with_delay, state, Source,
};
use super::*;
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::io::{AsyncWriteExt, DuplexStream};

const BOUND: Duration = Duration::from_secs(4);

#[tokio::test]
async fn admitted_outputs_preserve_other_uuid_order_and_one_shared_capacity() {
    let (sender, receiver) = mpsc::channel(CDP_PAGE_CHANNEL);
    let permits = Arc::new(Semaphore::new(CDP_PAGE_CHANNEL));
    let output = BrowserOutputSender::Admitted {
        sender,
        permits: permits.clone(),
        closed: Arc::new(Notify::new()),
    };
    let mut outputs = BrowserPageOutputs {
        receiver,
        deferred: VecDeque::new(),
    };
    for id in 0..CDP_PAGE_CHANNEL {
        output
            .send(BrowserPageOutput::Message {
                session_id: if id % 2 == 0 { "retired" } else { "other" }.into(),
                message: json!({"id":id}),
            })
            .await
            .unwrap();
    }
    assert_eq!(permits.available_permits(), 0);
    let retired = outputs.drain_session("retired").unwrap();
    assert_eq!(retired.len(), CDP_PAGE_CHANNEL / 2);
    assert_eq!(outputs.deferred.len(), CDP_PAGE_CHANNEL / 2);
    assert_eq!(
        permits.available_permits(),
        0,
        "draining retains original admission"
    );
    drop(retired);
    assert_eq!(permits.available_permits(), CDP_PAGE_CHANNEL / 2);
    for id in (1..CDP_PAGE_CHANNEL).step_by(2) {
        let next = outputs.next().await.unwrap();
        let BrowserPageOutput::Message {
            session_id,
            message,
        } = &next.value
        else {
            panic!("message required")
        };
        assert_eq!(session_id, "other");
        assert_eq!(message["id"], id);
        drop(next);
    }
    assert_eq!(permits.available_permits(), CDP_PAGE_CHANNEL);
}

#[test]
fn independent_discovery_retains_final_context_after_child_terminal_and_only_once() {
    let mut outcome = browser_tasks::PageOutcome::default();
    outcome.context_candidate(Some(
        json!({"method":"Runtime.executionContextDestroyed", "params":{"executionContextId":1}}),
    ));
    outcome.terminal();
    outcome.observe_discovery_closed();
    outcome.observe_discovery_closed();
    assert!(outcome.is_terminal());
    assert_eq!(
        outcome.take_final_message().unwrap()["params"]["executionContextId"],
        1
    );
    assert!(outcome.take_final_message().is_none());
}
static PAGE: AtomicU64 = AtomicU64::new(18000);
async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned browser child fixture stalled")
}

struct Child {
    source: Arc<Source>,
    peer: DuplexStream,
    owner: Option<BrowserPageSession>,
    output: mpsc::Receiver<BrowserPageOutput>,
    page: u64,
    session: String,
}

async fn opened() -> Child {
    let page = PAGE.fetch_add(1, Ordering::SeqCst);
    let session = format!("owned-browser-child-{page}");
    let target = format!("PID:42:page-{page}");
    let (source, mut peer) = source_with_delay(Duration::ZERO);
    let server = state(source.clone(), Duration::from_millis(50));
    let (output, received) = mpsc::channel(CDP_PAGE_CHANNEL);
    let (result,())=bounded(async {
        tokio::join!(open_cdp_browser_page(&server,&target,&session,output),async {
            setup_before_target(&mut peer,page).await;
            send_json(&mut peer,json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-cdp-target"}}})).await.unwrap();
        })
    }).await;
    let owner = Some(result.unwrap());
    Child {
        source,
        peer,
        owner,
        output: received,
        page,
        session,
    }
}

async fn released(child: &mut Child) {
    let frame = bounded(read_frame(&mut child.peer)).await.unwrap().unwrap();
    assert_eq!(
        frame["__selector"].as_string(),
        Some("_rpc_forwardDidClose:")
    );
    let argument = frame["__argument"].as_dictionary().unwrap();
    assert_eq!(
        argument["WIRApplicationIdentifierKey"].as_string(),
        Some("PID:42")
    );
    assert_eq!(
        argument["WIRPageIdentifierKey"].as_unsigned_integer(),
        Some(child.page)
    );
    assert!(argument["WIRSenderKey"]
        .as_string()
        .is_some_and(|sender| !sender.is_empty()));
    released_after_close(child).await;
}

async fn released_after_close(child: &mut Child) {
    no_more_requests(&mut child.peer).await;
    assert!(child.source.dropped.load(Ordering::SeqCst));
    assert_eq!(child.source.calls.load(Ordering::SeqCst), 1);
    if let Some(owner) = &child.owner {
        assert!(owner.sender.is_closed());
    }
    drop(
        bounded(lock_cdp_page(
            &format!("PID:42:page-{}", child.page),
            Duration::from_millis(100),
        ))
        .await
        .unwrap(),
    );
}

async fn closed(child: &mut Child) {
    released(child).await;
    match bounded(child.output.recv()).await.unwrap() {
        BrowserPageOutput::Closed { session_id, .. } => assert_eq!(session_id, child.session),
        BrowserPageOutput::Message { .. } => panic!("cleanup must not publish a page reply"),
    }
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_delayed_flush_finishes_under_independent_cleanup() {
    let mut child = opened().await;
    let before = child.source.flush_completed.load(Ordering::SeqCst);
    child.source.flush_fault.store(3, Ordering::SeqCst);
    let owner = child.owner.as_mut().unwrap();
    owner.cancel.send(true).unwrap();
    bounded(&mut owner.task).await.unwrap();
    assert_eq!(
        child.source.flush_completed.load(Ordering::SeqCst),
        before + 1
    );
    closed(&mut child).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_pending_flush_uses_real_default_cleanup() {
    let mut child = opened().await;
    child.source.flush_fault.store(2, Ordering::SeqCst);
    let began = tokio::time::Instant::now();
    let owner = child.owner.as_mut().unwrap();
    owner.cancel.send(true).unwrap();
    bounded(&mut owner.task).await.unwrap();
    assert!(
        began.elapsed() >= CDP_PAGE_CLEANUP_TIMEOUT,
        "child cleanup must not inherit the 50ms active timeout"
    );
    closed(&mut child).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_supervisor_keeps_native_flush_failure() {
    let mut child = opened().await;
    child.source.flush_fault.store(1, Ordering::SeqCst);
    let error = bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(error.chain().any(|source| source
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
    closed(&mut child).await;
}

async fn invalid_target(child: &mut Child) {
    send_json(
        &mut child.peer,
        json!({"method":"Target.dispatchMessageFromTarget",
        "params":{"targetId":"owned-cdp-target","message":"{"}}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_native_json_and_cleanup_errors_both_survive() {
    let mut child = opened().await;
    child.source.flush_fault.store(1, Ordering::SeqCst);
    invalid_target(&mut child).await;
    // Observe completion without polling the handle twice: the supervisor
    // exclusively owns the actual join and retained-result collection.
    bounded(async {
        while !child.owner.as_ref().unwrap().task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let error = bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap_err();
    let both = error
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .expect("both native causes remain owned");
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Protocol
    );
    assert!(both
        .primary
        .chain()
        .any(|source| source.is::<serde_json::Error>()));
    assert_eq!(
        ios_core::classify_error(both.cleanup.as_ref()),
        ios_core::ErrorClass::PermissionDenied
    );
    assert!(both.cleanup.chain().any(|source| source
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
    closed(&mut child).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_known_json_primary_survives_supervisor_abort_and_join() {
    let mut child = opened().await;
    child.source.flush_fault.store(2, Ordering::SeqCst);
    invalid_target(&mut child).await;
    // Actual cleanup frame proves the JSON failure preceded cleanup/abort.
    let close = bounded(read_frame(&mut child.peer)).await.unwrap().unwrap();
    assert_eq!(
        close["__selector"].as_string(),
        Some("_rpc_forwardDidClose:")
    );
    let error = bounded(stop_browser_pages(
        vec![child.owner.take().unwrap()],
        Duration::from_millis(50),
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Protocol
    );
    assert!(error.chain().any(|source| source.is::<serde_json::Error>()));
    let both = error
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .expect("native primary plus issued supervisor expiry");
    assert_eq!(
        ios_core::classify_error(both.cleanup.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(both
        .cleanup
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    no_more_requests(&mut child.peer).await;
    assert!(child.source.dropped.load(Ordering::SeqCst));
    assert!(bounded(child.output.recv()).await.is_none());
    drop(
        bounded(lock_cdp_page(
            &format!("PID:42:page-{}", child.page),
            Duration::from_millis(100),
        ))
        .await
        .unwrap(),
    );
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_group_stops_every_child_under_one_default_total() {
    let mut first = opened().await;
    let mut second = opened().await;
    first.source.flush_fault.store(2, Ordering::SeqCst);
    second.source.flush_fault.store(2, Ordering::SeqCst);
    let began = tokio::time::Instant::now();
    let error = bounded(stop_browser_pages(
        vec![first.owner.take().unwrap(), second.owner.take().unwrap()],
        CDP_PAGE_CLEANUP_TIMEOUT,
    ))
    .await
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    assert!(began.elapsed() >= CDP_PAGE_CLEANUP_TIMEOUT);
    assert!(
        began.elapsed() < Duration::from_millis(3500),
        "cleanup must not multiply 2s per child"
    );
    for child in [&mut first, &mut second] {
        let close = bounded(read_frame(&mut child.peer)).await.unwrap().unwrap();
        assert_eq!(
            close["__selector"].as_string(),
            Some("_rpc_forwardDidClose:")
        );
        no_more_requests(&mut child.peer).await;
        assert!(child.source.dropped.load(Ordering::SeqCst));
        assert_eq!(child.source.calls.load(Ordering::SeqCst), 1);
        drop(
            bounded(lock_cdp_page(
                &format!("PID:42:page-{}", child.page),
                Duration::from_millis(100),
            ))
            .await
            .unwrap(),
        );
    }
}

async fn reply(child: &mut Child, id: u64) {
    send_json(
        &mut child.peer,
        json!({"method":"Target.dispatchMessageFromTarget","params":{
            "targetId":"owned-cdp-target",
            "message":json!({"id":id,"result":{"value":42}}).to_string()
        }}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_empty_idle_resumes_before_normal_joined_stop() {
    let mut child = opened().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!child.owner.as_ref().unwrap().task.is_finished());
    reply(&mut child, 17).await;
    match bounded(child.output.recv()).await.unwrap() {
        BrowserPageOutput::Message {
            session_id,
            message,
        } => {
            assert_eq!(session_id, child.session);
            assert_eq!(message, json!({"id":17,"result":{"value":42}}));
        }
        BrowserPageOutput::Closed { .. } => panic!("healthy empty idle must resume"),
    }
    bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap();
    closed(&mut child).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_cancel_after_observed_partial_frame_skips_close() {
    let mut child = opened().await;
    let before = child.source.received.load(Ordering::SeqCst);
    child.peer.write_all(&[0, 0]).await.unwrap();
    bounded(async {
        while child.source.received.load(Ordering::SeqCst) != before + 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap();
    // Cancellation after committed bytes invalidates the original client.
    // A successful local stop does not justify sending a recovery request.
    no_more_requests(&mut child.peer).await;
    assert!(child.source.dropped.load(Ordering::SeqCst));
    assert_eq!(child.source.calls.load(Ordering::SeqCst), 1);
    assert!(matches!(bounded(child.output.recv()).await,
        Some(BrowserPageOutput::Closed {session_id, ..}) if session_id == child.session));
    drop(
        bounded(lock_cdp_page(
            &format!("PID:42:page-{}", child.page),
            Duration::from_millis(100),
        ))
        .await
        .unwrap(),
    );
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_cancel_unblocks_full_output_before_cleanup() {
    let mut child = opened().await;
    for id in 0..CDP_PAGE_CHANNEL as u64 {
        reply(&mut child, id).await;
    }
    bounded(async {
        while child.output.len() != CDP_PAGE_CHANNEL {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let before = child.source.received.load(Ordering::SeqCst);
    reply(&mut child, CDP_PAGE_CHANNEL as u64).await;
    bounded(async {
        while child.source.received.load(Ordering::SeqCst) == before {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(!child.owner.as_ref().unwrap().task.is_finished());
    bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap();
    released(&mut child).await;
    // The existing terminal try_send can miss a full queue. Actual join and
    // release are independently proved; no terminal delivery is invented.
    assert_eq!(child.output.len(), CDP_PAGE_CHANNEL);
    assert!(child.output.is_closed());
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_native_output_error_survives_join() {
    let mut child = opened().await;
    child.output.close();
    reply(&mut child, 19).await;
    bounded(async {
        while !child.owner.as_ref().unwrap().task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let error = bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap_err();
    assert!(error
        .chain()
        .any(|source| source.is::<mpsc::error::SendError<BrowserPageOutput>>()));
    released(&mut child).await;
    assert!(bounded(child.output.recv()).await.is_none());
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_supervisor_joins_all_after_panic_or_missing_outcome() {
    // These two standalone actors exercise only supervisor join/outcome
    // admission. The other child is always the actual forwarding constructor.
    for panic in [true, false] {
        let mut child = opened().await;
        let (sender, _) = mpsc::channel(1);
        let (cancel, _) = watch::channel(false);
        let task = tokio::spawn(async move {
            assert!(!panic, "synthetic owned child panic");
        });
        let unexpected = BrowserPageSession {
            sender,
            cancel,
            task,
            outcome: browser_tasks::outcome(),
        };
        let error = bounded(stop_browser_pages(
            vec![unexpected, child.owner.take().unwrap()],
            CDP_PAGE_CLEANUP_TIMEOUT,
        ))
        .await
        .unwrap_err();
        if panic {
            assert!(error.chain().any(|source| source
                .downcast_ref::<tokio::task::JoinError>()
                .is_some_and(|native| native.is_panic())));
        } else {
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Protocol
            );
        }
        closed(&mut child).await;
    }
}

async fn interrupted_cleanup(failure: bool) -> (Child, Child, browser_tasks::BrowserPageCleanup) {
    let mut finished = opened().await;
    let mut pending = opened().await;
    if failure {
        finished.source.flush_fault.store(1, Ordering::SeqCst);
        invalid_target(&mut finished).await;
        bounded(async {
            while !finished.owner.as_ref().unwrap().task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    pending.source.flush_fault.store(2, Ordering::SeqCst);
    let joined = finished.owner.as_ref().unwrap().outcome.clone();
    let mut cleanup = browser_tasks::BrowserPageCleanup::new(
        vec![finished.owner.take().unwrap()],
        Duration::from_millis(100),
    );
    cleanup.push(pending.owner.take().unwrap());
    let mut stop = Box::pin(cleanup.stop());
    bounded(async {
        tokio::select! {
            result = &mut stop => panic!("pending cleanup unexpectedly ended: {result:?}"),
            _ = async {
                let frame = read_frame(&mut pending.peer).await.unwrap().unwrap();
                assert_eq!(frame["__selector"].as_string(), Some("_rpc_forwardDidClose:"));
                while !joined.lock().unwrap().is_joined() {
                    tokio::task::yield_now().await;
                }
            } => {},
        }
    })
    .await;
    drop(stop);
    assert!(joined.lock().unwrap().is_joined());
    assert!(finished.source.dropped.load(Ordering::SeqCst));
    assert!(!pending.source.dropped.load(Ordering::SeqCst));
    (finished, pending, cleanup)
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_borrowed_stop_resumes_after_a_completed_join() {
    let (mut finished, mut pending, mut cleanup) = interrupted_cleanup(false).await;
    let error = bounded(cleanup.stop()).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    closed(&mut finished).await;
    released_after_close(&mut pending).await;
    // Already taken outcomes are never polled again by repeat stop.
    bounded(cleanup.stop()).await.unwrap();
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_borrowed_stop_retains_completed_native_dual_errors() {
    let (mut finished, mut pending, mut cleanup) = interrupted_cleanup(true).await;
    let error = bounded(cleanup.stop()).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Protocol
    );
    assert!(error.chain().any(|source| source.is::<serde_json::Error>()));
    let outer = error
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .unwrap();
    assert!(outer
        .cleanup
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    let child = outer
        .primary
        .downcast_ref::<super::super::tasks::control::CleanupFailure>()
        .unwrap();
    assert!(child.cleanup.chain().any(|source| source
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::PermissionDenied)));
    closed(&mut finished).await;
    released_after_close(&mut pending).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_reparenting_keeps_issued_expiry_during_abort_join() {
    let mut child = opened().await;
    child.source.flush_fault.store(2, Ordering::SeqCst);
    let progress = child.owner.as_ref().unwrap().outcome.clone();
    let mut cleanup = browser_tasks::BrowserPageCleanup::new(
        vec![child.owner.take().unwrap()],
        Duration::from_millis(50),
    );
    let mut stop = Box::pin(cleanup.stop());
    bounded(async {
        tokio::select! {
            biased;
            _ = async {
                while !progress.lock().unwrap().is_aborted() {
                    tokio::task::yield_now().await;
                }
            } => {},
            result = &mut stop => panic!("abort join must remain interruptible: {result:?}"),
        }
    })
    .await;
    drop(stop);
    assert!(!progress.lock().unwrap().is_joined());
    let (pages, issued) = cleanup.into_remaining();
    assert!(issued
        .as_ref()
        .unwrap_err()
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    let error = super::super::tasks::control::finish(
        issued,
        bounded(stop_browser_pages(pages, CDP_PAGE_CLEANUP_TIMEOUT)).await,
    )
    .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    released(&mut child).await;
}

#[tokio::test]
async fn lifecycle_cdp_browser_child_inbound_command_waits_for_partial_native_frame_completion() {
    let mut child = opened().await;
    let payload = super::super::bridge_tests::envelope(
        "_rpc_applicationSentData:",
        plist::Dictionary::from_iter([
            (
                "WIRApplicationIdentifierKey".to_owned(),
                plist::Value::String("PID:42".to_owned()),
            ),
            (
                "WIRMessageDataKey".to_owned(),
                plist::Value::Data(
                    serde_json::to_vec(&json!({
                        "method": "Target.dispatchMessageFromTarget",
                        "params": {
                            "targetId": "owned-cdp-target",
                            "message": json!({"id": 41, "result": {"value": "split"}}).to_string()
                        }
                    }))
                    .unwrap(),
                ),
            ),
        ]),
    );
    let mut raw_plist = Vec::new();
    payload.to_writer_xml(&mut raw_plist).unwrap();
    let mut framed = Vec::with_capacity(4 + raw_plist.len());
    framed.extend_from_slice(&(raw_plist.len() as u32).to_be_bytes());
    framed.extend_from_slice(&raw_plist);

    let split_at = framed.len() / 2;
    let before = child.source.received.load(Ordering::SeqCst);
    child.peer.write_all(&framed[..split_at]).await.unwrap();
    bounded(async {
        while child.source.received.load(Ordering::SeqCst) < before + split_at {
            tokio::task::yield_now().await;
        }
    })
    .await;

    // Queue an inbound command while the native plist frame is half-read.
    child
        .owner
        .as_ref()
        .unwrap()
        .sender
        .send(json!({"id": 42, "method": "Runtime.enable"}))
        .await
        .unwrap();
    tokio::task::yield_now().await;

    // Complete the native frame; the session must stay alive and deliver both
    // the completed native frame and the queued inbound command.
    child.peer.write_all(&framed[split_at..]).await.unwrap();
    match bounded(child.output.recv()).await.unwrap() {
        BrowserPageOutput::Message {
            session_id,
            message,
        } => {
            assert_eq!(session_id, child.session);
            assert_eq!(message, json!({"id": 41, "result": {"value": "split"}}));
        }
        BrowserPageOutput::Closed { .. } => {
            panic!("mid-frame inbound command must not detach the page session")
        }
    }
    let forwarded = bounded(read_frame(&mut child.peer)).await.unwrap().unwrap();
    assert_eq!(
        forwarded["__selector"].as_string(),
        Some("_rpc_forwardSocketData:")
    );
    bounded(stop_browser_page(child.owner.take().unwrap()))
        .await
        .unwrap();
    closed(&mut child).await;
}

//! Host fixtures for native task ownership and retryable bounded joins.

#[cfg(feature = "tunnel-kernel")]
mod identity_tests;

#[tokio::test]
async fn native_owner_invalid_join_accounting_cannot_certify_zero_pending() {
    let ledger = Ledger::new();
    let task = OwnedTask::new(
        tokio::spawn(async {
            Err(TunnelError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                NativeMarker,
            )))
        }),
        ParentRole::ForwardOrReader,
        ledger.clone(),
    );
    let first = join_task(&task, &ledger).await.unwrap_err();
    // Inject a private accounting fault after the actual native join completed.
    // A zero slot count alone cannot turn this invalid state into a certificate.
    ledger.joined();
    let invalid = ledger.result(None).unwrap_err();
    assert!(failure(&first).joins_complete());
    assert_eq!(failure(&invalid).pending_joins(), 0);
    assert!(!failure(&invalid).joins_complete());
    assert!(ledger.invalid_count.load(Ordering::Acquire));
    assert_eq!(
        crate::classify_error(&invalid),
        crate::ErrorClass::Disconnected
    );
    assert!(std::ptr::eq(
        failure(&first)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
        failure(&invalid)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
    ));
}

use super::*;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

const HOST_BUDGET: Duration = Duration::from_secs(2);

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct NativeMarker;

impl std::fmt::Display for NativeMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private-native-owner-io-marker")
    }
}

impl std::fmt::Debug for NativeMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeOwnerFixtureMarker")
    }
}

impl Error for NativeMarker {}

struct NativePanic;

impl std::fmt::Debug for NativePanic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeOwnerFixturePanic")
    }
}

fn failure(error: &TunnelError) -> &TunnelShutdownFailure {
    let mut current: &(dyn Error + 'static) = error;
    for _ in 0..32 {
        if let Some(failure) = current.downcast_ref::<TunnelShutdownFailure>() {
            return failure;
        }
        current = current.source().expect("native shutdown snapshot lost");
    }
    panic!("native shutdown snapshot source depth exceeded");
}

fn original_io(failure: &TunnelShutdownFailure) -> &std::io::Error {
    match failure.primary().downcast_ref::<TunnelError>().unwrap() {
        TunnelError::Io(error) => error,
        _ => panic!("original native IO variant lost"),
    }
}

fn assert_elapsed(error: &TunnelError) {
    let snapshot = failure(error);
    assert!(
        snapshot.primary().is::<Elapsed>() || snapshot.cleanup().any(|cause| cause.is::<Elapsed>())
    );
}

fn assert_safe(error: &TunnelError) {
    let snapshot = failure(error);
    for rendered in [
        format!("{snapshot:?}"),
        format!("{snapshot}"),
        format!("{error:?}"),
        crate::redaction::diagnostic_error(error).to_string(),
    ] {
        assert!(!rendered.contains("private-native-owner"));
    }
}

fn gated_task(
    ledger: Arc<Ledger>,
) -> (
    OwnedTask,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    Arc<AtomicUsize>,
) {
    let (release, released) = oneshot::channel();
    let (entered, observed_entry) = oneshot::channel();
    let drops = Arc::new(AtomicUsize::new(0));
    let task_drops = drops.clone();
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(task_drops);
        let _ = entered.send(());
        let _ = released.await;
        Ok(())
    });
    (
        OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger),
        release,
        observed_entry,
        drops,
    )
}

async fn wait_finished(task: &OwnedTask) {
    tokio::time::timeout(HOST_BUDGET, async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native task did not complete before host guard");
}

async fn wait_abort_finished(abort: &AbortHandle) {
    tokio::time::timeout(HOST_BUDGET, async {
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("native task did not complete before host guard");
}

async fn join_task(task: &OwnedTask, ledger: &Ledger) -> Result<(), TunnelError> {
    tokio::time::timeout(
        HOST_BUDGET,
        join_until(deadline(HOST_BUDGET), ledger, task.join()),
    )
    .await
    .expect("native join exceeded independent host guard")
}

#[tokio::test]
async fn native_owner_requested_abort_retains_real_cancelled_completion_quietly() {
    let ledger = Ledger::new();
    let (task, release, entered, drops) = gated_task(ledger.clone());
    let entered = tokio::time::timeout(HOST_BUDGET, entered).await;
    task.abort();
    let first = join_task(&task, &ledger).await;
    drop(release);
    let completion = {
        let state = task.join.lock().await;
        match &state.completion {
            Some(Completion::RequestedAbort { _cause }) => Some(_cause.clone()),
            _ => None,
        }
    };
    let repeated = join_task(&task, &ledger).await;
    assert!(matches!(entered, Ok(Ok(()))));
    first.unwrap();
    repeated.unwrap();
    let completion = completion.expect("requested native cancellation was not retained");
    assert!(completion.is_cancelled());
    assert!(!completion.is_panic());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    assert!(!ledger.invalid_count.load(Ordering::Acquire));
    let state = task.join.lock().await;
    assert!(state.handle.is_none());
    match &state.completion {
        Some(Completion::RequestedAbort { _cause }) => assert!(Arc::ptr_eq(_cause, &completion)),
        _ => panic!("repeat join changed the original native completion"),
    }
}

#[tokio::test]
async fn native_owner_completed_foreign_abort_stays_unexpected_after_stop() {
    let ledger = Ledger::new();
    let (release, released) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = released.await;
        Ok(())
    });
    let foreign = handle.abort_handle();
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    foreign.abort();
    wait_finished(&task).await;
    task.abort();
    drop(release);
    let first = join_task(&task, &ledger).await.unwrap_err();
    let repeated = join_task(&task, &ledger).await.unwrap_err();
    let native = failure(&first)
        .primary()
        .downcast_ref::<JoinError>()
        .unwrap();
    assert!(native.is_cancelled());
    assert!(!native.is_panic());
    assert!(!task.requested_abort.load(Ordering::Acquire));
    assert_eq!(crate::classify_error(&first), crate::ErrorClass::Cancelled);
    assert_eq!(failure(&first).pending_joins(), 0);
    assert!(std::ptr::eq(
        failure(&first)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap(),
        failure(&repeated)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap(),
    ));
    assert_safe(&first);
    let state = task.join.lock().await;
    match &state.completion {
        Some(Completion::UnexpectedJoin { _cause }) => {
            assert!(std::ptr::eq(_cause.as_ref(), native));
        }
        _ => panic!("completed foreign cancellation became quiet"),
    }
}

#[tokio::test]
async fn native_owner_original_marker_work_error_survives_late_stop_and_retry() {
    let ledger = Ledger::new();
    let handle = tokio::spawn(async {
        Err(TunnelError::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            NativeMarker,
        )))
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    wait_finished(&task).await;
    task.abort();
    let first = join_task(&task, &ledger).await.unwrap_err();
    let repeated = join_task(&task, &ledger).await.unwrap_err();
    let io = original_io(failure(&first));
    assert_eq!(io.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(io.get_ref().unwrap().is::<NativeMarker>());
    assert_eq!(
        crate::classify_error(&first),
        crate::ErrorClass::Disconnected
    );
    assert!(std::ptr::eq(
        failure(&first)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
        failure(&repeated)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
    ));
    assert_eq!(failure(&repeated).pending_joins(), 0);
    assert_safe(&first);
}

#[cfg(unix)]
#[tokio::test]
async fn native_owner_supplied_raw_errno_keeps_original_native_object() {
    let ledger = Ledger::new();
    let handle =
        tokio::spawn(async { Err(TunnelError::Io(std::io::Error::from_raw_os_error(13))) });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let error = join_task(&task, &ledger).await.unwrap_err();
    assert_eq!(original_io(failure(&error)).raw_os_error(), Some(13));
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::PermissionDenied
    );
    assert_eq!(failure(&error).pending_joins(), 0);
}

#[tokio::test]
async fn native_owner_actual_panic_remains_error_after_stop() {
    let ledger = Ledger::new();
    let handle = tokio::spawn(async {
        std::panic::panic_any(NativePanic);
        #[allow(unreachable_code)]
        Ok(())
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    wait_finished(&task).await;
    task.abort();
    let error = join_task(&task, &ledger).await.unwrap_err();
    let native = failure(&error)
        .primary()
        .downcast_ref::<JoinError>()
        .unwrap();
    assert!(native.is_panic());
    assert!(!native.is_cancelled());
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Unknown);
    assert_eq!(failure(&error).pending_joins(), 0);
    assert_safe(&error);
}

#[tokio::test]
async fn native_owner_mutex_wait_expiry_keeps_handle_and_retries_native_join() {
    let ledger = Ledger::new();
    let (task, release, entered, drops) = gated_task(ledger.clone());
    let entered = tokio::time::timeout(HOST_BUDGET, entered).await;
    let held = task.join.lock().await;
    let id = held.handle.as_ref().unwrap().id();
    task.abort();
    wait_finished(&task).await;
    let expired = tokio::time::timeout(
        HOST_BUDGET,
        join_until(deadline(Duration::from_millis(80)), &ledger, task.join()),
    )
    .await;
    let retained = held.handle.as_ref().map(JoinHandle::id);
    let incomplete = ledger.pending.load(Ordering::Acquire);
    drop(held);
    drop(release);
    let retry = join_task(&task, &ledger).await;
    assert!(matches!(entered, Ok(Ok(()))));
    let error = expired
        .expect("join mutex wait ignored cleanup total")
        .unwrap_err();
    assert_elapsed(&error);
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
    assert_eq!(failure(&error).pending_joins(), 1);
    assert_eq!(retained, Some(id));
    assert_eq!(incomplete, 1);
    retry.unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    assert_eq!(failure(&error).pending_joins(), 1);
}

#[tokio::test]
async fn native_owner_caller_drop_keeps_original_handle_during_mutex_and_native_wait() {
    for held_mutex in [true, false] {
        let ledger = Ledger::new();
        let (task, release, entered, drops) = gated_task(ledger.clone());
        let entered = tokio::time::timeout(HOST_BUDGET, entered).await;
        let mut held = if held_mutex {
            Some(task.join.lock().await)
        } else {
            None
        };
        let id = if let Some(held) = &held {
            held.handle.as_ref().unwrap().id()
        } else {
            task.join.lock().await.handle.as_ref().unwrap().id()
        };
        let mut joining = Box::pin(join_until(deadline(HOST_BUDGET), &ledger, task.join()));
        let stage = futures_util::poll!(&mut joining);
        drop(joining);
        let retained = if let Some(held) = &held {
            held.handle.as_ref().map(JoinHandle::id)
        } else {
            task.join.lock().await.handle.as_ref().map(JoinHandle::id)
        };
        drop(held.take());
        let _ = release.send(());
        wait_finished(&task).await;
        let result = join_task(&task, &ledger).await;
        let repeated = join_task(&task, &ledger).await;
        assert!(matches!(entered, Ok(Ok(()))));
        assert!(stage.is_pending());
        assert_eq!(retained, Some(id));
        result.unwrap();
        repeated.unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
        assert!(!ledger.invalid_count.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn native_owner_acquired_duplex_primary_survives_elapsed_and_retry_snapshot() {
    let ledger = Ledger::new();
    let (release, released) = oneshot::channel::<()>();
    let (acquired, observed_primary) = oneshot::channel();
    let drops = Arc::new(AtomicUsize::new(0));
    let native_drops = drops.clone();
    let native_ledger = ledger.clone();
    let handle = tokio::spawn(async move {
        let _resource = DropProbe(native_drops);
        let (mut stream, peer) = tokio::io::duplex(1);
        drop(peer);
        let error = stream.write_all(b"x").await.unwrap_err();
        let primary = Arc::new(TunnelError::Io(error));
        native_ledger.record_work(0, primary.clone());
        let _ = acquired.send(primary);
        let _ = released.await;
        drop(stream);
        Ok(())
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let acquired = tokio::time::timeout(HOST_BUDGET, observed_primary).await;
    let old = join_until(deadline(Duration::from_millis(80)), &ledger, task.join()).await;
    let remained_live = !task.is_finished();
    let prior_pending = ledger.pending.load(Ordering::Acquire);
    let _ = release.send(());
    wait_finished(&task).await;
    let retry = join_task(&task, &ledger).await;
    let original = acquired
        .expect("native IO did not return before host guard")
        .unwrap();
    let old = old.unwrap_err();
    let retry = retry.unwrap_err();
    assert!(remained_live);
    assert_eq!(prior_pending, 1);
    assert_eq!(
        original_io(failure(&old)).kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert_elapsed(&old);
    assert_eq!(crate::classify_error(&old), crate::ErrorClass::Disconnected);
    assert!(std::ptr::eq(
        failure(&old)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
        original.as_ref(),
    ));
    assert!(std::ptr::eq(
        failure(&old)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
        failure(&retry)
            .primary()
            .downcast_ref::<TunnelError>()
            .unwrap(),
    ));
    assert_eq!(failure(&old).pending_joins(), 1);
    assert_eq!(failure(&retry).pending_joins(), 0);
    assert!(!failure(&retry).cleanup().any(|cause| cause.is::<Elapsed>()));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_safe(&old);
}

#[tokio::test]
async fn native_owner_late_ready_handle_is_not_polled_after_nominal_deadline() {
    let ledger = Ledger::new();
    let (task, release, entered, drops) = gated_task(ledger.clone());
    let entered = tokio::time::timeout(HOST_BUDGET, entered).await;
    let polls = AtomicUsize::new(0);
    let mut work = Box::pin(task.join());
    let counted = std::future::poll_fn(|cx| {
        polls.fetch_add(1, Ordering::SeqCst);
        work.as_mut().poll(cx)
    });
    let mut cleanup = Box::pin(join_until(
        deadline(Duration::from_millis(500)),
        &ledger,
        counted,
    ));
    let first = futures_util::poll!(&mut cleanup);
    let initial_polls = polls.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(650)).await;
    let _ = release.send(());
    wait_finished(&task).await;
    let outcome = match first {
        Poll::Ready(result) => {
            drop(cleanup);
            result
        }
        Poll::Pending => cleanup.await,
    };
    drop(work);
    let late_polls = polls.load(Ordering::SeqCst);
    let before_retry = ledger.pending.load(Ordering::Acquire);
    let retry = join_task(&task, &ledger).await;
    assert!(matches!(entered, Ok(Ok(()))));
    assert!(initial_polls > 0);
    let error = outcome.unwrap_err();
    assert_elapsed(&error);
    assert_eq!(initial_polls, late_polls);
    assert_eq!(before_retry, 1);
    retry.unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn native_owner_ready_poll_crossing_deadline_returns_real_elapsed_once() {
    let ledger = Ledger::new();
    let due = deadline(Duration::from_millis(500)).unwrap();
    let polls = AtomicUsize::new(0);
    let work = std::future::poll_fn(|_cx| {
        let previous = polls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(previous, 0, "completed work was polled twice");
        while Instant::now() < due {
            std::thread::yield_now();
        }
        Poll::Ready(())
    });
    let result = tokio::time::timeout(HOST_BUDGET, join_until(Ok(due), &ledger, work)).await;
    let error = result
        .expect("native timer never completed post-poll expiry")
        .unwrap_err();
    assert_elapsed(&error);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert_eq!(failure(&error).pending_joins(), 0);
    ledger.result(None).unwrap();
}

#[tokio::test]
async fn native_owner_zero_and_overflow_never_poll_real_work() {
    for budget in [Duration::ZERO, Duration::MAX] {
        let ledger = Ledger::new();
        let polls = AtomicUsize::new(0);
        let work = std::future::poll_fn(|_cx| {
            polls.fetch_add(1, Ordering::SeqCst);
            Poll::Ready(())
        });
        let error = join_until(deadline(budget), &ledger, work)
            .await
            .unwrap_err();
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        if budget.is_zero() {
            assert_elapsed(&error);
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        } else {
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::InvalidInput
            );
        }
        assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
        assert!(ledger
            .outcomes
            .lock()
            .unwrap()
            .slots
            .iter()
            .all(Option::is_none));
    }
}

#[tokio::test]
async fn native_owner_child_completed_foreign_abort_survives_abort_all_prereap() {
    let ledger = Ledger::new();
    let children = SharedChildTaskSet::new(ledger.clone(), ChildRole::ForwardReader);
    let mut guard = children.lock().await;
    let foreign = guard.set.spawn(std::future::pending::<()>());
    ledger.registered();
    foreign.abort();
    wait_abort_finished(&foreign).await;
    guard.abort_all();
    let no_remaining = guard.is_empty();
    drop(guard);
    children.drain().await;
    let error = ledger.result(None).unwrap_err();
    let native = failure(&error)
        .primary()
        .downcast_ref::<JoinError>()
        .unwrap();
    assert!(native.is_cancelled());
    assert!(!native.is_panic());
    assert!(no_remaining);
    assert_eq!(failure(&error).pending_joins(), 0);
    assert!(children.first_expected_abort.lock().unwrap().is_none());
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Cancelled);
    assert_safe(&error);
}

#[tokio::test]
async fn native_owner_pending_child_abort_is_quiet_with_original_native_object() {
    let ledger = Ledger::new();
    let children = SharedChildTaskSet::new(ledger.clone(), ChildRole::ForwardReader);
    let drops = Arc::new(AtomicUsize::new(0));
    let child_drops = drops.clone();
    let (entered, observed_entry) = oneshot::channel();
    let mut guard = children.lock().await;
    guard.spawn(async move {
        let _resource = DropProbe(child_drops);
        let _ = entered.send(());
        std::future::pending::<()>().await;
    });
    let entered = tokio::time::timeout(HOST_BUDGET, observed_entry).await;
    guard.abort_all();
    while guard.join_next().await.is_some() {}
    drop(guard);
    let result = ledger.result(None);
    let saved = children
        .first_expected_abort
        .lock()
        .unwrap()
        .as_ref()
        .cloned();
    children.drain().await;
    let repeated = ledger.result(None);
    assert!(matches!(entered, Ok(Ok(()))));
    result.unwrap();
    repeated.unwrap();
    let saved = saved.expect("native requested child cancellation was lost");
    assert!(saved.is_cancelled());
    assert!(!saved.is_panic());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    assert!(!ledger.invalid_count.load(Ordering::Acquire));
    assert!(Arc::ptr_eq(
        &saved,
        children
            .first_expected_abort
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
    ));
}

#[tokio::test]
async fn native_owner_parent_panic_keeps_shared_children_for_explicit_join() {
    let ledger = Ledger::new();
    let children = SharedChildTaskSet::new(ledger.clone(), ChildRole::ForwardReader);
    let parent_children = children.clone();
    let drops = Arc::new(AtomicUsize::new(0));
    let child_drops = drops.clone();
    let (entered, observed_entry) = oneshot::channel();
    let (panic_now, should_panic) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let mut guard = parent_children.lock().await;
        guard.spawn(async move {
            let _resource = DropProbe(child_drops);
            let _ = entered.send(());
            std::future::pending::<()>().await;
        });
        let _ = should_panic.await;
        std::panic::panic_any(NativePanic);
        #[allow(unreachable_code)]
        Ok(())
    });
    let parent = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    let entered = tokio::time::timeout(HOST_BUDGET, observed_entry).await;
    let before_panic = ledger.pending.load(Ordering::Acquire);
    let _ = panic_now.send(());
    tokio::time::timeout(HOST_BUDGET, parent.join())
        .await
        .unwrap();
    let old = ledger.result(None).unwrap_err();
    let after_parent = ledger.pending.load(Ordering::Acquire);
    children.drain().await;
    let final_result = ledger.result(None).unwrap_err();
    assert!(matches!(entered, Ok(Ok(()))));
    assert_eq!(before_panic, 2);
    assert_eq!(after_parent, 1);
    assert_eq!(failure(&old).pending_joins(), 1);
    assert_eq!(failure(&final_result).pending_joins(), 0);
    assert!(failure(&final_result)
        .primary()
        .downcast_ref::<JoinError>()
        .unwrap()
        .is_panic());
    assert!(std::ptr::eq(
        failure(&old).primary().downcast_ref::<JoinError>().unwrap(),
        failure(&final_result)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap(),
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(children
        .first_expected_abort
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .is_cancelled());
    assert!(!ledger.invalid_count.load(Ordering::Acquire));
}

#[tokio::test]
async fn native_owner_repeated_native_panics_use_one_fixed_slot_and_balanced_count() {
    let ledger = Ledger::new();
    let mut first = None;
    for _ in 0..32 {
        let handle = tokio::spawn(async {
            std::panic::panic_any(NativePanic);
            #[allow(unreachable_code)]
            Ok(())
        });
        let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
        let error = join_task(&task, &ledger).await.unwrap_err();
        if first.is_none() {
            first = Some(error);
        }
    }
    let first = first.unwrap();
    let current = ledger.result(None).unwrap_err();
    let outcomes = ledger.outcomes.lock().unwrap();
    assert_eq!(outcomes.slots.len(), OUTCOME_SLOTS);
    assert_eq!(
        outcomes.slots.iter().filter(|slot| slot.is_some()).count(),
        1
    );
    assert_eq!(outcomes.additional_unretained_outcomes, 31);
    assert!(std::ptr::eq(
        failure(&first)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap(),
        failure(&current)
            .primary()
            .downcast_ref::<JoinError>()
            .unwrap(),
    ));
    assert_eq!(failure(&current).pending_joins(), 0);
    assert_eq!(failure(&current).cleanup().count(), 0);
    assert_eq!(failure(&current).additional_unretained_outcomes(), 31);
    assert!(!ledger.invalid_count.load(Ordering::Acquire));
    assert_safe(&current);
}

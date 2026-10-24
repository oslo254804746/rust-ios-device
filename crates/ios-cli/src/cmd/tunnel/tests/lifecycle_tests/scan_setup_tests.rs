use super::*;
use crate::access::test_support::{grant, PolicyFixture, TOKEN};

fn assert_elapsed(error: &anyhow::Error) {
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

fn assert_scan_released(state: &TunnelAgentState) {
    assert!(state.inner.scan_lock.try_lock().is_ok());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

fn assert_sources(source: &FakeSource, usb_calls: usize) {
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), usb_calls);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
}

async fn held_table_expires(post_snapshot: bool) {
    let source = FakeSource::new();
    let usb_gate = Arc::new(Semaphore::new(0));
    if post_snapshot {
        *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    }
    let state = state(source.clone(), Duration::from_millis(80));
    let mut scan = Box::pin(state.refresh_once());
    let table = if post_snapshot {
        assert!(futures_util::poll!(&mut scan).is_pending());
        assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
        state.inner.tunnels.write().await
    } else {
        state.inner.tunnels.write().await
    };
    if post_snapshot {
        usb_gate.add_permits(1);
    }
    assert!(futures_util::poll!(&mut scan).is_pending());
    let observed = tokio::time::timeout(LOCAL_BUDGET, scan).await;
    assert_elapsed(
        &observed
            .expect("setup table wait outlived its product deadline")
            .unwrap_err(),
    );
    assert!(table.is_empty());
    assert!(!state.is_ready().await);
    assert_sources(&source, usize::from(post_snapshot));
    assert_scan_released(&state);
    drop(table);
    *source.usb_gate.lock().unwrap() = None;
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
}

async fn held_table_cancels(post_snapshot: bool) {
    for terminal in [false, true] {
        let source = FakeSource::new();
        let usb_gate = Arc::new(Semaphore::new(0));
        if post_snapshot {
            *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
        }
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut cancel_input = PreparedSignal::new(&state, terminal);
        let mut scan = Box::pin(state.refresh_once());
        let table = if post_snapshot {
            assert!(futures_util::poll!(&mut scan).is_pending());
            assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
            state.inner.tunnels.write().await
        } else {
            state.inner.tunnels.write().await
        };
        if post_snapshot {
            usb_gate.add_permits(1);
        }
        assert!(futures_util::poll!(&mut scan).is_pending());
        cancel_input.notify_or_clear_epoch(&state);
        tokio::time::timeout(LOCAL_BUDGET, scan)
            .await
            .expect("setup table wait ignored its epoch cancellation")
            .unwrap();
        assert!(table.is_empty());
        assert!(!state.is_ready().await);
        assert_sources(&source, usize::from(post_snapshot));
        assert_scan_released(&state);
        drop(table);
        *source.usb_gate.lock().unwrap() = None;
        if terminal {
            let error = state.refresh_once().await.unwrap_err();
            assert!(error.is::<ios_core::OperationCancelled>());
        } else {
            state.refresh_once().await.unwrap();
            assert!(state.is_ready().await);
        }
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_initial_table_expires_before_release() {
    held_table_expires(false).await;
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_post_snapshot_table_expires_before_release() {
    held_table_expires(true).await;
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_initial_table_clear_and_shutdown_cancel_before_release() {
    held_table_cancels(false).await;
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_post_snapshot_table_clear_and_shutdown_cancel_before_release()
{
    held_table_cancels(true).await;
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_usb_coordinator_expires_before_release() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), Duration::from_millis(80));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    // Enter USB discovery before holding its later coordinator admission.
    // Initial desired-state capture has its own separate regression below.
    let usb_gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    usb_gate.add_permits(1);
    assert!(futures_util::poll!(&mut scan).is_pending());
    *source.usb_gate.lock().unwrap() = None;
    let observed = tokio::time::timeout(LOCAL_BUDGET, scan).await;
    assert_elapsed(
        &observed
            .expect("USB coordinator wait ignored setup total")
            .unwrap_err(),
    );
    assert_sources(&source, 1);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    drop(coordinator);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_usb_coordinator_clear_and_shutdown_cancel_before_release() {
    for terminal in [false, true] {
        let source = FakeSource::new();
        source.usb.lock().unwrap().push(A.into());
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut cancel_input = PreparedSignal::new(&state, terminal);
        let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

        // Enter USB discovery before holding its later coordinator admission.
        // Initial desired-state capture has its own separate regression below.
        let usb_gate = Arc::new(Semaphore::new(0));
        *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
        let mut scan = Box::pin(state.refresh_once());
        assert!(futures_util::poll!(&mut scan).is_pending());
        assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
        let coordinator = state.inner.lifecycle.hold_coordinator().await;
        usb_gate.add_permits(1);
        assert!(futures_util::poll!(&mut scan).is_pending());
        *source.usb_gate.lock().unwrap() = None;
        cancel_input.notify_or_clear_epoch(&state);
        tokio::time::timeout(LOCAL_BUDGET, scan)
            .await
            .expect("USB coordinator wait ignored setup cancellation")
            .unwrap();
        assert_sources(&source, 1);
        assert!(!state.is_ready().await);
        assert_scan_released(&state);
        drop(coordinator);
        if terminal {
            assert!(state
                .refresh_once()
                .await
                .unwrap_err()
                .is::<ios_core::OperationCancelled>());
        } else {
            state.refresh_once().await.unwrap();
            assert_eq!(source.count(A), 1);
        }
        state.clear(&mut control_input_0).await;
        drop(control_input_0);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_late_usb_coordinator_unlock_does_not_insert_gate() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), Duration::from_millis(80));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    // Enter USB discovery before holding its later coordinator admission.
    // Initial desired-state capture has its own separate regression below.
    let usb_gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    usb_gate.add_permits(1);
    assert!(futures_util::poll!(&mut scan).is_pending());
    *source.usb_gate.lock().unwrap() = None;
    tokio::time::sleep(Duration::from_millis(120)).await;
    drop(coordinator);
    let error = tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .expect("late coordinator release kept the expired scan waiting")
        .unwrap_err();
    assert_elapsed(&error);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert_sources(&source, 1);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    assert!(state.inner.tunnels.read().await.is_empty());
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

async fn denied_state(
    source: Arc<FakeSource>,
    timeout: Duration,
) -> (PolicyFixture, TunnelAgentState) {
    let policy = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::ManageServer, Permission::List],
    )]);
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    assert!(!access.permits_any_device());
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        Upstreams::default(),
        source,
        timeout,
    );
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let slot = state.reserve_failure_slot(A).await.unwrap();
    state
        .record_failed_attempt(
            A,
            AttemptIdentity {
                route: Route::Usbmux,
                native_id: Some(1),
            },
            slot,
            &ticket,
        )
        .await;
    (policy, state)
}

#[derive(Clone, Copy)]
enum SetupEnd {
    Expiry,
    Clear,
    Shutdown,
    LateExpiry,
}

async fn denied_wait_ends(wait_failures: bool, end: SetupEnd) {
    let source = FakeSource::new();
    let timeout = if matches!(end, SetupEnd::Clear | SetupEnd::Shutdown) {
        TUNNEL_CONNECT_TIMEOUT
    } else {
        Duration::from_millis(80)
    };
    let (_policy, state) = denied_state(source.clone(), timeout).await;
    let mut clear_input =
        matches!(end, SetupEnd::Clear).then(|| state.prepare_cutover::<2>().unwrap());
    let mut shutdown_input =
        matches!(end, SetupEnd::Shutdown).then(|| state.prepare_cutover::<1>().unwrap());
    let mut cleanup_input = state.prepare_cutover::<2>().unwrap();
    let mut failed = if wait_failures {
        Some(state.inner.failed_attempts.write().await)
    } else {
        None
    };
    let mut ready = if wait_failures {
        None
    } else {
        Some(state.inner.ready.write().await)
    };
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    match end {
        SetupEnd::Clear => state.inner.lifecycle.clear(clear_input.as_mut().unwrap()),
        SetupEnd::Shutdown => state.notify_shutdown(shutdown_input.as_mut().unwrap()),
        SetupEnd::Expiry => {}
        SetupEnd::LateExpiry => {
            // Stop polling the scan until both its lock and expired timer are
            // ready. A work-first timeout would clear rows or set ready here.
            tokio::time::sleep(Duration::from_millis(120)).await;
            drop(failed.take());
            drop(ready.take());
        }
    }
    let observed = tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .expect("no-device setup wait failed to complete before lock release");
    match end {
        SetupEnd::Clear | SetupEnd::Shutdown => observed.unwrap(),
        SetupEnd::Expiry | SetupEnd::LateExpiry => assert_elapsed(&observed.unwrap_err()),
    }
    if let Some(failed) = &failed {
        assert!(
            failed.contains_key(A),
            "expired setup cleared the held failure row"
        );
    }
    if let Some(ready) = &ready {
        assert!(!**ready, "expired setup marked the held state ready");
    }
    assert_sources(&source, 0);
    assert_scan_released(&state);
    drop(failed);
    drop(ready);
    assert!(!state.is_ready().await);
    assert_eq!(
        state.inner.failed_attempts.read().await.contains_key(A),
        wait_failures
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - usize::from(wait_failures)
    );
    if matches!(end, SetupEnd::Shutdown) {
        assert!(state
            .refresh_once()
            .await
            .unwrap_err()
            .is::<ios_core::OperationCancelled>());
    } else {
        state.refresh_once().await.unwrap();
        assert!(state.is_ready().await);
        assert!(state.inner.failed_attempts.read().await.is_empty());
    }
    state.clear(&mut cleanup_input).await;
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_no_device_failed_table_expires_before_release() {
    denied_wait_ends(true, SetupEnd::Expiry).await;
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_no_device_ready_table_expires_before_release() {
    denied_wait_ends(false, SetupEnd::Expiry).await;
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_no_device_waits_cancel_before_release() {
    for wait_failures in [true, false] {
        for end in [SetupEnd::Clear, SetupEnd::Shutdown] {
            denied_wait_ends(wait_failures, end).await;
        }
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_late_failed_and_ready_unlock_do_not_mutate() {
    for wait_failures in [true, false] {
        denied_wait_ends(wait_failures, SetupEnd::LateExpiry).await;
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_late_post_snapshot_unlock_keeps_old_publication() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(80));
    start(state.clone(), B).await.unwrap();
    source.usb.lock().unwrap().clear();
    let usb_gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    let table = state.inner.tunnels.write().await;
    assert!(table.contains_key(B));
    usb_gate.add_permits(1);
    assert!(futures_util::poll!(&mut scan).is_pending());
    tokio::time::sleep(Duration::from_millis(120)).await;
    drop(table);
    let error = tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .expect("late table release kept the expired scan waiting")
        .unwrap_err();
    assert_elapsed(&error);
    assert!(state.inner.tunnels.read().await.contains_key(B));
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 2);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(B), 1);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    *source.usb_gate.lock().unwrap() = None;
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_initial_snapshot_and_ticket_share_one_budget() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let usb_gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    let state = state(source.clone(), Duration::from_millis(1000));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let initial_table = state.inner.tunnels.write().await;
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(initial_table);
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    let post_table = state.inner.tunnels.write().await;
    usb_gate.add_permits(1);
    assert!(futures_util::poll!(&mut scan).is_pending());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    drop(post_table);
    assert!(futures_util::poll!(&mut scan).is_pending());
    let observed = tokio::time::timeout(Duration::from_millis(650), scan).await;
    assert_elapsed(
        &observed
            .expect("setup handoff reset the original scan budget")
            .unwrap_err(),
    );
    assert_sources(&source, 1);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    drop(coordinator);
    *source.usb_gate.lock().unwrap() = None;
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_list_error_keeps_primary_and_cleanup_elapsed() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(80));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    start(state.clone(), B).await.unwrap();
    source.list_failure.store(true, Ordering::SeqCst);
    let usb_gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    let table = state.inner.tunnels.write().await;
    usb_gate.add_permits(1);
    assert!(futures_util::poll!(&mut scan).is_pending());
    let error = tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .expect("list-error cleanup ignored setup deadline")
        .unwrap_err();
    let failure = error.downcast_ref::<ScanCleanupFailure>().unwrap();
    assert_eq!(
        failure
            .primary
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert_elapsed(&failure.cleanup);
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
    assert_eq!(
        std::error::Error::source(failure)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
    for rendered in [
        format!("{failure:?}"),
        crate::output::error_message(&error),
        crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "tunnel-scan",
            &error,
        )
        .unwrap(),
    ] {
        assert!(!rendered.contains("synthetic-private-list-input"));
        assert!(!rendered.contains(B));
    }
    assert!(table.contains_key(B));
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 2);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    drop(table);
    assert!(state.inner.tunnels.read().await.contains_key(B));
    source.list_failure.store(false, Ordering::SeqCst);
    *source.usb_gate.lock().unwrap() = None;
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_list_error_cancel_keeps_primary_without_old_cleanup() {
    for terminal in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut cancel_input = PreparedSignal::new(&state, terminal);
        let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

        start(state.clone(), B).await.unwrap();
        source.list_failure.store(true, Ordering::SeqCst);
        let usb_gate = Arc::new(Semaphore::new(0));
        *source.usb_gate.lock().unwrap() = Some(usb_gate.clone());
        let mut scan = Box::pin(state.refresh_once());
        assert!(futures_util::poll!(&mut scan).is_pending());
        let table = state.inner.tunnels.write().await;
        usb_gate.add_permits(1);
        assert!(futures_util::poll!(&mut scan).is_pending());
        cancel_input.notify_or_clear_epoch(&state);
        let error = tokio::time::timeout(LOCAL_BUDGET, scan)
            .await
            .expect("list-error cleanup ignored cancellation")
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        assert!(!error.is::<ScanCleanupFailure>());
        assert!(table.contains_key(B));
        assert_eq!(source.drops.load(Ordering::SeqCst), 0);
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
        assert!(!state.is_ready().await);
        assert_scan_released(&state);
        drop(table);
        assert!(state.inner.tunnels.read().await.contains_key(B));
        source.list_failure.store(false, Ordering::SeqCst);
        *source.usb_gate.lock().unwrap() = None;
        if !terminal {
            state.refresh_once().await.unwrap();
            assert!(state.is_ready().await);
        }
        state.clear(&mut control_input_0).await;
        drop(control_input_0);
    }
}

struct EpochErrorSource {
    prepared: StdMutex<Option<PreparedSignal>>,
    source: Arc<FakeSource>,
    state: StdMutex<Option<shared_body::Weak<TunnelAgentInner>>>,
    connect_error: bool,
    terminal: bool,
}

impl Source for EpochErrorSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.source.connect(udid, route, mode)
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            let snapshot = self.source.usb_devices().await;
            let state = TunnelAgentState {
                inner: self
                    .state
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .upgrade()
                    .unwrap(),
            };
            let mut prepared = self.prepared.lock().unwrap();
            let input = prepared.as_mut().unwrap();
            assert_eq!(matches!(input, PreparedSignal::Shutdown(_)), self.terminal);
            input.notify_or_clear_epoch(&state);
            drop(prepared);
            if self.connect_error {
                match snapshot {
                    UsbSnapshot::ListError(error) => UsbSnapshot::ConnectError(error),
                    _ => panic!("expected the delegated list IO error"),
                }
            } else {
                snapshot
            }
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        self.source.paired_devices()
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_returned_usb_errors_survive_same_poll_epoch_change() {
    for connect_error in [false, true] {
        for terminal in [false, true] {
            let source = FakeSource::new();
            source.list_failure.store(true, Ordering::SeqCst);
            let wrapper = Arc::new(EpochErrorSource {
                source: source.clone(),
                state: StdMutex::new(None),
                prepared: StdMutex::new(None),
                connect_error,
                terminal,
            });
            let state = TunnelAgentState::with_source(
                TunMode::Userspace,
                AccessControl::trusted_test_local(),
                Upstreams::default(),
                wrapper.clone(),
                TUNNEL_CONNECT_TIMEOUT,
            );
            *wrapper.state.lock().unwrap() = Some(state.inner.downgrade());
            *wrapper.prepared.lock().unwrap() = Some(PreparedSignal::new(&state, terminal));
            let error = state.refresh_once().await.unwrap_err();
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::BrokenPipe
            );
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Disconnected
            );
            assert!(!error.is::<ScanCleanupFailure>());
            assert!(!crate::output::error_message(&error).contains("synthetic-private"));
            assert!(!state.is_ready().await);
            assert_sources(&source, 1);
            assert_scan_released(&state);
            assert!(state.inner.tunnels.read().await.is_empty());
        }
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_initial_intent_coordinator_expires_before_release() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), Duration::from_millis(80));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    let observed = tokio::time::timeout(LOCAL_BUDGET, scan).await;
    assert_elapsed(
        &observed
            .expect("initial intent coordinator wait ignored setup total")
            .unwrap_err(),
    );
    assert_sources(&source, 0);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    drop(coordinator);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_initial_intent_coordinator_clear_and_shutdown_cancel_before_release(
) {
    for terminal in [false, true] {
        let source = FakeSource::new();
        source.usb.lock().unwrap().push(A.into());
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut cancel_input = PreparedSignal::new(&state, terminal);
        let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

        let coordinator = state.inner.lifecycle.hold_coordinator().await;
        let mut scan = Box::pin(state.refresh_once());
        assert!(futures_util::poll!(&mut scan).is_pending());
        cancel_input.notify_or_clear_epoch(&state);
        tokio::time::timeout(LOCAL_BUDGET, scan)
            .await
            .expect("initial intent coordinator wait ignored setup cancellation")
            .unwrap();
        assert_sources(&source, 0);
        assert!(!state.is_ready().await);
        assert_scan_released(&state);
        drop(coordinator);
        if terminal {
            assert!(state
                .refresh_once()
                .await
                .unwrap_err()
                .is::<ios_core::OperationCancelled>());
        } else {
            state.refresh_once().await.unwrap();
            assert_eq!(source.count(A), 1);
        }
        state.clear(&mut control_input_0).await;
        drop(control_input_0);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_scan_setup_late_initial_intent_coordinator_unlock_does_not_insert_gate() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), Duration::from_millis(80));
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    tokio::time::sleep(Duration::from_millis(120)).await;
    drop(coordinator);
    let error = tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .expect("late coordinator release kept the expired scan waiting")
        .unwrap_err();
    assert_elapsed(&error);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert_sources(&source, 0);
    assert!(!state.is_ready().await);
    assert_scan_released(&state);
    assert!(state.inner.tunnels.read().await.is_empty());
    state.refresh_once().await.unwrap();
    assert!(state.is_ready().await);
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

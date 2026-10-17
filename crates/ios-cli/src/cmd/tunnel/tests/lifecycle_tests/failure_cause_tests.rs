//! A real localhost TCP ConnectionRefused crosses Source into manager-owned
//! receipts. Discovery alone is the private synthetic device seam.
use super::*;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU32;

const REFUSAL_BUDGET: Duration = Duration::from_secs(8);
const PRIVATE_CONTEXT: &str = "synthetic-original-private-pair-token";

struct RefusedSource {
    address: SocketAddr,
    devices: StdMutex<Vec<String>>,
    id: AtomicU32,
    calls: StdMutex<Vec<String>>,
    originals: StdMutex<Vec<usize>>,
    ready: Notify,
    return_gate: StdMutex<Option<Arc<Semaphore>>>,
    on_return: StdMutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl RefusedSource {
    async fn new() -> Arc<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        Arc::new(Self {
            address,
            devices: StdMutex::new(vec![A.into()]),
            id: AtomicU32::new(1),
            calls: StdMutex::new(Vec::new()),
            originals: StdMutex::new(Vec::new()),
            ready: Notify::new(),
            return_gate: StdMutex::new(None),
            on_return: StdMutex::new(None),
        })
    }
    fn count(&self, udid: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|key| key.as_str() == udid)
            .count()
    }
    async fn until_error<F: std::future::Future>(&self, future: &mut std::pin::Pin<Box<F>>) {
        tokio::time::timeout(REFUSAL_BUDGET, async {
            loop {
                let ready = self.ready.notified();
                tokio::pin!(ready);
                ready.as_mut().enable();
                if !self.originals.lock().unwrap().is_empty() {
                    break;
                }
                assert!(futures_util::poll!(future.as_mut()).is_pending());
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("real TCP refusal was not acquired");
    }
}
impl Source for RefusedSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(udid.into());
            let error = TcpStream::connect(self.address)
                .await
                .expect_err("refusal fixture port was reused");
            assert_eq!(error.kind(), std::io::ErrorKind::ConnectionRefused);
            let error = anyhow::Error::new(error).context(PRIVATE_CONTEXT);
            let original =
                error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error as usize;
            self.originals.lock().unwrap().push(original);
            self.ready.notify_waiters();
            let gate = self.return_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            let callback = self.on_return.lock().unwrap().take();
            if let Some(callback) = callback {
                callback();
            }
            Err(error)
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            UsbSnapshot::Devices(
                self.devices
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|udid| source::UsbDevice {
                        udid: udid.clone(),
                        device_id: self.id.load(Ordering::SeqCst),
                        usb: true,
                    })
                    .collect(),
            )
        })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}
fn manager(source: Arc<RefusedSource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        REFUSAL_BUDGET,
    )
}
fn original(error: &anyhow::Error) -> usize {
    let io = error.downcast_ref::<std::io::Error>().unwrap();
    assert_eq!(io.kind(), std::io::ErrorKind::ConnectionRefused);
    io as *const std::io::Error as usize
}

#[tokio::test]
async fn failure_node_real_original_records_at_zero_quota_and_last_receipt_retains_input() {
    let source = RefusedSource::new().await;
    let state = manager(source.clone());
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(Vec::with_capacity(2)));
    let return_bank = Arc::clone(&bank);
    let return_held = Arc::clone(&held);
    *source.on_return.lock().unwrap() = Some(Box::new(move || {
        let permit = return_bank
            .clone()
            .try_acquire_many_owned(return_bank.available_permits() as u32)
            .unwrap();
        return_held.lock().unwrap().push(permit);
        assert_eq!(return_bank.available_permits(), 0);
    }));
    let write_bank = Arc::clone(&bank);
    let write_held = Arc::clone(&held);
    let wrote_at_zero = Arc::new(AtomicBool::new(false));
    let observed_zero = Arc::clone(&wrote_at_zero);
    state.inner.lifecycle.on_next_failure_record(move |_| {
        let permit = write_bank
            .clone()
            .try_acquire_many_owned(write_bank.available_permits() as u32)
            .unwrap();
        write_held.lock().unwrap().push(permit);
        assert_eq!(write_bank.available_permits(), 0);
        observed_zero.store(true, Ordering::SeqCst);
    });
    state.refresh_once().await.unwrap();
    assert!(wrote_at_zero.load(Ordering::SeqCst));
    assert_eq!(source.count(A), 1);
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 1);
    let receipt = state.inner.retry.receipt(A).unwrap();
    let address = original(receipt.primary());
    assert_eq!(address, source.originals.lock().unwrap()[0]);
    assert_eq!(receipt.completion(), reconnect::Completion::Recorded);
    let input = receipt
        .primary()
        .downcast_ref::<candidate_input::SourceInputContext>()
        .unwrap()
        .input
        .downgrade();
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    held.lock().unwrap().clear();
    assert!(input.upgrade().is_some());
    assert_eq!(original(receipt.primary()), address);
    assert!(bank.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(receipt);
    assert!(input.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - candidate_input::Input::body_bytes()
    );
    drop(input);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn failure_writer_late_after_adopt_refunds_empty_actual_backing_and_preserves_original() {
    let source = RefusedSource::new().await;
    let gate = Arc::new(Semaphore::new(0));
    *source.return_gate.lock().unwrap() = Some(Arc::clone(&gate));
    let mut state = manager(source.clone());
    Arc::get_mut(&mut state.inner)
        .unwrap()
        .failure_write_timeout = Duration::from_millis(40);
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let observed_bank = Arc::clone(&bank);
    let adopted = Arc::new(AtomicBool::new(false));
    let observed_adopt = Arc::clone(&adopted);
    let mut scan = Box::pin(state.refresh_once());
    source.until_error(&mut scan).await;
    {
        let mut failed = state.inner.failed_attempts.write().await;
        assert!(failed.is_empty());
        failed.on_after_adopt(move || {
            let permit = observed_bank
                .clone()
                .try_acquire_many_owned(observed_bank.available_permits() as u32)
                .unwrap();
            *retained.lock().unwrap() = Some(permit);
            assert_eq!(observed_bank.available_permits(), 0);
            observed_adopt.store(true, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(100));
        });
    }
    gate.add_permits(1);
    let error = tokio::time::timeout(REFUSAL_BUDGET, scan.as_mut())
        .await
        .unwrap()
        .unwrap_err();
    drop(scan);
    assert!(adopted.load(Ordering::SeqCst));
    assert!(error.is::<tokio::time::error::Elapsed>());
    let context = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .unwrap();
    assert_eq!(context.receipts().len(), 1);
    assert_eq!(
        original(context.receipts()[0].primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(
        context.receipts()[0].completion(),
        reconnect::Completion::Failed(ios_core::ErrorClass::Timeout)
    );
    assert_eq!(source.count(A), 1);
    {
        let failed = state.inner.failed_attempts.read().await;
        assert!(failed.is_empty());
        assert_eq!(
            failed.backing_bytes(),
            0,
            "late adoption retained an empty 32768-byte backing"
        );
    }
    assert!(bank.available_permits() >= failure_table::Destination::minimum_bytes());
    assert!(!state.is_ready().await);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert!(bank.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(error);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

async fn pending_manual_cleanup(state: &TunnelAgentState) {
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Wifi(None),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let version = state.inner.failed_attempts.read().await[B].version.clone();
    let authority = state
        .inner
        .access
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap()
        .device_authority(Permission::ManageTunnels, B)
        .ok()
        .expect("local authority refused");
    ticket
        .commit(|| {
            ticket.establish_manual(
                intent::RouteIntent::Wifi(None),
                authority.clone(),
                Some(version.clone()),
                None,
            )
        })
        .unwrap();
    ticket.fenced().remember_failure(Some(version));
    assert!(ticket.pending_success().is_some());
}

#[tokio::test]
async fn manual_success_preflight_real_source_new_gate_and_latest_pending_cleanup_at_zero_quota() {
    use futures_util::FutureExt;
    let source = RefusedSource::new().await;
    let state = manager(source.clone());
    pending_manual_cleanup(&state).await;
    let old = state
        .inner
        .lifecycle
        .ticket(B)
        .await
        .unwrap()
        .desired()
        .version;
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let worker = state.clone();
    let observed_bank = Arc::clone(&bank);
    *source.on_return.lock().unwrap() = Some(Box::new(move || {
        // This independent metadata creator admits its own gate/key before
        // publishing a pending manual proof. Scan storage was prepared earlier.
        for uid in [B, "source-created-new-manual-gate"] {
            let ticket = worker
                .inner
                .lifecycle
                .ticket(uid)
                .now_or_never()
                .unwrap()
                .unwrap();
            let failure = if uid == B {
                worker.inner.failed_attempts.try_read().unwrap()[B]
                    .version
                    .clone()
            } else {
                let slot = worker
                    .reserve_failure_slot(uid)
                    .now_or_never()
                    .unwrap()
                    .unwrap();
                worker
                    .record_failed_attempt(
                        uid,
                        AttemptIdentity {
                            route: Route::Wifi(None),
                            native_id: None,
                        },
                        slot,
                        &ticket,
                    )
                    .now_or_never()
                    .expect("independent fixture seed is uncontended");
                worker.inner.failed_attempts.try_read().unwrap()[uid]
                    .version
                    .clone()
            };
            let authority = worker
                .inner
                .access
                .authenticate(&axum::http::HeaderMap::new())
                .unwrap()
                .device_authority(Permission::ManageTunnels, uid)
                .ok()
                .unwrap();
            ticket
                .commit(|| {
                    ticket.establish_manual(
                        intent::RouteIntent::Wifi(None),
                        authority,
                        Some(failure),
                        None,
                    )
                })
                .unwrap();
        }
        *retained.lock().unwrap() = Some(
            observed_bank
                .clone()
                .try_acquire_many_owned(observed_bank.available_permits() as u32)
                .unwrap(),
        );
        assert_eq!(observed_bank.available_permits(), 0);
    }));
    state.refresh_once().await.unwrap();
    assert_eq!(
        (
            source.count(A),
            source.count(B),
            source.count("source-created-new-manual-gate")
        ),
        (1, 0, 0)
    );
    assert!(!Arc::ptr_eq(
        &old,
        &state
            .inner
            .lifecycle
            .ticket(B)
            .await
            .unwrap()
            .desired()
            .version
    ));
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 3);
    for uid in [B, "source-created-new-manual-gate"] {
        assert!(!state.inner.failed_attempts.read().await.contains_key(uid));
        assert!(state
            .inner
            .lifecycle
            .ticket(uid)
            .await
            .unwrap()
            .pending_success()
            .is_none());
    }
    let receipt = state.inner.retry.receipt(A).unwrap();
    assert_eq!(
        original(receipt.primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(receipt.completion(), reconnect::Completion::Recorded);
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 1);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    drop(receipt);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn write_batch_bytes_manual_cleanup_timeout_preserves_original_source_and_last_report_owner()
{
    let source = RefusedSource::new().await;
    let gate = Arc::new(Semaphore::new(0));
    *source.return_gate.lock().unwrap() = Some(gate.clone());
    let mut state = manager(source.clone());
    Arc::get_mut(&mut state.inner)
        .unwrap()
        .failure_write_timeout = Duration::from_millis(40);
    pending_manual_cleanup(&state).await;
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let mut scan = Box::pin(state.refresh_once());
    source.until_error(&mut scan).await;
    let writer = state.inner.failed_attempts.write().await;
    gate.add_permits(1);
    let error = tokio::time::timeout(REFUSAL_BUDGET, scan.as_mut())
        .await
        .unwrap()
        .unwrap_err();
    drop(scan);
    drop(writer);
    assert!(error.is::<tokio::time::error::Elapsed>());
    let report = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .expect("manual cleanup lost Source roots");
    assert_eq!(report.receipts().len(), 1);
    assert_eq!(
        original(report.receipts()[0].primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(
        report.receipts()[0].completion(),
        reconnect::Completion::Failed(ios_core::ErrorClass::Timeout)
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 0);
    assert!(!state.is_ready().await);
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert!(bank.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(error);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn write_batch_bytes_whole_deadline_keeps_original_source_after_manual_marker_conversion() {
    let source = RefusedSource::new().await;
    let mut state = manager(source.clone());
    Arc::get_mut(&mut state.inner).unwrap().scan_timeout = Duration::from_secs(3);
    pending_manual_cleanup(&state).await;
    *state.inner.success_before_remove.lock().unwrap() =
        Some(Box::new(|| std::thread::sleep(Duration::from_millis(3500))));
    let error = state.refresh_once().await.unwrap_err();
    assert!(
        !source.originals.lock().unwrap().is_empty(),
        "fixture never acquired the actual Source IO before total expiry"
    );
    assert!(
        error.is::<tokio::time::error::Elapsed>(),
        "original whole timer replaced"
    );
    let report = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .expect("whole marker conversion lost Source roots");
    assert_eq!(report.receipts().len(), 1);
    assert_eq!(
        original(report.receipts()[0].primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(
        report.receipts()[0].completion(),
        reconnect::Completion::Failed(ios_core::ErrorClass::Timeout)
    );
    assert!(state.inner.failed_attempts.read().await.contains_key(B));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 0);
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn write_batch_bytes_ready_timeout_keeps_recorded_source_after_successful_writer() {
    let source = RefusedSource::new().await;
    let mut state = manager(source.clone());
    Arc::get_mut(&mut state.inner).unwrap().scan_timeout = Duration::from_secs(3);
    let ready = state.inner.ready.write().await;
    let error = state.refresh_once().await.unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>());
    let report = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .expect("readiness timeout lost recorded Source roots");
    assert_eq!(report.receipts().len(), 1);
    assert_eq!(
        original(report.receipts()[0].primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(
        report.receipts()[0].completion(),
        reconnect::Completion::Recorded
    );
    assert!(state.inner.failed_attempts.read().await.contains_key(A));
    drop(ready);
    assert!(!state.is_ready().await);
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn write_batch_bytes_caller_drop_during_manual_cleanup_marks_actual_source_abandoned() {
    let source = RefusedSource::new().await;
    let gate = Arc::new(Semaphore::new(0));
    *source.return_gate.lock().unwrap() = Some(gate.clone());
    let state = manager(source.clone());
    pending_manual_cleanup(&state).await;
    let mut scan = Box::pin(state.refresh_once());
    source.until_error(&mut scan).await;
    let writer = state.inner.failed_attempts.write().await;
    gate.add_permits(1);
    assert!(futures_util::poll!(scan.as_mut()).is_pending());
    let receipt = state.inner.retry.receipt(A).unwrap();
    assert_eq!(receipt.completion(), reconnect::Completion::Pending);
    assert_eq!(
        original(receipt.primary()),
        source.originals.lock().unwrap()[0]
    );
    drop(scan);
    assert_eq!(receipt.completion(), reconnect::Completion::Abandoned);
    assert_eq!(
        original(receipt.primary()),
        source.originals.lock().unwrap()[0]
    );
    drop(writer);
    drop(receipt);
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn candidate_input_original_source_receipt_owns_input_after_manager_clear_and_drop() {
    let source = RefusedSource::new().await;
    let state = manager(source.clone());
    state.refresh_once().await.unwrap();
    let receipt = state.inner.retry.receipt(A).unwrap();
    let original_io = original(receipt.primary());
    assert_eq!(original_io, source.originals.lock().unwrap()[0]);
    let input = receipt
        .primary()
        .downcast_ref::<candidate_input::SourceInputContext>()
        .unwrap()
        .input
        .downgrade();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    assert!(!format!("{receipt:?} {receipt}").contains(PRIVATE_CONTEXT));
    state.clear().await;
    state.notify_shutdown();
    drop(state);
    assert!(input.upgrade().is_some());
    assert_eq!(original(receipt.primary()), original_io);
    assert!(bank.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(receipt);
    assert!(input.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - candidate_input::Input::body_bytes()
    );
    drop(input);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn failure_cause_real_refusal_keeps_first_latest_and_reconciles_changed_identity_and_detach()
{
    let source = RefusedSource::new().await;
    let state = manager(source.clone());
    state.refresh_once().await.unwrap();
    let first = state.inner.retry.receipt(A).unwrap();
    assert_eq!(
        original(first.primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(original(first.latest()), original(first.primary()));
    assert_eq!(first.completion(), reconnect::Completion::Recorded);
    assert!(!format!("{first:?} {first}").contains(PRIVATE_CONTEXT));
    let slot = first.slot();
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut(A)
        .unwrap()
        .last_attempt -= Duration::from_secs(31);
    state.refresh_once().await.unwrap();
    let second = state.inner.retry.receipt(A).unwrap();
    assert_eq!(original(second.primary()), original(first.primary()));
    assert_eq!(
        original(second.latest()),
        source.originals.lock().unwrap()[1]
    );
    assert_ne!(original(second.latest()), original(second.primary()));
    assert_eq!(second.failures(), 2);
    assert!(Arc::ptr_eq(&slot, &second.slot()));
    drop(first);
    drop(second);
    drop(slot);
    source.id.store(2, Ordering::SeqCst);
    state.refresh_once().await.unwrap();
    let fresh = state.inner.retry.receipt(A).unwrap();
    assert_eq!(fresh.failures(), 1);
    assert_eq!(
        original(fresh.primary()),
        source.originals.lock().unwrap()[2]
    );
    drop(fresh);
    source.devices.lock().unwrap().clear();
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 3);
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn failure_cause_source_receipt_survives_actual_write_elapsed_with_both_original_causes() {
    let source = RefusedSource::new().await;
    let gate = Arc::new(Semaphore::new(0));
    *source.return_gate.lock().unwrap() = Some(gate.clone());
    let mut state = manager(source.clone());
    Arc::get_mut(&mut state.inner)
        .unwrap()
        .failure_write_timeout = Duration::from_millis(40);
    let mut scan = Box::pin(state.refresh_once());
    source.until_error(&mut scan).await;
    let writer = state.inner.failed_attempts.write().await;
    gate.add_permits(1);
    let error = tokio::time::timeout(REFUSAL_BUDGET, scan.as_mut())
        .await
        .unwrap()
        .unwrap_err();
    drop(scan);
    drop(writer);
    assert!(
        error.is::<tokio::time::error::Elapsed>(),
        "write must preserve its original native timer"
    );
    let report = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .unwrap();
    assert_eq!(report.receipts().len(), 1);
    let receipt = &report.receipts()[0];
    assert_eq!(
        original(receipt.primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(
        receipt.completion(),
        reconnect::Completion::Failed(ios_core::ErrorClass::Timeout)
    );
    assert!(!format!("{report:?} {report} {receipt:?} {receipt}").contains(PRIVATE_CONTEXT));
    assert!(state.inner.failed_attempts.read().await.is_empty());
    state.clear().await;
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - 1
    );
    drop(error);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    state.notify_shutdown();
}

#[tokio::test]
async fn failure_cause_caller_drop_while_writer_queued_keeps_source_and_marks_abandoned() {
    let source = RefusedSource::new().await;
    let gate = Arc::new(Semaphore::new(0));
    *source.return_gate.lock().unwrap() = Some(gate.clone());
    let state = manager(source.clone());
    let mut scan = Box::pin(state.refresh_once());
    source.until_error(&mut scan).await;
    let writer = state.inner.failed_attempts.write().await;
    gate.add_permits(1);
    assert!(futures_util::poll!(&mut scan).is_pending());
    let receipt = state.inner.retry.receipt(A).unwrap();
    assert_eq!(receipt.completion(), reconnect::Completion::Pending);
    drop(scan);
    assert_eq!(receipt.completion(), reconnect::Completion::Abandoned);
    assert_eq!(
        original(receipt.primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - 1
    );
    drop(writer);
    drop(receipt);
    state.clear().await;
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    state.notify_shutdown();
}

#[tokio::test]
async fn failure_cause_slot_exhaustion_rejects_fresh_source_and_stop_releases_originals() {
    let source = RefusedSource::new().await;
    source.devices.lock().unwrap().push(B.into());
    let mut state = manager(source.clone());
    Arc::get_mut(&mut state.inner).unwrap().failure_slots = Arc::new(Semaphore::new(1));
    state.refresh_once().await.unwrap();
    assert_eq!((source.count(A), source.count(B)), (1, 0));
    assert_eq!(state.inner.retry.receipt_count(), 1);
    assert_eq!(state.inner.failure_slots.available_permits(), 0);
    state.stop_tunnel(A).await.unwrap();
    assert_eq!(state.inner.retry.receipt_count(), 0);
    // A committed failure-table row is removed at the next complete scan.
    source.devices.lock().unwrap().retain(|key| key != A);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(B), 1);
    assert_eq!(state.inner.retry.receipt_count(), 1);
    state.clear().await;
    assert_eq!(state.inner.failure_slots.available_permits(), 1);
    state.notify_shutdown();
}

#[tokio::test]
async fn failure_cause_same_poll_clear_keeps_acquired_original_without_current_row_or_backoff() {
    let source = RefusedSource::new().await;
    let state = manager(source.clone());
    let weak = Arc::downgrade(&state.inner);
    *source.on_return.lock().unwrap() = Some(Box::new(move || {
        weak.upgrade().unwrap().lifecycle.clear();
    }));
    state.refresh_once().await.unwrap();
    assert!(state.inner.retry.receipt(A).is_none());
    let history = state.inner.retry.retired_receipts();
    assert_eq!(history.len(), 1);
    assert_eq!(
        original(history[0].primary()),
        source.originals.lock().unwrap()[0]
    );
    assert_eq!(history[0].completion(), reconnect::Completion::Abandoned);
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert!(!state.is_ready().await);
    drop(history);
    state.notify_shutdown();
    state.clear().await;
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

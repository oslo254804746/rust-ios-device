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
async fn candidate_input_original_source_receipt_owns_input_after_manager_clear_and_drop() {
    let source = RefusedSource::new().await;
    let state = manager(source.clone());
    state.refresh_once().await.unwrap();
    let receipt = state.inner.retry.receipt(A).unwrap();
    let original_io = original(receipt.primary());
    assert_eq!(original_io, source.originals.lock().unwrap()[0]);
    let input = Arc::downgrade(
        &receipt
            .primary()
            .downcast_ref::<candidate_input::SourceInputContext>()
            .unwrap()
            .input,
    );
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

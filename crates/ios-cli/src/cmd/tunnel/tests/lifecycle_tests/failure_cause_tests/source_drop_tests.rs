//! Observe the actual admitted Source future's Drop, after a real TCP refusal.
use super::*;
use crate::cmd::tunnel::{future_storage::Dynamic, shared_body};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::OwnedSemaphorePermit;

type ClearEpoch = Arc<StdMutex<marker_identity::Cutover<2>>>;

struct DropProbeSource {
    base: Arc<RefusedSource>,
    state: StdMutex<Option<shared_body::Weak<TunnelAgentInner>>>,
    clear: StdMutex<Option<ClearEpoch>>,
    receipt_at_drop: StdMutex<Option<reconnect::Receipt>>,
    held: StdMutex<Option<OwnedSemaphorePermit>>,
    dropped: AtomicUsize,
    available_at_drop: AtomicUsize,
}

struct DropProbe<'a> {
    inner: Dynamic<'a, Result<ManagedDevice>>,
    source: &'a DropProbeSource,
    bank: &'a Arc<Semaphore>,
    ready_error: bool,
}

impl Future for DropProbe<'_> {
    type Output = Result<ManagedDevice>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll(cx);
        if matches!(&result, Poll::Ready(Err(_))) {
            this.ready_error = true;
            let available = this.bank.available_permits();
            *this.source.held.lock().unwrap() = Some(
                this.bank
                    .clone()
                    .try_acquire_many_owned(available.try_into().unwrap())
                    .unwrap(),
            );
            this.bank.close();
            assert_eq!(this.bank.available_permits(), 0);
        }
        result
    }
}

impl Drop for DropProbe<'_> {
    fn drop(&mut self) {
        if !self.ready_error {
            return;
        }
        // This runs inside the original concrete F's destructor, before its
        // inner Source allocation and either actual future fee are released.
        self.source
            .available_at_drop
            .store(self.bank.available_permits(), Ordering::SeqCst);
        let state = self
            .source
            .state
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .upgrade()
            .unwrap();
        *self.source.receipt_at_drop.lock().unwrap() = state.retry.receipt(A);
        let clear = self.source.clear.lock().unwrap().clone();
        if let Some(clear) = clear {
            state.lifecycle.clear(&mut *clear.lock().unwrap());
        }
        self.source.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

impl Source for DropProbeSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.base.connect(udid, route, mode)
    }

    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: RouteRef<'a>,
        mode: TunMode,
        owner: &'a dyn ios_core::TunnelOwner,
        bank: &'a Arc<Semaphore>,
    ) -> Result<Dynamic<'a, Result<ManagedDevice>>> {
        // Keep the existing fake Source's actual reservation/attachment path;
        // the additional actual concrete F only observes its destruction.
        let inner = self.base.connect_owned(udid, route, mode, owner, bank)?;
        Dynamic::new(
            DropProbe {
                inner,
                source: self,
                bank,
                ready_error: false,
            },
            bank,
        )
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.base.usb_devices()
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        self.base.paired_devices()
    }
}

async fn ready_error_is_captured_before_source_drop(clear_epoch: bool) {
    let base = RefusedSource::new().await;
    // Prepare the epoch destinations before the source/state link is installed.
    // The source's Drop uses these original, fully paid inputs at closed zero.
    let source = Arc::new(DropProbeSource {
        base: base.clone(),
        state: StdMutex::new(None),
        clear: StdMutex::new(None),
        receipt_at_drop: StdMutex::new(None),
        held: StdMutex::new(None),
        dropped: AtomicUsize::new(0),
        available_at_drop: AtomicUsize::new(usize::MAX),
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        REFUSAL_BUDGET,
    );
    let mut terminal = PreparedTerminal::new(&state);
    let clear = clear_epoch.then(|| Arc::new(StdMutex::new(state.prepare_cutover::<2>().unwrap())));
    *source.clear.lock().unwrap() = clear.clone();
    *source.state.lock().unwrap() = Some(state.inner.downgrade());
    let bank = state.inner.global_metadata_bytes.clone();

    let result = state.refresh_once().await;
    let receipt = source.receipt_at_drop.lock().unwrap().take();
    let active = state.inner.retry.receipt(A);
    let retired = state.inner.retry.retired_receipts();
    let failed = state.inner.failed_attempts.read().await;
    let fail_count = failed.get(A).map(|row| row.fail_count);
    drop(failed);
    let dropped = source.dropped.load(Ordering::SeqCst);
    let available_at_drop = source.available_at_drop.load(Ordering::SeqCst);
    let original_pointer = base.originals.lock().unwrap()[0];
    assert!(bank.is_closed());
    terminal.shutdown(&state);
    terminal.clear(&state).await;
    drop(terminal);
    drop(state);
    drop(source);
    drop(clear);

    result.unwrap();
    assert_eq!(dropped, 1);
    assert_eq!(available_at_drop, 0);
    assert_eq!(base.count(A), 1);
    let receipt = receipt.expect("the actual Source Drop must observe its captured original");
    assert_eq!(original(receipt.primary()), original_pointer);
    assert_eq!(original(receipt.latest()), original_pointer);
    assert_eq!(receipt.failures(), 1);
    if clear_epoch {
        assert!(active.is_none());
        assert_eq!(retired.len(), 1);
        assert!(retired[0].ptr_eq(&receipt));
        assert_eq!(receipt.completion(), reconnect::Completion::Abandoned);
        assert_eq!(fail_count, None);
    } else {
        assert!(active.as_ref().unwrap().ptr_eq(&receipt));
        assert!(retired.is_empty());
        assert_eq!(receipt.completion(), reconnect::Completion::Recorded);
        assert_eq!(fail_count, Some(1));
    }
    drop((active, retired));
    assert!(bank.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(receipt);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn source_ready_error_is_durable_before_original_source_drop_at_closed_zero() {
    ready_error_is_captured_before_source_drop(false).await;
}

#[tokio::test]
async fn source_ready_error_precedes_source_drop_epoch_change_at_closed_zero() {
    ready_error_is_captured_before_source_drop(true).await;
}

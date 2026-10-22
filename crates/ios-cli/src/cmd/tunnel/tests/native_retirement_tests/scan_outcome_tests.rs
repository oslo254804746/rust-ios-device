//! Original discovery errors across scanner Drop and independent native joins.
use super::*;
use std::collections::VecDeque;

enum Stage {
    Paired,
    List,
}

struct OutcomeSource {
    base: Arc<LocalSource>,
    stage: Stage,
    streams: StdMutex<VecDeque<TcpStream>>,
    originals: StdMutex<Vec<usize>>,
    usb_calls: AtomicUsize,
    entered: AtomicUsize,
    list_gate: Semaphore,
    connection_gate: Semaphore,
}

impl OutcomeSource {
    async fn new(stage: Stage, errors: usize) -> Arc<Self> {
        let mut streams = VecDeque::new();
        for _ in 0..errors {
            streams.push_back(eof_stream().await);
        }
        Arc::new(Self {
            base: LocalSource::new(),
            stage,
            streams: StdMutex::new(streams),
            originals: StdMutex::new(Vec::new()),
            usb_calls: AtomicUsize::new(0),
            entered: AtomicUsize::new(0),
            list_gate: Semaphore::new(0),
            connection_gate: Semaphore::new(0),
        })
    }

    async fn error(&self) -> anyhow::Error {
        let mut stream = self.streams.lock().unwrap().pop_front().unwrap();
        let error: anyhow::Error = stream.read_exact(&mut [0; 2]).await.unwrap_err().into();
        let address = error.downcast_ref::<io::Error>().unwrap() as *const io::Error as usize;
        self.originals.lock().unwrap().push(address);
        error
    }
}

impl Source for OutcomeSource {
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
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<tokio::sync::Semaphore>,
    ) -> anyhow::Result<
        crate::cmd::tunnel::future_storage::Dynamic<'a, anyhow::Result<ManagedDevice>>,
    > {
        crate::cmd::tunnel::future_storage::Dynamic::new(
            async move {
                let mut reservation = owner.reserve()?;
                let device = self.base.connect(udid, route.fixture_owned(), mode).await?;
                reservation.attach(device.native_handle().unwrap().clone())?;
                self.entered.fetch_add(1, Ordering::SeqCst);
                if matches!(self.stage, Stage::Paired) {
                    self.connection_gate.acquire().await?.forget();
                }
                reservation.complete();
                Ok(device)
            },
            bank,
        )
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async move {
            self.usb_calls.fetch_add(1, Ordering::SeqCst);
            match self.stage {
                Stage::List => {
                    self.list_gate.acquire().await.unwrap().forget();
                    UsbSnapshot::ListError(self.error().await)
                }
                Stage::Paired => self.base.usb_devices().await,
            }
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async move {
            match self.stage {
                Stage::Paired => Err(self.error().await),
                Stage::List => Ok(Vec::new()),
            }
        })
    }
}

async fn eof_stream() -> TcpStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (stream, peer) = tokio::join!(
        TcpStream::connect(listener.local_addr().unwrap()),
        listener.accept()
    );
    let (mut peer, _) = peer.unwrap();
    peer.write_all(&[1]).await.unwrap();
    peer.shutdown().await.unwrap();
    drop(peer);
    stream.unwrap()
}

fn manager(source: Arc<OutcomeSource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
    )
}

async fn drive_until<F: std::future::Future>(
    future: &mut std::pin::Pin<Box<F>>,
    condition: impl Fn() -> bool,
) -> bool {
    tokio::time::timeout(OBSERVE, async {
        loop {
            if futures_util::poll!(future.as_mut()).is_ready() {
                return false;
            }
            if condition() {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn recover(state: &TunnelAgentState, source: &OutcomeSource, expected: usize) {
    let handles = source.base.handles.lock().unwrap().clone();
    state.drain_native(OBSERVE).await.unwrap();
    let recovery = recovered(&source.base, expected).await;
    state.clear().await;
    // Native certification remains independent of the preserved source cause.
    assert!(
        recovery
            && handles
                .iter()
                .all(|handle| handle.shutdown_status().is_ok())
    );
    assert_eq!(state.inner.native.snapshot().native_handles, 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

fn original_address(error: &(dyn std::error::Error + 'static)) -> Option<usize> {
    let mut cause = Some(error);
    for _ in 0..32 {
        let current = cause?;
        if let Some(io) = current.downcast_ref::<io::Error>() {
            return Some(io as *const io::Error as usize);
        }
        cause = current.source();
    }
    None
}

#[tokio::test]
async fn native_scan_outcome_server_signal_keeps_original_paired_error_after_scanner_drop() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::Paired, 1).await;
    let state = manager(source.clone());
    let signal = Semaphore::new(0);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut owner = Box::pin(
        server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_secs(60),
            OBSERVE,
            async {
                signal.acquire().await.unwrap().forget();
                Ok(())
            },
        )
        .unwrap(),
    );
    let entered = drive_until(&mut owner, || source.entered.load(Ordering::SeqCst) == 1).await;
    let native = source.base.handle();
    let original = source.originals.lock().unwrap()[0];
    signal.add_permits(1);
    let result = tokio::time::timeout(OBSERVE, owner.as_mut()).await;
    drop(owner);
    recover(&state, &source, expected).await;
    assert!(entered && !native.is_alive() && native.shutdown_status().is_ok());
    let error = result
        .unwrap()
        .expect_err("acquired paired error vanished during server shutdown");
    assert_eq!(original_address(error.as_ref()), Some(original));
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Io
    );
}

#[tokio::test]
async fn native_scan_outcome_list_error_survives_held_metadata_cleanup_expiry() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::List, 1).await;
    let state = manager(source.clone());
    let signal = Semaphore::new(0);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut owner = Box::pin(
        server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_secs(60),
            Duration::from_millis(40),
            async {
                signal.acquire().await.unwrap().forget();
                Ok(())
            },
        )
        .unwrap(),
    );
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = source.base.handle();
    let discovery = drive_until(&mut owner, || source.usb_calls.load(Ordering::SeqCst) == 1).await;
    let table = state.inner.tunnels.write().await;
    source.list_gate.add_permits(1);
    let acquired = drive_until(&mut owner, || !source.originals.lock().unwrap().is_empty()).await;
    let original = source.originals.lock().unwrap()[0];
    signal.add_permits(1);
    let result = tokio::time::timeout(OBSERVE, owner.as_mut()).await;
    drop(owner);
    // Prove the original joins finish even while metadata cleanup is blocked.
    state.drain_native(OBSERVE).await.unwrap();
    let joined_while_held = native.shutdown_status().is_ok();
    let recovery = recovered(&source.base, expected).await;
    drop(table);
    state.clear().await;
    assert!(discovery && acquired && joined_while_held && recovery);
    assert!(state.inner.native.snapshot().closed_and_empty());
    let error = result.unwrap().unwrap_err();
    assert_eq!(original_address(error.as_ref()), Some(original));
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Io
    );
}

#[tokio::test]
async fn native_scan_outcome_eight_abandoned_errors_reject_ninth_before_source_poll() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::Paired, 9).await;
    let state = manager(source.clone());
    for count in 1..=8 {
        let mut scan = Box::pin(state.refresh_once());
        let entered =
            drive_until(&mut scan, || source.entered.load(Ordering::SeqCst) == count).await;
        drop(scan);
        recover(&state, &source, expected).await;
        assert!(entered);
        let history = state
            .inner
            .scan_outcomes
            .history()
            .expect("abandoned scan history");
        assert_eq!(history.len(), count);
        for (error, original) in history.iter().zip(source.originals.lock().unwrap().iter()) {
            assert_eq!(original_address(error.as_ref()), Some(*original));
        }
    }
    let before = source.usb_calls.load(Ordering::SeqCst);
    let mut ninth = Box::pin(state.refresh_once());
    let result = tokio::time::timeout(Duration::from_millis(100), ninth.as_mut()).await;
    drop(ninth);
    recover(&state, &source, expected).await;
    state.notify_shutdown();
    assert_eq!(
        source.usb_calls.load(Ordering::SeqCst),
        before,
        "full history polled another source"
    );
    assert_eq!(source.entered.load(Ordering::SeqCst), 8);
    let error = result
        .expect("full scan outcome history did not reject admission")
        .unwrap_err();
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-scan",
        &error,
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(json["error"]["class"], "resource_limit");
}

#[tokio::test]
async fn native_scan_outcome_normal_return_preserves_direct_io_downcast_and_releases_slot() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::Paired, 1).await;
    source.connection_gate.add_permits(1);
    let state = manager(source.clone());
    let error = state.refresh_once().await.unwrap_err();
    let direct = error.downcast_ref::<io::Error>().unwrap() as *const io::Error as usize;
    assert_eq!(direct, source.originals.lock().unwrap()[0]);
    assert!(state.inner.scan_outcomes.history().is_none());
    assert!(!state.is_ready().await);
    state.clear().await;
    recover(&state, &source, expected).await;
    state.notify_shutdown();
}

#[tokio::test]
async fn native_scan_outcome_caller_drop_retains_history_after_clear_and_terminal_close() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::Paired, 1).await;
    let state = manager(source.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut owner = Box::pin(
        server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_secs(60),
            OBSERVE,
            std::future::pending(),
        )
        .unwrap(),
    );
    assert!(drive_until(&mut owner, || source.entered.load(Ordering::SeqCst) == 1).await);
    drop(owner);
    let before = state
        .inner
        .scan_outcomes
        .history()
        .expect("abandoned scan history");
    recover(&state, &source, expected).await;
    let after = state
        .inner
        .scan_outcomes
        .history()
        .expect("abandoned scan history");
    assert_eq!(after.len(), 1);
    assert!(before[0].ptr_eq(&after[0]));
    assert_eq!(
        original_address(after[0].as_ref()),
        Some(source.originals.lock().unwrap()[0])
    );
    assert!(state.inner.scan_lock.try_lock().is_ok());
    assert!(state
        .refresh_once()
        .await
        .unwrap_err()
        .is::<ios_core::OperationCancelled>());
}

#[tokio::test]
async fn native_scan_outcome_whole_expiry_returns_original_io_and_secondary_elapsed() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::Paired, 1).await;
    let mut state = manager(source.clone());
    state.inner.get_mut().unwrap().scan_timeout = Duration::from_millis(40);
    let error = state.refresh_once().await.unwrap_err();
    recover(&state, &source, expected).await;
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap() as *const io::Error as usize,
        source.originals.lock().unwrap()[0]
    );
    assert!(error
        .downcast_ref::<bookkeeping::FailureContext>()
        .unwrap()
        .secondary()
        .is::<tokio::time::error::Elapsed>());
    assert!(state.inner.scan_outcomes.history().is_none());
    assert!(!state.is_ready().await);
    state.notify_shutdown();
}

#[tokio::test]
async fn native_scan_outcome_repeated_terminal_reports_share_original_history() {
    let expected = baseline();
    let source = OutcomeSource::new(Stage::Paired, 1).await;
    let state = manager(source.clone());
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    // Both complete future allocations exist BEFORE the original paired
    // failure. Reporting that acquired history later requires no new grant.
    let mut servers = Vec::new();
    for _ in 0..2 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let signal = std::future::ready(Ok::<(), io::Error>(()));
        let frame_bytes = server::frame_bytes(&signal);
        let prepared = server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_secs(60),
            OBSERVE,
            signal,
        )
        .unwrap();
        servers.push((prepared, frame_bytes));
    }
    let mut scan = Box::pin(state.refresh_once());
    assert!(drive_until(&mut scan, || source.entered.load(Ordering::SeqCst) == 1).await);
    drop(scan);
    recover(&state, &source, expected).await;
    let history = state
        .inner
        .scan_outcomes
        .history()
        .expect("abandoned scan history");
    let mut held = Arc::clone(&bank)
        .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
        .unwrap();
    for (prepared, frame_bytes) in servers {
        assert_eq!(bank.available_permits(), 0);
        let error = prepared.await.unwrap_err();
        // That actual F has now ended and its allocation was freed. Reapply
        // exactly its observed returned credit as pressure for the next report.
        assert_eq!(bank.available_permits(), frame_bytes);
        held.merge(
            bank.clone()
                .try_acquire_many_owned(frame_bytes.try_into().unwrap())
                .unwrap(),
        );
        let terminal = error
            .downcast_ref::<server::TerminalCleanupFailure>()
            .unwrap();
        let terminal_history = terminal.abandoned_scans().expect("shared prepaid history");
        assert!(history.ptr_eq(terminal_history));
        assert_eq!(terminal_history.len(), 1);
        assert!(history[0].ptr_eq(&terminal_history[0]));
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(
            original_address(terminal.primary()),
            Some(source.originals.lock().unwrap()[0])
        );
        assert!(terminal.native_snapshot().closed_and_empty());
    }
    drop(held);
}

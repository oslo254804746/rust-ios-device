//! Failure-table contention with original native owners and actual loopback EOF.
use super::*;

enum PairedReply {
    Complete,
    Io(TcpStream),
    Oversized,
}

struct FailureSource {
    base: Arc<LocalSource>,
    ids: Vec<String>,
    streams: StdMutex<HashMap<String, TcpStream>>,
    paired: StdMutex<Option<PairedReply>>,
    release_failures: Semaphore,
    release_healthy: Semaphore,
    entered: AtomicUsize,
    eof: AtomicUsize,
}

impl Source for FailureSource {
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
        route: Route,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
        bank: &'a Arc<tokio::sync::Semaphore>,
    ) -> anyhow::Result<
        crate::cmd::tunnel::future_storage::Dynamic<'a, anyhow::Result<ManagedDevice>>,
    > {
        crate::cmd::tunnel::future_storage::Dynamic::new(
            async move {
                let mut reservation = owner.reserve()?;
                let device = self.base.connect(udid, route, mode).await?;
                reservation.attach(device.native_handle().unwrap().clone())?;
                self.entered.fetch_add(1, Ordering::SeqCst);
                if udid == DEVICE {
                    self.release_healthy.acquire().await?.forget();
                    reservation.complete();
                    return Ok(device);
                }
                self.release_failures.acquire().await?.forget();
                let mut stream = self.streams.lock().unwrap().remove(udid).unwrap();
                let error = stream.read_exact(&mut [0; 2]).await.unwrap_err();
                self.eof.fetch_add(1, Ordering::SeqCst);
                // The reservation retains the exact original runtime on this Err.
                Err(error.into())
            },
            bank,
        )
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async move {
            UsbSnapshot::Devices(
                self.ids
                    .iter()
                    .map(|udid| source::UsbDevice {
                        udid: udid.clone(),
                        device_id: 7,
                        usb: true,
                    })
                    .collect(),
            )
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async move {
            let reply = self.paired.lock().unwrap().take().unwrap();
            match reply {
                PairedReply::Complete => Ok(Vec::new()),
                PairedReply::Io(mut stream) => {
                    Err(stream.read_exact(&mut [0; 2]).await.unwrap_err().into())
                }
                PairedReply::Oversized => Ok(vec![ios_core::device::PairedMobdev2Device {
                    udid: "x".repeat(4097),
                    host: "192.0.2.1".into(),
                }]),
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

async fn fixture(failures: usize, healthy: bool, paired: PairedReply) -> Arc<FailureSource> {
    let mut ids = Vec::new();
    let mut streams = HashMap::new();
    for index in 0..failures {
        let udid = format!("synthetic-native-bookkeeping-{index}");
        streams.insert(udid.clone(), eof_stream().await);
        ids.push(udid);
    }
    if healthy {
        ids.push(DEVICE.into());
    }
    Arc::new(FailureSource {
        base: LocalSource::new(),
        ids,
        streams: StdMutex::new(streams),
        paired: StdMutex::new(Some(paired)),
        release_failures: Semaphore::new(0),
        release_healthy: Semaphore::new(0),
        entered: AtomicUsize::new(0),
        eof: AtomicUsize::new(0),
    })
}

fn manager(source: Arc<FailureSource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
    )
}

async fn drive_until(
    scan: &mut std::pin::Pin<Box<impl std::future::Future<Output = Result<()>>>>,
    source: &FailureSource,
    entered: usize,
    eof: usize,
    budget: Duration,
) -> bool {
    tokio::time::timeout(budget, async {
        loop {
            if futures_util::poll!(scan.as_mut()).is_ready() {
                return false;
            }
            if source.entered.load(Ordering::SeqCst) == entered
                && source.eof.load(Ordering::SeqCst) == eof
            {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn recover(
    state: &TunnelAgentState,
    source: &FailureSource,
    expected: usize,
    held_errors: usize,
) {
    let handles = source.base.handles.lock().unwrap().clone();
    let recovery = finish(state, &source.base, expected).await;
    // Always recover original joins, actual duplex EOF and task count before
    // evaluating the changed contention/error behavior.
    assert!(
        recovery
            && handles
                .iter()
                .all(|native| native.shutdown_status().is_ok())
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - held_errors
    );
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn native_bookkeeping_failed_writers_do_not_occupy_later_candidate_window() {
    let expected = baseline();
    let source = fixture(4, true, PairedReply::Complete).await;
    let state = manager(source.clone());
    let mut scan = Box::pin(state.refresh_once());
    let first = drive_until(&mut scan, &source, 4, 0, OBSERVE).await;
    let failed = state.inner.failed_attempts.read().await;
    source.release_failures.add_permits(4);
    let later = drive_until(&mut scan, &source, 5, 4, Duration::from_millis(200)).await;
    let native = state.inner.native.snapshot();
    let later_native = if later {
        Some(source.base.handle())
    } else {
        None
    };
    let later_live = later_native
        .as_ref()
        .is_some_and(|native| native.is_alive());
    let no_backoff = failed.is_empty();
    let eof = source.eof.load(Ordering::SeqCst);
    drop(scan);
    let after_drop = state.inner.native.snapshot();
    let later_stopped = later_native
        .as_ref()
        .is_some_and(|native| !native.is_alive());
    drop(failed);
    let ready = state.is_ready().await;
    recover(&state, &source, expected, 0).await;
    assert!(
        first && later,
        "failure writers prevented the fifth native candidate from entering"
    );
    assert!(no_backoff && !ready && later_live && later_stopped);
    assert_eq!(eof, 4);
    assert_eq!(
        (native.retired, native.unpublished, native.native_handles),
        (4, 1, 5)
    );
    assert_eq!(native.inflight + native.active, 0);
    assert_eq!((after_drop.retired, after_drop.native_handles), (5, 5));
}

async fn blocked_flush(paired: PairedReply) -> (Result<()>, TunnelAgentState) {
    let expected = baseline();
    let source = fixture(1, false, paired).await;
    let state = manager(source.clone());
    let mut scan = Box::pin(state.refresh_once());
    let entered = drive_until(&mut scan, &source, 1, 0, OBSERVE).await;
    let failed = state.inner.failed_attempts.read().await;
    source.release_failures.add_permits(1);
    let result = tokio::time::timeout(OBSERVE, scan.as_mut()).await;
    let unchanged = failed.is_empty();
    drop(scan);
    drop(failed);
    let not_ready = !state.is_ready().await;
    let native = state.inner.native.snapshot();
    let result = result.expect("failure bookkeeping exceeded its independent product total");
    let error = result.as_ref().unwrap_err();
    let context = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .or_else(|| {
            error
                .downcast_ref::<bookkeeping::FailureContext>()
                .and_then(|context| {
                    context
                        .secondary()
                        .downcast_ref::<bookkeeping::SourceFailuresContext>()
                })
        })
        .unwrap();
    assert_eq!(context.receipts().len(), 1);
    assert_eq!(
        context.receipts()[0]
            .primary()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
    // The returned original-error receipt remains a real owner after manager
    // shutdown; its permit cannot be released while this caller holds it.
    recover(&state, &source, expected, 1).await;
    assert!(entered && unchanged && not_ready);
    assert_eq!(source.eof.load(Ordering::SeqCst), 1);
    assert_eq!((native.retired, native.native_handles), (1, 1));
    (result, state)
}

#[tokio::test]
async fn native_bookkeeping_held_failure_table_has_finite_wait_and_releases_slots() {
    let (result, state) = blocked_flush(PairedReply::Complete).await;
    let error = result.unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    drop(error);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn native_bookkeeping_flush_elapsed_keeps_acquired_paired_io_primary() {
    let (result, state) = blocked_flush(PairedReply::Io(eof_stream().await)).await;
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Io
    );
    let context = error.downcast_ref::<bookkeeping::FailureContext>().unwrap();
    assert!(context.secondary().is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(context.secondary().as_ref()),
        ios_core::ErrorClass::Timeout
    );
    assert!(!format!("{context:?} {context}").contains("synthetic"));
    drop(error);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn native_bookkeeping_flush_elapsed_keeps_paired_resource_primary() {
    let (result, state) = blocked_flush(PairedReply::Oversized).await;
    let error = result.unwrap_err();
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-scan",
        &error,
    )
    .unwrap();
    let decision: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(decision["error"]["class"], "resource_limit");
    let context = error.downcast_ref::<bookkeeping::FailureContext>().unwrap();
    assert!(context.secondary().is::<tokio::time::error::Elapsed>());
    drop(error);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

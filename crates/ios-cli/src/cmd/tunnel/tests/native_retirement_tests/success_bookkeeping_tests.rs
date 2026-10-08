//! Successful candidate bookkeeping with real native owners and loopback EOF.
use super::*;

const WRITE_BUDGET: Duration = Duration::from_millis(40);

struct CandidateDevice {
    udid: String,
    native: Arc<TunnelHandle>,
}

impl source::FakeDevice for CandidateDevice {
    fn record(&self) -> TunnelRecord {
        let mut record = fake_record(self.native.userspace_port.unwrap());
        record.udid = self.udid.clone();
        record
    }

    fn is_alive(&self) -> bool {
        self.native.is_alive()
    }

    fn device_id(&self) -> u32 {
        7
    }

    fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        Some(&self.native)
    }
}

struct SuccessSource {
    base: Arc<LocalSource>,
    ids: Vec<String>,
    originals: StdMutex<HashMap<String, Arc<TunnelHandle>>>,
    release_first: Semaphore,
    release_fifth: Semaphore,
    entered: AtomicUsize,
}

impl SuccessSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            base: LocalSource::new(),
            ids: (0..5)
                .map(|index| format!("synthetic-native-success-bookkeeping-{index}"))
                .collect(),
            originals: StdMutex::new(HashMap::new()),
            release_first: Semaphore::new(0),
            release_fifth: Semaphore::new(0),
            entered: AtomicUsize::new(0),
        })
    }
}

impl Source for SuccessSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let device = self.base.connect(udid, route, mode).await?;
            let native = device.native_handle().unwrap().clone();
            self.originals
                .lock()
                .unwrap()
                .insert(udid.into(), native.clone());
            Ok(ManagedDevice::Fake(Box::new(CandidateDevice {
                udid: udid.into(),
                native,
            })))
        })
    }

    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let mut reservation = owner.reserve()?;
            let device = self.connect(udid, route, mode).await?;
            // Attach the exact native Arc before the held source boundary.
            reservation.attach(device.native_handle().unwrap().clone())?;
            self.entered.fetch_add(1, Ordering::SeqCst);
            if udid == self.ids[4] {
                self.release_fifth.acquire().await?.forget();
            } else {
                self.release_first.acquire().await?.forget();
            }
            reservation.complete();
            Ok(device)
        })
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
        Box::pin(async { Ok(Vec::new()) })
    }
}

async fn fixture() -> (Arc<SuccessSource>, TunnelAgentState) {
    let source = SuccessSource::new();
    let mut state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        OBSERVE,
    );
    Arc::get_mut(&mut state.inner)
        .unwrap()
        .failure_write_timeout = WRITE_BUDGET;
    for udid in &source.ids {
        let ticket = state.inner.lifecycle.ticket(udid).await.unwrap();
        let slot = state.reserve_failure_slot(udid).await.unwrap();
        state
            .record_failed_attempt(
                udid,
                AttemptIdentity {
                    route: Route::Usbmux,
                    native_id: Some(7),
                },
                slot,
                &ticket,
            )
            .await;
        state
            .inner
            .failed_attempts
            .write()
            .await
            .get_mut(udid)
            .unwrap()
            .last_attempt -= Duration::from_secs(31);
    }
    (source, state)
}

async fn drive_until(
    scan: &mut std::pin::Pin<Box<impl std::future::Future<Output = Result<()>>>>,
    source: &SuccessSource,
    entered: usize,
    budget: Duration,
) -> bool {
    tokio::time::timeout(budget, async {
        loop {
            if futures_util::poll!(scan.as_mut()).is_ready() {
                return false;
            }
            if source.entered.load(Ordering::SeqCst) == entered {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn proxies(handles: &[Arc<TunnelHandle>]) -> Vec<TcpStream> {
    let mut proxies = Vec::new();
    for native in handles {
        let mut proxy = TcpStream::connect(("127.0.0.1", native.userspace_port.unwrap()))
            .await
            .unwrap();
        // A real proxy worker now owns an incomplete native preamble.
        proxy.write_all(&[0]).await.unwrap();
        proxies.push(proxy);
    }
    proxies
}

async fn recover(
    state: &TunnelAgentState,
    source: &SuccessSource,
    expected: usize,
    handles: &[Arc<TunnelHandle>],
    proxies: Vec<TcpStream>,
) {
    let recovery = finish(state, &source.base, expected).await;
    let eof = tokio::time::timeout(OBSERVE, async {
        for mut proxy in proxies {
            if !proxy.read_u8().await.is_err_and(|error| {
                matches!(
                    error.kind(),
                    io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
                )
            }) {
                return false;
            }
        }
        true
    })
    .await
    .unwrap_or(false);
    // Complete original joins, duplex/loopback EOF, registry and task recovery
    // before evaluating any changed candidate-window assertion.
    assert!(
        recovery
            && eof
            && handles
                .iter()
                .all(|handle| handle.shutdown_status().is_ok())
    );
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(baseline(), expected);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn native_success_bookkeeping_read_lock_allows_complete_window_and_bounded_flush() {
    let expected = baseline();
    let (source, state) = fixture().await;
    let mut scan = Box::pin(state.refresh_once());
    let first = drive_until(&mut scan, &source, 4, OBSERVE).await;
    let failed = state.inner.failed_attempts.read().await;
    source.release_first.add_permits(4);
    let later = drive_until(&mut scan, &source, 5, Duration::from_millis(200)).await;
    let first_published = state.list_records().await.len();
    let before_fifth = state.inner.native.snapshot();
    source.release_fifth.add_permits(1);
    let result = if later {
        Some(tokio::time::timeout(OBSERVE, scan.as_mut()).await)
    } else {
        None
    };
    let published = state.list_records().await.len();
    let after_flush = state.inner.native.snapshot();
    let unchanged = failed.len() == 5 && failed.values().all(|row| row.fail_count == 1);
    let slots = state.inner.failure_slots.available_permits();
    let not_ready = !state.is_ready().await;
    let handles = source.base.handles.lock().unwrap().clone();
    let original_owners = {
        let tunnels = state.inner.tunnels.read().await;
        let originals = source.originals.lock().unwrap();
        source.ids.iter().all(|udid| {
            tunnels
                .get(udid)
                .and_then(|tunnel| tunnel.device.native_handle())
                .is_some_and(|published| {
                    originals
                        .get(udid)
                        .is_some_and(|original| Arc::ptr_eq(original, published))
                })
        })
    };
    let proxies = proxies(&handles).await;
    drop(scan);
    drop(failed);
    recover(&state, &source, expected, &handles, proxies).await;

    assert!(
        first && later,
        "success writers blocked the fifth original native runtime"
    );
    assert_eq!(first_published, 4);
    assert_eq!(published, 5);
    assert!(original_owners && unchanged && not_ready);
    assert_eq!(source.base.connect_calls.load(Ordering::SeqCst), 5);
    assert_eq!(slots, MAX_FAILURE_RECORDS - 5);
    assert_eq!(
        (
            before_fifth.active,
            before_fifth.unpublished,
            before_fifth.native_handles
        ),
        (4, 1, 5)
    );
    assert_eq!((after_flush.active, after_flush.native_handles), (5, 5));
    assert_eq!(
        after_flush.reserved + after_flush.inflight + after_flush.retired,
        0
    );
    let error = result
        .unwrap()
        .expect("published native success bookkeeping exceeded its finite total")
        .unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

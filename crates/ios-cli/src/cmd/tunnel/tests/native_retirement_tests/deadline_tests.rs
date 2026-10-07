//! Nominal deadline crossing within one actual poll, with retained native IO.
use super::*;

const BUDGET: Duration = Duration::from_millis(200);
const CROSS: Duration = Duration::from_millis(250);

mod automatic_tests;

#[derive(Clone, Copy)]
enum Delay {
    Metadata,
    SourceSuccess,
    SourceFailure,
    Publication,
}

struct DelayedDevice {
    device: Box<dyn source::FakeDevice>,
    delay: Delay,
    records: Arc<AtomicUsize>,
}
impl source::FakeDevice for DelayedDevice {
    fn record(&self) -> TunnelRecord {
        self.records.fetch_add(1, Ordering::SeqCst);
        if matches!(self.delay, Delay::Metadata) {
            // A deliberately synchronous poll cannot be preempted by Tokio.
            std::thread::sleep(CROSS);
        }
        self.device.record()
    }
    fn is_alive(&self) -> bool {
        self.device.is_alive()
    }
    fn device_id(&self) -> u32 {
        self.device.device_id()
    }
    fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        self.device.native_handle()
    }
}

struct DelayedSource {
    base: Arc<LocalSource>,
    records: Arc<AtomicUsize>,
    paired: AtomicUsize,
    delay: Delay,
}
impl Source for DelayedSource {
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
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let mut reservation = owner.reserve()?;
            let device = self.base.connect(udid, route, mode).await?;
            reservation.attach(device.native_handle().unwrap().clone())?;
            if matches!(self.delay, Delay::SourceSuccess | Delay::SourceFailure) {
                std::thread::sleep(CROSS);
            }
            if matches!(self.delay, Delay::SourceFailure) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic-private-late-source",
                )
                .into());
            }
            reservation.complete();
            let ManagedDevice::Fake(device) = device else {
                unreachable!("private native metadata seam")
            };
            Ok(ManagedDevice::Fake(Box::new(DelayedDevice {
                device,
                delay: self.delay,
                records: self.records.clone(),
            })))
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.base.usb_devices()
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async {
            self.paired.fetch_add(1, Ordering::SeqCst);
            self.base.paired_devices().await
        })
    }
}

async fn exercise(
    delay: Delay,
    route: Option<StartTunnelConnectionType>,
) -> (Result<TunnelRecord>, bool, usize, usize, usize, bool) {
    let expected = baseline();
    let base = LocalSource::new();
    let source = Arc::new(DelayedSource {
        base: base.clone(),
        records: Arc::new(AtomicUsize::new(0)),
        paired: AtomicUsize::new(0),
        delay,
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        BUDGET,
    );
    let result = state.ensure_tunnel(DEVICE, None, route).await;
    let unpublished = state.inner.tunnels.read().await.is_empty();
    let queries = base.usb_calls.load(Ordering::SeqCst);
    let records = source.records.load(Ordering::SeqCst);
    let paired = source.paired.load(Ordering::SeqCst);
    let recovery = finish(&state, &base, expected).await;
    // Recover actual original native tasks/stream before checking any regression.
    assert!(recovery && base.handle().shutdown_status().is_ok());
    (result, unpublished, queries, records, paired, recovery)
}

fn elapsed(error: &anyhow::Error) {
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

#[tokio::test]
async fn deadline_crossing_manual_metadata_cannot_query_or_publish_after_same_poll_expiry() {
    let (result, unpublished, queries, records, _, _) =
        exercise(Delay::Metadata, Some(StartTunnelConnectionType::Usbmux)).await;
    assert!(unpublished);
    assert_eq!((queries, records), (0, 1));
    elapsed(&result.unwrap_err());
}

#[tokio::test]
async fn deadline_crossing_manual_source_success_cannot_read_metadata_or_publish() {
    let (result, unpublished, queries, records, _, _) = exercise(
        Delay::SourceSuccess,
        Some(StartTunnelConnectionType::Usbmux),
    )
    .await;
    assert!(unpublished);
    assert_eq!((queries, records), (0, 0));
    elapsed(&result.unwrap_err());
}

#[tokio::test]
async fn deadline_crossing_manual_acquired_source_io_survives_without_fallback_contact() {
    let (result, unpublished, queries, records, paired, _) =
        exercise(Delay::SourceFailure, None).await;
    assert!(unpublished);
    assert_eq!((queries, records, paired), (0, 0, 0));
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
}

#[tokio::test]
async fn deadline_crossing_generic_late_ready_success_returns_original_native_elapsed() {
    let deadline = lifecycle::ConnectionDeadline::new(BUDGET).unwrap();
    let result = deadline
        .run(async {
            std::thread::sleep(CROSS);
            Ok::<_, anyhow::Error>(7)
        })
        .await;
    elapsed(&result.unwrap_err());
}

#[tokio::test]
async fn deadline_crossing_generic_acquired_io_remains_primary_after_long_ready_poll() {
    let deadline = lifecycle::ConnectionDeadline::new(BUDGET).unwrap();
    let result = deadline
        .run(async {
            std::thread::sleep(CROSS);
            Err::<(), _>(
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "synthetic-private-deadline-io",
                )
                .into(),
            )
        })
        .await;
    assert_eq!(
        result
            .unwrap_err()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

async fn publication_crossing(replace: bool) {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let old = if replace {
        publish(&state, Route::Wifi(None)).await.unwrap();
        Some(source.handle())
    } else {
        None
    };
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    *state.inner.publication_after_insert.lock().unwrap() =
        Some(Box::new(|| std::thread::sleep(CROSS)));
    let deadline = lifecycle::ConnectionDeadline::new(BUDGET).unwrap();
    let result = deadline
        .run(state.connect_and_store(DEVICE, Route::Wifi(None), &ticket, None, Some(&deadline)))
        .await;
    let new = source.handle();
    let table = state.inner.tunnels.read().await;
    let restored = match &old {
        Some(old) => table
            .get(DEVICE)
            .and_then(|row| row.device.native_handle())
            .is_some_and(|handle| Arc::ptr_eq(handle, old)),
        None => table.is_empty(),
    };
    drop(table);
    let stopped = !new.is_alive();
    let old_alive = old.as_ref().map_or(true, |old| old.is_alive());
    let snapshot = state.inner.native.snapshot();
    let recovery = finish(&state, &source, expected).await;
    // Both positive and mutation runs join the exact original runtimes and
    // observe their actual stream EOF/task recovery before regression checks.
    assert!(recovery && new.shutdown_status().is_ok());
    assert!(old
        .as_ref()
        .map_or(true, |old| old.shutdown_status().is_ok()));
    assert!(restored && stopped && old_alive);
    assert_eq!(snapshot.active, usize::from(replace));
    assert_eq!(snapshot.retired, 1);
    assert_eq!(snapshot.native_handles, 1 + usize::from(replace));
    elapsed(&result.unwrap_err());
}

#[tokio::test]
async fn deadline_crossing_manual_insert_rolls_back_unobserved_new_row() {
    publication_crossing(false).await;
}

#[tokio::test]
async fn deadline_crossing_manual_replacement_restores_original_live_native_arc() {
    publication_crossing(true).await;
}

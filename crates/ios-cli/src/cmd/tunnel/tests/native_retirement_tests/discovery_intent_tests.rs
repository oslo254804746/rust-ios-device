//! Actual TCP discovery IO causes and original userspace TUN joins/peer EOF.
use super::*;
use crate::access::test_support::JsonRequest;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8};

const USB_UNKNOWN: &str = "manual-capacity-unknown-usb";
const PAIRED_UNKNOWN: &str = "manual-capacity-unknown-paired";

struct DiscoverySource {
    base: Arc<LocalSource>,
    // 0: complete; 1: two independent native IO errors; 2: malformed paired.
    mode: AtomicU8,
    blocked: AtomicBool,
    entered: AtomicUsize,
    usb_calls: AtomicUsize,
    gate: Semaphore,
    usb: StdMutex<Vec<source::UsbDevice>>,
    paired: StdMutex<Vec<ios_core::device::PairedMobdev2Device>>,
    errors: StdMutex<VecDeque<TcpStream>>,
    originals: StdMutex<Vec<(u8, usize)>>,
    routes: StdMutex<Vec<(String, Route)>>,
}

impl DiscoverySource {
    async fn new(errors: usize) -> Arc<Self> {
        let mut streams = VecDeque::new();
        for _ in 0..errors {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let (stream, accepted) = tokio::join!(
                TcpStream::connect(listener.local_addr().unwrap()),
                listener.accept()
            );
            let (mut peer, _) = accepted.unwrap();
            peer.write_all(&[1]).await.unwrap();
            peer.shutdown().await.unwrap();
            drop(peer);
            streams.push_back(stream.unwrap());
        }
        Arc::new(Self {
            base: LocalSource::new(),
            mode: AtomicU8::new(0),
            blocked: AtomicBool::new(false),
            entered: AtomicUsize::new(0),
            usb_calls: AtomicUsize::new(0),
            gate: Semaphore::new(0),
            usb: StdMutex::new(Vec::new()),
            paired: StdMutex::new(Vec::new()),
            errors: StdMutex::new(streams),
            originals: StdMutex::new(Vec::new()),
            routes: StdMutex::new(Vec::new()),
        })
    }

    async fn original_error(&self, source: u8) -> anyhow::Error {
        let mut stream = self.errors.lock().unwrap().pop_front().unwrap();
        let error: anyhow::Error = stream.read_exact(&mut [0; 2]).await.unwrap_err().into();
        let address = error.downcast_ref::<io::Error>().unwrap() as *const io::Error as usize;
        self.originals.lock().unwrap().push((source, address));
        error
    }
}

impl Source for DiscoverySource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            self.routes
                .lock()
                .unwrap()
                .push((udid.into(), route.clone()));
            if udid != DEVICE {
                return Err(anyhow::anyhow!("unknown capacity target polled Source"));
            }
            self.base.connect(udid, route, mode).await
        })
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
                let mut reservation = owner.reserve_owned()?;
                let device = self.connect(udid, route.fixture_owned(), mode).await?;
                reservation.attach(device.native_handle().unwrap().clone())?;
                self.entered.fetch_add(1, Ordering::SeqCst);
                if self.blocked.load(Ordering::SeqCst) {
                    self.gate.acquire().await?.forget();
                }
                reservation.complete();
                Ok(device)
            },
            bank,
        )
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            self.usb_calls.fetch_add(1, Ordering::SeqCst);
            if self.mode.load(Ordering::SeqCst) == 1 {
                UsbSnapshot::ListError(self.original_error(0).await)
            } else {
                UsbSnapshot::Devices(self.usb.lock().unwrap().clone())
            }
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async {
            match self.mode.load(Ordering::SeqCst) {
                1 => Err(self.original_error(1).await),
                2 => Ok(vec![ios_core::device::PairedMobdev2Device {
                    udid: PAIRED_UNKNOWN.into(),
                    host: "x".repeat(4097),
                }]),
                _ => Ok(self.paired.lock().unwrap().clone()),
            }
        })
    }
}

fn manager(source: Arc<DiscoverySource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
    )
}

async fn start_manual(state: &TunnelAgentState) -> StatusCode {
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = reqwest::Client::builder()
        .no_proxy()
        .timeout(OBSERVE)
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .json(&json!({"udid": DEVICE, "connection_type": "wifi", "ip": "192.0.2.90"}))
        .send()
        .await
        .map(|response| response.status());
    server.abort();
    let _ = server.await;
    response.unwrap()
}

async fn finish_all(state: &TunnelAgentState, source: &DiscoverySource, expected: usize) {
    state.notify_shutdown();
    state.clear().await;
    let drained = state.drain_native(OBSERVE).await;
    let eof = recovered(&source.base, expected).await;
    let originals = source.base.handles.lock().unwrap().clone();
    assert!(
        drained.is_ok() && eof,
        "original workers joined and original native peers reached EOF"
    );
    assert!(originals
        .iter()
        .all(|handle| handle.shutdown_status().is_ok()));
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
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

fn assert_originals(error: &anyhow::Error, source: &DiscoverySource, offset: usize) {
    let originals = source.originals.lock().unwrap();
    assert_eq!(originals[offset].0, 0);
    assert_eq!(originals[offset + 1].0, 1);
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap() as *const io::Error as usize,
        originals[offset].1
    );
    let context = error.downcast_ref::<scan::SourceContext>().unwrap();
    assert_eq!(
        context
            .secondary(scan::Source::Paired)
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap() as *const io::Error as usize,
        originals[offset + 1].1
    );
    assert!(context.secondary(scan::Source::Usb).is_none());
    assert!(error.chain().count() <= 3);
    let safe = format!("{context:?} {context}");
    assert!(!safe.contains(DEVICE) && !safe.contains("192.0.2.90"));
}

#[tokio::test]
async fn intent_discovery_two_original_tcp_causes_preserve_wifi_recovery_and_native_owner() {
    let expected = baseline();
    let source = DiscoverySource::new(2).await;
    let state = manager(source.clone());
    let started = start_manual(&state).await;
    let old = source.base.handle();
    let old_join = old.shutdown(OBSERVE).await;
    source.mode.store(1, Ordering::SeqCst);
    let result = state.refresh_once().await;
    let fresh = source.base.handle();
    let live = fresh.is_alive();
    let routes = source.routes.lock().unwrap().clone();
    finish_all(&state, &source, expected).await;
    assert_eq!(started, StatusCode::OK);
    assert!(old_join.is_ok() && live && !Arc::ptr_eq(&old, &fresh));
    assert_eq!(
        routes,
        vec![(DEVICE.into(), Route::Wifi(Some("192.0.2.90".into()))); 2]
    );
    let error = result.unwrap_err();
    assert_originals(&error, &source, 0);
    assert!(state.inner.scan_outcomes.history().is_none());
}

#[tokio::test]
async fn intent_discovery_full_durable_gate_table_recovers_known_native_without_polling_unknowns() {
    let expected = baseline();
    let source = DiscoverySource::new(0).await;
    let state = manager(source.clone());
    let started = start_manual(&state).await;
    let old = source.base.handle();
    let old_join = old.shutdown(OBSERVE).await;
    for index in 0..4095 {
        state
            .stop_tunnel(&format!("intent-capacity-stopped-{index}"))
            .await
            .unwrap();
    }
    source
        .usb
        .lock()
        .unwrap()
        .extend((0..512).map(|index| source::UsbDevice {
            udid: format!("{USB_UNKNOWN}-{index}"),
            device_id: 8,
            usb: true,
        }));
    source.paired.lock().unwrap().extend((0..512).map(|index| {
        ios_core::device::PairedMobdev2Device {
            udid: format!("{PAIRED_UNKNOWN}-{index}"),
            host: "192.0.2.91".into(),
        }
    }));
    let result = state.refresh_once().await;
    let fresh = source.base.handle();
    let live = fresh.is_alive();
    let rows = state.inner.lifecycle.coordinator_count().await;
    let routes = source.routes.lock().unwrap().clone();
    finish_all(&state, &source, expected).await;
    assert_eq!(started, StatusCode::OK);
    assert!(old_join.is_ok() && live && !Arc::ptr_eq(&old, &fresh));
    assert_eq!(rows, 4096);
    assert_eq!(
        routes,
        vec![(DEVICE.into(), Route::Wifi(Some("192.0.2.90".into()))); 2]
    );
    let error = result.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(
        error.chain().count() <= 2,
        "1024 unknown keys retain one refusal"
    );
}

#[tokio::test]
async fn intent_discovery_two_tcp_causes_and_actual_completion_elapsed_keep_all_originals() {
    let expected = baseline();
    let source = DiscoverySource::new(2).await;
    let mut state = manager(source.clone());
    state.inner.get_mut().unwrap().scan_timeout = Duration::from_millis(250);
    let started = start_manual(&state).await;
    let old_join = source.base.handle().shutdown(OBSERVE).await;
    source.mode.store(1, Ordering::SeqCst);
    source.blocked.store(true, Ordering::SeqCst);
    let result = state.refresh_once().await;
    let calls = source.entered.load(Ordering::SeqCst);
    finish_all(&state, &source, expected).await;
    assert_eq!(started, StatusCode::OK);
    assert!(old_join.is_ok());
    assert_eq!(
        calls, 2,
        "native recovery owner existed before total cancellation"
    );
    let error = result.unwrap_err();
    assert_originals(&error, &source, 0);
    assert!(error
        .downcast_ref::<scan::SourceContext>()
        .unwrap()
        .completion()
        .unwrap()
        .is::<tokio::time::error::Elapsed>());
    assert!(state.inner.scan_outcomes.history().is_none());
}

#[tokio::test]
async fn intent_discovery_eight_abandoned_two_source_errors_keep_originals_and_reject_ninth() {
    let expected = baseline();
    let source = DiscoverySource::new(16).await;
    let state = manager(source.clone());
    let started = start_manual(&state).await;
    let old_join = source.base.handle().shutdown(OBSERVE).await;
    source.mode.store(1, Ordering::SeqCst);
    source.blocked.store(true, Ordering::SeqCst);
    let mut progress = Vec::new();
    let mut eof = Vec::new();
    for count in 1..=8 {
        let mut scan = Box::pin(state.refresh_once());
        progress.push(
            drive_until(&mut scan, || {
                source.entered.load(Ordering::SeqCst) == count + 1
            })
            .await,
        );
        drop(scan);
        let drained = state.drain_native(OBSERVE).await;
        eof.push(drained.is_ok() && recovered(&source.base, expected).await);
    }
    let before = source.usb_calls.load(Ordering::SeqCst);
    let ninth = state.refresh_once().await;
    let after = source.usb_calls.load(Ordering::SeqCst);
    let history = state
        .inner
        .scan_outcomes
        .history()
        .expect("abandoned scan history");
    finish_all(&state, &source, expected).await;
    assert_eq!(started, StatusCode::OK);
    assert!(
        old_join.is_ok() && progress.iter().all(|value| *value) && eof.iter().all(|value| *value)
    );
    assert_eq!(history.len(), 8);
    assert_eq!(
        before, after,
        "full outcome slots reject before either source is polled"
    );
    assert_eq!(
        crate::output::operation_error_class(ninth.unwrap_err().as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    for (index, abandoned) in history.iter().enumerate() {
        assert_originals(abandoned.original(), &source, index * 2);
    }
}

#[tokio::test]
async fn intent_discovery_incomplete_paired_metadata_is_retained_before_owned_candidate_drop() {
    let expected = baseline();
    let source = DiscoverySource::new(0).await;
    let state = manager(source.clone());
    let started = start_manual(&state).await;
    let old_join = source.base.handle().shutdown(OBSERVE).await;
    for index in 0..4095 {
        state
            .stop_tunnel(&format!("intent-incomplete-stopped-{index}"))
            .await
            .unwrap();
    }
    source.mode.store(2, Ordering::SeqCst);
    source.blocked.store(true, Ordering::SeqCst);
    source.usb.lock().unwrap().push(source::UsbDevice {
        udid: USB_UNKNOWN.into(),
        device_id: 8,
        usb: true,
    });
    let mut scan = Box::pin(state.refresh_once());
    let progress = drive_until(&mut scan, || source.entered.load(Ordering::SeqCst) == 2).await;
    drop(scan);
    let history = state
        .inner
        .scan_outcomes
        .history()
        .expect("abandoned scan history");
    let routes = source.routes.lock().unwrap().clone();
    finish_all(&state, &source, expected).await;
    assert_eq!(started, StatusCode::OK);
    assert!(old_join.is_ok() && progress);
    assert_eq!(routes.len(), 2);
    assert!(routes.iter().all(|(udid, _)| udid == DEVICE));
    assert_eq!(history.len(), 1);
    let error = history[0].original();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(error.to_string().contains("additional source"));
    let context = error.downcast_ref::<scan::SourceContext>().unwrap();
    assert!(
        context.secondary(scan::Source::Paired).is_none(),
        "paired metadata remains the primary"
    );
    assert_eq!(
        crate::output::operation_error_class(
            context
                .secondary(scan::Source::Coordinator)
                .unwrap()
                .as_ref()
        ),
        ios_core::ErrorClass::ResourceLimit
    );
}

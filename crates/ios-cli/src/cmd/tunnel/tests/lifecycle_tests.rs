use super::*;
use crate::access::test_support::JsonRequest;
use futures_util::future::BoxFuture;
use ios_core::device::PairedMobdev2Device;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;
use tokio::sync::Semaphore;

const A: &str = "synthetic-device-a";
const B: &str = "synthetic-device-b";
const LOCAL_BUDGET: Duration = Duration::from_secs(1);

mod backoff_tests;
mod discovery_tests;
mod identity_tests;
mod liveness_tests;
mod parallel_tests;
mod publication_tests;
mod retention_tests;

enum Behavior {
    Success,
    Failure(std::io::ErrorKind),
    Pending,
    Held(Arc<Semaphore>),
}

struct OwnedDevice {
    record: TunnelRecord,
    alive: Arc<AtomicBool>,
    drops: Arc<AtomicUsize>,
    id: u32,
}

impl source::FakeDevice for OwnedDevice {
    fn record(&self) -> TunnelRecord {
        self.record.clone()
    }
    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
    fn device_id(&self) -> u32 {
        self.id
    }
}

impl Drop for OwnedDevice {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeSource {
    scripts: StdMutex<HashMap<String, VecDeque<Behavior>>>,
    calls: StdMutex<Vec<(String, Route)>>,
    changed: Notify,
    active: AtomicUsize,
    peak: AtomicUsize,
    exited: AtomicUsize,
    drops: Arc<AtomicUsize>,
    alive: Arc<AtomicBool>,
    usb: StdMutex<Vec<String>>,
    usb_details: StdMutex<Option<Vec<source::UsbDevice>>>,
    connected_ids: StdMutex<HashMap<String, u32>>,
    usb_gate: StdMutex<Option<Arc<Semaphore>>>,
    usb_calls: AtomicUsize,
    usb_active: AtomicUsize,
    usb_exited: AtomicUsize,
    paired_calls: AtomicUsize,
    paired_active: AtomicUsize,
    paired_exited: AtomicUsize,
    paired_gate: StdMutex<Option<Arc<Semaphore>>>,
    list_failure: AtomicBool,
    paired_failure: AtomicBool,
    paired: StdMutex<Vec<PairedMobdev2Device>>,
}

struct Active<'a>(&'a FakeSource);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        self.0.exited.fetch_add(1, Ordering::SeqCst);
    }
}

struct Probe<'a> {
    active: &'a AtomicUsize,
    exited: &'a AtomicUsize,
}

impl Drop for Probe<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.exited.fetch_add(1, Ordering::SeqCst);
    }
}

impl FakeSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            scripts: StdMutex::new(HashMap::new()),
            calls: StdMutex::new(Vec::new()),
            changed: Notify::new(),
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            exited: AtomicUsize::new(0),
            drops: Arc::new(AtomicUsize::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
            usb: StdMutex::new(Vec::new()),
            usb_details: StdMutex::new(None),
            connected_ids: StdMutex::new(HashMap::new()),
            usb_gate: StdMutex::new(None),
            usb_calls: AtomicUsize::new(0),
            usb_active: AtomicUsize::new(0),
            usb_exited: AtomicUsize::new(0),
            paired_calls: AtomicUsize::new(0),
            paired_active: AtomicUsize::new(0),
            paired_exited: AtomicUsize::new(0),
            paired_gate: StdMutex::new(None),
            list_failure: AtomicBool::new(false),
            paired_failure: AtomicBool::new(false),
            paired: StdMutex::new(Vec::new()),
        })
    }

    fn script(&self, udid: &str, scripts: impl IntoIterator<Item = Behavior>) {
        self.scripts
            .lock()
            .unwrap()
            .insert(udid.into(), scripts.into_iter().collect());
    }

    fn count(&self, udid: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| id == udid)
            .count()
    }

    async fn entered(&self, udid: &str, count: usize) {
        tokio::time::timeout(LOCAL_BUDGET, async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if self.count(udid) >= count {
                    break;
                }
                notified.await;
            }
        })
        .await
        .unwrap();
    }
}

impl Source for FakeSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        _mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let behavior = self
                .scripts
                .lock()
                .unwrap()
                .get_mut(udid)
                .and_then(VecDeque::pop_front)
                .unwrap_or(Behavior::Success);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let _active = Active(self);
            self.calls
                .lock()
                .unwrap()
                .push((udid.into(), route.clone()));
            self.changed.notify_waiters();
            match behavior {
                Behavior::Success => {}
                Behavior::Failure(kind) => {
                    return Err(std::io::Error::new(
                        kind,
                        "synthetic-private-key-token-device-data",
                    )
                    .into())
                }
                Behavior::Pending => std::future::pending::<()>().await,
                Behavior::Held(gate) => gate.acquire().await.unwrap().forget(),
            }
            let mut record = fake_record(12345);
            record.udid = udid.into();
            record.interface = format!("fake-{udid}");
            // A successful legacy fake USB connection represents an attached
            // device. Explicit metadata fixtures control their own snapshots.
            if matches!(route, Route::Usbmux | Route::Usb(_))
                && self.usb_details.lock().unwrap().is_none()
            {
                let mut attached = self.usb.lock().unwrap();
                if !attached.iter().any(|id| id == udid) {
                    attached.push(udid.into());
                }
            }
            Ok(ManagedDevice::Fake(Box::new(OwnedDevice {
                record,
                alive: self.alive.clone(),
                drops: self.drops.clone(),
                id: if matches!(route, Route::Wifi(_) | Route::Mobdev2(_)) {
                    0
                } else {
                    self.connected_ids
                        .lock()
                        .unwrap()
                        .get(udid)
                        .copied()
                        .unwrap_or(1)
                },
            })))
        })
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            self.usb_calls.fetch_add(1, Ordering::SeqCst);
            self.usb_active.fetch_add(1, Ordering::SeqCst);
            let _probe = Probe {
                active: &self.usb_active,
                exited: &self.usb_exited,
            };
            let devices = self.usb_details.lock().unwrap().clone().unwrap_or_else(|| {
                self.usb
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|udid| source::UsbDevice {
                        udid: udid.clone(),
                        device_id: 1,
                        usb: true,
                    })
                    .collect()
            });
            let gate = self.usb_gate.lock().unwrap().clone();
            let failed = self.list_failure.load(Ordering::SeqCst);
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            if failed {
                return UsbSnapshot::ListError(
                    std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "synthetic-private-list-input",
                    )
                    .into(),
                );
            }
            UsbSnapshot::Devices(devices)
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        Box::pin(async {
            self.paired_calls.fetch_add(1, Ordering::SeqCst);
            self.paired_active.fetch_add(1, Ordering::SeqCst);
            let _probe = Probe {
                active: &self.paired_active,
                exited: &self.paired_exited,
            };
            let failed = self.paired_failure.load(Ordering::SeqCst);
            let devices = self.paired.lock().unwrap().clone();
            let gate = self.paired_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            if failed {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic-private-paired-discovery",
                )
                .into());
            }
            Ok(devices)
        })
    }
}

fn state(source: Arc<FakeSource>, timeout: Duration) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        timeout,
    )
}

async fn start(state: TunnelAgentState, udid: &str) -> Result<TunnelRecord> {
    state
        .ensure_tunnel(udid, None, Some(StartTunnelConnectionType::Usbmux))
        .await
}

fn spawn_start(
    state: &TunnelAgentState,
    udid: &str,
) -> tokio::task::JoinHandle<Result<TunnelRecord>> {
    let state = state.clone();
    let udid = udid.to_owned();
    tokio::spawn(async move { start(state, &udid).await })
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(LOCAL_BUDGET)
        .build()
        .unwrap()
}

#[tokio::test]
async fn lifecycle_tunnel_http_stalled_a_does_not_delay_b_list_get_stop() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), B).await.unwrap();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let a_base = base.clone();
    let a = tokio::spawn(async move {
        client()
            .post(format!("{a_base}/start-tunnel"))
            .json(&json!({"udid":A,"connection_type":"usbmux"}))
            .send()
            .await
    });
    source.entered(A, 1).await;
    let before = Instant::now();
    assert_eq!(
        client()
            .get(format!("{base}/tunnels"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client()
            .get(format!("{base}/tunnel/{B}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client()
            .delete(format!("{base}/tunnel/{B}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(before.elapsed() < LOCAL_BUDGET);
    assert_eq!(source.active.load(Ordering::SeqCst), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        client()
            .delete(format!("{base}/tunnel/{A}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let response = a.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-ios-error-class"], "cancelled");
    assert!(state.list_records().await.is_empty());
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    server.abort();
    server.await.unwrap_err();
}

#[tokio::test]
async fn lifecycle_tunnel_same_device_requests_publish_one_owned_tunnel() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let first = spawn_start(&state, A);
    source.entered(A, 1).await;
    let second = spawn_start(&state, A);
    let third = spawn_start(&state, A);
    tokio::task::yield_now().await;
    assert_eq!(source.count(A), 1);
    gate.add_permits(1);
    let first = first.await.unwrap().unwrap();
    assert_eq!(second.await.unwrap().unwrap(), first);
    assert_eq!(third.await.unwrap().unwrap(), first);
    assert_eq!(source.count(A), 1);
    assert_eq!(state.list_records().await.len(), 1);
    assert!(state.remove_tunnel(A).await);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_stop_cancels_active_and_queued_generation_then_allows_new_start() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending, Behavior::Success]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let first = spawn_start(&state, A);
    source.entered(A, 1).await;
    let queued = state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux));
    tokio::pin!(queued);
    assert!(futures_util::poll!(&mut queued).is_pending());
    assert!(!state.remove_tunnel(A).await);
    assert!(first
        .await
        .unwrap()
        .unwrap_err()
        .is::<ios_core::OperationCancelled>());
    assert!(queued
        .await
        .unwrap_err()
        .is::<ios_core::OperationCancelled>());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert!(state.get_record(A).await.is_none());
    start(state.clone(), A).await.unwrap();
    assert_eq!(source.count(A), 2);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_global_connect_limit_and_clear_release_all_owned_futures() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut jobs = Vec::new();
    for i in 0..8 {
        let udid = format!("synthetic-limit-{i}");
        source.script(&udid, [Behavior::Pending]);
        let state = state.clone();
        let mut future = Box::pin(async move { start(state, &udid).await });
        assert!(futures_util::poll!(&mut future).is_pending());
        jobs.push(future);
    }
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.active.load(Ordering::SeqCst) < lifecycle::MAX_CONNECTS {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        source.active.load(Ordering::SeqCst),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(source.peak.load(Ordering::SeqCst), lifecycle::MAX_CONNECTS);
    state.clear().await;
    for job in jobs {
        assert!(job.await.unwrap_err().is::<ios_core::OperationCancelled>());
    }
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_queue_uses_original_whole_connect_deadline() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(80));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let serial = ticket.serial().await;
    let error = state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux))
        .await
        .unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    drop(serial);
    start(state.clone(), A).await.unwrap();
    assert_eq!(source.count(A), 1);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_http_preserves_native_failure_without_private_inputs() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Failure(std::io::ErrorKind::PermissionDenied)]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let (port, server) = spawn_test_router(agent_router(state)).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .json(&json!({"udid":A,"connection_type":"usbmux"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-ios-error-class"], "permission_denied");
    let body = response.text().await.unwrap();
    assert!(!body.contains("synthetic-private"));
    assert!(!body.contains(A));
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
    server.abort();
    server.await.unwrap_err();
}

#[tokio::test]
async fn lifecycle_tunnel_requested_routes_and_existing_trust_fallback_are_unchanged() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    for (requested, route) in [
        (StartTunnelConnectionType::Usbmux, Route::Usbmux),
        (
            StartTunnelConnectionType::Usb,
            Route::Usb(Some("192.0.2.2".into())),
        ),
        (
            StartTunnelConnectionType::Wifi,
            Route::Wifi(Some("192.0.2.2".into())),
        ),
    ] {
        state
            .ensure_tunnel(A, Some("192.0.2.2"), Some(requested))
            .await
            .unwrap();
        assert_eq!(source.calls.lock().unwrap().last().unwrap().1, route);
        assert!(state.remove_tunnel(A).await);
    }
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
            Behavior::Success,
        ],
    );
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: A.into(),
        host: "192.0.2.2".into(),
    });
    state
        .ensure_tunnel(A, Some("192.0.2.2"), None)
        .await
        .unwrap();
    assert_eq!(
        source.calls.lock().unwrap().last().unwrap().1,
        Route::Mobdev2("192.0.2.2".into())
    );
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn lifecycle_tunnel_scan_and_explicit_start_share_same_device_owner() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan_state = state.clone();
    let scan = tokio::spawn(async move { scan_state.refresh_once().await });
    source.entered(A, 1).await;
    let manual = spawn_start(&state, A);
    gate.add_permits(1);
    scan.await.unwrap().unwrap();
    manual.await.unwrap().unwrap();
    assert_eq!(source.count(A), 1);
    assert!(state.is_ready().await);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_stop_invalidates_device_queued_behind_stalled_scan() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![
        A.into(),
        "synthetic-c".into(),
        "synthetic-d".into(),
        "synthetic-e".into(),
        B.into(),
    ];
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    for udid in ["synthetic-c", "synthetic-d", "synthetic-e"] {
        source.script(udid, [Behavior::Held(gate.clone())]);
    }
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan_state = state.clone();
    let scan = tokio::spawn(async move { scan_state.refresh_once().await });
    source.entered(A, 1).await;
    for udid in ["synthetic-c", "synthetic-d", "synthetic-e"] {
        source.entered(udid, 1).await;
    }
    assert!(!tokio::time::timeout(LOCAL_BUDGET, state.remove_tunnel(B))
        .await
        .unwrap());
    gate.add_permits(4);
    scan.await.unwrap().unwrap();
    assert_eq!(source.count(B), 0);
    assert!(state.get_record(B).await.is_none());
    assert!(state.get_record(A).await.is_some());
    assert!(!state.inner.failed_attempts.read().await.contains_key(B));
    state.clear().await;
}

#[tokio::test]
async fn lifecycle_tunnel_stop_during_discovery_rejects_unpublished_snapshot() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(gate.clone());
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan = state.refresh_once();
    tokio::pin!(scan);
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert!(!state.remove_tunnel(A).await);
    gate.add_permits(1);
    scan.await.unwrap();
    assert_eq!(source.count(A), 0);
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_scan_backoff_and_disconnect_release_remain_observable() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 1);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut(A)
        .unwrap()
        .last_attempt -= Duration::from_secs(31);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2);
    assert!(!state.inner.failed_attempts.read().await.contains_key(A));
    source.usb.lock().unwrap().clear();
    state.refresh_once().await.unwrap();
    assert!(state.get_record(A).await.is_none());
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_stale_failed_scan_does_not_remove_new_publication() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), A).await.unwrap();
    source.list_failure.store(true, Ordering::SeqCst);
    *source.usb_gate.lock().unwrap() = Some(gate.clone());
    let scan = state.refresh_once();
    tokio::pin!(scan);
    assert!(futures_util::poll!(&mut scan).is_pending());
    source.list_failure.store(false, Ordering::SeqCst);
    *source.usb_gate.lock().unwrap() = None;
    start(state.clone(), B).await.unwrap();
    gate.add_permits(1);
    let error = scan.await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert!(state.get_record(B).await.is_some());
    assert!(state.get_record(A).await.is_none());
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_native_failure_and_unmatched_fallback_retain_original_io() {
    let source = FakeSource::new();
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::PermissionDenied),
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let direct = start(state.clone(), A).await.unwrap_err();
    assert_eq!(
        direct.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let fallback = state.ensure_tunnel(A, None, None).await.unwrap_err();
    assert_eq!(
        fallback.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::ConnectionReset
    );
    let safe = crate::output::error_message(&fallback);
    assert!(!safe.contains("synthetic-private"));
    assert!(!safe.contains(A));
    assert_eq!(source.count(A), 2);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_caller_drop_releases_completed_device_waiting_for_publication() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut request =
        Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut request).is_pending());
    let table = state.inner.tunnels.write().await;
    gate.add_permits(1);
    assert!(futures_util::poll!(&mut request).is_pending());
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    drop(request);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    drop(table);
    assert!(state.get_record(A).await.is_none());
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_dead_record_is_pruned_and_owned_device_released() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), A).await.unwrap();
    source.alive.store(false, Ordering::SeqCst);
    assert!(state.get_record(A).await.is_none());
    assert!(state.list_records().await.is_empty());
    assert!(!state.remove_tunnel(A).await);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

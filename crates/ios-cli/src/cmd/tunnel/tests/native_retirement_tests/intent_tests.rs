//! Production HTTP/scan ownership with original userspace TUN workers.
//! Device/discovery metadata use the private Source seam; no OS/device required.
use super::*;
use crate::access::test_support::{
    grant, JsonRequest, PolicyFixture, DEVICE as AUTH_DEVICE, REPLACEMENT, TOKEN,
};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;

const OTHER: &str = "native-manual-intent-b";
const HTTP_BUDGET: Duration = Duration::from_secs(1);

mod expiry_tests;

enum Fate {
    Success,
    Failure,
    Pending,
}

struct IntentDevice {
    native: Arc<TunnelHandle>,
    udid: String,
}
impl source::FakeDevice for IntentDevice {
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

struct IntentSource {
    a: Arc<LocalSource>,
    b: Arc<LocalSource>,
    fates: StdMutex<VecDeque<Fate>>,
    routes: StdMutex<Vec<(String, Route)>>,
    usb_failure: AtomicBool,
    attached: AtomicBool,
    owned_entered: AtomicUsize,
}

impl IntentSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            a: LocalSource::new(),
            b: LocalSource::new(),
            fates: StdMutex::new(VecDeque::new()),
            routes: StdMutex::new(Vec::new()),
            usb_failure: AtomicBool::new(false),
            attached: AtomicBool::new(true),
            owned_entered: AtomicUsize::new(0),
        })
    }
    fn script(&self, fate: Fate) {
        self.fates.lock().unwrap().push_back(fate);
    }
    async fn entered(&self, calls: usize) -> bool {
        tokio::time::timeout(OBSERVE, async {
            while self.owned_entered.load(Ordering::SeqCst) < calls {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok()
    }
}

impl Source for IntentSource {
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
            let base = if udid == AUTH_DEVICE {
                &self.a
            } else {
                &self.b
            };
            let device = base.connect(udid, route, mode).await?;
            Ok(ManagedDevice::Fake(Box::new(IntentDevice {
                native: device.native_handle().unwrap().clone(),
                udid: udid.into(),
            })))
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
                let mut reservation = owner.reserve()?;
                let device = self.connect(udid, route.fixture_owned(), mode).await?;
                reservation.attach(device.native_handle().unwrap().clone())?;
                if udid == AUTH_DEVICE {
                    self.owned_entered.fetch_add(1, Ordering::SeqCst);
                }
                let fate = if udid == AUTH_DEVICE {
                    self.fates
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or(Fate::Success)
                } else {
                    Fate::Success
                };
                match fate {
                    Fate::Pending => std::future::pending::<()>().await,
                    Fate::Failure => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "synthetic-native-manual-primary",
                        )
                        .into())
                    }
                    Fate::Success => {}
                }
                reservation.complete();
                Ok(device)
            },
            bank,
        )
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            if self.usb_failure.load(Ordering::SeqCst) {
                return UsbSnapshot::ListError(
                    io::Error::new(
                        io::ErrorKind::ConnectionRefused,
                        "synthetic-private-usb-offline",
                    )
                    .into(),
                );
            }
            UsbSnapshot::Devices(if self.attached.load(Ordering::SeqCst) {
                [AUTH_DEVICE, OTHER]
                    .into_iter()
                    .map(|udid| source::UsbDevice {
                        udid: udid.into(),
                        device_id: 7,
                        usb: true,
                    })
                    .collect()
            } else {
                Vec::new()
            })
        })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn manager(source: Arc<IntentSource>, access: AccessControl) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        Upstreams::default(),
        source,
        OBSERVE,
    )
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(HTTP_BUDGET)
        .build()
        .unwrap()
}

async fn post(
    client: &reqwest::Client,
    base: &str,
    udid: &str,
    token: Option<&str>,
    host: Option<&str>,
) -> Result<StatusCode, reqwest::Error> {
    let mut body = json!({"udid":udid,"connection_type":"wifi"});
    if let Some(host) = host {
        body["ip"] = json!(host);
    }
    let mut request = client.post(format!("{base}/start-tunnel")).json(&body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    Ok(request.send().await?.status())
}

async fn cleanup(state: &TunnelAgentState, source: &IntentSource, expected: usize) {
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(OBSERVE).await.unwrap();
    assert!(recovered(&source.a, expected).await);
    assert!(recovered(&source.b, expected).await);
    assert!(state.inner.native.snapshot().closed_and_empty());
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
async fn intent_native_http_route_recovery_retains_actual_selector_and_joins_each_original() {
    let expected = baseline();
    let source = IntentSource::new();
    let state = manager(source.clone(), AccessControl::trusted_test_local());
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let started = post(&client, &base, AUTH_DEVICE, None, Some("192.0.2.88")).await;
    let original = source.a.handle();
    let old_record = state.get_record(AUTH_DEVICE).await.unwrap();
    original.request_stop();
    source.usb_failure.store(true, Ordering::SeqCst);
    let scan = state.refresh_once().await;
    let fresh = source.a.handle();
    let fresh_record = state.get_record(AUTH_DEVICE).await.unwrap();
    let original_drained = state.drain_native(OBSERVE).await;
    let original_join = original.shutdown(OBSERVE).await;
    let still_live = fresh.is_alive();
    let routes = source.routes.lock().unwrap().clone();
    let stop = client
        .delete(format!("{base}/tunnel/{AUTH_DEVICE}"))
        .send()
        .await
        .map(|response| response.status());
    source.usb_failure.store(false, Ordering::SeqCst);
    source.attached.store(false, Ordering::SeqCst);
    state.refresh_once().await.unwrap();
    source.attached.store(true, Ordering::SeqCst);
    state.refresh_once().await.unwrap();
    let after_stop = source.a.connect_calls.load(Ordering::SeqCst);
    drop(client);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source, expected).await;
    assert_eq!(started.unwrap(), StatusCode::OK);
    assert!(scan.is_err());
    assert!(original_drained.is_ok() && original_join.is_ok());
    assert!(!Arc::ptr_eq(&original, &fresh));
    assert!(
        still_live,
        "a selector change must not rotate the newly published native owner"
    );
    assert_ne!(
        old_record.unwrap().userspace_port,
        fresh_record.unwrap().userspace_port
    );
    assert_eq!(
        routes[..2],
        [
            (AUTH_DEVICE.into(), Route::Wifi(Some("192.0.2.88".into()))),
            (AUTH_DEVICE.into(), Route::Wifi(Some("192.0.2.88".into())))
        ]
    );
    assert_eq!(stop.unwrap(), StatusCode::OK);
    assert_eq!(after_stop, 2);
}

#[tokio::test]
async fn intent_native_a_pending_b_real_http_query_and_stop_remain_isolated() {
    let expected = baseline();
    let source = IntentSource::new();
    let state = manager(source.clone(), AccessControl::trusted_test_local());
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let a_started = post(&client, &base, AUTH_DEVICE, None, None).await;
    let b_started = post(&client, &base, OTHER, None, None).await;
    let a_original = source.a.handle();
    let b_original = source.b.handle();
    a_original.request_stop();
    source.script(Fate::Pending);
    let scan_state = state.clone();
    let scan = tokio::spawn(async move { scan_state.refresh_once().await });
    let entered = source.entered(2).await;
    let query_start = Instant::now();
    let query = client
        .get(format!("{base}/tunnel/{OTHER}"))
        .send()
        .await
        .map(|response| response.status());
    let query_time = query_start.elapsed();
    let stop_start = Instant::now();
    let b_stop = client
        .delete(format!("{base}/tunnel/{OTHER}"))
        .send()
        .await
        .map(|response| response.status());
    let stop_time = stop_start.elapsed();
    let b_join = b_original.shutdown(OBSERVE).await;
    let a_stop = client
        .delete(format!("{base}/tunnel/{AUTH_DEVICE}"))
        .send()
        .await
        .map(|response| response.status());
    let ended = tokio::time::timeout(OBSERVE, scan).await;
    state.refresh_once().await.unwrap();
    let calls = (
        source.a.connect_calls.load(Ordering::SeqCst),
        source.b.connect_calls.load(Ordering::SeqCst),
    );
    drop(client);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source, expected).await;
    assert_eq!(
        (a_started.unwrap(), b_started.unwrap()),
        (StatusCode::OK, StatusCode::OK)
    );
    assert!(entered);
    assert_eq!(query.unwrap(), StatusCode::OK);
    assert!(query_time < HTTP_BUDGET);
    assert_eq!(b_stop.unwrap(), StatusCode::OK);
    assert!(stop_time < HTTP_BUDGET);
    assert!(b_join.is_ok());
    assert_eq!(a_stop.unwrap(), StatusCode::NOT_FOUND);
    ended.unwrap().unwrap().unwrap();
    assert_eq!(calls, (2, 1));
}

#[tokio::test]
async fn intent_native_revoked_original_authority_retires_native_without_any_grant_fallback() {
    let expected = baseline();
    let fixture = PolicyFixture::new(vec![
        grant(
            TOKEN,
            vec![
                Permission::ManageServer,
                Permission::ManageTunnels,
                Permission::List,
            ],
        ),
        grant(REPLACEMENT, vec![Permission::Connect]),
    ]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let source = IntentSource::new();
    let state = manager(source.clone(), access);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let started = post(&client, &base, AUTH_DEVICE, Some(TOKEN), None).await;
    let original = source.a.handle();
    fixture.replace(vec![grant(
        REPLACEMENT,
        vec![Permission::Connect, Permission::List],
    )]);
    let reloaded = client
        .post(format!("{base}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .map(|response| response.status());
    state.refresh_once().await.unwrap();
    state.refresh_once().await.unwrap();
    let absent = state.get_record(AUTH_DEVICE).await.unwrap().is_none();
    let stopped = matches!(
        state
            .inner
            .lifecycle
            .ticket(AUTH_DEVICE)
            .await
            .unwrap()
            .desired()
            .desired,
        intent::Desired::Stopped
    );
    let drained = state.drain_native(OBSERVE).await;
    let joined = original.shutdown(OBSERVE).await;
    let count = source.a.connect_calls.load(Ordering::SeqCst);
    drop(client);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source, expected).await;
    assert_eq!(
        (started.unwrap(), reloaded.unwrap()),
        (StatusCode::OK, StatusCode::OK)
    );
    assert!(absent && stopped && drained.is_ok() && joined.is_ok());
    assert_eq!(count, 1);
}

#[tokio::test]
async fn intent_native_manual_success_discards_old_deferred_failure_without_retiring_new_owner() {
    let expected = baseline();
    let source = IntentSource::new();
    let state = manager(source.clone(), AccessControl::trusted_test_local());
    source.script(Fate::Failure);
    let old_ticket = state
        .inner
        .lifecycle
        .ticket(AUTH_DEVICE)
        .await
        .unwrap()
        .fenced();
    let deferred = state
        .refresh_connect_deferred(AUTH_DEVICE, Route::Wifi(None), &old_ticket, None)
        .await
        .unwrap();
    let failed_original = source.a.handle();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let started = post(&client, &base, AUTH_DEVICE, None, Some("192.0.2.99")).await;
    let fresh = source.a.handle();
    let published = state.inner.tunnels.read().await[AUTH_DEVICE]
        .publication
        .clone();
    state
        .flush_failure_intents(vec![FailureIntent {
            ticket: old_ticket.clone(),
            failed: deferred,
        }])
        .await
        .unwrap();
    let failed_empty = state.inner.failed_attempts.read().await.is_empty();
    let retained = state.inner.tunnels.read().await[AUTH_DEVICE]
        .publication
        .clone();
    let selector = state
        .inner
        .lifecycle
        .ticket(AUTH_DEVICE)
        .await
        .unwrap()
        .desired();
    let drained = state.drain_native(OBSERVE).await;
    let old_joined = failed_original.shutdown(OBSERVE).await;
    let still_alive = fresh.is_alive();
    let failure_slots = state.inner.failure_slots.available_permits();
    drop(client);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source, expected).await;
    assert_eq!(started.unwrap(), StatusCode::OK);
    assert!(!old_ticket.is_current());
    assert!(failed_empty && published.ptr_eq(&retained));
    assert!(
        matches!(selector.desired, intent::Desired::Manual { route, .. } if matches!(&route.route, intent::RouteIntent::Wifi(Some(host)) if host == "192.0.2.99"))
    );
    assert!(drained.is_ok() && old_joined.is_ok() && still_alive);
    assert_eq!(failure_slots, MAX_FAILURE_RECORDS);
}

#[tokio::test]
async fn intent_native_manual_retry_exhaustion_survives_partial_discovery_without_source_poll() {
    let expected = baseline();
    let source = IntentSource::new();
    let policy = reconnect::RetryPolicy::new(
        Duration::from_millis(10),
        Duration::from_millis(40),
        std::num::NonZeroU32::new(1),
        None,
    )
    .unwrap();
    let state = TunnelAgentState::with_source_and_retry(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        OBSERVE,
        policy,
    )
    .unwrap();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let started = post(&client, &base, AUTH_DEVICE, None, Some("192.0.2.77")).await;
    source.a.handle().request_stop();
    source.script(Fate::Failure);
    state.refresh_once().await.unwrap();
    let exhausted = state.inner.retry.snapshot(AUTH_DEVICE);
    source.usb_failure.store(true, Ordering::SeqCst);
    let partial = state.refresh_once().await;
    let after = state.inner.retry.snapshot(AUTH_DEVICE);
    let calls = source.a.connect_calls.load(Ordering::SeqCst);
    let originals = source.a.handles.lock().unwrap().clone();
    drop(client);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source, expected).await;
    assert_eq!(started.unwrap(), StatusCode::OK);
    assert!(partial.is_err());
    let exhausted = exhausted.unwrap();
    let after = after.unwrap();
    assert_eq!(
        (exhausted.phase, after.phase),
        (reconnect::Phase::Exhausted, reconnect::Phase::Exhausted)
    );
    assert_eq!((after.attempts, after.failures), (1, 1));
    assert_eq!(calls, 2);
    assert!(originals
        .iter()
        .all(|original| original.shutdown_status().is_ok()));
}

#[tokio::test]
async fn intent_native_pending_recovery_reloaded_authority_cancels_and_joins_original_owner() {
    let expected = baseline();
    let fixture = PolicyFixture::new(vec![
        grant(
            TOKEN,
            vec![Permission::ManageServer, Permission::ManageTunnels],
        ),
        grant(REPLACEMENT, vec![Permission::Connect]),
    ]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let source = IntentSource::new();
    let state = manager(source.clone(), access);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = client();
    let started = post(&client, &base, AUTH_DEVICE, Some(TOKEN), None).await;
    source.a.handle().request_stop();
    source.script(Fate::Pending);
    let scan_state = state.clone();
    let scan = tokio::spawn(async move { scan_state.refresh_once().await });
    let entered = source.entered(2).await;
    let pending_original = source.a.handle();
    fixture.replace(vec![grant(REPLACEMENT, vec![Permission::Connect])]);
    let reload = client
        .post(format!("{base}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .map(|response| response.status());
    let ended = tokio::time::timeout(HTTP_BUDGET, scan).await;
    state.refresh_once().await.unwrap();
    let stopped = matches!(
        state
            .inner
            .lifecycle
            .ticket(AUTH_DEVICE)
            .await
            .unwrap()
            .desired()
            .desired,
        intent::Desired::Stopped
    );
    let calls = source.a.connect_calls.load(Ordering::SeqCst);
    drop(client);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source, expected).await;
    assert_eq!(
        (started.unwrap(), reload.unwrap()),
        (StatusCode::OK, StatusCode::OK)
    );
    assert!(entered && stopped);
    ended.unwrap().unwrap().unwrap();
    assert!(pending_original.shutdown_status().is_ok());
    assert_eq!(calls, 2);
}

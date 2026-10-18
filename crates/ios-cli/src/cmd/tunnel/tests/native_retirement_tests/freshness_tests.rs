//! Current RSD refresh with actual userspace workers and strict EOF/join proofs.
//! Only metadata/routing uses the cfgtest seam; every catalogue reads real H2/XPC.
use super::*;
use indexmap::IndexMap;
use ios_core::device::{ExistingRsdRoute, PreparedRsdCatalogue};
use ios_core::test_support::{decode_message, encode_message, flags, XpcMessage, H2_PREFACE};
use ios_core::XpcValue;
use std::collections::VecDeque;
use std::error::Error;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::atomic::AtomicBool;

const UID: &str = crate::access::test_support::DEVICE;
const SERVICE: &str = "freshness.local.service";

fn directory(port: u16) -> IndexMap<String, XpcValue> {
    IndexMap::from([
        ("MessageType".into(), XpcValue::String("Handshake".into())),
        (
            "Properties".into(),
            XpcValue::Dictionary(IndexMap::from([(
                "UniqueDeviceID".into(),
                XpcValue::String(UID.into()),
            )])),
        ),
        (
            "Services".into(),
            XpcValue::Dictionary(IndexMap::from([(
                SERVICE.into(),
                XpcValue::Dictionary(IndexMap::from([
                    ("Port".into(), XpcValue::Uint64(u64::from(port))),
                    (
                        "Properties".into(),
                        XpcValue::Dictionary(IndexMap::from([(
                            "Features".into(),
                            XpcValue::Array(vec![XpcValue::String("current-feature".into())]),
                        )])),
                    ),
                ])),
            )])),
        ),
    ])
}
fn frame(kind: u8, flag: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    let mut wire = Vec::new();
    wire.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    wire.extend_from_slice(&[kind, flag]);
    wire.extend_from_slice(&stream.to_be_bytes());
    wire.extend_from_slice(body);
    wire
}
async fn read_frame(peer: &mut TcpStream) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let n = (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
    assert!(n <= 4096);
    let mut body = vec![0; n];
    peer.read_exact(&mut body).await.unwrap();
    (
        header[3],
        header[4],
        u32::from_be_bytes(header[5..].try_into().unwrap()),
        body,
    )
}
async fn producer(peer: &mut TcpStream, port: u16) {
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, H2_PREFACE);
    assert_eq!(read_frame(peer).await.0, 4);
    assert_eq!(read_frame(peer).await.0, 8);
    peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
        let incoming = read_frame(peer).await;
        assert_eq!((incoming.0, incoming.2), (kind, stream));
    }
    let ack = read_frame(peer).await;
    assert_eq!((ack.0, ack.1, ack.2), (4, 1, 0));
    let handshake = read_frame(peer).await;
    assert_eq!((handshake.0, handshake.2), (0, 1));
    let decoded = decode_message(bytes::Bytes::from(handshake.3)).unwrap();
    assert_eq!(
        decoded.body.unwrap().as_dict().unwrap()["MessageType"].as_str(),
        Some("Handshake")
    );
    let wire = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(directory(port))),
    })
    .unwrap();
    peer.write_all(&frame(0, 0, 1, &wire)).await.unwrap();
    let mut tail = Vec::new();
    peer.read_to_end(&mut tail).await.unwrap();
    while !tail.is_empty() {
        assert!(tail.len() >= 13);
        assert_eq!(tail[3], 8);
        let n = (usize::from(tail[0]) << 16) | (usize::from(tail[1]) << 8) | usize::from(tail[2]);
        assert_eq!(n, 4);
        tail.drain(..9 + n);
    }
}
async fn directories(ports: Vec<u16>) -> (ExistingRsdRoute, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
    let std::net::SocketAddr::V6(endpoint) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    let task = tokio::spawn(async move {
        for port in ports {
            let (mut peer, _) = listener.accept().await.unwrap();
            producer(&mut peer, port).await;
        }
    });
    (ExistingRsdRoute::direct(endpoint).unwrap(), task)
}
struct FreshDevice {
    native: Arc<TunnelHandle>,
    route: ExistingRsdRoute,
    catalogue: Option<Arc<PreparedRsdCatalogue>>,
}
impl source::FakeDevice for FreshDevice {
    fn record(&self) -> TunnelRecord {
        let mut row = fake_record(self.native.userspace_port.unwrap());
        row.udid = UID.into();
        row
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
    fn freshness_route(&self) -> Result<Option<ExistingRsdRoute>> {
        Ok(Some(self.route))
    }
    fn freshness_catalogue(&self) -> Option<&Arc<PreparedRsdCatalogue>> {
        self.catalogue.as_ref()
    }
    fn swap_freshness_catalogue(
        &mut self,
        runtime: &Arc<TunnelHandle>,
        proposed: Option<Arc<PreparedRsdCatalogue>>,
    ) -> Result<Option<Arc<PreparedRsdCatalogue>>> {
        anyhow::ensure!(Arc::ptr_eq(runtime, &self.native), lifecycle::cancelled());
        Ok(std::mem::replace(&mut self.catalogue, proposed))
    }
}
#[derive(Debug)]
struct Original {
    marker: Arc<usize>,
}
impl std::fmt::Display for Original {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private original refresh cause")
    }
}
impl Error for Original {}

enum Refresh {
    Directory,
    Failure {
        marker: Arc<usize>,
        delay: Duration,
        zero: bool,
    },
    Pending,
}
struct FreshSource {
    base: Arc<LocalSource>,
    route: ExistingRsdRoute,
    fates: StdMutex<VecDeque<Refresh>>,
    refresh_calls: AtomicUsize,
    entered: tokio::sync::Notify,
    zero_at_ready: AtomicBool,
    deadlines: StdMutex<Vec<Instant>>,
    held: StdMutex<Option<tokio::sync::OwnedSemaphorePermit>>,
    ready: StdMutex<Option<Box<dyn FnOnce() + Send>>>,
}
impl FreshSource {
    fn new(route: ExistingRsdRoute) -> Arc<Self> {
        Arc::new(Self {
            base: LocalSource::new(),
            route,
            fates: StdMutex::new(VecDeque::new()),
            refresh_calls: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
            zero_at_ready: AtomicBool::new(false),
            held: StdMutex::new(None),
            ready: StdMutex::new(None),
            deadlines: StdMutex::new(Vec::new()),
        })
    }
    fn script(&self, fate: Refresh) {
        self.fates.lock().unwrap().push_back(fate);
    }
}
impl Source for FreshSource {
    fn observes_freshness(&self) -> bool {
        true
    }
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let original = self.base.connect(udid, route, mode).await?;
            Ok(ManagedDevice::Fake(Box::new(FreshDevice {
                native: original.native_handle().unwrap().clone(),
                route: self.route,
                catalogue: None,
            })))
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            UsbSnapshot::Devices(vec![source::UsbDevice {
                udid: UID.into(),
                device_id: 7,
                usb: true,
            }])
        })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn refresh_existing_rsd<'a>(
        &'a self,
        route: ExistingRsdRoute,
        deadline: Instant,
        bank: &'a Arc<tokio::sync::Semaphore>,
    ) -> Result<crate::cmd::tunnel::future_storage::Dynamic<'a, Result<Arc<PreparedRsdCatalogue>>>>
    {
        crate::cmd::tunnel::future_storage::Dynamic::new(
            async move {
                assert_eq!(route, self.route);
                self.deadlines.lock().unwrap().push(deadline);
                self.refresh_calls.fetch_add(1, Ordering::SeqCst);
                self.entered.notify_one();
                let fate = self
                    .fates
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(Refresh::Directory);
                let result = match fate {
                    Refresh::Directory => {
                        Ok(ios_core::device::prepare_existing_rsd_with_identity_at(
                            route,
                            deadline,
                            bank,
                            uuid::Uuid::nil(),
                        )
                        .await?)
                    }
                    Refresh::Failure {
                        marker,
                        delay,
                        zero,
                    } => {
                        if zero {
                            *self.held.lock().unwrap() = Some(
                                bank.clone()
                                    .try_acquire_many_owned(bank.available_permits() as u32)
                                    .unwrap(),
                            );
                            self.zero_at_ready
                                .store(bank.available_permits() == 0, Ordering::SeqCst);
                        }
                        std::thread::sleep(delay);
                        Err(anyhow::Error::new(Original { marker }))
                    }
                    Refresh::Pending => std::future::pending().await,
                };
                let ready = self.ready.lock().unwrap().take();
                if let Some(ready) = ready {
                    ready();
                }
                result
            },
            bank,
        )
    }
}
fn fresh_state(source: Arc<FreshSource>, timeout: Duration) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        timeout,
    )
}
async fn fresh_publish(state: &TunnelAgentState) {
    let ticket = state.inner.lifecycle.ticket(UID).await.unwrap();
    state
        .connect_and_store(UID, Route::Wifi(None), &ticket, None, None)
        .await
        .unwrap();
}
async fn refresh(state: &TunnelAgentState, duration: Duration) -> Result<()> {
    let deadline = lifecycle::ConnectionDeadline::new(duration)?;
    let ticket = state.inner.lifecycle.ticket(UID).await?;
    let _serial = ticket.serial().await;
    state
        .refresh_alive_rsd(UID, &ticket, &deadline, None, None)
        .await
}
async fn fresh_finish(state: &TunnelAgentState, source: &FreshSource, expected: usize) {
    drop(source.held.lock().unwrap().take());
    assert!(finish(state, &source.base, expected).await);
    assert!(source.base.handle().shutdown_status().is_ok());
}

#[tokio::test]
async fn freshness_production_alive_scan_changes_current_ports_without_new_lease_or_original_buffers(
) {
    let expected = baseline();
    let (route, peer) = directories(vec![10001, 10002]).await;
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    state.refresh_once().await.unwrap();
    let native = source.base.handle();
    let (key, publication, first_fee, second_fee) = {
        let table = state.inner.tunnels.read().await;
        let (key, row) = table.get_key_value(UID).unwrap();
        (
            key.as_ptr(),
            row.publication.clone(),
            row._metadata.clone(),
            row._global_metadata.clone(),
        )
    };
    state.refresh_once().await.unwrap();
    let old = state.inner.tunnels.read().await[UID]
        ._fresh_metadata
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(old.catalogue.service_port(SERVICE), Some(10001));
    state.refresh_once().await.unwrap();
    {
        let table = state.inner.tunnels.read().await;
        let (same_key, row) = table.get_key_value(UID).unwrap();
        assert_eq!(same_key.as_ptr(), key);
        assert!(row.publication.ptr_eq(&publication));
        assert!(
            Arc::ptr_eq(&row._metadata, &first_fee)
                && Arc::ptr_eq(&row._global_metadata, &second_fee)
        );
        let latest = row._fresh_metadata.as_ref().unwrap();
        assert_eq!(latest.catalogue.service_port(SERVICE), Some(10002));
        assert!(Arc::ptr_eq(
            row.device.freshness_catalogue().unwrap(),
            &latest.catalogue
        ));
        assert!(Arc::ptr_eq(row.device.native_handle().unwrap(), &native));
    }
    assert_eq!(source.base.connect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 2);
    assert_eq!(state.inner.native.snapshot().active, 1);
    assert_eq!(old.catalogue.service_port(SERVICE), Some(10001));
    tokio::time::timeout(OBSERVE, peer).await.unwrap().unwrap();
    fresh_finish(&state, &source, expected).await;
    drop((old, publication, first_fee, second_fee, native));
    let bank = state.inner.global_metadata_bytes.clone();
    drop(state);
    drop(source);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn freshness_actual_late_swap_restores_old_catalogue_and_all_original_fee_owners() {
    let expected = baseline();
    let (route, peer) = directories(vec![10001, 10002]).await;
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    refresh(&state, OBSERVE).await.unwrap();
    let original = state.inner.tunnels.read().await[UID]
        ._fresh_metadata
        .as_ref()
        .unwrap()
        .clone();
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let crossed = Arc::new(AtomicBool::new(false));
    let happened = crossed.clone();
    freshness::on_after_swap(move || {
        happened.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(600));
    });
    let error = refresh(&state, Duration::from_millis(500))
        .await
        .unwrap_err();
    assert!(
        crossed.load(Ordering::SeqCst),
        "actual mutation boundary was not reached"
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
    {
        let table = state.inner.tunnels.read().await;
        let row = &table[UID];
        assert!(row._fresh_metadata.as_ref().unwrap().ptr_eq(&original));
        assert!(Arc::ptr_eq(
            row.device.freshness_catalogue().unwrap(),
            &original.catalogue
        ));
        assert!(row.is_alive());
    }
    assert_eq!(bank.available_permits(), before);
    assert_eq!(state.inner.native.snapshot().active, 1);
    tokio::time::timeout(OBSERVE, peer).await.unwrap().unwrap();
    drop(error);
    drop(original);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_ready_error_at_zero_quota_and_crossed_clock_keeps_original_typed_payload() {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let marker = Arc::new(42);
    source.script(Refresh::Failure {
        marker: marker.clone(),
        delay: Duration::from_millis(60),
        zero: true,
    });
    let error = refresh(&state, Duration::from_millis(20))
        .await
        .unwrap_err();
    let primary = error
        .downcast_ref::<freshness::Failure>()
        .unwrap()
        .original()
        .downcast_ref::<Original>()
        .unwrap();
    assert!(Arc::ptr_eq(&primary.marker, &marker));
    assert!(source.zero_at_ready.load(Ordering::SeqCst));
    assert!(
        state.inner.global_metadata_bytes.available_permits() > 0,
        "temporary observation fee refunded"
    );
    let originals = state.inner.native.payload_freshness().unwrap();
    assert!(originals.as_slice().iter().any(|receipt| Arc::ptr_eq(
        &receipt
            .original()
            .downcast_ref::<Original>()
            .unwrap()
            .marker,
        &marker
    )));
    assert!(state.inner.tunnels.read().await[UID].is_alive());
    drop(originals);
    drop(error);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_cancelled_permit_wait_has_source_zero_and_original_native_joins() {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let held = state
        .inner
        .lifecycle
        .connects
        .acquire_many(lifecycle::MAX_CONNECTS as u32)
        .await
        .unwrap();
    let mut operation = Box::pin(refresh(&state, OBSERVE));
    assert!(futures_util::poll!(&mut operation).is_pending());
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 0);
    state.notify_shutdown();
    let error = tokio::time::timeout(OBSERVE, operation)
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    drop(error);
    drop(held);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_pending_source_device_epoch_stop_preserves_original_row_until_join() {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    source.script(Refresh::Pending);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let mut operation = Box::pin(refresh(&state, OBSERVE));
    assert!(futures_util::poll!(&mut operation).is_pending());
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 1);
    state.inner.lifecycle.stop(UID).await;
    let error = tokio::time::timeout(OBSERVE, operation)
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert!(state.inner.tunnels.read().await[UID].is_alive());
    drop(error);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_pre_source_zero_quota_preserves_publication_and_original_native() {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let bank = state.inner.global_metadata_bytes.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let error = refresh(&state, OBSERVE).await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
    assert!(state.inner.tunnels.read().await[UID].is_alive());
    drop(error);
    drop(held);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn refresh_future_last_byte_constructor_refusal_keeps_original_catalogue_and_native() {
    let expected = baseline();
    let (route, peer) = directories(vec![10203]).await;
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    refresh(&state, OBSERVE).await.unwrap();
    tokio::time::timeout(OBSERVE, peer).await.unwrap().unwrap();
    let (publication, previous) = {
        let table = state.inner.tunnels.read().await;
        let row = &table[UID];
        (
            row.publication.clone(),
            row._fresh_metadata.as_ref().unwrap().clone(),
        )
    };
    let native = source.base.handle();
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let held = Arc::new(StdMutex::new(None));
    let captured = held.clone();
    let charged = bank.clone();
    let hook = crate::cmd::tunnel::future_storage::on_dynamic_layout(move |bytes| {
        assert!(bytes > 0);
        *captured.lock().unwrap() = Some(
            charged
                .clone()
                .try_acquire_many_owned((charged.available_permits() - bytes + 1) as u32)
                .unwrap(),
        );
    });
    let error = refresh(&state, OBSERVE).await.unwrap_err();
    drop(hook);
    assert!(held.lock().unwrap().is_some());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.base.connect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.native.snapshot().active, 1);
    {
        let table = state.inner.tunnels.read().await;
        let row = &table[UID];
        assert!(row.publication.ptr_eq(&publication));
        assert!(row._fresh_metadata.as_ref().unwrap().ptr_eq(&previous));
        assert!(Arc::ptr_eq(
            row.device.freshness_catalogue().unwrap(),
            &previous.catalogue
        ));
        assert!(Arc::ptr_eq(row.device.native_handle().unwrap(), &native));
        assert_eq!(previous.catalogue.service_port(SERVICE), Some(10203));
        assert!(row.is_alive());
    }
    drop(error);
    held.lock().unwrap().take();
    assert_eq!(bank.available_permits(), before);
    drop((publication, previous, native));
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_ready_error_crossing_clear_keeps_original_before_any_cancelled_await() {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let marker = Arc::new(43);
    source.script(Refresh::Failure {
        marker: marker.clone(),
        delay: Duration::ZERO,
        zero: true,
    });
    let clear = state.clone();
    *source.ready.lock().unwrap() = Some(Box::new(move || clear.inner.lifecycle.clear()));
    let error = refresh(&state, OBSERVE).await.unwrap_err();
    let failure = error.downcast_ref::<freshness::Failure>().unwrap();
    assert!(Arc::ptr_eq(
        &failure
            .original()
            .downcast_ref::<Original>()
            .unwrap()
            .marker,
        &marker
    ));
    assert!(state
        .inner
        .native
        .payload_freshness()
        .unwrap()
        .as_slice()
        .iter()
        .any(|receipt| Arc::ptr_eq(
            &receipt
                .original()
                .downcast_ref::<Original>()
                .unwrap()
                .marker,
            &marker
        )));
    assert!(state.inner.tunnels.read().await[UID]
        ._fresh_metadata
        .is_none());
    drop(error);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_new_publication_during_source_refuses_old_proof_without_stopping_original() {
    let expected = baseline();
    let (route, peer) = directories(vec![10001]).await;
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let old_publication = state.inner.tunnels.read().await[UID].publication.clone();
    let replacement = publication::Life::new(
        &state.inner.global_metadata_bytes,
        &state.inner.metadata_bytes,
    )
    .unwrap();
    let change = state.clone();
    *source.ready.lock().unwrap() = Some(Box::new(move || {
        change
            .inner
            .tunnels
            .try_write()
            .unwrap()
            .get_mut(UID)
            .unwrap()
            .publication = replacement;
    }));
    let error = refresh(&state, OBSERVE).await.unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    {
        let table = state.inner.tunnels.read().await;
        let row = &table[UID];
        assert!(!row.publication.ptr_eq(&old_publication));
        assert!(row._fresh_metadata.is_none() && row.device.freshness_catalogue().is_none());
        assert!(row.is_alive());
    }
    assert_eq!(state.inner.native.snapshot().active, 1);
    tokio::time::timeout(OBSERVE, peer).await.unwrap().unwrap();
    drop((error, old_publication));
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_revoked_manual_authority_cancels_pending_source_and_preserves_old_cache() {
    use crate::access::test_support::{grant, PolicyFixture, TOKEN};
    use crate::access::Permission;
    let expected = baseline();
    let policy = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![
            Permission::ManageTunnels,
            Permission::ManageServer,
            Permission::List,
        ],
    )]);
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let principal = access.authenticate(&headers).unwrap();
    let authority = principal
        .device_authority(Permission::ManageTunnels, UID)
        .ok()
        .unwrap();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    source.script(Refresh::Pending);
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access.clone(),
        Upstreams::default(),
        source.clone(),
        OBSERVE,
    );
    fresh_publish(&state).await;
    let deadline = lifecycle::ConnectionDeadline::new(OBSERVE).unwrap();
    let ticket = state.inner.lifecycle.ticket(UID).await.unwrap();
    let serial = ticket.serial().await;
    let mut operation =
        Box::pin(state.refresh_alive_rsd(UID, &ticket, &deadline, None, Some(&authority)));
    assert!(futures_util::poll!(&mut operation).is_pending());
    assert_eq!(source.refresh_calls.load(Ordering::SeqCst), 1);
    policy.replace(vec![grant(TOKEN, vec![Permission::ManageServer])]);
    access.reload(&principal).await.unwrap();
    let error = tokio::time::timeout(OBSERVE, operation)
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert!(state.inner.tunnels.read().await[UID]
        ._fresh_metadata
        .is_none());
    drop(error);
    drop(serial);
    drop(ticket);
    fresh_finish(&state, &source, expected).await;
}

#[tokio::test]
async fn freshness_source_receives_original_min_retry_candidate_whole_deadline() {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    for retry_shorter in [false, true] {
        source.script(Refresh::Pending);
        let control = state.inner.lifecycle.scan_control(OBSERVE).unwrap();
        let whole = scan::Deadline::new(Duration::from_millis(100)).unwrap();
        let deadline = lifecycle::ConnectionDeadline::for_scan_candidate(OBSERVE)
            .unwrap()
            .within_scan(Some(&whole));
        let expected_at = if retry_shorter {
            let at = Instant::now() + Duration::from_millis(50);
            deadline.clamp_retry(at);
            at
        } else {
            whole.at
        };
        let ticket = state.inner.lifecycle.ticket(UID).await.unwrap();
        let serial = ticket.serial().await;
        let result = whole
            .run(
                &control.epoch,
                state.refresh_alive_rsd(UID, &ticket, &deadline, None, None),
            )
            .await;
        assert_eq!(
            source.deadlines.lock().unwrap().last().copied(),
            Some(expected_at)
        );
        let error = result.unwrap_err();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::Timeout
        );
        assert!(state.inner.tunnels.read().await[UID].is_alive());
        drop(error);
        drop(serial);
        drop((ticket, control));
    }
    fresh_finish(&state, &source, expected).await;
}

#[test]
fn freshness_capsule_exact_body_and_report_admission_refusal_and_zero_quota_original() {
    let bytes =
        std::mem::size_of::<freshness::Receipt>() + 2 * std::mem::size_of::<freshness::Failure>();
    let short = Arc::new(tokio::sync::Semaphore::new(bytes - 1));
    assert!(freshness::Receipt::prepare(&short).is_err());
    assert_eq!(short.available_permits(), bytes - 1);
    // Real Ready(Err), downcast, and zero-quota preservation are exercised by
    // native production tests above; this checks pre-Source exact admission only.
    let exact = Arc::new(tokio::sync::Semaphore::new(bytes));
    let capsule = freshness::Receipt::prepare(&exact).unwrap();
    assert_eq!(exact.available_permits(), 0);
    drop(capsule);
    assert_eq!(exact.available_permits(), bytes);
}

#[tokio::test]
async fn freshness_current_owner_survives_either_table_removal_or_native_joins_until_both_release()
{
    for clear_first in [true, false] {
        let expected = baseline();
        let (route, peer) = directories(vec![10001]).await;
        let source = FreshSource::new(route);
        let state = fresh_state(source.clone(), OBSERVE);
        fresh_publish(&state).await;
        refresh(&state, OBSERVE).await.unwrap();
        let owner = state.inner.tunnels.read().await[UID]
            ._fresh_metadata
            .as_ref()
            .unwrap()
            .downgrade();
        tokio::time::timeout(OBSERVE, peer).await.unwrap().unwrap();
        let global = state.inner.global_metadata_bytes.clone();
        let publication = state.inner.metadata_bytes.clone();
        let charged = publication.available_permits();
        let table_storage = state.inner.tunnels.read().await.owned_publication_bytes();
        if clear_first {
            state.clear().await;
            assert!(
                owner.upgrade().is_some(),
                "fresh cache fees released before original native joins"
            );
            assert_eq!(
                publication.available_permits(),
                charged + table_storage + super::super::super::publication::Life::bytes()
            );
            state.drain_native(OBSERVE).await.unwrap();
        } else {
            state.notify_shutdown();
            state.drain_native(OBSERVE).await.unwrap();
            assert!(
                owner.upgrade().is_some(),
                "fresh cache fees released while table still owns cache"
            );
            assert_eq!(publication.available_permits(), charged);
            state.clear().await;
        }
        assert!(owner.upgrade().is_none());
        assert!(source.base.handle().shutdown_status().is_ok());
        assert!(recovered(&source.base, expected).await);
        state.notify_shutdown();
        drop(state);
        drop(source);
        let body = freshness::MetadataOwner::body_bytes();
        assert_eq!(
            global.available_permits(),
            MAX_GLOBAL_KNOWN_METADATA_BYTES - body
        );
        assert_eq!(
            publication.available_permits(),
            MAX_PUBLISHED_METADATA_BYTES - body
        );
        // Value fields/catalogue fees ended at last strong; BOTH body leases
        // still cover the original allocation and paid guard through this Weak.
        drop(owner);
        assert_eq!(global.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
        assert_eq!(
            publication.available_permits(),
            MAX_PUBLISHED_METADATA_BYTES
        );
    }
}

#[tokio::test]
async fn freshness_census_backing_refusal_preserves_original_roots_and_marks_attribution_incomplete(
) {
    let expected = baseline();
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0)).unwrap();
    let source = FreshSource::new(route);
    let state = fresh_state(source.clone(), OBSERVE);
    fresh_publish(&state).await;
    let marker = Arc::new(44);
    source.script(Refresh::Failure {
        marker: marker.clone(),
        delay: Duration::ZERO,
        zero: true,
    });
    let error = refresh(&state, OBSERVE).await.unwrap_err();
    let bank = state.inner.global_metadata_bytes.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    assert_eq!(bank.available_permits(), 0);
    let refused = state.inner.native.payload_freshness().err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(refused.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let incomplete = state.retained_error_payloads();
    assert!(incomplete.truncated && !incomplete.fully_measured_payloads());
    assert!(Arc::ptr_eq(
        &error
            .downcast_ref::<freshness::Failure>()
            .unwrap()
            .original()
            .downcast_ref::<Original>()
            .unwrap()
            .marker,
        &marker
    ));
    drop((held, refused));
    let roots = state.inner.native.payload_freshness().unwrap();
    assert_eq!(roots.len(), 2);
    assert!(roots.as_slice().iter().all(|receipt| Arc::ptr_eq(
        &receipt
            .original()
            .downcast_ref::<Original>()
            .unwrap()
            .marker,
        &marker
    )));
    assert!(bank.available_permits() > 0);
    drop((roots, error));
    fresh_finish(&state, &source, expected).await;
}

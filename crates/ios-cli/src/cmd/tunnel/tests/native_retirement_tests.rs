//! Real native workers through the production publication and server owners.
//! Only discovery/device metadata are supplied by the CLI's private test seam.
use super::*;
use futures_util::future::BoxFuture;
use ios_core::{TunnelHandle, TunnelOwner};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

const DEVICE: &str = "native-cli-retirement-fixture";
const OBSERVE: Duration = Duration::from_secs(2);

mod bookkeeping_tests;
mod deadline_tests;
mod manual_admission_tests;
mod paired_discovery_tests;
mod paired_fence_tests;
mod reconnect_tests;
mod scan_outcome_tests;
mod snapshot_admission_tests;
mod success_bookkeeping_tests;

struct NativeDevice {
    native: Arc<TunnelHandle>,
}
impl source::FakeDevice for NativeDevice {
    fn record(&self) -> TunnelRecord {
        let mut record = fake_record(self.native.userspace_port.unwrap());
        record.udid = DEVICE.into();
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

struct LocalSource {
    handles: StdMutex<Vec<Arc<TunnelHandle>>>,
    peers: StdMutex<Vec<DuplexStream>>,
    connect_calls: AtomicUsize,
    usb_calls: AtomicUsize,
    usb_gate: StdMutex<Option<Arc<Semaphore>>>,
    usb_failure: StdMutex<Option<io::ErrorKind>>,
    attached: StdMutex<bool>,
}
impl LocalSource {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            handles: StdMutex::new(Vec::new()),
            peers: StdMutex::new(Vec::new()),
            connect_calls: AtomicUsize::new(0),
            usb_calls: AtomicUsize::new(0),
            usb_gate: StdMutex::new(None),
            usb_failure: StdMutex::new(None),
            attached: StdMutex::new(true),
        })
    }
    fn handle(&self) -> Arc<TunnelHandle> {
        self.handles.lock().unwrap().last().unwrap().clone()
    }
}
impl Source for LocalSource {
    fn connect<'a>(
        &'a self,
        _: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            self.connect_calls.fetch_add(1, Ordering::SeqCst);
            let (stream, peer) = tokio::io::duplex(1280);
            let runtime =
                ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream).await?;
            let native = Arc::new(TunnelHandle::new_userspace(
                DEVICE.into(),
                ios_core::TunnelInfo {
                    server_address: "fd00::1".into(),
                    server_rsd_port: 58783,
                    client_address: "fd00::2".into(),
                    client_mtu: 1280,
                },
                runtime,
            ));
            self.handles.lock().unwrap().push(native.clone());
            self.peers.lock().unwrap().push(peer);
            Ok(ManagedDevice::Fake(Box::new(NativeDevice { native })))
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async move {
            self.usb_calls.fetch_add(1, Ordering::SeqCst);
            let gate = self.usb_gate.lock().unwrap().clone();
            if let Some(gate) = gate {
                gate.acquire().await.unwrap().forget();
            }
            if let Some(kind) = *self.usb_failure.lock().unwrap() {
                return UsbSnapshot::ListError(
                    io::Error::new(kind, "synthetic-private-usb-failure").into(),
                );
            }
            UsbSnapshot::Devices(if *self.attached.lock().unwrap() {
                vec![source::UsbDevice {
                    udid: DEVICE.into(),
                    device_id: 7,
                    usb: true,
                }]
            } else {
                Vec::new()
            })
        })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

fn state(source: Arc<LocalSource>) -> TunnelAgentState {
    TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
    )
}
fn baseline() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}
async fn recovered(source: &LocalSource, expected: usize) -> bool {
    let peers = std::mem::take(&mut *source.peers.lock().unwrap());
    tokio::time::timeout(OBSERVE, async {
        for mut peer in peers {
            match peer.read_u8().await {
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {}
                _ => return false,
            }
        }
        while baseline() != expected {
            tokio::task::yield_now().await;
        }
        true
    })
    .await
    .unwrap_or(false)
}
async fn publish(state: &TunnelAgentState, route: Route) -> Result<TunnelRecord> {
    let ticket = state.inner.lifecycle.ticket(DEVICE).await?;
    state
        .connect_and_store(DEVICE, route, &ticket, None, None)
        .await
}
async fn finish(state: &TunnelAgentState, source: &LocalSource, expected: usize) -> bool {
    state.notify_shutdown();
    let result = state.drain_native(OBSERVE).await;
    state.clear().await;
    let recovery = recovered(source, expected).await;
    result.is_ok() && recovery && state.inner.native.snapshot().closed_and_empty()
}

#[tokio::test]
async fn native_cli_server_caller_drop_stops_and_joins_with_table_lock_and_state_clone_held() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = source.handle();
    let mut proxy = TcpStream::connect(("127.0.0.1", native.userspace_port.unwrap()))
        .await
        .unwrap();
    proxy.write_all(&[0]).await.unwrap(); // Actual proxy child waits for the rest of its preamble.
    let child_started = tokio::time::timeout(OBSERVE, async {
        while !native.shutdown_status().is_err_and(|error| {
            let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
            for _ in 0..32 {
                let Some(current) = cause else { break };
                if let Some(report) = current.downcast_ref::<ios_core::TunnelShutdownFailure>() {
                    return report.pending_joins() >= 5;
                }
                cause = if let Some(io) = current.downcast_ref::<io::Error>() {
                    io.get_ref()
                        .map(|inner| inner as &(dyn std::error::Error + 'static))
                        .or_else(|| current.source())
                } else {
                    current.source()
                };
            }
            false
        }) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let table = state.inner.tunnels.write().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut owner = Box::pin(server::serve(
        listener,
        agent_router(state.clone()),
        state.clone(),
        Duration::from_millis(5),
        OBSERVE,
        std::future::pending(),
    ));
    let pending = futures_util::poll!(&mut owner).is_pending();
    drop(owner);
    let stopped = !native.is_alive();
    let before_join = state.inner.native.snapshot();
    let joined = state.drain_native(OBSERVE).await;
    let after_join = state.inner.native.snapshot();
    let eof = tokio::time::timeout(OBSERVE, proxy.read_u8()).await;
    let recovery = recovered(&source, expected).await;
    drop(table);
    state.clear().await;
    joined.unwrap();
    assert!(child_started && pending && stopped && recovery);
    assert_eq!((before_join.retired, before_join.native_handles), (1, 1));
    assert!(after_join.closed_and_empty());
    assert!(native.shutdown_status().is_ok());
    assert!(eof.unwrap().is_err_and(|error| matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
    )));
}

#[tokio::test]
async fn native_cli_server_native_joins_survive_metadata_lock_timeout_and_signal_primary() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = source.handle();
    let table = state.inner.tunnels.write().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let result = tokio::time::timeout(
        OBSERVE,
        server::serve(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_millis(5),
            Duration::from_millis(30),
            async {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "synthetic-private-signal",
                ))
            },
        ),
    )
    .await;
    let snapshot = state.inner.native.snapshot();
    let recovery = recovered(&source, expected).await;
    drop(table);
    state.clear().await;
    assert!(recovery && snapshot.closed_and_empty() && native.shutdown_status().is_ok());
    let result = result.unwrap().unwrap_err();
    let dual = result.downcast_ref::<server::CleanupFailure>().unwrap();
    assert_eq!(
        dual.primary.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    let terminal = dual
        .cleanup
        .downcast_ref::<server::TerminalCleanupFailure>()
        .unwrap();
    assert!(terminal.native_snapshot().closed_and_empty());
    assert!(terminal.primary().is::<tokio::time::error::Elapsed>());
    assert!(!format!("{terminal:?} {terminal}").contains("synthetic-private"));
    assert_eq!(
        ios_core::classify_error(result.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
}

#[tokio::test]
async fn native_cli_publication_query_caller_drop_retains_original_join_owner() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let gate = Arc::new(Semaphore::new(0));
    *source.usb_gate.lock().unwrap() = Some(gate.clone());
    let mut connection = Box::pin(publish(&state, Route::Usbmux));
    let waiting = tokio::time::timeout(OBSERVE, async {
        while source.usb_calls.load(Ordering::SeqCst) == 0 {
            if futures_util::poll!(&mut connection).is_ready() {
                return false;
            }
            tokio::task::yield_now().await;
        }
        true
    })
    .await
    .unwrap_or(false);
    let native = source.handle();
    let admitted = state.inner.native.snapshot();
    drop(connection);
    let retired = state.inner.native.snapshot();
    let stopped = !native.is_alive();
    gate.add_permits(1);
    let recovery = finish(&state, &source, expected).await;
    assert!(waiting && stopped && recovery);
    assert_eq!((admitted.unpublished, admitted.native_handles), (1, 1));
    assert_eq!((retired.retired, retired.native_handles), (1, 1));
    assert!(native.shutdown_status().is_ok());
}

#[tokio::test]
async fn native_cli_publication_table_caller_drop_keeps_owner_outside_locked_metadata() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let table = state.inner.tunnels.write().await;
    let mut connection = Box::pin(publish(&state, Route::Wifi(None)));
    let pending = futures_util::poll!(&mut connection).is_pending();
    let native = source.handle();
    drop(connection);
    let snapshot = state.inner.native.snapshot();
    let stopped = !native.is_alive();
    let joined = state.drain_native(OBSERVE).await;
    let recovery = recovered(&source, expected).await;
    drop(table);
    state.clear().await;
    joined.unwrap();
    assert!(pending && stopped && recovery && native.shutdown_status().is_ok());
    assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
}

#[tokio::test]
async fn native_cli_publication_usb_error_preserves_primary_and_retires_actual_workers() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    *source.usb_failure.lock().unwrap() = Some(io::ErrorKind::PermissionDenied);
    let result = publish(&state, Route::Usbmux).await;
    let native = source.handle();
    let snapshot = state.inner.native.snapshot();
    let stopped = !native.is_alive();
    let recovery = finish(&state, &source, expected).await;
    assert!(stopped && recovery && native.shutdown_status().is_ok());
    assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    assert_eq!(
        result
            .unwrap_err()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn native_cli_stop_and_clear_retire_exact_runtime_before_metadata_drop() {
    for clear in [false, true] {
        let expected = baseline();
        let source = LocalSource::new();
        let state = state(source.clone());
        publish(&state, Route::Wifi(None)).await.unwrap();
        let native = source.handle();
        let response = if clear {
            state.clear().await;
            true
        } else {
            state.remove_tunnel(DEVICE).await
        };
        let stopped = !native.is_alive();
        let snapshot = state.inner.native.snapshot();
        let recovery = finish(&state, &source, expected).await;
        assert!(response && stopped && recovery && native.shutdown_status().is_ok());
        assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    }
}

#[tokio::test]
async fn native_cli_replacement_retires_old_arc_and_keeps_new_owner_live() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    let old = source.handle();
    publish(&state, Route::Wifi(None)).await.unwrap();
    let new = source.handle();
    let snapshot = state.inner.native.snapshot();
    let correct = !Arc::ptr_eq(&old, &new) && !old.is_alive() && new.is_alive();
    let first = state.drain_native(OBSERVE).await;
    let still_live = new.is_alive();
    let recovery = finish(&state, &source, expected).await;
    first.unwrap();
    assert!(correct && still_live && recovery);
    assert_eq!((snapshot.active, snapshot.retired), (1, 1));
    assert!(old.shutdown_status().is_ok() && new.shutdown_status().is_ok());
}

#[tokio::test]
async fn native_cli_core_candidate_guard_keeps_failed_owner_before_fresh_target_handoff() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let owner = ownership::Ownership::admitted(state.inner.native.clone(), &ticket).unwrap();
    let mut first = owner.reserve().unwrap();
    let device = source
        .connect(DEVICE, Route::Wifi(None), TunMode::Userspace)
        .await
        .unwrap();
    let old = device.native_handle().unwrap().clone();
    first.attach(old.clone()).unwrap();
    drop(first); // Simulates the native target future expiring before complete.
    let mut second = owner.reserve().unwrap();
    let next = source
        .connect(DEVICE, Route::Wifi(None), TunMode::Userspace)
        .await
        .unwrap();
    let new = next.native_handle().unwrap().clone();
    second.attach(new.clone()).unwrap();
    second.complete();
    drop(second);
    let lease = owner.take_completed(&new).unwrap();
    let snapshot = state.inner.native.snapshot();
    let correct = !old.is_alive() && new.is_alive() && !Arc::ptr_eq(&old, &new);
    drop(lease);
    drop(owner);
    let recovery = finish(&state, &source, expected).await;
    assert!(correct && recovery);
    assert_eq!(
        (
            snapshot.retired,
            snapshot.unpublished,
            snapshot.native_handles
        ),
        (1, 1, 2)
    );
    assert!(old.shutdown_status().is_ok() && new.shutdown_status().is_ok());
}

#[tokio::test]
async fn native_cli_owner_capacity_rejects_before_the_actual_source_is_polled() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = source.handle();
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let stamp = ticket.ownership_stamp();
    let reservations = (1..retirement::MAX_NATIVE_OWNERS)
        .map(|_| state.inner.native.reserve(stamp.clone()).unwrap())
        .collect::<Vec<_>>();
    let full = state.inner.native.snapshot();
    let rejected = publish(&state, Route::Wifi(None)).await;
    let source_calls = source.connect_calls.load(Ordering::SeqCst);
    drop(reservations);
    let recovery = finish(&state, &source, expected).await;
    assert!(recovery && native.shutdown_status().is_ok());
    assert_eq!(
        (full.active, full.reserved, full.occupied()),
        (
            1,
            retirement::MAX_NATIVE_OWNERS - 1,
            retirement::MAX_NATIVE_OWNERS
        )
    );
    assert_eq!(source_calls, 1);
    assert!(rejected
        .unwrap_err()
        .to_string()
        .contains("native tunnel owner capacity exhausted"));
}

#[tokio::test]
async fn native_cli_dead_get_and_list_keep_original_native_joins_for_collection() {
    for list in [false, true] {
        let expected = baseline();
        let source = LocalSource::new();
        let state = state(source.clone());
        publish(&state, Route::Wifi(None)).await.unwrap();
        let native = source.handle();
        native.request_stop();
        let removed = if list {
            state.list_records().await.is_empty()
        } else {
            state.get_record(DEVICE).await.is_none()
        };
        let snapshot = state.inner.native.snapshot();
        let recovery = finish(&state, &source, expected).await;
        assert!(removed && recovery && native.shutdown_status().is_ok());
        assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    }
}

#[tokio::test]
async fn native_cli_scan_list_failure_preserves_source_io_and_keeps_removed_native_owner() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = source.handle();
    *source.usb_failure.lock().unwrap() = Some(io::ErrorKind::BrokenPipe);
    let result = state.refresh_once().await;
    let snapshot = state.inner.native.snapshot();
    let stopped = !native.is_alive();
    let recovery = finish(&state, &source, expected).await;
    assert!(stopped && recovery && native.shutdown_status().is_ok());
    assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    assert_eq!(
        result
            .unwrap_err()
            .downcast_ref::<io::Error>()
            .unwrap()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[tokio::test]
async fn native_cli_scan_detachment_retires_the_exact_published_usb_runtime() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    publish(&state, Route::Usbmux).await.unwrap();
    let native = source.handle();
    *source.attached.lock().unwrap() = false;
    let result = state.refresh_once().await;
    let snapshot = state.inner.native.snapshot();
    let stopped = !native.is_alive();
    let absent = state.get_record(DEVICE).await.is_none();
    let recovery = finish(&state, &source, expected).await;
    result.unwrap();
    assert!(stopped && absent && recovery && native.shutdown_status().is_ok());
    assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
}

#[tokio::test]
async fn native_cli_clear_and_stop_fence_unpublished_real_runtime_before_query_returns() {
    for clear in [false, true] {
        let expected = baseline();
        let source = LocalSource::new();
        let state = state(source.clone());
        let gate = Arc::new(Semaphore::new(0));
        *source.usb_gate.lock().unwrap() = Some(gate.clone());
        let mut connection = Box::pin(publish(&state, Route::Usbmux));
        let waiting = tokio::time::timeout(OBSERVE, async {
            while source.usb_calls.load(Ordering::SeqCst) == 0 {
                if futures_util::poll!(&mut connection).is_ready() {
                    return false;
                }
                tokio::task::yield_now().await;
            }
            true
        })
        .await
        .unwrap_or(false);
        let native = source.handle();
        let response = if clear {
            state.clear().await;
            false
        } else {
            state.remove_tunnel(DEVICE).await
        };
        let stopped_before_return = !native.is_alive();
        let snapshot = state.inner.native.snapshot();
        gate.add_permits(1);
        let rejected = connection.await;
        let absent = state.get_record(DEVICE).await.is_none();
        let recovery = finish(&state, &source, expected).await;
        assert!(waiting && !response && stopped_before_return && absent && recovery);
        assert!(rejected.is_err() && native.shutdown_status().is_ok());
        assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    }
}

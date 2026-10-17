//! Original userspace runtimes through real retry admission and retirement.
//! Discovery/device metadata use the private CLI seam; no OS TUN or device.
use super::*;
use std::num::NonZeroU32;

#[derive(Clone, Copy)]
enum Fate {
    Failure,
    Pending,
    LateSuccess,
    LateFailure,
    Success,
}

struct ReconnectSource {
    base: Arc<LocalSource>,
    fates: StdMutex<std::collections::VecDeque<Fate>>,
}

impl Source for ReconnectSource {
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
                reservation.attach(Arc::clone(device.native_handle().unwrap()))?;
                let fate = self
                    .fates
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or(Fate::Success);
                if matches!(fate, Fate::Pending) {
                    std::future::pending::<()>().await;
                }
                if matches!(fate, Fate::LateSuccess | Fate::LateFailure) {
                    // Deliberate non-preemptible ready poll crosses the total clock.
                    std::thread::sleep(Duration::from_millis(100));
                }
                if matches!(fate, Fate::Failure | Fate::LateFailure) {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "synthetic-private-retry-native-primary",
                    )
                    .into());
                }
                reservation.complete();
                Ok(device)
            },
            bank,
        )
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.base.usb_devices()
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        self.base.paired_devices()
    }
}

fn configured(
    fates: impl IntoIterator<Item = Fate>,
    attempts: Option<u32>,
    total: Option<Duration>,
) -> (TunnelAgentState, Arc<LocalSource>) {
    let base = LocalSource::new();
    let source = Arc::new(ReconnectSource {
        base: base.clone(),
        fates: StdMutex::new(fates.into_iter().collect()),
    });
    let policy = reconnect::RetryPolicy::new(
        Duration::from_millis(10),
        Duration::from_millis(40),
        attempts.map(|value| NonZeroU32::new(value).unwrap()),
        total,
    )
    .unwrap();
    let state = TunnelAgentState::with_source_and_retry(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
        policy,
    )
    .unwrap();
    (state, base)
}

async fn retired(state: &TunnelAgentState, base: &LocalSource, expected: usize) -> bool {
    let drain = state.drain_native(OBSERVE).await;
    let resources = recovered(base, expected).await;
    drain.is_ok() && resources && state.inner.native.snapshot().occupied() == 0
}

#[tokio::test]
async fn reconnect_native_failed_episodes_join_original_workers_before_new_attempts() {
    let expected = baseline();
    let (state, base) = configured([Fate::Failure, Fate::Failure, Fate::Success], Some(3), None);
    for expected_calls in 1..=2 {
        state.refresh_once().await.unwrap();
        let original = base.handle();
        let row = state.inner.retry.snapshot(DEVICE);
        let native = state.inner.native.snapshot();
        let port = original.userspace_port.unwrap();
        let collected = retired(&state, &base, expected).await;
        assert!(collected && original.shutdown_status().is_ok());
        let row = row.unwrap();
        assert_eq!(base.connect_calls.load(Ordering::SeqCst), expected_calls);
        assert_eq!(row.attempts, expected_calls as u32);
        assert_eq!((native.retired, native.native_handles), (1, 1));
        assert!(!original.is_alive());
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        tokio::time::sleep(Duration::from_millis(45)).await;
    }
    state.refresh_once().await.unwrap();
    let fresh = base.handle();
    let record = state.get_record(DEVICE).await.unwrap();
    let alive = fresh.is_alive();
    let recovered_episode = state.inner.retry.snapshot(DEVICE).is_none();
    let active = state.inner.native.snapshot().active;
    let available_permits = state.inner.failure_slots.available_permits();
    let old_handles = base.handles.lock().unwrap().clone();
    let resources = finish(&state, &base, expected).await;
    assert!(
        resources
            && old_handles
                .iter()
                .all(|handle| handle.shutdown_status().is_ok())
    );
    assert_eq!(record.unwrap().userspace_port, fresh.userspace_port);
    assert!(alive && recovered_episode);
    assert_eq!(active, 1);
    assert_eq!(available_permits, MAX_FAILURE_RECORDS);
    assert_eq!(state.inner.retry.len(), 0);
}

#[tokio::test]
async fn reconnect_native_exhaustion_keeps_owned_retirement_without_polling_source() {
    let expected = baseline();
    let (state, base) = configured([Fate::Failure], Some(1), None);
    state.refresh_once().await.unwrap();
    let original = base.handle();
    let retained = state.inner.native.snapshot();
    for _ in 0..3 {
        state.refresh_once().await.unwrap();
    }
    let after = state.inner.native.snapshot();
    let budget = state.inner.retry.snapshot(DEVICE);
    let calls = base.connect_calls.load(Ordering::SeqCst);
    let resources = finish(&state, &base, expected).await;
    assert!(resources && original.shutdown_status().is_ok());
    let budget = budget.unwrap();
    assert_eq!(calls, 1);
    assert_eq!(budget.phase, reconnect::Phase::Exhausted);
    assert_eq!((retained.retired, retained.native_handles), (1, 1));
    assert_eq!((after.retired, after.native_handles), (1, 1));
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn reconnect_native_total_expiry_after_attachment_retains_and_joins_exact_runtime() {
    let expected = baseline();
    let (state, base) = configured([Fate::Pending], None, Some(Duration::from_millis(60)));
    state.refresh_once().await.unwrap();
    let original = base.handle();
    let snapshot = state.inner.native.snapshot();
    let budget = state.inner.retry.snapshot(DEVICE);
    state.refresh_once().await.unwrap();
    let calls = base.connect_calls.load(Ordering::SeqCst);
    let resources = finish(&state, &base, expected).await;
    assert!(resources && original.shutdown_status().is_ok());
    let budget = budget.unwrap();
    assert_eq!(calls, 1);
    assert_eq!(budget.phase, reconnect::Phase::Exhausted);
    assert_eq!(budget.failures, 1);
    assert_eq!(budget.last_class, Some(ios_core::ErrorClass::Timeout));
    assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
    assert!(state.get_record(DEVICE).await.unwrap().is_none());
}

#[tokio::test]
async fn reconnect_native_ready_poll_crossing_total_cannot_publish_or_replace_original_err() {
    for fate in [Fate::LateSuccess, Fate::LateFailure] {
        let expected = baseline();
        let (state, base) = configured([fate], None, Some(Duration::from_millis(40)));
        let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
        let slot = state.reserve_failure_slot(DEVICE).await.unwrap();
        let clock = lifecycle::ConnectionDeadline::for_scan_candidate(OBSERVE).unwrap();
        let result = clock
            .run(state.connect_and_store_with_retry(
                DEVICE,
                Route::Wifi(None),
                &ticket,
                None,
                Some(&clock),
                Some((
                    AttemptIdentity {
                        route: Route::Wifi(None),
                        native_id: None,
                    },
                    slot,
                )),
            ))
            .await;
        let original = base.handle();
        let unpublished = state.inner.tunnels.read().await.is_empty();
        let snapshot = state.inner.native.snapshot();
        let budget = state.inner.retry.snapshot(DEVICE);
        let resources = finish(&state, &base, expected).await;
        assert!(resources && original.shutdown_status().is_ok() && unpublished);
        let budget = budget.unwrap();
        let error = result.unwrap_err();
        match fate {
            Fate::LateSuccess => assert!(error.is::<tokio::time::error::Elapsed>()),
            Fate::LateFailure => {
                assert_eq!(
                    error
                        .chain()
                        .find_map(|cause| cause.downcast_ref::<io::Error>())
                        .unwrap()
                        .kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert_eq!(budget.last_class, Some(ios_core::ErrorClass::Disconnected));
                let receipt = error
                    .downcast_ref::<reconnect::SourceFailure>()
                    .unwrap()
                    .receipt();
                assert_eq!(
                    receipt.latest().downcast_ref::<io::Error>().unwrap().kind(),
                    io::ErrorKind::BrokenPipe
                );
            }
            _ => unreachable!(),
        }
        assert_eq!(budget.phase, reconnect::Phase::Exhausted);
        assert_eq!((snapshot.retired, snapshot.native_handles), (1, 1));
        assert_eq!(base.connect_calls.load(Ordering::SeqCst), 1);
    }
}

struct OversizedPublicationSource(Arc<LocalSource>);

impl Source for OversizedPublicationSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.0.connect(udid, route, mode)
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async move {
            match self.0.usb_devices().await {
                UsbSnapshot::Devices(mut devices) => {
                    // A valid matching native attachment precedes the rejected
                    // tail, so this exercises post-Source publication admission.
                    devices.push(source::UsbDevice {
                        udid: "x".repeat(4097),
                        device_id: 7,
                        usb: true,
                    });
                    UsbSnapshot::Devices(devices)
                }
                other => other,
            }
        })
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        self.0.paired_devices()
    }
}

#[tokio::test]
async fn reconnect_native_post_source_resource_limit_keeps_class_and_original_publication() {
    let expected = baseline();
    let base = LocalSource::new();
    let state = TunnelAgentState::with_source_and_retry(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        Arc::new(OversizedPublicationSource(base.clone())),
        OBSERVE,
        reconnect::RetryPolicy::new(
            Duration::from_millis(10),
            Duration::from_millis(40),
            None,
            None,
        )
        .unwrap(),
    )
    .unwrap();
    publish(&state, Route::Wifi(None)).await.unwrap();
    let old = base.handle();
    let publication = state.inner.tunnels.read().await[DEVICE].publication.clone();
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let slot = state.reserve_failure_slot(DEVICE).await.unwrap();
    let clock = lifecycle::ConnectionDeadline::for_scan_candidate(OBSERVE).unwrap();
    let result = clock
        .run(state.connect_and_store_with_retry(
            DEVICE,
            Route::Usbmux,
            &ticket,
            None,
            Some(&clock),
            Some((
                AttemptIdentity {
                    route: Route::Usbmux,
                    native_id: None,
                },
                slot,
            )),
        ))
        .await;
    let new = base.handle();
    let old_alive = old.is_alive();
    let new_stopped = !new.is_alive();
    let unchanged = state
        .inner
        .tunnels
        .read()
        .await
        .get(DEVICE)
        .is_some_and(|row| {
            Arc::ptr_eq(&row.publication, &publication)
                && row
                    .device
                    .native_handle()
                    .is_some_and(|native| Arc::ptr_eq(native, &old))
        });
    let owners = state.inner.native.snapshot();
    let budget = state.inner.retry.snapshot(DEVICE);
    let resources = finish(&state, &base, expected).await;
    // Original joins, native duplex EOF and task/registry/permit recovery are
    // checked before the changed classification/publication assertions.
    assert!(resources && old.shutdown_status().is_ok() && new.shutdown_status().is_ok());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    let error = result.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let budget = budget.unwrap();
    assert_eq!(budget.last_class, Some(ios_core::ErrorClass::ResourceLimit));
    assert_eq!((budget.attempts, budget.failures), (1, 1));
    assert_eq!(budget.phase, reconnect::Phase::Backoff);
    assert!(old_alive && new_stopped && unchanged);
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(
        (owners.active, owners.retired, owners.native_handles),
        (1, 1, 2)
    );
    assert_eq!(base.connect_calls.load(Ordering::SeqCst), 2);
}

const PENDING_DEVICE: &str = "native-reconnect-pending-a";
const HEALTHY_DEVICE: &str = "native-reconnect-healthy-b";

struct IsolatedDevice(Arc<TunnelHandle>);

impl source::FakeDevice for IsolatedDevice {
    fn record(&self) -> TunnelRecord {
        let mut record = fake_record(self.0.userspace_port.unwrap());
        record.udid = self.0.udid.clone();
        record
    }

    fn is_alive(&self) -> bool {
        self.0.is_alive()
    }

    fn device_id(&self) -> u32 {
        7
    }

    fn native_handle(&self) -> Option<&Arc<TunnelHandle>> {
        Some(&self.0)
    }
}

struct IsolatedSource {
    pending: Arc<LocalSource>,
    healthy: Arc<LocalSource>,
}

impl Source for IsolatedSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let base = if udid == PENDING_DEVICE {
                &self.pending
            } else {
                &self.healthy
            };
            base.connect_calls.fetch_add(1, Ordering::SeqCst);
            let (stream, peer) = tokio::io::duplex(1280);
            let runtime =
                ios_core::UserspaceTunDevice::start("fd00::2", "fd00::1", 1280, stream).await?;
            let native = Arc::new(TunnelHandle::new_userspace(
                udid.into(),
                ios_core::TunnelInfo {
                    server_address: "fd00::1".into(),
                    server_rsd_port: 58783,
                    client_address: "fd00::2".into(),
                    client_mtu: 1280,
                },
                runtime,
            ));
            base.handles.lock().unwrap().push(native.clone());
            base.peers.lock().unwrap().push(peer);
            Ok(ManagedDevice::Fake(Box::new(IsolatedDevice(native))))
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
                if udid == PENDING_DEVICE {
                    std::future::pending::<()>().await;
                }
                reservation.complete();
                Ok(device)
            },
            bank,
        )
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async {
            UsbSnapshot::Devices(
                [PENDING_DEVICE, HEALTHY_DEVICE]
                    .into_iter()
                    .map(|udid| source::UsbDevice {
                        udid: udid.into(),
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

#[tokio::test]
async fn reconnect_native_auto_a_total_expiry_keeps_b_http_query_and_stop_isolated() {
    const HTTP_BUDGET: Duration = Duration::from_secs(1);
    const TOTAL: Duration = Duration::from_millis(1_500);
    let expected = baseline();
    let source = Arc::new(IsolatedSource {
        pending: LocalSource::new(),
        healthy: LocalSource::new(),
    });
    let state = TunnelAgentState::with_source_and_retry(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        OBSERVE,
        reconnect::RetryPolicy::new(
            Duration::from_millis(10),
            Duration::from_millis(40),
            None,
            Some(TOTAL),
        )
        .unwrap(),
    )
    .unwrap();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let scan_state = state.clone();
    let mut scan = tokio::spawn(async move { scan_state.refresh_once().await });
    let entered = tokio::time::timeout(HTTP_BUDGET, async {
        loop {
            let native_started = !source.pending.handles.lock().unwrap().is_empty();
            let record = state.get_record(HEALTHY_DEVICE).await.unwrap();
            if let Some(record) = record.filter(|_| native_started) {
                break record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    let a_before_http = state.inner.retry.snapshot(PENDING_DEVICE);
    let b_handle = source.healthy.handles.lock().unwrap().last().cloned();
    let mut proxy = if let Some(handle) = &b_handle {
        TcpStream::connect(("127.0.0.1", handle.userspace_port.unwrap()))
            .await
            .ok()
    } else {
        None
    };
    let proxy_started = if let Some(proxy) = &mut proxy {
        proxy.write_all(&[0]).await.is_ok()
    } else {
        false
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(HTTP_BUDGET)
        .build()
        .unwrap();
    let query_start = Instant::now();
    let query = async {
        let response = client
            .get(format!("http://127.0.0.1:{port}/tunnel/{HEALTHY_DEVICE}"))
            .send()
            .await?;
        let status = response.status();
        let body = serde_json::from_str::<TunnelRecord>(&response.text().await?)?;
        Ok::<_, anyhow::Error>((status, body))
    }
    .await;
    let query_time = query_start.elapsed();
    let a_before_stop = state.inner.retry.snapshot(PENDING_DEVICE);
    let stop_start = Instant::now();
    let stop = async {
        let response = client
            .delete(format!("http://127.0.0.1:{port}/tunnel/{HEALTHY_DEVICE}"))
            .send()
            .await?;
        let status = response.status();
        let body = serde_json::from_str::<serde_json::Value>(&response.text().await?)?;
        Ok::<_, anyhow::Error>((status, body))
    }
    .await;
    let stop_time = stop_start.elapsed();
    drop(client);
    let proxy_eof = if let Some(proxy) = &mut proxy {
        matches!(tokio::time::timeout(HTTP_BUDGET, proxy.read_u8()).await, Ok(Err(error)) if matches!(error.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset))
    } else {
        false
    };
    drop(proxy);
    let scan_result = tokio::time::timeout(OBSERVE, &mut scan).await;
    if scan_result.is_err() {
        scan.abort();
        let _ = scan.await;
    }
    let a_after = state.inner.retry.snapshot(PENDING_DEVICE);
    let b_absent = state.get_record(HEALTHY_DEVICE).await.unwrap().is_none();
    let a_unpublished = state.get_record(PENDING_DEVICE).await.unwrap().is_none();
    let owners = state.inner.native.snapshot();
    let handles = [
        source.pending.handles.lock().unwrap().clone(),
        source.healthy.handles.lock().unwrap().clone(),
    ]
    .concat();
    state.notify_shutdown();
    let drain = state.drain_native(OBSERVE).await;
    state.clear().await;
    server.abort();
    let _ = server.await;
    let a_resources = recovered(&source.pending, expected).await;
    let b_resources = recovered(&source.healthy, expected).await;
    assert!(
        drain.is_ok()
            && a_resources
            && b_resources
            && state.inner.native.snapshot().closed_and_empty()
    );
    assert!(handles
        .iter()
        .all(|handle| handle.shutdown_status().is_ok()));
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    let entered = entered.expect("healthy native B did not publish while A was pending");
    let before = a_before_http.unwrap();
    assert_eq!(before.phase, reconnect::Phase::Connecting);
    assert_eq!(before.attempts, 1);
    assert_eq!(a_before_stop.unwrap().phase, reconnect::Phase::Connecting);
    let (status, record) = query.expect("B query waited behind A's pending Source");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(&record, &*entered);
    assert!(query_time < HTTP_BUDGET);
    let (status, reply) = stop.expect("B stop waited behind A's pending Source");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["data"], true);
    assert!(stop_time < HTTP_BUDGET);
    assert!(proxy_started && proxy_eof && b_absent && a_unpublished);
    scan_result
        .expect("A retry episode failed to end at its own total")
        .unwrap()
        .unwrap();
    let after = a_after.unwrap();
    assert_eq!(
        (after.phase, after.attempts),
        (reconnect::Phase::Exhausted, 1)
    );
    assert_eq!(after.failures, 1);
    assert_eq!(after.last_class, Some(ios_core::ErrorClass::Timeout));
    assert_eq!(after.deadline, Some(before.first_attempt + TOTAL));
    assert!(Instant::now() >= after.deadline.unwrap());
    assert_eq!(
        (
            source.pending.connect_calls.load(Ordering::SeqCst),
            source.healthy.connect_calls.load(Ordering::SeqCst)
        ),
        (1, 1)
    );
    assert_eq!(
        (owners.active, owners.retired, owners.native_handles),
        (0, 2, 2)
    );
}

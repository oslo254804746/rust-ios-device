use super::*;
use crate::cmd::tunnel::future_storage::Dynamic;

struct LimitedSource {
    base: Arc<FakeSource>,
    usb: AtomicBool,
    paired: AtomicBool,
}

fn construct<'a, F: std::future::Future + Send + 'a>(
    future: F,
    bank: &Arc<Semaphore>,
    deny: bool,
) -> Result<Dynamic<'a, F::Output>> {
    // Temporary unrelated usage leaves the ACTUAL concrete F one byte short.
    // It is released when the constructor returns, before USB work resumes.
    let _other_usage = deny.then(|| {
        let bytes = std::mem::size_of_val(&future);
        assert!(bytes > 0);
        bank.clone()
            .try_acquire_many_owned((bank.available_permits() - bytes + 1) as u32)
            .unwrap()
    });
    Dynamic::new(future, bank).map_err(|error| error.context(source::DiscoveryAdmission))
}
impl Source for LimitedSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.base.connect(udid, route, mode)
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.base.usb_devices()
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        self.base.paired_devices()
    }
    fn usb_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> Result<Dynamic<'a, UsbSnapshot>> {
        construct(
            async move { self.usb_devices().await },
            bank,
            self.usb.load(Ordering::SeqCst),
        )
    }
    fn paired_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> Result<Dynamic<'a, Result<Vec<PairedMobdev2Device>>>> {
        construct(
            async move { self.paired_devices().await },
            bank,
            self.paired.load(Ordering::SeqCst),
        )
    }
}
fn fixture() -> (Arc<LimitedSource>, TunnelAgentState) {
    let source = Arc::new(LimitedSource {
        base: FakeSource::new(),
        usb: AtomicBool::new(false),
        paired: AtomicBool::new(false),
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    (source, state)
}

#[tokio::test]
async fn discovery_future_usb_scan_constructor_refusal_preserves_old_publication_and_backoff() {
    let (source, state) = fixture();
    let mut prepared_direct_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_direct_1 = state.prepare_cutover::<1>().unwrap();

    start(state.clone(), A).await.unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Mobdev2("192.0.2.9".into()),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    drop(ticket);
    let previous = state.inner.failed_attempts.read().await[B].clone();
    let counts = (
        source.base.usb_calls.load(Ordering::SeqCst),
        source.base.paired_calls.load(Ordering::SeqCst),
    );
    source.usb.store(true, Ordering::SeqCst);
    let error = state.refresh_once().await.unwrap_err();
    assert!(error.is::<source::DiscoveryAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(
        (
            source.base.usb_calls.load(Ordering::SeqCst),
            source.base.paired_calls.load(Ordering::SeqCst)
        ),
        counts
    );
    assert!(state.inner.tunnels.read().await[A]
        .publication
        .ptr_eq(&publication));
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[B].version,
        &previous.version
    ));
    assert_eq!(source.base.count(A), 1);
    assert!(!state.is_ready().await);
    let bank = state.inner.global_metadata_bytes.clone();
    drop((error, publication, previous));
    state.notify_shutdown(&mut prepared_direct_0);
    drop(prepared_direct_0);
    state.clear(&mut prepared_direct_1).await;
    drop(prepared_direct_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn discovery_future_paired_constructor_refusal_keeps_paid_usb_window_and_unknown_backoff() {
    let (source, state) = fixture();
    let mut prepared_direct_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_direct_1 = state.prepare_cutover::<1>().unwrap();

    source
        .base
        .paired
        .lock()
        .unwrap()
        .push(PairedMobdev2Device {
            udid: B.into(),
            host: "192.0.2.9".into(),
        });
    source
        .base
        .script(B, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    state.refresh_once().await.unwrap();
    let previous = state.inner.failed_attempts.read().await[B].clone();
    source.base.usb.lock().unwrap().push(A.into());
    source.paired.store(true, Ordering::SeqCst);
    let error = state.refresh_once().await.unwrap_err();
    assert!(error.is::<source::DiscoveryAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(state.get_record(A).await.unwrap().is_some());
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[B].version,
        &previous.version
    ));
    assert_eq!(source.base.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!((source.base.count(A), source.base.count(B)), (1, 1));
    assert!(state.is_ready().await);
    let bank = state.inner.global_metadata_bytes.clone();
    drop((error, previous));
    state.notify_shutdown(&mut prepared_direct_0);
    drop(prepared_direct_0);
    state.clear(&mut prepared_direct_1).await;
    drop(prepared_direct_1);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

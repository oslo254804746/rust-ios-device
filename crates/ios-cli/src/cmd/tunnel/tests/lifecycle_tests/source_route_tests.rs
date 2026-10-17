use super::*;
use crate::cmd::tunnel::future_storage::Dynamic;

struct BorrowedSource {
    host: String,
    pointer: AtomicUsize,
    dropped: AtomicUsize,
    gate: Semaphore,
    original: StdMutex<Option<anyhow::Error>>,
}
struct Borrow<'a> {
    host: &'a str,
    source: &'a BorrowedSource,
    bank: &'a Arc<Semaphore>,
}
impl Drop for Borrow<'_> {
    fn drop(&mut self) {
        assert_eq!(
            self.host.as_ptr() as usize,
            self.source.pointer.load(Ordering::SeqCst)
        );
        assert_eq!(self.host, self.source.host);
        assert!(self.bank.available_permits() <= MAX_GLOBAL_KNOWN_METADATA_BYTES - self.host.len());
        self.source.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl Source for BorrowedSource {
    fn connect<'a>(
        &'a self,
        _: &'a str,
        _: Route,
        _: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async { panic!("borrowed Source must never clone into a legacy fixture route") })
    }
    fn connect_owned<'a>(
        &'a self,
        _: &'a str,
        route: RouteRef<'a>,
        _: TunMode,
        _: &'a dyn ios_core::TunnelOwner,
        bank: &'a Arc<Semaphore>,
    ) -> Result<Dynamic<'a, Result<ManagedDevice>>> {
        Dynamic::new(
            async move {
                let Some(host) = route.host() else {
                    return Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into());
                };
                self.pointer.store(host.as_ptr() as usize, Ordering::SeqCst);
                let _borrow = Borrow {
                    host,
                    source: self,
                    bank,
                };
                self.gate.acquire().await.unwrap().forget();
                Err(self.original.lock().unwrap().take().unwrap())
            },
            bank,
        )
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async { UsbSnapshot::Devices(Vec::new()) })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        Box::pin(async {
            Ok(vec![PairedMobdev2Device {
                udid: A.into(),
                host: self.host.clone(),
            }])
        })
    }
}
fn fixture() -> (Arc<BorrowedSource>, TunnelAgentState, usize) {
    let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let source = Arc::new(BorrowedSource {
        host: "h".repeat(173),
        pointer: AtomicUsize::new(0),
        dropped: AtomicUsize::new(0),
        gate: Semaphore::new(0),
        original: StdMutex::new(Some(original)),
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    (source, state, pointer)
}

#[tokio::test]
async fn source_route_pending_manual_and_fallback_borrow_actual_paid_observation_until_caller_drop()
{
    for family in [
        Some(StartTunnelConnectionType::Usb),
        Some(StartTunnelConnectionType::Wifi),
        None,
    ] {
        let (source, state, _) = fixture();
        // Automatic mobdev2 desired state keeps no discovery host filter;
        // explicit Usb/Wifi keep it in both desired and observed destinations.
        let expected_copies = if family.is_none() { 1 } else { 2 };
        let bank = state.inner.global_metadata_bytes.clone();
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let captured = seen.clone();
        let charged = bank.clone();
        let hook = metadata_vec::on_allocation(move |layout, pointer| {
            if layout.size() == 173 {
                assert!(charged.available_permits() <= MAX_GLOBAL_KNOWN_METADATA_BYTES - 173);
                captured.lock().unwrap().push(pointer);
            }
        });
        let mut work = Box::pin(state.ensure_tunnel(A, Some(&source.host), family));
        assert!(futures_util::poll!(&mut work).is_pending());
        let pointer = source.pointer.load(Ordering::SeqCst);
        assert_ne!(pointer, source.host.as_ptr() as usize);
        assert!(
            seen.lock().unwrap().contains(&pointer),
            "Source did not borrow the admitted original destination"
        );
        assert_eq!(
            seen.lock().unwrap().len(),
            expected_copies,
            "only required desired/observed host destinations are copied"
        );
        drop(work);
        drop(hook);
        assert_eq!(source.dropped.load(Ordering::SeqCst), 1);
        assert!(state.inner.tunnels.read().await.is_empty());
        assert_eq!(state.inner.native.snapshot().occupied(), 0);
        state.notify_shutdown();
        state.clear().await;
        drop(state);
        assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    }
}

#[tokio::test]
async fn source_route_original_candidate_host_survives_same_error_receipt_after_manager_drop() {
    let (source, state, original) = fixture();
    let bank = state.inner.global_metadata_bytes.clone();
    let mut work = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut work).is_pending());
    let pointer = source.pointer.load(Ordering::SeqCst);
    assert_ne!(pointer, 0);
    source.gate.add_permits(1);
    work.await.unwrap();
    assert_eq!(source.dropped.load(Ordering::SeqCst), 1);
    let receipt = state.inner.retry.receipt(A).unwrap();
    let context = receipt
        .primary()
        .downcast_ref::<candidate_input::SourceInputContext>()
        .unwrap();
    assert_eq!(
        context
            .input
            .source_route()
            .unwrap()
            .host()
            .unwrap()
            .as_ptr() as usize,
        pointer
    );
    assert_eq!(
        receipt.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        original
    );
    let weak = Arc::downgrade(&context.input);
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert!(weak.upgrade().is_some());
    assert!(bank.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    assert_eq!(
        receipt
            .primary()
            .downcast_ref::<candidate_input::SourceInputContext>()
            .unwrap()
            .input
            .source_route()
            .unwrap()
            .host()
            .unwrap()
            .as_ptr() as usize,
        pointer
    );
    drop(receipt);
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

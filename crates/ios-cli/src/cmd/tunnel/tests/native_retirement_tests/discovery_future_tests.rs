use super::*;
use crate::cmd::tunnel::future_storage::Dynamic;

struct RefusingUsb {
    base: Arc<LocalSource>,
    paired_calls: AtomicUsize,
}
impl Source for RefusingUsb {
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
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async {
            self.paired_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        })
    }
    fn usb_devices_owned<'a>(
        &'a self,
        bank: &'a Arc<Semaphore>,
    ) -> Result<Dynamic<'a, UsbSnapshot>> {
        let future = async move { self.usb_devices().await };
        let bytes = std::mem::size_of_val(&future);
        assert!(bytes > 0);
        let _other_usage = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - bytes + 1) as u32)
            .unwrap();
        Dynamic::new(future, bank).map_err(|error| error.context(source::DiscoveryAdmission))
    }
}

#[tokio::test]
async fn discovery_future_post_connect_usb_refusal_never_falls_back_and_joins_original_native() {
    let expected = baseline();
    let source = Arc::new(RefusingUsb {
        base: LocalSource::new(),
        paired_calls: AtomicUsize::new(0),
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        OBSERVE,
    );
    let mut terminal_inputs = PreparedTerminal::new(&state);
    let error = state.ensure_tunnel(DEVICE, None, None).await.unwrap_err();
    let empty = state.inner.tunnels.read().await.is_empty();
    let native = source.base.handle();
    assert!(finish(&state, &source.base, expected, &mut terminal_inputs).await);
    drop(terminal_inputs);
    assert!(native.shutdown_status().is_ok());
    assert!(empty);
    assert!(error.is::<source::DiscoveryAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.base.connect_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.base.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    let bank = state.inner.global_metadata_bytes.clone();
    drop((error, native, state, source));
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

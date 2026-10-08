//! Snapshot rejection through actual native publication/retirement owners.
use super::*;
use futures_util::FutureExt;

struct SizedSource {
    base: Arc<LocalSource>,
    oversized_usb: bool,
    oversized_paired: bool,
}

impl Source for SizedSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.base.connect(udid, route, mode)
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        Box::pin(async move {
            let snapshot = self.base.usb_devices().await;
            match snapshot {
                UsbSnapshot::Devices(mut devices) if self.oversized_usb => {
                    // Matching native row is first; the later tail must still
                    // be validated before matching or publication succeeds.
                    devices.push(source::UsbDevice {
                        udid: "x".repeat(4097),
                        device_id: 7,
                        usb: true,
                    });
                    UsbSnapshot::Devices(devices)
                }
                snapshot => snapshot,
            }
        })
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async move {
            Ok(if self.oversized_paired {
                vec![ios_core::device::PairedMobdev2Device {
                    udid: "x".repeat(4097),
                    host: "192.0.2.8".into(),
                }]
            } else {
                Vec::new()
            })
        })
    }
}

#[tokio::test]
async fn admission_native_usb_publication_checks_oversized_tail_and_keeps_old_arc() {
    let expected = baseline();
    let base = LocalSource::new();
    let source = Arc::new(SizedSource {
        base: base.clone(),
        oversized_usb: true,
        oversized_paired: false,
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
    );
    publish(&state, Route::Wifi(None)).await.unwrap();
    let old = base.handle();
    let publication = state.inner.tunnels.read().await[DEVICE].publication.clone();
    let result = publish(&state, Route::Usbmux).await;
    let new = base.handle();
    let old_alive = old.is_alive();
    let new_stopped = !new.is_alive();
    let unchanged = state
        .inner
        .tunnels
        .read()
        .await
        .get(DEVICE)
        .is_some_and(|row| Arc::ptr_eq(&row.publication, &publication));
    let recovery = finish(&state, &base, expected).await;
    assert!(recovery && old.shutdown_status().is_ok() && new.shutdown_status().is_ok());
    let error = result.unwrap_err();
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-publication",
        &error,
    )
    .unwrap();
    let body: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(body["error"]["class"], "resource_limit");
    assert!(old_alive && new_stopped && unchanged);
    assert_eq!(base.connect_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn admission_stale_source_success_with_invalid_paired_metadata_stays_quiet() {
    let expected = baseline();
    let base = LocalSource::new();
    let source = Arc::new(SizedSource {
        base: base.clone(),
        oversized_usb: false,
        oversized_paired: true,
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        OBSERVE,
    );
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = base.handle();
    let control = state.clone();
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        control.clear().now_or_never().unwrap();
    }));
    let result = state.refresh_once().await;
    let stopped = !native.is_alive();
    let not_ready = !state.is_ready().await;
    let unused = state.inner.paired_after_first_check.lock().unwrap().take();
    drop(unused);
    let recovery = finish(&state, &base, expected).await;
    assert!(recovery && native.shutdown_status().is_ok());
    result.unwrap();
    assert!(stopped && not_ready);
    assert_eq!(base.connect_calls.load(Ordering::SeqCst), 1);
}

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

struct Trap;
impl TunnelOwner for Trap {
    fn reserve(&self) -> Result<Box<dyn ios_core::TunnelReservation + '_>, ios_core::CoreError> {
        panic!("unpolled NativeSource must not reserve or contact a device")
    }
}

#[test]
fn source_future_native_actual_constructor_is_admitted_before_allocation_without_device_poll() {
    let source = NativeSource;
    let owner = Trap;
    for route in [
        Route::Usbmux,
        Route::Usb(None),
        Route::Wifi(Some("synthetic-host".into())),
        Route::Mobdev2("synthetic-host".into()),
    ] {
        let bank = Arc::new(Semaphore::new(
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let charged = bank.clone();
        let future = super::super::metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                let mut seen = captured.lock().unwrap();
                let prior: usize = seen.iter().map(|(size, _)| size).sum();
                assert_eq!(
                    charged.available_permits(),
                    super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES - prior - layout.size()
                );
                seen.push((layout.size(), pointer));
            },
            || {
                source
                    .connect_owned(
                        "synthetic-source-no-device",
                        (&route).into(),
                        TunMode::Userspace,
                        &owner,
                        &bank,
                    )
                    .unwrap()
            },
        );
        assert_eq!(seen.lock().unwrap().len(), 2);
        let body_bytes = seen.lock().unwrap()[0].0;
        assert_eq!(
            body_bytes,
            std::mem::size_of::<ios_core::OwnedConnectedDevice>()
        );
        let (bytes, pointer) = seen.lock().unwrap()[1];
        assert!(bytes > 0);
        assert_eq!(future.data_pointer(), pointer);
        drop(future); // Intentionally NEVER poll production NativeSource.
        assert_eq!(
            bank.available_permits(),
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
        );
        for (available, expected_allocations) in
            [(0, 0), (body_bytes - 1, 0), (body_bytes + bytes - 1, 1)]
        {
            let limited = Arc::new(Semaphore::new(available));
            let allocated = Arc::new(AtomicUsize::new(0));
            let captured = allocated.clone();
            let error = super::super::metadata_vec::with_allocation_hook(
                move |_, _| {
                    captured.fetch_add(1, Ordering::SeqCst);
                },
                || {
                    source
                        .connect_owned(
                            "synthetic-source-no-device",
                            (&route).into(),
                            TunMode::Userspace,
                            &owner,
                            &limited,
                        )
                        .err()
                        .unwrap()
                },
            );
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(allocated.load(Ordering::SeqCst), expected_allocations);
            assert_eq!(limited.available_permits(), available);
        }
    }
}

#[tokio::test]
async fn returned_device_box_native_legacy_fixture_cannot_contact_a_device() {
    let error = NativeSource
        .connect("synthetic", Route::Usbmux, TunMode::Userspace)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("device IO disabled"));
}

#[test]
fn refresh_future_actual_native_constructor_prepays_original_box_without_device_poll() {
    use ios_core::device::ExistingRsdRoute;
    use std::net::{Ipv6Addr, SocketAddrV6};
    let source = NativeSource;
    let endpoint = SocketAddrV6::new(Ipv6Addr::LOCALHOST, 12345, 0, 0);
    for route in [
        ExistingRsdRoute::direct(endpoint).unwrap(),
        ExistingRsdRoute::via_userspace_proxy(endpoint, 23456).unwrap(),
    ] {
        let bank = Arc::new(Semaphore::new(
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        ));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let charged = bank.clone();
        let future = super::super::metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(
                    charged.available_permits(),
                    super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES - layout.size()
                );
                captured.lock().unwrap().push((layout.size(), pointer));
            },
            || {
                source
                    .refresh_existing_rsd(route, tokio::time::Instant::now(), &bank)
                    .unwrap()
            },
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        let (bytes, pointer) = seen.lock().unwrap()[0];
        assert!(bytes > 0);
        assert_eq!(future.data_pointer(), pointer);
        drop(future); // Never poll production NativeSource or open a socket.
        assert_eq!(
            bank.available_permits(),
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
        );
        for available in [0, bytes - 1] {
            let bank = Arc::new(Semaphore::new(available));
            let error = super::super::metadata_vec::with_allocation_hook(
                |_, _| panic!("refused refresh Source frame allocated"),
                || {
                    source
                        .refresh_existing_rsd(route, tokio::time::Instant::now(), &bank)
                        .err()
                        .unwrap()
                },
            );
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), available);
        }
    }
}

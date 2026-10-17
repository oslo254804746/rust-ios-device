use super::*;
use std::sync::Mutex;

fn admitted<'a, T>(
    bank: &Arc<Semaphore>,
    construct: impl FnOnce() -> anyhow::Result<super::super::future_storage::Dynamic<'a, T>>,
) -> usize {
    let available = bank.available_permits();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let future = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), available - layout.size());
            captured.lock().unwrap().push((layout.size(), pointer));
        },
        || construct().unwrap(),
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let (bytes, pointer) = seen.lock().unwrap()[0];
    assert!(bytes > 0);
    assert_eq!(future.data_pointer(), pointer);
    drop(future); // NEVER poll production NativeSource or enumerate any device.
    assert_eq!(bank.available_permits(), available);
    bytes
}

fn refused<'a, T>(
    bank: &Arc<Semaphore>,
    construct: impl FnOnce() -> anyhow::Result<super::super::future_storage::Dynamic<'a, T>>,
) {
    let available = bank.available_permits();
    let error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("refused discovery frame allocated"),
        || construct().err().unwrap(),
    );
    assert!(error.is::<DiscoveryAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), available);
}

#[test]
fn discovery_future_native_usb_original_constructor_prepays_box_never_polls() {
    let source = NativeSource;
    let bank = Arc::new(Semaphore::new(
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
    ));
    let bytes = admitted(&bank, || source.usb_devices_owned(&bank));
    for available in [0, bytes - 1] {
        let bank = Arc::new(Semaphore::new(available));
        refused(&bank, || source.usb_devices_owned(&bank));
    }
}

#[test]
fn discovery_future_native_paired_original_constructor_prepays_box_never_polls() {
    let source = NativeSource;
    let bank = Arc::new(Semaphore::new(
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
    ));
    let bytes = admitted(&bank, || source.paired_devices_owned(&bank));
    for available in [0, bytes - 1] {
        let bank = Arc::new(Semaphore::new(available));
        refused(&bank, || source.paired_devices_owned(&bank));
    }
}

#[tokio::test]
async fn discovery_future_native_legacy_fixture_cannot_contact_usbmux_or_mdns() {
    let UsbSnapshot::ConnectError(error) = NativeSource.usb_devices().await else {
        panic!("legacy NativeSource fixture must refuse discovery")
    };
    assert!(error.to_string().contains("device IO disabled"));
    let error = NativeSource.paired_devices().await.err().unwrap();
    assert!(error.to_string().contains("device IO disabled"));
}

use super::*;

#[test]
fn global_metadata_usb_paired_and_publication_use_one_atomic_known_byte_pool() {
    let devices = vec![usb(short_reserved(1024))];
    let targets = vec![PairedMobdev2Device {
        udid: short_reserved(2048),
        host: short_reserved(4096),
    }];
    let usb_bytes =
        devices.capacity() * std::mem::size_of::<UsbDevice>() + devices[0].udid.capacity();
    let paired_bytes = targets.capacity() * std::mem::size_of::<PairedMobdev2Device>()
        + targets[0].udid.capacity()
        + targets[0].host.capacity();
    let bank = Arc::new(Semaphore::new(usb_bytes + paired_bytes));
    let usb_charge = retain_usb_snapshot(&bank, &devices).unwrap();
    let publication = known_metadata_charge(&bank, 1).unwrap();
    let error = retain_paired_snapshot(&bank, &targets).unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), paired_bytes - 1);
    drop(publication);
    let paired_charge = retain_paired_snapshot(&bank, &targets).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(usb_charge);
    assert_eq!(bank.available_permits(), usb_bytes);
    drop(paired_charge);
    assert_eq!(bank.available_permits(), usb_bytes + paired_bytes);
}

fn short_reserved(capacity: usize) -> String {
    let mut value = String::with_capacity(capacity);
    value.push_str("synthetic");
    value
}

fn usb(udid: String) -> UsbDevice {
    UsbDevice {
        udid,
        device_id: 7,
        usb: true,
    }
}

#[test]
fn snapshot_capacity_short_reserved_strings_cannot_bypass_usb_or_paired_limit() {
    let devices = vec![usb(short_reserved(MAX_RAW_TEXT_BYTES + 1))];
    assert_eq!(devices[0].udid.len(), 9);
    let error = usb_snapshot(&devices).unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let targets = vec![PairedMobdev2Device {
        udid: short_reserved(MAX_RAW_TEXT_BYTES / 2),
        host: short_reserved(MAX_RAW_TEXT_BYTES / 2 + 1),
    }];
    let error = paired_snapshot(&targets).unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(targets[0].udid.capacity() + targets[0].host.capacity() > MAX_RAW_TEXT_BYTES);
}

#[test]
fn snapshot_capacity_exact_text_bound_stays_admissible_with_original_buffers_intact() {
    let devices = vec![usb(short_reserved(MAX_RAW_TEXT_BYTES))];
    let buffer = devices[0].udid.as_ptr();
    usb_snapshot(&devices).unwrap();
    assert_eq!(devices[0].udid.as_ptr(), buffer);
    assert_eq!(devices[0].udid.capacity(), MAX_RAW_TEXT_BYTES);
    let targets = vec![PairedMobdev2Device {
        udid: short_reserved(MAX_RAW_TEXT_BYTES / 2),
        host: short_reserved(MAX_RAW_TEXT_BYTES / 2),
    }];
    paired_snapshot(&targets).unwrap();
    assert_eq!(
        targets[0].udid.capacity() + targets[0].host.capacity(),
        MAX_RAW_TEXT_BYTES
    );
}

#[test]
fn snapshot_capacity_unused_vector_storage_and_overflow_fail_before_selection() {
    let devices: Vec<UsbDevice> =
        Vec::with_capacity(MAX_RAW_STORAGE_BYTES / std::mem::size_of::<UsbDevice>() + 1);
    assert!(devices.is_empty());
    let error = usb_snapshot(&devices).unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let targets: Vec<PairedMobdev2Device> =
        Vec::with_capacity(MAX_RAW_STORAGE_BYTES / std::mem::size_of::<PairedMobdev2Device>() + 1);
    assert!(paired_snapshot(&targets).is_err());
    assert!(snapshot_storage(usize::MAX, 2, 0).is_err());
    assert!(snapshot_storage(1, 1, usize::MAX).is_err());
}

#[test]
fn snapshot_capacity_vector_and_text_are_admitted_together() {
    let capacity = MAX_RAW_STORAGE_BYTES / std::mem::size_of::<UsbDevice>();
    let mut devices = Vec::with_capacity(capacity);
    devices.push(usb(short_reserved(1024)));
    assert!(devices.capacity() * std::mem::size_of::<UsbDevice>() <= MAX_RAW_STORAGE_BYTES);
    assert!(usb_snapshot(&devices).is_err());
}

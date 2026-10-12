use super::*;

fn row(key: &str, device_id: u32, usb: bool) -> UsbDevice {
    UsbDevice {
        udid: key.to_owned(),
        device_id,
        usb,
    }
}

#[test]
fn selection_bytes_preserves_first_identity_order_first_usb_and_last_network() {
    let bank = Arc::new(Semaphore::new(4096));
    let devices = vec![
        row("z", 1, false),
        row("a", 2, false),
        row("z", 3, true),
        row("a", 4, false),
        row("z", 5, true),
        row("b", 6, true),
    ];
    let (selected, charge) = selected_devices(devices, &bank).unwrap();
    assert_eq!(
        selected
            .iter()
            .map(|device| (device.udid.as_str(), device.device_id, device.usb))
            .collect::<Vec<_>>(),
        vec![("z", 3, true), ("a", 4, false), ("b", 6, true)]
    );
    assert_eq!(
        bank.available_permits(),
        4096 - selected.capacity() * std::mem::size_of::<UsbDevice>()
    );
    drop(selected);
    drop(charge);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn selection_bytes_moves_original_buffer_without_key_clone_or_capacity_loss() {
    let bank = Arc::new(Semaphore::new(4096));
    let mut key = String::with_capacity(8192);
    key.push_str("synthetic-original-selection");
    let original = key.as_ptr();
    let capacity = key.capacity();
    let mut devices = Vec::with_capacity(64);
    devices.push(UsbDevice {
        udid: key,
        device_id: 7,
        usb: true,
    });
    let (selected, charge) = selected_devices(devices, &bank).unwrap();
    assert_eq!(selected[0].udid.as_ptr(), original);
    assert_eq!(selected[0].udid.capacity(), capacity);
    assert_eq!(
        selected.capacity(),
        1,
        "output does not inherit vacant raw row capacity"
    );
    assert_eq!(
        bank.available_permits(),
        4096 - std::mem::size_of::<UsbDevice>()
    );
    drop(selected);
    drop(charge);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn selection_bytes_exact_peak_admission_and_one_byte_refusal_refund_all_scratch() {
    let index_bytes = 3 * std::mem::size_of::<(usize, usize)>();
    let output_bytes = 2 * std::mem::size_of::<UsbDevice>();
    for limit in [
        index_bytes - 1,
        index_bytes + output_bytes - 1,
        index_bytes + output_bytes,
    ] {
        let bank = Arc::new(Semaphore::new(limit));
        let result = selected_devices(
            vec![row("b", 1, false), row("a", 2, true), row("b", 3, true)],
            &bank,
        );
        if limit < index_bytes + output_bytes {
            let error = result.err().expect("unadmitted selection backing accepted");
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), limit);
        } else {
            let (devices, charge) = result.unwrap();
            assert_eq!(devices.len(), 2);
            assert_eq!(bank.available_permits(), index_bytes);
            drop(devices);
            drop(charge);
            assert_eq!(bank.available_permits(), limit);
        }
    }
}

#[test]
fn selection_bytes_empty_input_with_vacant_capacity_needs_no_selection_storage() {
    let bank = Arc::new(Semaphore::new(0));
    let devices = Vec::with_capacity(128);
    let (selected, charge) = selected_devices(devices, &bank).unwrap();
    assert!(selected.is_empty());
    assert_eq!(selected.capacity(), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(charge);
}

#[test]
fn selection_bytes_matches_original_selection_for_many_duplicate_orders() {
    let bank = Arc::new(Semaphore::new(64 * 1024));
    let mut seed = 7_u32;
    for length in 0..128 {
        let mut original = Vec::<(String, u32, bool)>::new();
        let mut devices = Vec::new();
        for index in 0..length {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = format!("synthetic-{}", seed % 17);
            let usb = seed & 32 != 0;
            if let Some(old) = original
                .iter_mut()
                .find(|(identity, _, _)| identity == &key)
            {
                if !old.2 {
                    *old = (key.clone(), index, usb);
                }
            } else {
                original.push((key.clone(), index, usb));
            }
            devices.push(row(&key, index, usb));
        }
        let (selected, charge) = selected_devices(devices, &bank).unwrap();
        let actual = selected
            .iter()
            .map(|device| (device.udid.clone(), device.device_id, device.usb))
            .collect::<Vec<_>>();
        assert_eq!(actual, original);
        drop(selected);
        drop(charge);
        assert_eq!(bank.available_permits(), 64 * 1024);
    }
}

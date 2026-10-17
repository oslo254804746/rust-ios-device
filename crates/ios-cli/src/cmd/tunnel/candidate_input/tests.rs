use super::*;

#[test]
fn candidate_input_original_capacity_and_shared_last_owner_release() {
    let mut host = String::with_capacity(8192);
    host.push_str("synthetic-host");
    let original = host.as_ptr();
    let uid = "synthetic-input".to_owned();
    let route = Route::Wifi(Some(host));
    let bytes = Input::bytes(&uid, &route);
    let bank = Arc::new(Semaphore::new(bytes));
    let input = Input::owned(uid, route, &bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let Route::Wifi(Some(host)) = &input.route else {
        panic!("route changed")
    };
    assert_eq!(host.as_ptr(), original);
    assert_eq!(host.capacity(), 8192);
    let source = input.source_route().unwrap();
    assert_eq!(source.host().unwrap().as_ptr(), original);
    assert_eq!(source.host().unwrap(), "synthetic-host");
    let clone = input.clone();
    drop(input);
    assert_eq!(bank.available_permits(), 0);
    drop(clone);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn candidate_input_usb_transfers_original_snapshot_charge_without_double_count() {
    let uid = "synthetic-usb-input".to_owned();
    let original = uid.as_ptr();
    let backing = 128;
    let raw_bytes = backing + uid.capacity();
    let supplemental = std::mem::size_of::<Input>() + uid.len();
    let bank = Arc::new(Semaphore::new(raw_bytes + supplemental));
    let mut raw = super::super::admission::known_metadata_charge(&bank, raw_bytes).unwrap();
    let input = Input::usb(uid, &mut raw, &bank).unwrap();
    assert_eq!(input.udid.as_ptr(), original);
    assert_eq!(raw.num_permits(), backing);
    assert_eq!(bank.available_permits(), 0);
    drop(raw);
    assert_eq!(bank.available_permits(), backing);
    drop(input);
    assert_eq!(bank.available_permits(), raw_bytes + supplemental);
}

#[test]
fn candidate_input_one_byte_refusal_and_wrong_bank_leave_existing_snapshot_intact() {
    let uid = "synthetic-input".to_owned();
    let bytes = Input::bytes(&uid, &Route::Usbmux);
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let error = Input::owned(uid.clone(), Route::Usbmux, &bank)
        .err()
        .unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), bytes - 1);
    let other = Arc::new(Semaphore::new(4096));
    let mut raw = super::super::admission::known_metadata_charge(&other, uid.capacity()).unwrap();
    assert!(Input::usb(uid, &mut raw, &bank).is_err());
    assert_eq!(raw.num_permits(), "synthetic-input".len());
    assert_eq!(bank.available_permits(), bytes - 1);
}

#[test]
fn candidate_input_original_error_downcast_and_lease_follow_returned_context() {
    let bank = Arc::new(Semaphore::new(4096));
    let input = Input::copied("synthetic-source", RouteRef::Usbmux, &bank).unwrap();
    let charged = 4096 - bank.available_permits();
    let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionRefused));
    let original = error.downcast_ref::<std::io::Error>().unwrap() as *const _;
    let error = error.context(SourceInputContext {
        input: input.clone(),
    });
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _,
        original
    );
    drop(input);
    assert_eq!(bank.available_permits(), 4096 - charged);
    drop(error);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn candidate_text_original_allocations_prepaid_before_copy_and_shared_last_error_owner() {
    use std::sync::Mutex;
    let uid = "候选-identity";
    let host = "原始-host";
    for route in [
        RouteRef::Usbmux,
        RouteRef::Usb(None),
        RouteRef::Usb(Some("")),
        RouteRef::Usb(Some(host)),
        RouteRef::Wifi(None),
        RouteRef::Wifi(Some("")),
        RouteRef::Wifi(Some(host)),
        RouteRef::Mobdev2(host),
        RouteRef::Mobdev2(""),
    ] {
        let expected_host = route.host();
        let host_len = expected_host.map_or(0, str::len);
        let reserved = std::mem::size_of::<Input>() + host_len + uid.len();
        let total = reserved + uid.len() + host_len;
        let bank = Arc::new(Semaphore::new(total));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let captured = seen.clone();
        let charged = bank.clone();
        let input = super::super::metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(layout.align(), 1);
                let mut seen = captured.lock().unwrap();
                let prior: usize = seen.iter().map(|(size, _)| size).sum();
                assert_eq!(
                    charged.available_permits(),
                    total - reserved - prior - layout.size()
                );
                seen.push((layout.size(), pointer));
            },
            || Input::copied(uid, route, &bank).unwrap(),
        );
        assert_eq!(bank.available_permits(), 0);
        let original = seen.lock().unwrap().clone();
        assert_eq!(original[0], (uid.len(), input.udid.as_ptr() as usize));
        assert_eq!(input.udid.capacity(), uid.len());
        assert_eq!(original.len(), 1 + usize::from(host_len != 0));
        let copied_host = match &input.route {
            Route::Usbmux => None,
            Route::Usb(host) | Route::Wifi(host) => host.as_ref(),
            Route::Mobdev2(host) => Some(host),
        };
        if let Some(copied) = copied_host {
            if host_len != 0 {
                assert_eq!(original[1], (host_len, copied.as_ptr() as usize));
            }
            assert_eq!(copied.capacity(), host_len);
            assert_eq!(copied, expected_host.unwrap());
        } else {
            assert!(expected_host.is_none());
        }
        let held = input.clone();
        let cause = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionAborted));
        let pointer = cause.downcast_ref::<std::io::Error>().unwrap() as *const _;
        let error = cause.context(SourceInputContext { input });
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap() as *const _,
            pointer
        );
        assert_eq!(
            error
                .downcast_ref::<SourceInputContext>()
                .unwrap()
                .input
                .udid
                .as_ptr() as usize,
            original[0].1
        );
        drop(error);
        assert_eq!(bank.available_permits(), 0);
        drop(held);
        assert_eq!(bank.available_permits(), total);
    }
}

#[test]
fn candidate_text_partial_last_byte_refusal_allocates_only_admitted_original_fields() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let uid = "candidate-identity";
    let host = "candidate-host";
    let reserved = std::mem::size_of::<Input>() + host.len() + uid.len();
    for (available, expected_allocations) in [
        (0, 0),
        (reserved - 1, 0),
        (reserved + uid.len() - 1, 0),
        (reserved + uid.len() + host.len() - 1, 1),
    ] {
        let bank = Arc::new(Semaphore::new(available));
        let seen = Arc::new(AtomicUsize::new(0));
        let captured = seen.clone();
        let error = super::super::metadata_vec::with_allocation_hook(
            move |layout, _| {
                assert_eq!(layout, std::alloc::Layout::array::<u8>(uid.len()).unwrap());
                captured.fetch_add(1, Ordering::SeqCst);
            },
            || {
                Input::copied(uid, RouteRef::Mobdev2(host), &bank)
                    .err()
                    .unwrap()
            },
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(seen.load(Ordering::SeqCst), expected_allocations);
        assert_eq!(bank.available_permits(), available);
    }
}

#[test]
fn candidate_text_selector_refusal_precedes_any_original_field_allocation() {
    let oversized = "x".repeat(4097);
    let bank = Arc::new(Semaphore::new(32 * 1024));
    for (uid, route) in [
        (oversized.as_str(), RouteRef::Usbmux),
        ("uid", RouteRef::Wifi(Some(&oversized))),
    ] {
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("invalid candidate selector allocated"),
            || Input::copied(uid, route, &bank).err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), 32 * 1024);
    }
}

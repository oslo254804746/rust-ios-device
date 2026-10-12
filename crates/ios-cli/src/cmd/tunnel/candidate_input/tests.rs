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
    assert_eq!(route_capacity(&source), "synthetic-host".len());
    let clone = input.clone();
    drop(input);
    assert_eq!(bank.available_permits(), 0);
    drop(source);
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
    let input = Input::owned("synthetic-source".into(), Route::Usbmux, &bank).unwrap();
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

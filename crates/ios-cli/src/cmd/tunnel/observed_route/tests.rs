use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn host(route: &Route) -> Option<&str> {
    match route {
        Route::Usbmux => None,
        Route::Usb(host) | Route::Wifi(host) => host.as_deref(),
        Route::Mobdev2(host) => Some(host),
    }
}

#[test]
fn observed_route_owned_text_actual_layout_is_paid_and_outlives_original_input() {
    for original in [
        Route::Usb(Some("usb-雪".into())),
        Route::Wifi(Some("wifi-雪".into())),
        Route::Mobdev2("observed-discovered-host".into()),
    ] {
        let expected = host(&original).unwrap().to_owned();
        let bank = Arc::new(Semaphore::new(expected.len()));
        let charged = bank.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let observed = metadata_vec::with_allocation_hook(
            move |layout, _| {
                assert_eq!(layout.size(), expected.len());
                assert_eq!(charged.available_permits(), 0);
                captured.fetch_add(1, Ordering::SeqCst);
            },
            || Observed::new(&original, None, &bank).unwrap(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(observed.route(), &original);
        assert_ne!(
            host(observed.route()).unwrap().as_ptr(),
            host(&original).unwrap().as_ptr()
        );
        let expected = host(&original).unwrap().to_owned();
        drop(original);
        assert_eq!(host(observed.route()), Some(expected.as_str()));
        assert_eq!(observed.text_bytes(), expected.len());
        assert_eq!(bank.available_permits(), 0);
        drop(observed);
        assert_eq!(bank.available_permits(), expected.len());
    }
}

#[test]
fn observed_route_one_byte_refusal_has_no_allocation_and_none_empty_keep_original_variant() {
    let original = Route::Mobdev2("original-host".into());
    let bytes = host(&original).unwrap().len() - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("observed refusal allocated its destination"),
        || Observed::new(&original, None, &bank).err().unwrap(),
    );
    assert!(error.is::<Admission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(error
        .chain()
        .any(|cause| cause.to_string() == "tunnel global known metadata byte limit exceeded"));
    assert_eq!(bank.available_permits(), bytes);
    let bank = Arc::new(Semaphore::new(0));
    for route in [
        Route::Usbmux,
        Route::Usb(None),
        Route::Wifi(None),
        Route::Usb(Some(String::new())),
        Route::Wifi(Some(String::new())),
        Route::Mobdev2(String::new()),
    ] {
        let observed = Observed::new(&route, None, &bank).unwrap();
        assert_eq!(observed.route(), &route);
        assert_eq!(observed.text_bytes(), 0);
    }
}

#[test]
fn observed_route_original_candidate_input_is_shared_at_zero_quota_until_last_owner() {
    let bank = Arc::new(Semaphore::new(4096));
    let mut text = String::with_capacity(1024);
    text.push_str("observed-original");
    let input = candidate_input::Input::owned("uid".into(), Route::Mobdev2(text), &bank).unwrap();
    let weak = input.downgrade();
    let pointer = host(&input.route).unwrap().as_ptr();
    let source_route = input.source_route().unwrap();
    let paid = 4096 - bank.available_permits();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let observed = metadata_vec::with_allocation_hook(
        |_, _| panic!("shared observed input allocated a destination"),
        || Observed::new(source_route, Some(&input), &bank).unwrap(),
    );
    assert_eq!(host(observed.route()).unwrap().as_ptr(), pointer);
    assert_eq!(observed.text_bytes(), 1024);
    drop(input);
    assert!(weak.upgrade().is_some());
    assert_eq!(bank.available_permits(), 0);
    drop(observed);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        paid - candidate_input::Input::body_bytes()
    );
    drop(weak);
    assert_eq!(bank.available_permits(), paid);
    drop(held);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn observed_route_input_mismatch_never_substitutes_a_different_bank_or_observation() {
    let bank = Arc::new(Semaphore::new(4096));
    let input =
        candidate_input::Input::owned("uid".into(), Route::Mobdev2("original".into()), &bank)
            .unwrap();
    let foreign = Arc::new(Semaphore::new(0));
    let before = bank.available_permits();
    for (route, selected_bank) in [
        (&input.route, &foreign),
        (&Route::Mobdev2("different".into()), &bank),
    ] {
        let error = Observed::new(route, Some(&input), selected_bank)
            .err()
            .unwrap();
        assert!(error.is::<Admission>());
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), before);
    }
    assert_eq!(foreign.available_permits(), 0);
    drop(input);
    assert_eq!(bank.available_permits(), 4096);
}

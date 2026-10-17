use super::*;
use std::sync::Mutex;
use tokio::sync::Semaphore;

#[test]
fn prepaid_selector_all_route_families_use_exact_original_buffers_at_zero_free_bytes() {
    let host = "原-selector-host";
    for original in [
        Route::Usbmux,
        Route::Usb(None),
        Route::Usb(Some(String::new())),
        Route::Usb(Some(host.into())),
        Route::Wifi(None),
        Route::Wifi(Some(host.into())),
        Route::Mobdev2(host.into()),
    ] {
        let length = super::super::candidate_input::route_length(&original);
        let total = 19 + length;
        let bank = Arc::new(Semaphore::new(total));
        let mut storage = super::super::admission::known_metadata_charge(&bank, total).unwrap();
        let mut copier = Copier::new(storage, 19).unwrap();
        let pointers = Arc::new(Mutex::new(Vec::new()));
        let captured = pointers.clone();
        let charged = bank.clone();
        let copied = metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(charged.available_permits(), 0);
                assert_eq!(layout, std::alloc::Layout::array::<u8>(length).unwrap());
                captured.lock().unwrap().push(pointer);
            },
            || copier.route(&original).unwrap(),
        );
        assert_eq!(copied, original);
        storage = copier.finish();
        assert_eq!(
            super::super::candidate_input::route_capacity(&copied),
            length
        );
        assert_eq!(storage.num_permits(), total);
        if length > 0 {
            let pointer = super::super::candidate_input::RouteRef::from(&copied)
                .host()
                .unwrap()
                .as_ptr() as usize;
            assert_eq!(pointers.lock().unwrap().as_slice(), &[pointer]);
        } else {
            assert!(pointers.lock().unwrap().is_empty());
        }
        drop(copied);
        assert_eq!(bank.available_permits(), 0);
        drop(storage);
        assert_eq!(bank.available_permits(), total);
    }
}

#[test]
fn prepaid_selector_partial_credit_refusal_does_not_allocate_or_lose_prior_original_field() {
    let uid = "original-uid";
    let host = "original-host";
    let body = 97;
    let total = body + uid.len() + host.len() - 1;
    let bank = Arc::new(Semaphore::new(total));
    let storage = super::super::admission::known_metadata_charge(&bank, total).unwrap();
    let mut copier = Copier::new(storage, body).unwrap();
    let copied = copier.text(uid).unwrap();
    let pointer = copied.as_ptr();
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("short selector field allocated"),
        || copier.text(host).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(copied.as_ptr(), pointer);
    assert_eq!(copied, uid);
    assert_eq!(copier.retained.num_permits(), body + uid.len());
    assert_eq!(copier.available.num_permits(), host.len() - 1);
    assert_eq!(bank.available_permits(), 0);
    drop(copied);
    drop(copier);
    assert_eq!(bank.available_permits(), total);
}

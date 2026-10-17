use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn authorization_payload_original_pointer_and_last_header_owner_zeroize_before_refund() {
    let mut token = String::from("synthetic-private-token");
    let bytes = 7 + token.len();
    let bank = Arc::new(Semaphore::new(bytes));
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let charged = bank.clone();
    let header = super::super::metadata_vec::with_allocation_hook(
        move |layout, address| {
            assert_eq!(layout.size(), bytes);
            assert_eq!(charged.available_permits(), 0);
            captured.store(address, Ordering::SeqCst);
        },
        || header(&token, &bank).unwrap(),
    );
    assert_eq!(
        header.as_bytes().as_ptr() as usize,
        pointer.load(Ordering::SeqCst)
    );
    assert!(header.is_sensitive());
    assert!(!format!("{header:?}").contains(&token));
    let last = header.clone();
    let mut request = axum::http::Request::new(());
    request
        .headers_mut()
        .insert(axum::http::header::AUTHORIZATION, header);
    assert_eq!(
        request.headers()["authorization"].as_bytes().as_ptr(),
        last.as_bytes().as_ptr()
    );
    token.zeroize();
    drop(token);
    drop(request);
    assert_eq!(bank.available_permits(), 0);
    let drops = Arc::new(AtomicUsize::new(0));
    let captured = drops.clone();
    let charged = bank.clone();
    let _hook = on_zeroized(move |data| {
        assert_eq!(data.len(), bytes);
        assert!(data.iter().all(|byte| *byte == 0));
        assert_eq!(charged.available_permits(), 0);
        captured.fetch_add(1, Ordering::SeqCst);
    });
    drop(last);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn authorization_payload_one_byte_refusal_allocates_nothing_and_preserves_resource_cause() {
    let token = "synthetic-private-token";
    let bytes = 7 + token.len() - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let _hook = on_zeroized(|_| panic!("refused payload was never allocated"));
    let error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("refused header allocated payload"),
        || header(token, &bank).unwrap_err(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(error
        .chain()
        .any(|cause| cause.to_string() == "tunnel global known metadata byte limit exceeded"));
    assert!(!format!("{error:?}").contains(token));
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn authorization_payload_invalid_header_keeps_typed_error_and_zeroizes_before_partial_refund() {
    let token = "synthetic\ninvalid";
    let bytes = 7 + token.len();
    let bank = Arc::new(Semaphore::new(bytes));
    let charged = bank.clone();
    let drops = Arc::new(AtomicUsize::new(0));
    let captured = drops.clone();
    let _hook = on_zeroized(move |data| {
        assert!(data.iter().all(|byte| *byte == 0));
        assert_eq!(charged.available_permits(), 0);
        captured.fetch_add(1, Ordering::SeqCst);
    });
    let error = header(token, &bank).unwrap_err();
    assert!(error.is::<axum::http::header::InvalidHeaderValue>());
    assert!(!format!("{error:?}").contains(token));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

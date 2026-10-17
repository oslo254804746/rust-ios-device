use super::*;
use crate::cmd::tunnel::Route;

#[test]
fn seen_bytes_share_exact_input_and_keep_coordinator_rejected_identity_until_drop() {
    let bank = Arc::new(Semaphore::new(4096));
    let input = Input::owned("rejected".into(), Route::Usbmux, &bank).unwrap();
    let weak = input.downgrade();
    let pointer = input.udid.as_ptr();
    let before = bank.available_permits();
    let mut seen = Seen::new(&bank);
    assert!(seen.insert(&input).unwrap());
    assert_eq!(bank.available_permits(), before - Seen::first_bytes());
    assert!(seen.rows.as_slice()[0].ptr_eq(&input));
    assert_eq!(seen.rows.as_slice()[0].udid.as_ptr(), pointer);
    assert!(!seen.insert(&input).unwrap());
    drop(input);
    assert!(weak.upgrade().is_some());
    assert!(seen.contains("rejected"));
    assert!(!seen.contains("another"));
    drop(seen);
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), 4096 - Input::body_bytes());
    drop(weak);
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn seen_bytes_growth_one_byte_refusal_preserves_original_backing_keys_and_permits() {
    let bank = Arc::new(Semaphore::new(4096));
    let inputs = ["z", "a", "m"].map(|key| Input::owned(key.into(), Route::Usbmux, &bank).unwrap());
    let mut seen = Seen::new(&bank);
    assert!(seen.insert(&inputs[0]).unwrap());
    assert!(seen.insert(&inputs[1]).unwrap());
    let original_backing = seen.rows.as_slice().as_ptr();
    let replacement = 4 * std::mem::size_of::<super::super::candidate_input::Owner>();
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - replacement + 1) as u32)
        .unwrap();
    let error = seen.insert(&inputs[2]).unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(seen.rows.as_slice().as_ptr(), original_backing);
    assert!(seen.contains("a") && seen.contains("z"));
    assert!(!seen.contains("m"));
    assert_eq!(bank.available_permits(), replacement - 1);
    drop(held);
    seen.reserve(1).unwrap();
    assert!(seen.insert_reserved(&inputs[2]));
    assert_eq!(
        seen.rows
            .as_slice()
            .iter()
            .map(|input| input.udid.as_str())
            .collect::<Vec<_>>(),
        ["a", "m", "z"]
    );
    drop(seen);
    drop(inputs);
    assert_eq!(bank.available_permits(), 4096);
}

use super::super::{
    bookkeeping::{DeferredSuccess, SuccessIntent, SuccessKey},
    failure_table::Node,
    lifecycle::Lifecycle,
    metadata_vec, FailureVersion,
};
use super::*;

#[test]
fn outcome_key_usb_last_byte_refusal_leaves_original_raw_snapshot_grant_unsplit() {
    let mut uid = String::with_capacity(2048);
    uid.push_str("original-usb-key");
    let raw_bytes = 128 + uid.capacity();
    let remaining = Input::body_bytes() + uid.len() - 1;
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(raw_bytes + remaining);
    let bank = bank_birth.raw().clone();
    let mut raw = super::super::admission::known_metadata_charge(&bank, raw_bytes).unwrap();
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("refused USB key destination allocated"),
        || Input::usb(uid, &mut raw, &bank).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(raw.num_permits(), raw_bytes);
    assert_eq!(bank.available_permits(), remaining);
    drop(raw);
    assert_eq!(bank.available_permits(), raw_bytes + remaining);
}

#[test]
fn outcome_key_original_empty_destination_fills_at_zero_quota_once_and_outlives_input() {
    for uid in ["", "原-key-identity"] {
        let total = Input::bytes(uid, &Route::Usbmux);
        let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
        let bank = bank_birth.raw().clone();
        let input = Input::copied(uid, RouteRef::Usbmux, &bank).unwrap();
        let pointer = input
            .outcome
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .values
            .as_ptr();
        assert_eq!(bank.available_permits(), 0);
        let (key, fee) = metadata_vec::with_allocation_hook(
            |_, _| panic!("prepaid key fill allocated another destination"),
            || input.outcome_key().unwrap(),
        );
        assert_eq!(key.as_ptr(), pointer);
        assert_eq!(key.capacity(), uid.len());
        assert_eq!(key, uid);
        assert_eq!(fee.num_permits(), uid.len());
        assert!(Arc::ptr_eq(fee.semaphore(), &bank));
        let error = metadata_vec::with_allocation_hook(
            |_, _| panic!("duplicate key allocated"),
            || input.outcome_key().err().unwrap(),
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), 0);
        drop(input);
        assert_eq!(bank.available_permits(), total - uid.len());
        assert_eq!(key.as_ptr(), pointer);
        assert_eq!(key, uid);
        drop(key);
        assert_eq!(bank.available_permits(), total - uid.len());
        drop(fee);
        assert_eq!(bank.available_permits(), total);
    }
}

#[test]
fn outcome_key_node_uses_original_destination_and_duplicate_refusal_keeps_original_node() {
    let total = 8192;
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let input = Input::copied("original-node-key", RouteRef::Usbmux, &bank).unwrap();
    let pointer = input
        .outcome
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .values
        .as_ptr();
    let allocations = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = allocations.clone();
    let node = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            captured.lock().unwrap().push((layout, pointer));
        },
        || Node::input(input.clone(), None, &bank).unwrap(),
    );
    let observed = allocations.lock().unwrap();
    assert_eq!(observed.len(), 3);
    assert_eq!(
        observed[0],
        (
            std::alloc::Layout::new::<Node>(),
            std::ptr::from_ref(&*node) as usize
        )
    );
    let identity_bytes = Node::input_bytes(&Route::Usbmux)
        - std::mem::size_of::<Node>()
        - FailureVersion::body_bytes();
    assert_eq!(observed[1].0.size(), identity_bytes);
    let identity_pointer = std::ptr::from_ref(&*node.row.identity) as usize;
    assert!(observed[1].1 <= identity_pointer && identity_pointer < observed[1].1 + identity_bytes);
    assert_eq!(observed[2].0.size(), FailureVersion::body_bytes());
    let version_pointer = node.row.version.as_ptr() as usize;
    assert!(
        observed[2].1 <= version_pointer && version_pointer <= observed[2].1 + observed[2].0.size()
    );
    assert!(node.row.version.is_from(&bank));
    drop(observed);
    assert_eq!(node.udid.as_ptr(), pointer);
    let before = bank.available_permits();
    let second = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = second.clone();
    let error = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            captured.lock().unwrap().push((layout, pointer));
        },
        || Node::input(input.clone(), None, &bank).err().unwrap(),
    );
    let refused = second.lock().unwrap();
    assert_eq!(refused.len(), 3);
    assert_eq!(refused[0].0, std::alloc::Layout::new::<Node>());
    assert_eq!(refused[1].0.size(), identity_bytes);
    assert_eq!(refused[2].0.size(), FailureVersion::body_bytes());
    assert_ne!(refused[0].1, std::ptr::from_ref(&*node) as usize);
    assert_ne!(refused[1].1, allocations.lock().unwrap()[1].1);
    assert_ne!(refused[2].1, allocations.lock().unwrap()[2].1);
    drop(refused);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), before);
    assert_eq!(node.udid.as_ptr(), pointer);
    assert_eq!(node.udid, "original-node-key");
    drop((error, node, input));
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn outcome_key_actual_success_intent_keeps_original_key_fee_after_input_and_node_drop() {
    let total = 32 * 1024;
    let uid = "success-key-identity";
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    let ticket = lifecycle.ticket(uid).await.unwrap();
    let input = Input::copied(uid, RouteRef::Usbmux, &bank).unwrap();
    let weak = input.downgrade();
    let pointer = input
        .outcome
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .values
        .as_ptr();
    let node = Node::input(input, None, &bank).unwrap();
    let version = node.row.version.clone();
    let (key, input, key_storage) = node.into_inner().into_success_key();
    assert_eq!(key.as_ptr(), pointer);
    let mut success = SuccessIntent {
        udid: key.into(),
        ticket: ticket.clone(),
        success: DeferredSuccess { version },
        _input: input,
        _key_storage: key_storage,
    };
    assert_eq!(
        success._key_storage.as_ref().unwrap().num_permits(),
        uid.len()
    );
    let before = bank.available_permits();
    drop(success._input.take());
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        before + Input::bytes(uid, &Route::Usbmux) - uid.len() - Input::body_bytes()
    );
    let SuccessKey::Owned(key) = &success.udid else {
        panic!("original prepared key missing")
    };
    assert_eq!(key.as_ptr(), pointer);
    assert_eq!(key, uid);
    let other_usage = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(success);
    assert_eq!(
        bank.available_permits(),
        uid.len() + FailureVersion::body_bytes()
    );
    drop(weak);
    drop((other_usage, ticket, lifecycle));
    assert_eq!(bank.available_permits(), total);
}

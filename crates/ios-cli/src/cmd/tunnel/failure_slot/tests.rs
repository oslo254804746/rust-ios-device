use super::*;

#[test]
fn failure_slot_body_refusal_returns_original_count_and_preserves_count_refusal_priority() {
    let body = Slot::body_bytes();
    for available in [0, body - 1] {
        let counts = Counts::fixture(1);
        let bytes = Arc::new(Semaphore::new(available));
        let error = Slot::reserve(&counts, &bytes).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(counts.available_permits(), 1);
        assert_eq!(bytes.available_permits(), available);
    }
    let counts = Counts::fixture(0);
    let bytes = Arc::new(Semaphore::new(body));
    bytes.close();
    let error = Slot::reserve(&counts, &bytes).err().unwrap();
    assert_eq!(error.to_string(), "tunnel failure record capacity");
    assert_eq!(bytes.available_permits(), body);
    assert_eq!(counts.available_permits(), 0);
}

#[test]
fn failure_slot_body_exact_closed_pools_parallel_last_roots_return_one_count_and_actual_bytes() {
    let body = Slot::body_bytes();
    let counts = Counts::fixture(1);
    let bytes = Arc::new(Semaphore::new(body));
    let slot = Slot::reserve(&counts, &bytes).unwrap();
    assert_eq!(slot.num_permits(), 1);
    assert!(slot.counts().ptr_eq(&counts));
    assert_eq!(slot.storage().num_permits(), body);
    assert!(Arc::ptr_eq(slot.storage().semaphore(), &bytes));
    assert_eq!(counts.available_permits(), 0);
    assert_eq!(bytes.available_permits(), 0);
    let pointer = &*slot as *const Slot as usize;
    counts.close();
    bytes.close();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let kept = slot.clone();
            assert!(kept.ptr_eq(&slot));
            let counts = counts.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*kept as *const Slot as usize, pointer);
                assert_eq!(kept.num_permits(), 1);
                assert!(kept.counts().ptr_eq(&counts));
                barrier.wait();
                drop(kept);
            });
        }
        barrier.wait();
        drop(slot);
    });
    assert_eq!(counts.available_permits(), 1);
    assert_eq!(bytes.available_permits(), body);
}

#[tokio::test]
async fn failure_slot_body_actual_manager_reuses_original_receipt_slot_at_closed_zero_bytes() {
    use super::super::{reconnect::SourceFailure, AttemptIdentity, Route, TunnelAgentState};
    let state = TunnelAgentState::new(ios_core::TunMode::Userspace);
    let mut shutdown = state.prepare_cutover::<1>().unwrap();
    let mut clear = state.prepare_cutover::<2>().unwrap();
    let bytes = state.inner.global_metadata_bytes.clone();
    let counts = state.inner.failure_slots.clone();
    let key = "original-count-slot-fixture";
    let ticket = state.inner.lifecycle.ticket(key).await.unwrap();
    let slot = state.reserve_failure_slot(key).await.unwrap();
    let attempt = ticket
        .commit(|| {
            state.inner.retry.start(
                key,
                AttemptIdentity {
                    route: Route::Usbmux,
                    native_id: Some(7),
                },
                &ticket,
                slot.clone(),
            )
        })
        .unwrap();
    let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let result: Result<()> = Err(attempt.source_failed(original));
    attempt.finish(&result);
    let receipt = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<SourceFailure>()
        .unwrap()
        .receipt();
    let held = bytes
        .clone()
        .try_acquire_many_owned(bytes.available_permits() as u32)
        .unwrap();
    bytes.close();
    assert_eq!(bytes.available_permits(), 0);
    let reused = state.reserve_failure_slot(key).await.unwrap();
    assert!(slot.ptr_eq(&reused));
    assert!(slot.ptr_eq(&receipt.slot()));
    assert_eq!(
        counts.available_permits(),
        super::super::MAX_FAILURE_RECORDS - 1
    );
    state.notify_shutdown(&mut shutdown);
    state.clear(&mut clear).await;
    drop((shutdown, clear));
    assert_eq!(
        receipt.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    drop((result, receipt, ticket, state, held));
    assert_eq!(
        bytes.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES - Slot::body_bytes() - Counts::body_bytes()
    );
    drop(slot);
    assert_eq!(
        counts.available_permits(),
        super::super::MAX_FAILURE_RECORDS - 1
    );
    drop(reused);
    assert_eq!(
        counts.available_permits(),
        super::super::MAX_FAILURE_RECORDS
    );
    assert_eq!(
        bytes.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES - Counts::body_bytes()
    );
    let charged = bytes.clone();
    super::super::shared_allocation::after_free(
        move || {
            assert_eq!(
                charged.available_permits(),
                super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES - Counts::body_bytes(),
                "count root refunded before its actual free"
            );
        },
        || drop(counts),
    );
    assert_eq!(
        bytes.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

#[test]
fn failure_slot_exact_control_block_uses_actual_layout_before_backing_and_closed_pool_final_release(
) {
    let bytes = Slot::body_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let counts = Counts::fixture(1);
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let capture = seen.clone();
    let charged = bank.clone();
    let original_counts = counts.clone();
    let root = super::super::metadata_vec::with_allocation_hook(
        move |layout, _| {
            assert_eq!(layout.size(), bytes);
            assert_eq!(charged.available_permits(), 0);
            assert_eq!(original_counts.available_permits(), 0);
            capture.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        },
        || Slot::reserve(&counts, &bank).unwrap(),
    );
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);
    let last = root.clone();
    let pointer = std::ptr::from_ref(&*root);
    drop(root);
    bank.close();
    counts.close();
    assert_eq!(std::ptr::from_ref(&*last), pointer);
    assert_eq!(counts.available_permits(), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(last);
    assert_eq!(counts.available_permits(), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn failure_count_root_short_and_closed_refusal_precedes_backing_or_count_construction() {
    let bytes = Counts::body_bytes();
    for closed in [false, true] {
        let bank = Arc::new(Semaphore::new(bytes - 1));
        if closed {
            bank.close();
        }
        let _hook = super::super::metadata_vec::on_allocation(|_, _| {
            panic!("unadmitted count root allocated");
        });
        let error = Counts::new(super::super::MAX_FAILURE_RECORDS, &bank)
            .err()
            .unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), bytes - 1);
    }
}

#[test]
fn failure_count_root_exact_backing_survives_bank_close_and_original_count_owners_until_last_free()
{
    use std::sync::atomic::{AtomicUsize, Ordering};
    let bytes = Counts::body_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let allocations = Arc::new(AtomicUsize::new(0));
    let start = Arc::new(AtomicUsize::new(0));
    let observed = allocations.clone();
    let address = start.clone();
    let charged = bank.clone();
    let hook = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), bytes);
        assert_eq!(charged.available_permits(), 0);
        address.store(pointer, Ordering::SeqCst);
        observed.fetch_add(1, Ordering::SeqCst);
        charged.close(); // Admission is complete before Semaphore construction.
    });
    let counts = Counts::new(3, &bank).unwrap();
    drop(hook);
    assert_eq!(allocations.load(Ordering::SeqCst), 1);
    assert!(bank.is_closed());
    assert!(!counts.0.is_closed());
    assert!(Arc::ptr_eq(counts.0.storage().semaphore(), &bank));
    assert_eq!(counts.0.storage().num_permits(), bytes);
    let pointer = &*counts.0 as *const Semaphore as usize;
    let start = start.load(Ordering::SeqCst);
    assert!(pointer >= start && pointer + std::mem::size_of::<Semaphore>() <= start + bytes);
    let one = counts.clone().try_acquire_owned().unwrap();
    let two = counts.clone().try_acquire_owned().unwrap();
    let three = counts.clone().try_acquire_owned().unwrap();
    assert!(matches!(
        counts.clone().try_acquire_owned(),
        Err(TryAcquireError::NoPermits)
    ));
    assert_eq!(bank.available_permits(), 0);
    drop(one);
    assert_eq!(counts.available_permits(), 1);
    let one = counts.clone().try_acquire_owned().unwrap();
    counts.close();
    assert!(matches!(
        counts.clone().try_acquire_owned(),
        Err(TryAcquireError::Closed)
    ));
    assert!(one.counts.ptr_eq(&counts) && two.counts.ptr_eq(&counts));
    drop(counts);
    drop(one);
    drop(two);
    assert_eq!(three.counts.available_permits(), 2);
    assert_eq!(&*three.counts.0 as *const Semaphore as usize, pointer);
    assert_eq!(bank.available_permits(), 0);
    let freed = Arc::new(AtomicUsize::new(0));
    let observed = freed.clone();
    let charged = bank.clone();
    super::super::shared_allocation::after_free(
        move || {
            assert_eq!(charged.available_permits(), 0);
            observed.fetch_add(1, Ordering::SeqCst);
        },
        || drop(three),
    );
    assert_eq!(freed.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn failure_count_root_fixture_rebind_preserves_original_semaphore_and_requires_unused_exact_grant()
{
    let bytes = Counts::body_bytes();
    let first = Arc::new(Semaphore::new(bytes));
    let second = Arc::new(Semaphore::new(bytes));
    let mut counts = Counts::new(2, &first).unwrap();
    let pointer = &*counts.0 as *const Semaphore as usize;
    let held = second.clone().try_acquire_owned().unwrap();
    assert!(counts.rebind_empty_metadata(&second).is_err());
    assert_eq!(second.available_permits(), bytes - 1);
    drop(held);
    let alias = counts.clone();
    assert!(counts.rebind_empty_metadata(&second).is_err());
    assert_eq!(second.available_permits(), bytes);
    drop(alias);
    let original = counts.clone().try_acquire_owned().unwrap();
    assert!(counts.rebind_empty_metadata(&second).is_err());
    assert_eq!(counts.available_permits(), 1);
    assert_eq!(second.available_permits(), bytes);
    drop(original);
    let closed = Arc::new(Semaphore::new(bytes));
    closed.close();
    assert!(counts.rebind_empty_metadata(&closed).is_err());
    assert_eq!(first.available_permits(), 0);
    let hook = super::super::metadata_vec::on_allocation(|_, _| {
        panic!("fixture rebind replaced original allocation");
    });
    counts.rebind_empty_metadata(&second).unwrap();
    drop(hook);
    assert_eq!(&*counts.0 as *const Semaphore as usize, pointer);
    assert!(Arc::ptr_eq(counts.0.storage().semaphore(), &second));
    assert_eq!(counts.available_permits(), 2);
    assert_eq!(first.available_permits(), bytes);
    assert_eq!(second.available_permits(), 0);
    second.close();
    let one = counts.clone().try_acquire_owned().unwrap();
    assert_eq!(counts.available_permits(), 1);
    drop(one);
    assert_eq!(counts.available_permits(), 2);
    drop(counts);
    assert_eq!(second.available_permits(), bytes);
}

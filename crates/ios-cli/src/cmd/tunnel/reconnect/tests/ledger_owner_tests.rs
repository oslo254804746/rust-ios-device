use super::*;

#[test]
fn ledger_owner_exact_body_refusal_and_parallel_last_roots_refund_once() {
    let bytes = Ledger::structural_bytes();
    for available in [0, bytes - 1] {
        let bank = Arc::new(Semaphore::new(available));
        let error = super::super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("refused whole Ledger allocation created backing"),
            || {
                Ledger::with_metadata(RetryPolicy::default(), bank.clone())
                    .err()
                    .unwrap()
            },
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(bytes));
    let allocations = Arc::new(Mutex::new(Vec::new()));
    let captured = allocations.clone();
    let charged = bank.clone();
    let ledger = super::super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(layout.size(), bytes);
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap(),
    );
    assert_eq!(ledger.storage().num_permits(), bytes);
    assert!(Arc::ptr_eq(ledger.storage().semaphore(), &bank));
    let observed = allocations.lock().unwrap();
    assert_eq!(observed.len(), 1);
    let data = std::ptr::from_ref(&*ledger) as usize;
    assert!(observed[0].1 <= data && data < observed[0].1 + observed[0].0.size());
    drop(observed);
    assert_eq!(bank.available_permits(), 0);
    let pointer = &*ledger as *const Ledger as usize;
    bank.close();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let retained = ledger.clone();
            assert!(retained.ptr_eq(&ledger));
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*retained as *const Ledger as usize, pointer);
                barrier.wait();
                drop(retained);
            });
        }
        barrier.wait();
        drop(ledger);
    });
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn ledger_owner_actual_attempt_retains_body_at_closed_quota_while_snapshot_keeps_original_io()
{
    let total = 32 * 1024;
    let bank = Arc::new(Semaphore::new(total));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let attempt = ticket
        .commit(|| ledger.start(A, identity(), &ticket, slot(&bank)))
        .unwrap();
    assert!(attempt.ledger.ptr_eq(&ledger));
    let (original, original_pointer) = io_original("original-ledger-last-owner");
    let result = failed_result(attempt.source_failed(original));
    let snapshot = ledger.payload_receipts().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert!(snapshot.as_slice()[0].ptr_eq(&attempt.receipt));
    assert_eq!(pointer(snapshot.as_slice()[0].primary()), original_pointer);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    assert_eq!(bank.available_permits(), 0);
    ledger.close();
    assert_eq!(ledger.len(), 0);
    assert_eq!(snapshot.as_slice()[0].completion(), Completion::Pending);
    let before = bank.available_permits();
    drop(ledger);
    assert_eq!(
        bank.available_permits(),
        before,
        "Attempt still owns the original ledger"
    );
    attempt.finish(&result);
    assert_eq!(
        bank.available_permits(),
        before + Ledger::structural_bytes()
    );
    assert_eq!(pointer(snapshot.as_slice()[0].primary()), original_pointer);
    assert_eq!(snapshot.as_slice()[0].completion(), Completion::Pending);
    drop(held);
    drop(result);
    assert_eq!(pointer(snapshot.as_slice()[0].primary()), original_pointer);
    assert!(bank.available_permits() < total);
    drop(snapshot);
    assert_eq!(bank.available_permits(), total);
}

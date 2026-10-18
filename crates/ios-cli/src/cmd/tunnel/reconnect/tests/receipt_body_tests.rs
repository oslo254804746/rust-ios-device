use super::*;
use crate::cmd::tunnel::metadata_vec;
use std::sync::Barrier;

#[tokio::test]
async fn receipt_body_preflight_and_last_weak_keep_exact_body_and_paid_guard_after_fields_end() {
    let identity = buffered_identity();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let bytes = FailureReceipt::initial_bytes(A, &identity);
    for available in [0, FailureReceipt::body_bytes() - 1, bytes - 1] {
        let bank = Arc::new(Semaphore::new(available + slot_bytes()));
        let admitted = slot(&bank);
        let error = metadata_vec::with_allocation_hook(
            |_, _| panic!("receipt selectors allocated before all capsules were admitted"),
            || {
                FailureReceipt::reserve(A, &identity, &ticket, admitted, Arc::new(()), &bank)
                    .err()
                    .unwrap()
            },
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available + slot_bytes());
    }
    let bytes = bytes + slot_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let receipt =
        FailureReceipt::reserve(A, &identity, &ticket, slot(&bank), Arc::new(()), &bank).unwrap();
    let weak = receipt.downgrade();
    let retained_weak = weak.clone();
    let same = weak.upgrade().unwrap();
    assert!(receipt.ptr_eq(&same));
    assert_eq!(receipt.udid.as_ptr(), same.udid.as_ptr());
    assert_eq!(receipt.completion(), Completion::Pending);
    assert_eq!(bank.available_permits(), 0);
    drop((receipt, same));
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        bytes - FailureReceipt::body_bytes()
    );
    drop(weak);
    assert_eq!(
        bank.available_permits(),
        bytes - FailureReceipt::body_bytes()
    );
    drop(retained_weak);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn receipt_body_actual_ledger_snapshot_and_report_share_original_until_last_snapshot_then_weak(
) {
    let total = 32 * 1024;
    let bank = Arc::new(Semaphore::new(total));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let attempt = ticket
        .commit(|| ledger.start(A, buffered_identity(), &ticket, slot(&bank)))
        .unwrap();
    let (original, original_pointer) = io_original("original-receipt-body-snapshot");
    let result = failed_result(attempt.source_failed(original));
    attempt.finish(&result);
    let receipt = ledger.receipt(A).unwrap();
    let weak = receipt.downgrade();
    let snapshots = ledger.payload_receipts().unwrap();
    assert_eq!(snapshots.len(), 1);
    assert!(snapshots.as_slice()[0].ptr_eq(&receipt));
    assert_eq!(pointer(snapshots.as_slice()[0].primary()), original_pointer);
    // Source was captured without a bookkeeping writer certifying a row.
    // Snapshot transport must preserve Pending rather than invent Recorded.
    assert_eq!(receipt.completion(), Completion::Pending);
    drop(receipt);
    drop(ledger.remove_where(|_, _, _| true));
    drop(ledger);
    drop(result);
    assert!(weak.upgrade().is_some());
    assert_eq!(pointer(snapshots.as_slice()[0].primary()), original_pointer);
    assert_eq!(snapshots.as_slice()[0].completion(), Completion::Pending);
    drop(snapshots);
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        total - FailureReceipt::body_bytes()
    );
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn receipt_body_pending_source_clear_and_parallel_report_drops_keep_original_io_and_last_weak(
) {
    let total = 32 * 1024;
    let bank = Arc::new(Semaphore::new(total));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let attempt = ticket
        .commit(|| ledger.start(A, buffered_identity(), &ticket, slot(&bank)))
        .unwrap();
    let weak = attempt.receipt.downgrade();
    // The original pending capsule remains owned across cutover, and captures
    // the same typed error after its original ticket has become stale.
    lifecycle.clear();
    ledger.close();
    let (original, original_pointer) = io_original("original-receipt-body-after-clear");
    let result = failed_result(attempt.source_failed(original));
    attempt.finish(&result);
    let receipt = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<SourceFailure>()
        .unwrap()
        .receipt();
    assert_eq!(receipt.completion(), Completion::Abandoned);
    assert_eq!(pointer(receipt.primary()), original_pointer);
    let roots = ledger.retired_receipts();
    assert_eq!(roots.len(), 1);
    assert!(roots[0].ptr_eq(&receipt));
    drop(ledger.take_retired_causes());
    drop(ledger);
    drop(result);
    let barrier = Arc::new(Barrier::new(8));
    let mut owners = (0..6).map(|_| receipt.clone()).collect::<Vec<_>>();
    owners.push(receipt);
    owners.extend(roots);
    std::thread::scope(|scope| {
        for owner in owners {
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(pointer(owner.primary()), original_pointer);
                assert_eq!(owner.completion(), Completion::Abandoned);
                barrier.wait();
                drop(owner);
            });
        }
    });
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        total - FailureReceipt::body_bytes()
    );
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

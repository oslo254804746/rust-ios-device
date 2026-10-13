//! Exact owned-storage boundaries; no device discovery or native operations.
use super::super::{lifecycle::Lifecycle, source::Route};
use super::*;

const A: &str = "receipt-device-a";
const B: &str = "receipt-device-b";
const C: &str = "receipt-device-c";

fn identity() -> AttemptIdentity {
    AttemptIdentity {
        route: Route::Usbmux,
        native_id: Some(7),
    }
}
fn lifecycle() -> Lifecycle {
    Lifecycle::with_metadata(Arc::new(Semaphore::new(
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
    )))
}
fn slot() -> Arc<OwnedSemaphorePermit> {
    Arc::new(Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap())
}
fn io_original(message: &'static str) -> (anyhow::Error, usize) {
    let error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        message,
    ));
    let pointer = error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error as usize;
    (error, pointer)
}
fn pointer(error: &anyhow::Error) -> usize {
    error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error as usize
}
fn failed_result(error: anyhow::Error) -> Result<super::super::TunnelRecord> {
    Err(error)
}

#[tokio::test]
async fn retry_receipt_exact_pre_source_fee_rejects_one_byte_short_without_advancing_episode() {
    let identity = identity();
    let bytes = Ledger::structural_bytes() + Ledger::initial_attempt_bytes(A, &identity) - 1;
    let bank = Arc::new(Semaphore::new(bytes));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), Arc::clone(&bank)).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let admission = slot();
    let error = ticket
        .commit(|| ledger.start(A, identity, &ticket, Arc::clone(&admission)))
        .err()
        .expect("one byte short admitted Source");
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::ResourceLimit
    );
    assert!(ledger.snapshot(A).is_none());
    assert_eq!(ledger.receipt_count(), 0);
    // These actual empty table backings were admitted before the final capsule
    // rejection; no key, episode, receipt or source object was published.
    assert_eq!(
        bank.available_permits(),
        bytes - Ledger::structural_bytes() - 4 * std::mem::size_of::<Box<Removed>>()
    );
    drop(ledger);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn retry_receipt_stale_same_poll_original_uses_reserved_history_even_with_zero_bytes() {
    let identity = identity();
    let bytes = Ledger::structural_bytes() + Ledger::initial_attempt_bytes(A, &identity);
    let bank = Arc::new(Semaphore::new(bytes));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), Arc::clone(&bank)).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let attempt = ticket
        .commit(|| ledger.start(A, identity, &ticket, slot()))
        .unwrap();
    assert_eq!(bank.available_permits(), 0);
    let (original, original_pointer) = io_original("private-original-refusal-token");
    lifecycle.clear();
    ledger.close();
    // Block every refund from the now-removed episode. Source capture has no
    // remaining quota and must retain the acquired object in its existing node.
    let block = bank
        .clone()
        .try_acquire_many_owned(u32::try_from(bank.available_permits()).unwrap())
        .unwrap();
    assert_eq!(bank.available_permits(), 0);
    let result = failed_result(attempt.source_failed(original));
    assert_eq!(bank.available_permits(), 0);
    let receipt = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<SourceFailure>()
        .unwrap()
        .receipt();
    assert_eq!(pointer(receipt.primary()), original_pointer);
    assert_eq!(pointer(receipt.latest()), original_pointer);
    assert_eq!(receipt.completion(), Completion::Abandoned);
    assert!(!format!("{receipt:?} {receipt}").contains("private-original-refusal-token"));
    attempt.finish(&result);
    assert!(ledger.receipt(A).is_none());
    assert_eq!(ledger.receipt_count(), 1);
    ledger.clear_retired_causes();
    drop(ledger);
    drop(block);
    assert!(
        bank.available_permits() < bytes,
        "caller lost byte owner at history removal"
    );
    assert_eq!(pointer(receipt.primary()), original_pointer);
    drop(result);
    assert!(
        bank.available_permits() < bytes,
        "shared receipt lost its original capsule fee"
    );
    drop(receipt);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn retry_receipt_first_latest_share_exact_originals_and_refund_only_after_final_report() {
    let bytes = 16 * 1024;
    let bank = Arc::new(Semaphore::new(bytes));
    let ledger = Ledger::with_metadata(RetryPolicy::default(), Arc::clone(&bank)).unwrap();
    let lifecycle = lifecycle();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let admission = slot();
    let first = ticket
        .commit(|| ledger.start(A, identity(), &ticket, Arc::clone(&admission)))
        .unwrap();
    let (original, first_pointer) = io_original("first-private-original");
    let first_result = failed_result(first.source_failed(original));
    first.finish(&first_result);
    let first_receipt = ledger.receipt(A).unwrap();
    let second = ticket
        .commit(|| ledger.start(A, identity(), &ticket, Arc::clone(&admission)))
        .unwrap();
    let (original, latest_pointer) = io_original("latest-private-original");
    let second_result = failed_result(second.source_failed(original));
    second.finish(&second_result);
    let latest = ledger.receipt(A).unwrap();
    assert_eq!(pointer(first_receipt.primary()), first_pointer);
    assert_eq!(pointer(latest.primary()), first_pointer);
    assert_eq!(pointer(latest.latest()), latest_pointer);
    assert_ne!(first_pointer, latest_pointer);
    assert_eq!(latest.failures(), 2);
    assert!(Arc::ptr_eq(&admission, &latest.slot()));
    drop(first_receipt);
    drop(first_result);
    let removed = ledger.remove_where(|_, _, _| true);
    drop(ledger);
    assert_eq!(pointer(latest.primary()), first_pointer);
    drop(removed);
    drop(latest);
    assert!(
        bank.available_permits() < bytes,
        "last caller report no longer owns its capsules"
    );
    let final_receipt = second_result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<SourceFailure>()
        .unwrap()
        .receipt();
    assert_eq!(pointer(final_receipt.primary()), first_pointer);
    assert_eq!(pointer(final_receipt.latest()), latest_pointer);
    drop(final_receipt);
    drop(second_result);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn retry_receipt_actual_backing_removal_rollback_preserves_order_versions_and_roots_at_zero_quota(
) {
    let bytes = 32 * 1024;
    let bank = Arc::new(Semaphore::new(bytes));
    let policy = RetryPolicy::new(
        Duration::from_secs(1),
        Duration::from_secs(1),
        NonZeroU32::new(1),
        None,
    )
    .unwrap();
    let ledger = Ledger::with_metadata(policy, Arc::clone(&bank)).unwrap();
    let lifecycle = lifecycle();
    let mut versions = Vec::new();
    let mut originals = Vec::new();
    for key in [A, B, C] {
        let ticket = lifecycle.ticket(key).await.unwrap();
        let attempt = ticket
            .commit(|| ledger.start(key, identity(), &ticket, slot()))
            .unwrap();
        let (original, original_pointer) = io_original("rollback-original");
        let result = failed_result(attempt.source_failed(original));
        attempt.finish(&result);
        versions.push(ledger.version(key, &ticket).unwrap());
        originals.push(original_pointer);
    }
    let block = bank
        .clone()
        .try_acquire_many_owned(u32::try_from(bank.available_permits()).unwrap())
        .unwrap();
    let before = bank.available_permits();
    {
        let mut state = ledger.lock();
        let row_capacity = state.rows.capacity();
        let cause_capacity = state.causes.capacity();
        state.select(&|key, _, _| key != B);
        let removed = state.remove_selected();
        assert_eq!(state.rows.len(), 1);
        assert_eq!(state.causes.len(), 1);
        state.restore(removed);
        assert_eq!(state.rows.capacity(), row_capacity);
        assert_eq!(state.causes.capacity(), cause_capacity);
        assert_eq!(
            state
                .rows
                .as_slice()
                .iter()
                .map(|node| node.udid.as_str())
                .collect::<Vec<_>>(),
            [A, B, C]
        );
        assert_eq!(
            state
                .causes
                .as_slice()
                .iter()
                .map(|node| node.cause.as_ref().unwrap().udid.as_str())
                .collect::<Vec<_>>(),
            [A, B, C]
        );
        for (index, key) in [A, B, C].iter().enumerate() {
            assert!(Arc::ptr_eq(
                &state.row(key).unwrap().version,
                &versions[index]
            ));
            assert_eq!(
                pointer(state.cause(key).unwrap().primary()),
                originals[index]
            );
        }
    }
    assert_eq!(bank.available_permits(), before);
    let removed = ledger.remove_where(|_, _, _| true);
    assert_eq!(ledger.receipt_count(), 0);
    assert_eq!(ledger.len(), 0);
    drop(ledger);
    drop(block);
    assert!(
        bank.available_permits() < bytes,
        "removed original nodes refunded before batch release"
    );
    drop(removed);
    assert_eq!(bank.available_permits(), bytes);
}

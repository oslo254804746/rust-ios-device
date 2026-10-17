use super::super::failure_table::Node;
use super::super::{
    reconnect::{Completion, Ledger, Receipt, RetryPolicy, SourceFailure},
    AttemptIdentity, Route,
};
use super::*;
use std::sync::Barrier;

const TOTAL: usize = 512 * 1024;
const UID: &str = "write-owner-original";

async fn fixture() -> (Arc<Semaphore>, WriteBatch, Receipt, usize) {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let lifecycle = super::super::lifecycle::Lifecycle::new();
    let ticket = lifecycle.ticket(UID).await.unwrap();
    let ledger = Ledger::with_metadata(RetryPolicy::default(), bank.clone()).unwrap();
    let identity = AttemptIdentity {
        route: Route::Usbmux,
        native_id: Some(3),
    };
    let slot = Arc::new(Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap());
    let attempt = ticket
        .commit(|| ledger.start(UID, identity.clone(), &ticket, slot))
        .unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "original-write-owner-source",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let result: Result<()> = Err(attempt.source_failed(original));
    attempt.finish(&result);
    let receipt = result
        .as_ref()
        .unwrap_err()
        .downcast_ref::<SourceFailure>()
        .unwrap()
        .receipt();
    let node = Node::owned(UID.into(), identity, &bank).unwrap();
    let mut batch = WriteBatch::new(&bank, 1).unwrap();
    batch
        .record(ScanIntent::Failure(super::FailureIntent {
            ticket,
            failed: DeferredFailure {
                node,
                receipt: Some(receipt.clone()),
                destination: None,
            },
        }))
        .unwrap();
    drop(ledger.remove_where(|_, _, _| true));
    drop(ledger);
    drop(result);
    assert_eq!(receipt.completion(), Completion::Pending);
    (bank, batch, receipt, pointer)
}
fn original(receipt: &Receipt) -> usize {
    receipt.primary().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize
}

#[tokio::test]
async fn write_owner_actual_session_retains_original_batch_root_and_finishes_error_at_zero_quota() {
    let (bank, batch, receipt, pointer) = fixture().await;
    let owner = batch.owner();
    let lock = tokio::sync::Mutex::new(());
    let outcomes = super::super::scan::Outcomes::with_metadata(bank.clone());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain_writes(owner.clone()).unwrap();
    assert_eq!(owner.state.lock().unwrap().receipts.len(), 1);
    assert!(owner.state.lock().unwrap().receipts.as_slice()[0].ptr_eq(&receipt));
    drop(batch);
    assert_eq!(receipt.completion(), Completion::Pending);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    assert_eq!(bank.available_permits(), 0);
    let completion = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "original-scan-completion",
    ));
    let completion_pointer =
        completion.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let error = session.finish(Err(completion)).unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        completion_pointer
    );
    let context = error.downcast_ref::<SourceFailuresContext>().unwrap();
    assert!(context.receipts()[0].ptr_eq(&receipt));
    assert_eq!(original(&context.receipts()[0]), pointer);
    assert_eq!(
        receipt.completion(),
        Completion::Failed(ios_core::ErrorClass::Timeout)
    );
    drop(owner);
    assert_eq!(original(&receipt), pointer);
    drop(error);
    assert_eq!(
        receipt.completion(),
        Completion::Failed(ios_core::ErrorClass::Timeout)
    );
    drop((held, receipt, outcomes));
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn write_owner_parallel_last_roots_complete_only_pending_receipts_and_refund_exact_storage() {
    for completion in [
        Completion::Pending,
        Completion::Recorded,
        Completion::Failed(ios_core::ErrorClass::Protocol),
    ] {
        let (bank, batch, receipt, pointer) = fixture().await;
        if completion != Completion::Pending {
            receipt.complete(completion);
        }
        let expected = if completion == Completion::Pending {
            Completion::Abandoned
        } else {
            completion
        };
        let owner = batch.owner();
        let original_owner = &*owner as *const FailureWriteOwner as usize;
        let released = std::mem::size_of::<FailureWriteOwner>()
            + std::mem::size_of::<SourceFailuresContext>()
            + owner.state.lock().unwrap().receipts.capacity() * std::mem::size_of::<Receipt>();
        let mut owners = (0..7).map(|_| owner.clone()).collect::<Vec<_>>();
        owners.push(owner);
        drop(batch);
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        assert_eq!(bank.available_permits(), 0);
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for owner in owners {
                let barrier = barrier.clone();
                scope.spawn(move || {
                    assert_eq!(&*owner as *const FailureWriteOwner as usize, original_owner);
                    barrier.wait();
                    drop(owner);
                });
            }
        });
        assert_eq!(receipt.completion(), expected);
        assert_eq!(original(&receipt), pointer);
        assert_eq!(bank.available_permits(), released);
        drop(held);
        assert!(bank.available_permits() < TOTAL);
        drop(receipt);
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[tokio::test]
async fn write_owner_session_caller_drop_abandons_original_receipt_without_losing_last_report() {
    let (bank, batch, receipt, pointer) = fixture().await;
    let owner = batch.owner();
    let lock = tokio::sync::Mutex::new(());
    let outcomes = super::super::scan::Outcomes::with_metadata(bank.clone());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain_writes(owner.clone()).unwrap();
    drop(batch);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    drop(session);
    assert_eq!(receipt.completion(), Completion::Abandoned);
    assert_eq!(original(&receipt), pointer);
    assert!(outcomes.history().is_none());
    assert_eq!(owner.state.lock().unwrap().receipts.len(), 1);
    drop(owner);
    assert_eq!(original(&receipt), pointer);
    drop((held, receipt, outcomes));
    assert_eq!(bank.available_permits(), TOTAL);
}

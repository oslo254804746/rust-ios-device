use super::*;
use tokio::sync::Mutex;

struct Original(Arc<()>, Vec<u8>);

impl std::fmt::Debug for Original {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Original")
    }
}

impl std::fmt::Display for Original {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        panic!("capsule admission must not format an original payload")
    }
}
impl Error for Original {}

fn original() -> (anyhow::Error, Arc<()>, *const Original) {
    let id = Arc::new(());
    let error = anyhow::Error::new(Original(Arc::clone(&id), vec![7; 1024 * 1024]));
    let pointer = error.downcast_ref::<Original>().unwrap() as *const Original;
    (error, id, pointer)
}

fn same(error: &anyhow::Error, id: &Arc<()>, pointer: *const Original) {
    let owned = error.downcast_ref::<Original>().unwrap();
    assert_eq!(owned as *const Original, pointer);
    assert!(Arc::ptr_eq(&owned.0, id));
    assert_eq!(owned.1.len(), 1024 * 1024);
    assert_eq!(owned.1[0], 7);
}

#[tokio::test]
async fn scan_capsule_exact_preflight_and_one_byte_rejection_leave_slots_empty() {
    let bytes = capsule_reservation_bytes();
    let lock = Mutex::new(());
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let error = outcomes.begin(lock.lock().await).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(outcomes.payload_history().0, 0);
    assert!(outcomes.history().is_none());
    assert_eq!(bank.available_permits(), bytes - 1);
    bank.add_permits(1);
    let session = outcomes.begin(lock.lock().await).unwrap();
    assert_eq!(bank.available_permits(), 0);
    session.finish(Ok(Some(()))).unwrap();
    assert_eq!(bank.available_permits(), bytes);
    assert_eq!(outcomes.payload_history().0, 0);
}

#[tokio::test]
async fn scan_capsule_plain_original_adds_no_context_and_refunds_unused_reservations() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    let depth = primary.chain().count();
    session.retain(Source::Usb, primary);
    let error = session.finish(Ok(Some(()))).unwrap_err();
    same(&error, &id, pointer);
    assert_eq!(error.chain().count(), depth);
    assert_eq!(bank.available_permits(), bytes);
    assert!(outcomes.history().is_none());
}

#[tokio::test]
async fn scan_capsule_compound_originals_and_payload_charge_follow_caller_until_drop() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    let (secondary, second_id, second_pointer) = original();
    let depth = primary.chain().count();
    session.retain(Source::Usb, primary);
    session.retain(Source::Paired, secondary);
    let error = session
        .finish(Err(super::super::lifecycle::cancelled()))
        .unwrap_err();
    same(&error, &id, pointer);
    let context = error.downcast_ref::<SourceContext>().unwrap();
    same(
        context.secondary(Source::Paired).unwrap(),
        &second_id,
        second_pointer,
    );
    assert!(context
        .completion()
        .unwrap()
        .is::<ios_core::OperationCancelled>());
    assert_eq!(error.chain().count(), depth + 1);
    assert_eq!(
        bank.available_permits(),
        bytes - std::mem::size_of::<SourceContext>()
    );
    drop(outcomes);
    assert_eq!(
        bank.available_permits(),
        bytes - std::mem::size_of::<SourceContext>()
    );
    drop(error);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_capsule_single_source_completion_trims_max_and_keeps_typed_secondary() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    session.retain(Source::Usb, primary);
    let error = session
        .finish(Err(super::super::lifecycle::cancelled()))
        .unwrap_err();
    same(&error, &id, pointer);
    assert!(error
        .downcast_ref::<super::super::bookkeeping::FailureContext>()
        .unwrap()
        .secondary()
        .is::<ios_core::OperationCancelled>());
    assert_eq!(
        bank.available_permits(),
        bytes - std::mem::size_of::<super::super::bookkeeping::FailureContext>()
    );
    drop(error);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_capsule_terminal_take_cleanup_finish_keeps_exact_chain_without_second_allocation() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    session.retain(Source::Usb, primary);
    session.retain(Source::Paired, original().0);
    let primary = session.take().unwrap();
    let error = super::super::ScanCleanupFailure {
        primary,
        cleanup: super::super::lifecycle::cancelled(),
        _storage: session.cleanup_storage(),
    };
    let error = session.finish(Err(error.into())).unwrap_err();
    let cleanup = error
        .downcast_ref::<super::super::ScanCleanupFailure>()
        .unwrap();
    same(&cleanup.primary, &id, pointer);
    assert!(cleanup.primary.is::<SourceContext>());
    assert!(cleanup.cleanup.is::<ios_core::OperationCancelled>());
    let kept = std::mem::size_of::<SourceContext>()
        + std::mem::size_of::<super::super::ScanCleanupFailure>();
    assert_eq!(bank.available_permits(), bytes - kept);
    assert!(outcomes.history().is_none());
    drop(error);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_capsule_abandoned_context_and_history_refund_only_after_last_shared_root() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    session.retain(Source::Usb, primary);
    session.retain(Source::Paired, original().0);
    drop(session);
    let roots = outcomes.history().expect("abandoned scan history");
    assert_eq!(roots.len(), 1);
    same(roots[0].original(), &id, pointer);
    let originals = std::mem::size_of::<SourceContext>() + std::mem::size_of::<AbandonedScan>();
    let kept = originals + std::mem::size_of::<HistorySnapshot>();
    assert_eq!(bank.available_permits(), bytes - kept);
    let copy = roots[0].clone();
    drop(outcomes);
    drop(roots);
    // A bare original does not retain the independent snapshot body fee.
    assert_eq!(bank.available_permits(), bytes - originals);
    drop(copy);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_capsule_consumed_context_keeps_conservative_reservation_until_value_drops() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain(Source::Usb, original().0);
    let (secondary, id, pointer) = original();
    session.retain(Source::Paired, secondary);
    let error = session.finish(Ok(Some(()))).unwrap_err();
    let context = error.downcast::<SourceContext>().unwrap();
    same(context.secondary(Source::Paired).unwrap(), &id, pointer);
    // Consuming anyhow extraction moves the typed context out of its old heap.
    // Its reservation follows the extracted value, conservatively; this is
    // expressly not a byte-for-byte claim about current live heap/RSS.
    assert_eq!(
        bank.available_permits(),
        bytes - std::mem::size_of::<SourceContext>()
    );
    drop(context);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_capsule_foreign_context_does_not_waive_new_scan_payload_or_duplicate_layers() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes * 2));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let first = outcomes.begin(lock.lock().await).unwrap();
    first.retain(Source::Usb, original().0);
    first.retain(Source::Paired, original().0);
    let previous = first.finish(Ok(Some(()))).unwrap_err();
    let depth = previous.chain().count();
    let next = outcomes.begin(lock.lock().await).unwrap();
    next.retain(Source::Usb, previous);
    next.retain(Source::Coordinator, original().0);
    let error = next.finish(Ok(Some(()))).unwrap_err();
    assert_eq!(error.chain().count(), depth + 1);
    assert_eq!(
        bank.available_permits(),
        2 * bytes - 2 * std::mem::size_of::<SourceContext>()
    );
    drop(error);
    assert_eq!(bank.available_permits(), 2 * bytes);
}

#[tokio::test]
async fn scan_history_prepaid_snapshot_survives_zero_quota_repeated_reads_and_last_owner() {
    let bytes = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    session.retain(Source::Usb, primary);
    drop(session);
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
        .unwrap();
    let first = outcomes.history().expect("prepaid abandoned snapshot");
    let second = outcomes.history().expect("same prepaid snapshot");
    assert!(first.ptr_eq(&second));
    assert_eq!(first.len(), 1);
    same(first[0].original(), &id, pointer);
    assert_eq!(bank.available_permits(), 0);
    let (active, refused) = outcomes.payload_history();
    assert_eq!(active, 0);
    let Err(error) = refused else {
        panic!("census output needs admission");
    };
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let unchanged = outcomes.history().unwrap();
    assert!(first.ptr_eq(&unchanged));
    drop(unchanged);
    drop(outcomes);
    drop(first);
    assert_eq!(bank.available_permits(), 0);
    same(second[0].original(), &id, pointer);
    drop(second);
    assert_eq!(
        bank.available_permits(),
        std::mem::size_of::<AbandonedScan>() + std::mem::size_of::<HistorySnapshot>()
    );
    drop(held);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_history_late_closed_bank_drop_keeps_slot_order_and_frozen_earlier_snapshot() {
    let bytes = capsule_reservation_bytes() * 2;
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    // Independent guards exercise order even though production scans serialize.
    let first_lock = Mutex::new(());
    let second_lock = Mutex::new(());
    let first = outcomes.begin(first_lock.lock().await).unwrap();
    let second = outcomes.begin(second_lock.lock().await).unwrap();
    let (first_original, first_id, first_pointer) = original();
    let (second_original, second_id, second_pointer) = original();
    first.retain(Source::Usb, first_original);
    second.retain(Source::Paired, second_original);
    drop(second);
    let older = outcomes.history().unwrap();
    assert_eq!(older.len(), 1);
    same(older[0].original(), &second_id, second_pointer);
    outcomes.close();
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(bank.available_permits().try_into().unwrap())
        .unwrap();
    bank.close();
    drop(first);
    let newer = outcomes.history().unwrap();
    assert!(!older.ptr_eq(&newer));
    assert_eq!(older.len(), 1);
    assert_eq!(newer.len(), 2);
    same(newer[0].original(), &first_id, first_pointer);
    same(newer[1].original(), &second_id, second_pointer);
    assert!(older[0].ptr_eq(&newer[1]));
    drop(held);
    drop(outcomes);
    drop(newer);
    same(older[0].original(), &second_id, second_pointer);
    drop(older);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn scan_history_census_exact_backing_refund_preserves_prepaid_snapshot() {
    let bytes = capsule_reservation_bytes() + 64;
    let bank = Arc::new(Semaphore::new(bytes));
    let outcomes = Outcomes::with_metadata(Arc::clone(&bank));
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain(Source::Usb, original().0);
    drop(session);
    let history = outcomes.history().unwrap();
    let before = bank.available_permits();
    let (active, roots) = outcomes.payload_history();
    let roots = roots.unwrap();
    assert_eq!(active, 0);
    assert_eq!(roots.len(), 1);
    assert!(roots.as_slice()[0].ptr_eq(&history[0]));
    assert_eq!(
        bank.available_permits(),
        before - 2 * std::mem::size_of::<AbandonedOwner>()
    );
    drop(roots);
    assert_eq!(bank.available_permits(), before);
    drop(outcomes);
    drop(history);
    assert_eq!(bank.available_permits(), bytes);
}

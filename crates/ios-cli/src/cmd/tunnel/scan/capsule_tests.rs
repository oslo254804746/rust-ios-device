use super::*;
use tokio::sync::Mutex;

fn fixture(child_bytes: usize) -> (Arc<Semaphore>, OutcomesOwner) {
    // Fund the real coordinator in addition to the original exact child quota.
    let bank = Arc::new(Semaphore::new(Outcomes::body_bytes() + child_bytes));
    let outcomes = Outcomes::with_metadata(bank.clone()).unwrap();
    assert_eq!(bank.available_permits(), child_bytes);
    (bank, outcomes)
}

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
    let (bank, outcomes) = fixture(bytes - 1);
    let error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("short whole scan grant allocated a context payload"),
        || outcomes.begin(lock.try_lock().unwrap()).err().unwrap(),
    );
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
    drop(outcomes);
    assert_eq!(bank.available_permits(), (bytes) + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_plain_original_adds_no_context_and_refunds_unused_reservations() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
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
    drop(outcomes);
    assert_eq!(bank.available_permits(), (bytes) + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_compound_originals_and_payload_charge_follow_caller_until_drop() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
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
        bytes - std::mem::size_of::<SourcePayload>()
    );
    drop(outcomes);
    assert_eq!(
        bank.available_permits(),
        bytes + Outcomes::body_bytes() - std::mem::size_of::<SourcePayload>()
    );
    drop(error);
    assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_single_source_completion_frees_unused_branch_and_keeps_typed_secondary() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
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
        bytes - std::mem::size_of::<super::super::bookkeeping::FailurePayload>()
    );
    drop(error);
    assert_eq!(bank.available_permits(), bytes);
    drop(outcomes);
    assert_eq!(bank.available_permits(), (bytes) + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_terminal_take_cleanup_finish_keeps_exact_chain_without_second_allocation() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    session.retain(Source::Usb, primary);
    session.retain(Source::Paired, original().0);
    let primary = session.take().unwrap();
    let error = super::super::ScanCleanupFailure::admitted(
        primary,
        super::super::lifecycle::cancelled(),
        session.cleanup_destination(),
    );
    let error = session.finish(Err(error.into())).unwrap_err();
    let cleanup = error
        .downcast_ref::<super::super::ScanCleanupFailure>()
        .unwrap();
    same(&cleanup.primary, &id, pointer);
    assert!(cleanup.primary.is::<SourceContext>());
    assert!(cleanup.cleanup.is::<ios_core::OperationCancelled>());
    let kept = std::mem::size_of::<SourcePayload>()
        + std::mem::size_of::<super::super::ScanCleanupPayload>();
    assert_eq!(bank.available_permits(), bytes - kept);
    assert!(outcomes.history().is_none());
    drop(error);
    assert_eq!(bank.available_permits(), bytes);
    drop(outcomes);
    assert_eq!(bank.available_permits(), (bytes) + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_abandoned_context_and_history_refund_only_after_last_shared_root() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let (primary, id, pointer) = original();
    session.retain(Source::Usb, primary);
    session.retain(Source::Paired, original().0);
    drop(session);
    let roots = outcomes.history().expect("abandoned scan history");
    assert_eq!(roots.len(), 1);
    same(roots[0].original(), &id, pointer);
    let originals = std::mem::size_of::<SourcePayload>() + AbandonedScan::body_bytes();
    let kept = originals + HistorySnapshot::body_bytes();
    assert_eq!(bank.available_permits(), bytes - kept);
    let copy = roots[0].clone();
    drop(outcomes);
    drop(roots);
    // A bare original does not retain the independent snapshot body fee.
    assert_eq!(
        bank.available_permits(),
        bytes + Outcomes::body_bytes() - originals
    );
    drop(copy);
    assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_consumed_context_keeps_original_payload_allocation_until_value_drops() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    session.retain(Source::Usb, original().0);
    let (secondary, id, pointer) = original();
    session.retain(Source::Paired, secondary);
    let error = session.finish(Ok(Some(()))).unwrap_err();
    let payload_pointer =
        &*error.downcast_ref::<SourceContext>().unwrap().payload as *const SourcePayload;
    let context = error.downcast::<SourceContext>().unwrap();
    assert_eq!(&*context.payload as *const SourcePayload, payload_pointer);
    assert_eq!(
        context.payload.bytes_in(&bank).unwrap(),
        std::mem::size_of::<SourcePayload>()
    );
    same(context.secondary(Source::Paired).unwrap(), &id, pointer);
    // Consuming the typed wrapper keeps the actual independent payload Box.
    // The outer opaque anyhow allocation is a separate unmeasured boundary.
    assert_eq!(
        bank.available_permits(),
        bytes - std::mem::size_of::<SourcePayload>()
    );
    drop(context);
    assert_eq!(bank.available_permits(), bytes);
    drop(outcomes);
    assert_eq!(bank.available_permits(), (bytes) + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_capsule_foreign_context_does_not_waive_new_scan_payload_or_duplicate_layers() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes * 2);
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
        2 * bytes - 2 * std::mem::size_of::<SourcePayload>()
    );
    drop(error);
    assert_eq!(bank.available_permits(), 2 * bytes);
    drop(outcomes);
    assert_eq!(
        bank.available_permits(),
        (bytes * 2) + Outcomes::body_bytes()
    );
}

#[tokio::test]
async fn scan_history_prepaid_snapshot_survives_zero_quota_repeated_reads_and_last_owner() {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
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
    assert_eq!(bank.available_permits(), Outcomes::body_bytes());
    same(second[0].original(), &id, pointer);
    drop(second);
    assert_eq!(
        bank.available_permits(),
        AbandonedScan::body_bytes() + HistorySnapshot::body_bytes() + Outcomes::body_bytes()
    );
    drop(held);
    assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_history_late_closed_bank_drop_keeps_slot_order_and_frozen_earlier_snapshot() {
    let bytes = capsule_reservation_bytes() * 2;
    let (bank, outcomes) = fixture(bytes);
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
    assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_history_census_exact_backing_refund_preserves_prepaid_snapshot() {
    let bytes = capsule_reservation_bytes() + 64;
    let (bank, outcomes) = fixture(bytes);
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
    assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
}

#[tokio::test]
async fn scan_context_actual_pre_source_destinations_survive_closed_pool_selection_and_byvalue_extraction(
) {
    for source_branch in [false, true] {
        let bytes = capsule_reservation_bytes();
        let (bank, outcomes) = fixture(bytes);
        let lock = Mutex::new(());
        let session = outcomes.begin(lock.lock().await).unwrap();
        let (source_pointer, cleanup_pointer) = {
            let compound = session.compound.lock().unwrap();
            let compound = compound.as_ref().unwrap();
            (
                compound.source.as_ptr(),
                session.cleanup.lock().unwrap().as_ref().unwrap().as_ptr(),
            )
        };
        let (primary, id, pointer) = original();
        let (secondary, secondary_id, secondary_pointer) = original();
        session.retain(Source::Usb, primary);
        if source_branch {
            session.retain(Source::Paired, secondary);
        }
        assert_eq!(bank.available_permits(), 0);
        bank.close();
        let freed = Arc::new(StdMutex::new(Vec::new()));
        let observed = freed.clone();
        let charged = bank.clone();
        let error = super::super::owned_box::after_free(
            move || observed.lock().unwrap().push(charged.available_permits()),
            || {
                super::super::metadata_vec::with_allocation_hook(
                    |_, _| panic!("Source completion allocated another controlled payload body"),
                    || {
                        let primary = session.take().unwrap();
                        let cleanup = super::super::ScanCleanupFailure::admitted(
                            primary,
                            super::super::lifecycle::cancelled(),
                            session.cleanup_destination(),
                        );
                        session.finish(Err(cleanup.into())).unwrap_err()
                    },
                )
            },
        );
        if source_branch {
            assert_eq!(freed.lock().unwrap().as_slice(), [0]);
            let cleanup = error
                .downcast_ref::<super::super::ScanCleanupFailure>()
                .unwrap();
            assert_eq!(
                &*cleanup.payload as *const super::super::ScanCleanupPayload,
                cleanup_pointer
            );
            let context = cleanup.primary.downcast_ref::<SourceContext>().unwrap();
            assert_eq!(&*context.payload as *const SourcePayload, source_pointer);
            same(
                context.secondary(Source::Paired).unwrap(),
                &secondary_id,
                secondary_pointer,
            );
            same(&cleanup.primary, &id, pointer);
            let cleanup = error
                .downcast::<super::super::ScanCleanupFailure>()
                .unwrap();
            assert_eq!(
                &*cleanup.payload as *const super::super::ScanCleanupPayload,
                cleanup_pointer
            );
            assert!(std::ptr::eq(
                cleanup.source().unwrap(),
                cleanup.primary.as_ref()
            ));
            let kept = std::mem::size_of::<SourcePayload>()
                + std::mem::size_of::<super::super::ScanCleanupPayload>();
            assert_eq!(bank.available_permits(), bytes - kept);
            drop(cleanup);
        } else {
            // With no secondary Source, both unused compound allocations are
            // physically gone before their independent credits return.
            assert_eq!(
                freed.lock().unwrap().as_slice(),
                [0, std::mem::size_of::<SourcePayload>()]
            );
            let cleanup = error
                .downcast::<super::super::ScanCleanupFailure>()
                .unwrap();
            assert_eq!(
                &*cleanup.payload as *const super::super::ScanCleanupPayload,
                cleanup_pointer
            );
            same(&cleanup.primary, &id, pointer);
            assert_eq!(
                bank.available_permits(),
                bytes - std::mem::size_of::<super::super::ScanCleanupPayload>()
            );
            drop(cleanup);
        }
        assert_eq!(bank.available_permits(), bytes);
        drop(outcomes);
        assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
    }
}

#[tokio::test]
async fn scan_completion_context_keeps_original_pre_source_payload_after_byvalue_wrapper_extraction(
) {
    let bytes = capsule_reservation_bytes();
    let (bank, outcomes) = fixture(bytes);
    let lock = Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let payload_pointer = session
        .compound
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .failure
        .as_ptr() as usize;
    let (primary, id, pointer) = original();
    let (secondary, secondary_id, secondary_pointer) = original();
    session.retain(Source::Usb, primary);
    assert_eq!(bank.available_permits(), 0);
    bank.close();
    let freed = Arc::new(StdMutex::new(Vec::new()));
    let observed = freed.clone();
    let charged = bank.clone();
    let error = super::super::owned_box::after_free(
        move || observed.lock().unwrap().push(charged.available_permits()),
        || {
            super::super::metadata_vec::with_allocation_hook(
                |_, _| panic!("completion context allocated new backing after Source"),
                || session.finish(Err(secondary)).unwrap_err(),
            )
        },
    );
    assert_eq!(
        freed.lock().unwrap().as_slice(),
        [0, std::mem::size_of::<SourcePayload>()]
    );
    same(&error, &id, pointer);
    let context = error
        .downcast_ref::<super::super::bookkeeping::FailureContext>()
        .unwrap();
    same(context.secondary(), &secondary_id, secondary_pointer);
    let field_pointer = context.secondary() as *const anyhow::Error as usize;
    let kept = std::mem::size_of::<super::super::bookkeeping::FailurePayload>();
    assert!(
        payload_pointer <= field_pointer
            && field_pointer + std::mem::size_of::<anyhow::Error>() <= payload_pointer + kept
    );
    let context = error
        .downcast::<super::super::bookkeeping::FailureContext>()
        .unwrap();
    assert_eq!(
        context.secondary() as *const anyhow::Error as usize,
        field_pointer
    );
    same(context.secondary(), &secondary_id, secondary_pointer);
    assert_eq!(bank.available_permits(), bytes - kept);
    let charged = bank.clone();
    super::super::owned_box::after_free(
        move || assert_eq!(charged.available_permits(), bytes - kept),
        || drop(context),
    );
    assert_eq!(bank.available_permits(), bytes);
    drop(outcomes);
    assert_eq!(bank.available_permits(), bytes + Outcomes::body_bytes());
}

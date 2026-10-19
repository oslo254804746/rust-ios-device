use super::*;

#[tokio::test]
async fn outcomes_init_exact_body_and_last_clone_refund_before_any_child_admission() {
    let body = Outcomes::body_bytes();
    for available in [0, body - 1] {
        let bank = Arc::new(Semaphore::new(available));
        let error = Outcomes::with_metadata(bank.clone()).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(body));
    let outcomes = Outcomes::with_metadata(bank.clone()).unwrap();
    assert!(Arc::ptr_eq(outcomes.storage().semaphore(), &bank));
    assert_eq!(outcomes.storage().num_permits(), body);
    assert_eq!(bank.available_permits(), 0);
    let retained = outcomes.clone();
    assert!(outcomes.ptr_eq(&retained));
    let lock = tokio::sync::Mutex::new(());
    let error = outcomes.begin(lock.lock().await).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(outcomes.history().is_none());
    assert_eq!(outcomes.payload_history().0, 0);
    assert_eq!(bank.available_permits(), 0);
    drop((error, outcomes));
    assert_eq!(bank.available_permits(), 0);
    drop(retained);
    assert_eq!(bank.available_permits(), body);
}

#[tokio::test]
async fn outcomes_init_child_last_byte_refusal_and_session_last_owner_keep_body_independent() {
    let body = Outcomes::body_bytes();
    let children = capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(body + children - 1));
    let outcomes = Outcomes::with_metadata(bank.clone()).unwrap();
    assert_eq!(bank.available_permits(), children - 1);
    let lock = tokio::sync::Mutex::new(());
    let error = outcomes.begin(lock.lock().await).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), children - 1);
    assert_eq!(outcomes.payload_history().0, 0);
    bank.add_permits(1);
    let session = outcomes.begin(lock.lock().await).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(outcomes);
    assert_eq!(
        bank.available_permits(),
        0,
        "Session must retain the original initialization body"
    );
    session.finish(Ok(Some(()))).unwrap();
    assert_eq!(bank.available_permits(), body + children);
}

#[tokio::test]
async fn outcomes_init_snapshot_outlives_root_and_keeps_original_io_and_own_body_fees() {
    let body = Outcomes::body_bytes();
    let total = body + capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(total));
    let outcomes = Outcomes::with_metadata(bank.clone()).unwrap();
    let lock = tokio::sync::Mutex::new(());
    let session = outcomes.begin(lock.lock().await).unwrap();
    let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    session.retain(Source::Usb, original);
    drop(session);
    let snapshot = outcomes.history().unwrap();
    let kept = HistorySnapshot::body_bytes() + AbandonedScan::body_bytes();
    assert_eq!(bank.available_permits(), total - body - kept);
    drop(outcomes);
    assert_eq!(bank.available_permits(), total - kept);
    assert_eq!(
        snapshot[0]
            .original()
            .downcast_ref::<std::io::Error>()
            .unwrap() as *const _ as usize,
        pointer
    );
    drop(snapshot);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn scan_all_shared_roots_are_preallocated_before_source_and_closed_drop_fills_original_cell_without_backing(
) {
    use super::super::metadata_vec;
    let total = Outcomes::body_bytes() + capsule_reservation_bytes();
    let bank = Arc::new(Semaphore::new(total));
    let initialized = Arc::new(StdMutex::new(Vec::new()));
    let captured = initialized.clone();
    let charged = bank.clone();
    let outcomes = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(layout.size(), Outcomes::body_bytes());
            assert_eq!(charged.available_permits(), capsule_reservation_bytes());
            captured.lock().unwrap().push((layout, pointer));
        },
        || Outcomes::with_metadata(bank.clone()).unwrap(),
    );
    let seen = initialized.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let data = std::ptr::from_ref(&*outcomes) as usize;
    assert!(seen[0].1 <= data && data < seen[0].1 + seen[0].0.size());
    drop(seen);
    let lock = tokio::sync::Mutex::new(());
    let allocated = Arc::new(StdMutex::new(Vec::new()));
    let captured = allocated.clone();
    let charged = bank.clone();
    let session = metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || outcomes.begin(lock.try_lock().unwrap()).unwrap(),
    );
    let before = allocated.lock().unwrap();
    assert_eq!(before.len(), 2);
    assert_eq!(before[0].0.size(), HistorySnapshot::body_bytes());
    assert_eq!(before[1].0.size(), AbandonedScan::body_bytes());
    let history_pointer =
        std::ptr::from_ref(&**session.history_snapshot.as_ref().unwrap()) as usize;
    let abandoned_pointer = std::ptr::from_ref(&**session.abandoned.as_ref().unwrap()) as usize;
    assert!(before[0].1 <= history_pointer && history_pointer < before[0].1 + before[0].0.size());
    assert!(
        before[1].1 <= abandoned_pointer && abandoned_pointer < before[1].1 + before[1].0.size()
    );
    assert!(session.abandoned.as_ref().unwrap().original.get().is_none());
    assert_eq!(
        session
            .history_snapshot
            .as_ref()
            .unwrap()
            .storage()
            .num_permits(),
        HistorySnapshot::body_bytes()
    );
    assert_eq!(
        session.abandoned.as_ref().unwrap().storage().num_permits(),
        AbandonedScan::body_bytes()
    );
    drop(before);
    let original = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let class = crate::output::operation_error_class(original.as_ref());
    session.retain(Source::Usb, original);
    bank.close();
    outcomes.close();
    metadata_vec::with_allocation_hook(
        |_, _| panic!("abandoned scan allocated a replacement after Source"),
        || drop(session),
    );
    let snapshot = outcomes.history().unwrap();
    assert_eq!(std::ptr::from_ref(&*snapshot) as usize, history_pointer);
    let record = snapshot.first().unwrap().clone();
    assert_eq!(std::ptr::from_ref(&*record) as usize, abandoned_pointer);
    assert_eq!(
        record.original().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        crate::output::operation_error_class(record.original().as_ref()),
        class
    );
    drop((outcomes, snapshot));
    assert_eq!(
        bank.available_permits(),
        total - AbandonedScan::body_bytes()
    );
    assert_eq!(
        record.original().downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    drop(record);
    assert_eq!(bank.available_permits(), total);
}

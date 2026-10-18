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
    assert!(Arc::ptr_eq(outcomes._storage.semaphore(), &bank));
    assert_eq!(outcomes._storage.num_permits(), body);
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
    let kept = std::mem::size_of::<HistorySnapshot>() + std::mem::size_of::<AbandonedScan>();
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

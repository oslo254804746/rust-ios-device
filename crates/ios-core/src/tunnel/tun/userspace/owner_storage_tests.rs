use super::*;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn task_handle_real_backing_one_byte_refusal_and_original_joins_keep_fee_until_drop() {
    let required = 4 * std::mem::size_of::<OwnedTask>();
    let short = Arc::new(Semaphore::new(required - 1));
    assert!(runtime_storage::task_handles(Some(&short)).is_err());
    assert_eq!(short.available_permits(), required - 1);
    let bank = Arc::new(Semaphore::new(required));
    let mut handles = runtime_storage::task_handles(Some(&bank)).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let ledger = Ledger::new();
    handles.initialize(std::array::from_fn(|_| {
        OwnedTask::new(
            tokio::spawn(async { Ok(()) }),
            ParentRole::Writer,
            ledger.clone(),
        )
    }));
    assert_eq!(handles.len(), 4);
    for handle in &handles {
        handle.join().await;
    }
    ledger.result(None).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(handles);
    assert_eq!(bank.available_permits(), required);
}

#[tokio::test]
async fn owner_constructor_last_byte_refusal_refunds_actual_bodies_before_workers_and_strict_eof() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let needed = IPV6_HEADER_LEN + runtime_channel_bytes() + runtime_owner_bytes();
    let bank = Arc::new(Semaphore::new(needed - 1));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let error = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(Arc::clone(&bank)),
    )
    .await
    .err()
    .expect("owner last-byte shortage started workers");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), needed - 1);
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
}

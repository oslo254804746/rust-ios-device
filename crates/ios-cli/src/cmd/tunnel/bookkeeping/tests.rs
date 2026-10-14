use super::*;

#[test]
fn write_batch_bytes_exact_three_backings_and_one_byte_refusal_refund() {
    for capacity in [0, 1, 4] {
        let bytes = WriteBatch::backing_bytes(capacity);
        let bank = Arc::new(Semaphore::new(bytes));
        let batch = WriteBatch::new(&bank, capacity).unwrap();
        assert_eq!(bank.available_permits(), 0);
        drop(batch);
        assert_eq!(bank.available_permits(), bytes);
        if bytes != 0 {
            let bank = Arc::new(Semaphore::new(bytes - 1));
            let error = WriteBatch::new(&bank, capacity).err().unwrap();
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), bytes - 1);
        }
    }
}

#[test]
fn write_batch_bytes_rejects_count_before_retaining_any_storage() {
    let bank = Arc::new(Semaphore::new(4096));
    assert!(WriteBatch::new(&bank, super::super::MAX_FAILURE_RECORDS + 1).is_err());
    assert_eq!(bank.available_permits(), 4096);
}

#[test]
fn write_batch_payload_last_shared_owner_keeps_actual_body_and_refunds_unused_context() {
    let bytes = WriteBatch::backing_bytes(0);
    assert_eq!(
        bytes,
        std::mem::size_of::<FailureWriteOwner>() + std::mem::size_of::<SourceFailuresContext>()
    );
    let bank = Arc::new(Semaphore::new(bytes));
    let batch = WriteBatch::new(&bank, 0).unwrap();
    let owner = batch.owner();
    let copied = Arc::clone(&owner);
    assert_eq!(bank.available_permits(), 0);
    batch.finish(Ok(())).unwrap();
    assert_eq!(
        bank.available_permits(),
        std::mem::size_of::<SourceFailuresContext>()
    );
    drop(owner);
    assert_eq!(
        bank.available_permits(),
        std::mem::size_of::<SourceFailuresContext>()
    );
    drop(copied);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn write_batch_payload_refusal_precedes_every_destination_backing_and_preserves_original_bank() {
    let bytes =
        std::mem::size_of::<FailureWriteOwner>() + std::mem::size_of::<SourceFailuresContext>();
    let bank = Arc::new(Semaphore::new(bytes - 1));
    let error = WriteBatch::new(&bank, 1).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), bytes - 1);
    bank.add_permits(1);
    let empty = WriteBatch::new(&bank, 0).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(empty);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn success_only_and_empty_batches_never_allocate_failure_destination() {
    let capacity = 1;
    let bytes = WriteBatch::success_only_bytes(capacity);
    let bank = Arc::new(Semaphore::new(bytes));
    let batch = WriteBatch::success_only(&bank, capacity).unwrap();
    assert!(batch.destination.is_none());
    assert_eq!(bank.available_permits(), 0);
    drop(batch);
    assert_eq!(bank.available_permits(), bytes);
    let empty = WriteBatch::new(&bank, 0).unwrap();
    assert!(empty.destination.is_none());
    drop(empty);
    assert_eq!(bank.available_permits(), bytes);
}

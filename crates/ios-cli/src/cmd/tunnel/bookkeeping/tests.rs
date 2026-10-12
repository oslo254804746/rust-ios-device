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

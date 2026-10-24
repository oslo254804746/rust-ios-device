use super::*;

#[test]
fn write_batch_bytes_exact_backings_and_whole_shared_owner_refuse_last_byte_and_refund() {
    for capacity in [0, 1, 4] {
        let bytes = WriteBatch::backing_bytes(capacity);
        let bank = Arc::new(Semaphore::new(bytes));
        let allocated = Arc::new(Mutex::new(Vec::new()));
        let captured = allocated.clone();
        let charged = bank.clone();
        let batch = super::super::metadata_vec::with_allocation_hook(
            move |layout, pointer| {
                captured
                    .lock()
                    .unwrap()
                    .push((layout, pointer, charged.available_permits()));
            },
            || WriteBatch::new(&bank, capacity).unwrap(),
        );
        let observed = allocated.lock().unwrap();
        assert_eq!(observed.len(), if capacity == 0 { 2 } else { 6 });
        if capacity != 0 {
            assert_eq!(observed[0].0.size(), Destination::minimum_bytes());
            assert_eq!(
                observed[2].0,
                std::alloc::Layout::array::<ScanIntent>(capacity).unwrap()
            );
            assert_eq!(observed[2].1, batch.intents.as_slice().as_ptr() as usize);
            assert_eq!(
                observed[3].0,
                std::alloc::Layout::array::<super::super::reconnect::Receipt>(capacity).unwrap()
            );
            assert_eq!(
                observed[3].1,
                batch
                    .owner
                    .state
                    .lock()
                    .unwrap()
                    .receipts
                    .as_slice()
                    .as_ptr() as usize
            );
            assert_eq!(
                observed[4].0,
                std::alloc::Layout::array::<PhysicalKeepAlive>(3 * capacity).unwrap()
            );
            assert_eq!(observed[4].1, batch.releases.as_slice().as_ptr() as usize);
        }
        let payload_index = usize::from(capacity != 0);
        assert_eq!(
            observed[payload_index].0,
            std::alloc::Layout::new::<ReceiptPayload>()
        );
        assert_eq!(
            observed[payload_index].1,
            batch
                .owner
                .state
                .lock()
                .unwrap()
                .context_destination
                .as_ref()
                .unwrap()
                .as_ptr() as usize
        );
        // The owner and this actual payload were admitted together before alloc.
        let destination_bytes = if capacity == 0 {
            0
        } else {
            Destination::minimum_bytes()
        };
        assert_eq!(
            observed[payload_index].2,
            bytes
                - destination_bytes
                - FailureWriteOwner::body_bytes()
                - std::mem::size_of::<ReceiptPayload>()
        );
        let last = observed.last().unwrap();
        assert_eq!(last.0.size(), FailureWriteOwner::body_bytes());
        assert_eq!(last.2, 0);
        let owner = std::ptr::from_ref(&*batch.owner) as usize;
        assert!(last.1 <= owner && owner < last.1 + last.0.size());
        assert_eq!(
            batch.owner.storage().num_permits(),
            FailureWriteOwner::body_bytes()
        );
        assert!(Arc::ptr_eq(batch.owner.storage().semaphore(), &bank));
        assert_eq!(
            observed.iter().map(|entry| entry.0.size()).sum::<usize>(),
            bytes
        );
        drop(observed);
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
        FailureWriteOwner::body_bytes() + std::mem::size_of::<ReceiptPayload>()
    );
    let bank = Arc::new(Semaphore::new(bytes));
    let batch = WriteBatch::new(&bank, 0).unwrap();
    let owner = batch.owner();
    let copied = owner.clone();
    assert_eq!(bank.available_permits(), 0);
    batch.finish(Ok(())).unwrap();
    assert_eq!(
        bank.available_permits(),
        std::mem::size_of::<ReceiptPayload>()
    );
    drop(owner);
    assert_eq!(
        bank.available_permits(),
        std::mem::size_of::<ReceiptPayload>()
    );
    drop(copied);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn write_batch_payload_refusal_precedes_every_destination_backing_and_preserves_original_bank() {
    let bytes = FailureWriteOwner::body_bytes() + std::mem::size_of::<ReceiptPayload>();
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

use super::*;

#[test]
fn metadata_vec_growth_refusal_preserves_original_rows_and_last_iterator_release() {
    let bank = Arc::new(Semaphore::new(3 * std::mem::size_of::<usize>()));
    let mut rows = KnownVec::new(Arc::clone(&bank), 8);
    rows.push(1_usize).unwrap();
    rows.push(2).unwrap();
    assert!(rows.push(3).is_err());
    assert_eq!(rows.as_slice(), &[1, 2]);
    assert_eq!(bank.available_permits(), std::mem::size_of::<usize>());
    let mut iterator = rows.into_iter();
    assert_eq!(iterator.next(), Some(1));
    assert_eq!(bank.available_permits(), std::mem::size_of::<usize>());
    drop(iterator);
    assert_eq!(bank.available_permits(), 3 * std::mem::size_of::<usize>());
}

#[test]
fn metadata_vec_actual_capacity_and_count_ceiling_refund_all_storage() {
    let bank = Arc::new(Semaphore::new(1024));
    let mut rows = KnownVec::new(Arc::clone(&bank), 3);
    for index in 0..3_u64 {
        rows.push(index).unwrap();
    }
    assert!(rows.push(4).is_err());
    assert_eq!(rows.len(), 3);
    assert_eq!(
        bank.available_permits(),
        1024 - rows.values.capacity() * std::mem::size_of::<u64>()
    );
    drop(rows);
    assert_eq!(bank.available_permits(), 1024);
}

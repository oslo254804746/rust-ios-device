use super::*;

#[tokio::test]
async fn desired_listing_backing_and_key_follow_iterator_and_transferred_owner() {
    let bank = Arc::new(Semaphore::new(65536));
    let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
    let key = "synthetic-desired-key";
    let ticket = lifecycle.ticket(key).await.unwrap();
    lifecycle.stop(key).await;
    let before = bank.available_permits();
    let rows = lifecycle.desired().await.unwrap();
    assert_eq!(rows.len(), 1);
    let copied = &rows.as_slice()[0];
    assert_eq!(copied.udid, key);
    assert!(matches!(
        copied.snapshot.desired,
        super::super::intent::Desired::Stopped
    ));
    assert!(Arc::ptr_eq(&copied.ticket.gate, &ticket.gate));
    let key_bytes = copied.udid.capacity();
    let row_bytes = 2 * std::mem::size_of::<DesiredRow>();
    assert_eq!(bank.available_permits(), before - key_bytes - row_bytes);
    let mut iterator = rows.into_iter();
    let DesiredRow {
        udid,
        _key_storage: key_storage,
        ..
    } = iterator.next().unwrap();
    assert_eq!(bank.available_permits(), before - key_bytes - row_bytes);
    // A writer can move this exact key and permit out of the listing. Its
    // lifetime is independent of the source list/iterator backing.
    drop(iterator);
    assert_eq!(bank.available_permits(), before - key_bytes);
    assert_eq!(udid, key);
    drop(udid);
    drop(key_storage);
    assert_eq!(bank.available_permits(), before);
    drop(ticket);
    drop(lifecycle);
    assert_eq!(bank.available_permits(), 65536);
}

#[tokio::test]
async fn desired_listing_refused_key_restores_all_partial_rows_and_original_states() {
    let bank = Arc::new(Semaphore::new(65536));
    let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
    let a = lifecycle.ticket("synthetic-a").await.unwrap();
    let b = lifecycle.ticket("synthetic-b").await.unwrap();
    lifecycle.stop("synthetic-a").await;
    lifecycle.stop("synthetic-b").await;
    let old_a = a.desired();
    let old_b = b.desired();
    // Enough for two inline rows and only the first independent key: a
    // partially built batch must be dropped in full, with no intent mutation.
    let allowed = 2 * std::mem::size_of::<DesiredRow>() + "synthetic-a".len();
    let held = Arc::clone(&bank)
        .try_acquire_many_owned((bank.available_permits() - allowed) as u32)
        .unwrap();
    let error = lifecycle.desired().await.err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(bank.available_permits(), allowed);
    assert!(Arc::ptr_eq(&old_a.version, &a.desired().version));
    assert!(Arc::ptr_eq(&old_b.version, &b.desired().version));
    assert_eq!(lifecycle.coordinator_count().await, 2);
    drop(held);
    assert_eq!(lifecycle.desired().await.unwrap().len(), 2);
}

#[tokio::test]
async fn desired_listing_one_byte_backing_refusal_does_not_change_stopped_gate() {
    let key = "synthetic-stopped-key";
    let bank = Arc::new(Semaphore::new(65536));
    let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
    let ticket = lifecycle.ticket(key).await.unwrap();
    lifecycle.stop(key).await;
    let old = ticket.desired();
    let available = 2 * std::mem::size_of::<DesiredRow>() - 1;
    let held = Arc::clone(&bank)
        .try_acquire_many_owned((bank.available_permits() - available) as u32)
        .unwrap();
    assert!(lifecycle.desired().await.is_err());
    assert_eq!(bank.available_permits(), available);
    assert!(Arc::ptr_eq(&old.version, &ticket.desired().version));
    assert!(matches!(
        ticket.desired().desired,
        super::super::intent::Desired::Stopped
    ));
    drop(held);
}

#[tokio::test]
async fn desired_listing_empty_clear_and_terminal_need_no_storage() {
    let bank = Arc::new(Semaphore::new(65536));
    let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank));
    let ticket = lifecycle.ticket("synthetic-old-epoch").await.unwrap();
    lifecycle.stop("synthetic-old-epoch").await;
    let old = lifecycle.desired().await.unwrap();
    let before = bank.available_permits();
    lifecycle.clear();
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(before as u32)
        .unwrap();
    assert_eq!(lifecycle.desired().await.unwrap().len(), 0);
    assert_eq!(bank.available_permits(), 0);
    assert!(!old.as_slice()[0].ticket.is_current());
    lifecycle.close();
    assert_eq!(lifecycle.desired().await.unwrap().len(), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(held);
    drop(ticket);
    drop(old);
    drop(lifecycle);
    assert_eq!(bank.available_permits(), 65536);
}

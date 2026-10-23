use super::*;

#[tokio::test]
async fn desired_listing_backing_and_key_follow_iterator_and_transferred_owner() {
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    let key = "synthetic-desired-key";
    let ticket = lifecycle.ticket(key).await.unwrap();
    lifecycle.stop(key).await;
    let row_bytes = 2 * std::mem::size_of::<DesiredRow>();
    let before = row_bytes + key.len();
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - before) as u32)
        .unwrap();
    let observed_bank = bank.clone();
    let allocations = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = allocations.clone();
    let guard = super::super::metadata_vec::on_allocation(move |layout, pointer| {
        let mut observed = observed.lock().unwrap();
        if observed.is_empty() {
            assert_eq!(layout.size(), row_bytes);
            assert_eq!(observed_bank.available_permits(), key.len());
        } else {
            assert_eq!(observed.len(), 1);
            assert_eq!(layout.size(), key.len());
            assert_eq!(layout.align(), 1);
            assert_eq!(
                observed_bank.available_permits(),
                0,
                "original key grant precedes allocation"
            );
        }
        observed.push((layout.size(), pointer));
    });
    let rows = lifecycle.desired().await.unwrap();
    drop(guard);
    assert_eq!(rows.len(), 1);
    let copied = &rows.as_slice()[0];
    assert_eq!(copied.udid, key);
    assert!(matches!(
        copied.snapshot.desired,
        super::super::intent::Desired::Stopped
    ));
    assert!(copied.ticket.gate.ptr_eq(&ticket.gate));
    let key_bytes = copied.udid.capacity();
    assert_eq!(key_bytes, key.len());
    assert_eq!(copied._key_storage.num_permits(), key_bytes);
    assert!(Arc::ptr_eq(copied._key_storage.semaphore(), &bank));
    assert_eq!(
        *allocations.lock().unwrap(),
        [
            (row_bytes, rows.as_slice().as_ptr() as usize),
            (key_bytes, copied.udid.as_ptr() as usize),
        ]
    );
    assert_ne!(copied.udid.as_ptr(), ticket.gate.udid.as_ptr());
    let key_pointer = copied.udid.as_ptr();
    assert_eq!(bank.available_permits(), before - key_bytes - row_bytes);
    bank.close();
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
    assert_eq!(udid.as_ptr(), key_pointer);
    drop(udid);
    assert_eq!(
        bank.available_permits(),
        before - key_bytes,
        "key fee survives actual String deallocation"
    );
    drop(key_storage);
    assert_eq!(bank.available_permits(), before);
    drop(held);
    drop(ticket);
    drop(lifecycle);
    assert_eq!(bank.available_permits(), 65536);
}

#[tokio::test]
async fn desired_listing_one_byte_key_refusal_never_allocates_the_copy_or_changes_original() {
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    let key = "synthetic-独立-key";
    let ticket = lifecycle.ticket(key).await.unwrap();
    lifecycle.stop(key).await;
    let original = ticket.desired();
    let pointer = ticket.gate.udid.as_ptr();
    let row_bytes = 2 * std::mem::size_of::<DesiredRow>();
    let available = row_bytes + key.len() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - available) as u32)
        .unwrap();
    let allocated = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = allocated.clone();
    let observed_bank = bank.clone();
    let guard = super::super::metadata_vec::on_allocation(move |layout, _| {
        assert_eq!(
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
            0,
            "no refused key backing"
        );
        assert_eq!(layout.size(), row_bytes);
        assert_eq!(observed_bank.available_permits(), key.len() - 1);
    });
    let error = lifecycle.desired().await.err().unwrap();
    drop(guard);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(allocated.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), available);
    assert_eq!(ticket.gate.udid.as_ptr(), pointer);
    assert!(crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &original.version,
        &ticket.desired().version
    ));
    assert!(matches!(
        ticket.desired().desired,
        super::super::intent::Desired::Stopped
    ));
    assert_eq!(lifecycle.coordinator_count().await, 1);
    drop(held);
    let rows = lifecycle.desired().await.unwrap();
    assert_eq!(rows.as_slice()[0].udid, key);
    assert_ne!(rows.as_slice()[0].udid.as_ptr(), pointer);
    drop(rows);
    drop(ticket);
    drop(original);
    drop(lifecycle);
    assert_eq!(bank.available_permits(), 65536);
}

#[tokio::test]
async fn desired_listing_refused_key_restores_all_partial_rows_and_original_states() {
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
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
    assert!(crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &old_a.version,
        &a.desired().version
    ));
    assert!(crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &old_b.version,
        &b.desired().version
    ));
    assert_eq!(lifecycle.coordinator_count().await, 2);
    drop(held);
    assert_eq!(lifecycle.desired().await.unwrap().len(), 2);
}

#[tokio::test]
async fn desired_listing_one_byte_backing_refusal_does_not_change_stopped_gate() {
    let key = "synthetic-stopped-key";
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    let ticket = lifecycle.ticket(key).await.unwrap();
    lifecycle.stop(key).await;
    let old = ticket.desired();
    let available = 2 * std::mem::size_of::<DesiredRow>() - 1;
    let held = Arc::clone(&bank)
        .try_acquire_many_owned((bank.available_permits() - available) as u32)
        .unwrap();
    assert!(lifecycle.desired().await.is_err());
    assert_eq!(bank.available_permits(), available);
    assert!(crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &old.version,
        &ticket.desired().version
    ));
    assert!(matches!(
        ticket.desired().desired,
        super::super::intent::Desired::Stopped
    ));
    drop(held);
}

#[tokio::test]
async fn desired_listing_empty_clear_and_terminal_need_no_storage() {
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(65536);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
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

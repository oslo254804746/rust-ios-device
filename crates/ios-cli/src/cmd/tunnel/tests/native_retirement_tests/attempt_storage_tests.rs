use super::*;

#[tokio::test]
async fn native_attempt_storage_closed_zero_keeps_each_original_runtime_and_all_joins() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let scope = ownership::Ownership::admitted(state.inner.native.clone(), &ticket).unwrap();
    let mut first = scope.reserve_owned().unwrap();
    let first_device = source
        .connect(DEVICE, Route::Wifi(None), TunMode::Userspace)
        .await
        .unwrap();
    let old = first_device.native_handle().unwrap().clone();
    first.attach(old.clone()).unwrap();
    let mut second = scope.reserve_owned().unwrap();
    let next_device = source
        .connect(DEVICE, Route::Wifi(None), TunMode::Userspace)
        .await
        .unwrap();
    let new = next_device.native_handle().unwrap().clone();
    second.attach(new.clone()).unwrap();
    assert_ne!(
        &*first as *const _ as *const () as usize,
        &*second as *const _ as *const () as usize
    );
    assert!(!Arc::ptr_eq(&old, &new));
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let bytes = ownership::Ownership::attempt_bytes();
    assert_eq!(bank.available_permits(), 0);
    drop(first);
    assert_eq!(bank.available_permits(), bytes);
    assert!(!old.is_alive());
    assert!(new.is_alive());
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert_eq!(state.inner.native.snapshot().unpublished, 1);
    second.complete();
    drop(second);
    assert_eq!(bank.available_permits(), 2 * bytes);
    let lease = scope.take_completed(&new).unwrap();
    drop(scope);
    assert!(new.is_alive());
    assert_eq!(bank.available_permits(), 2 * bytes);
    drop(lease);
    assert_eq!(state.inner.native.snapshot().retired, 2);
    assert!(!new.is_alive());
    assert_eq!(source.connect_calls.load(Ordering::SeqCst), 2);
    state.notify_shutdown();
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert!(old.shutdown_status().is_ok() && new.shutdown_status().is_ok());
    let peers = std::mem::take(&mut *source.peers.lock().unwrap());
    tokio::time::timeout(OBSERVE, async {
        for mut peer in peers {
            assert_eq!(peer.read(&mut [0u8; 1]).await.unwrap(), 0);
        }
        while baseline() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(next_device);
    drop(first_device);
    drop(ticket);
    state.clear().await;
    drop(state);
    drop(held);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

use super::*;

fn text_bytes(record: &TunnelRecord) -> usize {
    [
        &record.udid,
        &record.interface,
        &record.connection_type,
        &record.mode,
        &record.address,
        &record.tunnel_address,
    ]
    .iter()
    .map(|s| s.len())
    .sum::<usize>()
        + record.userspace_tun_host.as_ref().map_or(0, |s| s.len())
}

#[tokio::test]
async fn record_independent_copies_pay_after_original_native_workers_join() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let initial = publish(&state, Route::Wifi(None)).await.unwrap();
    let native = source.handle();
    let bank = state.inner.global_metadata_bytes.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    assert!(state
        .get_record(DEVICE)
        .await
        .unwrap_err()
        .is::<record_owner::CopyAdmission>());
    assert!(native.is_alive());
    assert_eq!(source.connect_calls.load(Ordering::SeqCst), 1);
    drop(held);
    let single = state.get_record(DEVICE).await.unwrap().unwrap();
    let list = state.list_records().await.unwrap();
    let bytes = text_bytes(&initial);
    let wire = serde_json::to_vec(&initial).unwrap();
    assert_ne!(initial.udid.as_ptr(), single.udid.as_ptr());
    assert_ne!(initial.udid.as_ptr(), list.as_slice()[0].udid.as_ptr());
    state.notify_shutdown();
    state.drain_native(OBSERVE).await.unwrap();
    state.clear().await;
    let peers = std::mem::take(&mut *source.peers.lock().unwrap());
    tokio::time::timeout(OBSERVE, async {
        for mut peer in peers {
            assert_eq!(
                peer.read(&mut [0u8; 1]).await.unwrap(),
                0,
                "strict native EOF"
            );
        }
        while baseline() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(native.shutdown_status().is_ok());
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(serde_json::to_vec(&single).unwrap(), wire);
    assert_eq!(serde_json::to_vec(&list.as_slice()[0]).unwrap(), wire);
    let remaining = bank.available_permits();
    drop(initial);
    assert_eq!(bank.available_permits(), remaining + bytes);
    drop(single);
    assert_eq!(bank.available_permits(), remaining + 2 * bytes);
    let list_backing = list.capacity() * std::mem::size_of::<PaidRecord>();
    drop(list);
    assert_eq!(
        bank.available_permits(),
        remaining + 3 * bytes + list_backing
    );
}

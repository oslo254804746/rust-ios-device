use super::*;

#[tokio::test]
async fn coordinator_gate_owner_parallel_old_tickets_keep_original_key_and_closed_pool_body() {
    let key = "original-gate-key";
    let total = Lifecycle::initial_metadata_bytes(key);
    let bank = Arc::new(Semaphore::new(total));
    let lifecycle = Lifecycle::with_metadata(bank.clone());
    let ticket = lifecycle.ticket(key).await.unwrap();
    let alias = lifecycle.ticket(key).await.unwrap();
    assert_eq!(ticket.gate.strong_count(), 3);
    assert!(ticket.same_incarnation(&alias));
    assert_eq!(bank.available_permits(), 0);
    drop(alias);
    let pointer = &*ticket.gate as *const DeviceGate as usize;
    let key_pointer = ticket.gate.udid.as_ptr() as usize;
    bank.close();
    lifecycle.clear();
    assert!(!ticket.is_current());
    drop(lifecycle);
    let backing = 2 * std::mem::size_of::<DeviceRow>();
    assert_eq!(bank.available_permits(), backing);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    std::thread::scope(|scope| {
        for _ in 0..7 {
            let retained = ticket.clone();
            assert!(retained.same_incarnation(&ticket));
            let barrier = barrier.clone();
            scope.spawn(move || {
                assert_eq!(&*retained.gate as *const DeviceGate as usize, pointer);
                assert_eq!(retained.gate.udid.as_ptr() as usize, key_pointer);
                assert_eq!(retained.gate.udid, key);
                assert!(!retained.is_current());
                barrier.wait();
                drop(retained);
            });
        }
        barrier.wait();
        drop(ticket);
    });
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn coordinator_gate_owner_original_live_and_stopped_roots_survive_capacity_pruning_and_clear()
{
    let total = super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES;
    let bank = Arc::new(Semaphore::new(total));
    let lifecycle = Lifecycle::with_metadata(bank.clone());
    let live = lifecycle.ticket("retained-live-gate").await.unwrap();
    drop(lifecycle.ticket("persistent-stopped-gate").await.unwrap());
    lifecycle.stop("persistent-stopped-gate").await;
    let stopped_pointer = {
        let devices = lifecycle.devices.lock().await;
        let stopped = devices.get("persistent-stopped-gate").unwrap();
        assert_eq!(
            stopped.strong_count(),
            1,
            "persistent state is the only non-table retention reason"
        );
        &**stopped as *const DeviceGate as usize
    };
    for index in 2..MAX_DEVICE_GATES {
        drop(
            lifecycle
                .ticket(&format!("idle-gate-{index}"))
                .await
                .unwrap(),
        );
    }
    assert_eq!(lifecycle.coordinator_count().await, MAX_DEVICE_GATES);
    assert_eq!(live.gate.strong_count(), 2);
    let before = bank.available_permits();
    let fresh = lifecycle.ticket("next-admitted-gate").await.unwrap();
    assert_eq!(lifecycle.coordinator_count().await, 3);
    assert!(bank.available_permits() > before);
    assert!(live.is_current());
    let stopped = lifecycle.ticket("persistent-stopped-gate").await.unwrap();
    assert_eq!(
        &*stopped.gate as *const DeviceGate as usize,
        stopped_pointer
    );
    assert!(matches!(
        stopped.desired().desired,
        super::super::intent::Desired::Stopped
    ));
    lifecycle.clear();
    let renewed = lifecycle.ticket("retained-live-gate").await.unwrap();
    assert!(live.gate.ptr_eq(&renewed.gate));
    assert!(!live.same_incarnation(&renewed));
    assert!(!live.is_current());
    assert!(renewed.is_current());
    assert_eq!(live.gate.strong_count(), 3);
    drop((lifecycle, live, fresh, stopped, renewed));
    assert_eq!(bank.available_permits(), total);
}

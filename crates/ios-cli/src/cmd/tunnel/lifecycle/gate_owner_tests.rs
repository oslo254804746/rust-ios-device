use super::*;

#[test]
fn lifecycle_two_watch_roots_whole_admission_precedes_any_backing_and_final_views_hold_original_fees(
) {
    let total = Lifecycle::watch_bytes();
    let short_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total - 1);
    let short = short_birth.raw().clone();
    let error = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("partial lifecycle watch prefix allocated"),
        || Lifecycle::with_metadata(short_birth.clone()).err().unwrap(),
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(short.available_permits(), total - 1);
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = observed.clone();
    let credit = bank.clone();
    let lifecycle = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(credit.available_permits(), 0);
            seen.lock().unwrap().push((layout.size(), pointer));
        },
        || Lifecycle::with_metadata(bank_birth.clone()).unwrap(),
    );
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert_eq!(
        observed[0].0,
        watch::body_bytes::<Option<crate::cmd::tunnel::ControlIdentity>>()
    );
    assert_eq!(
        observed[1].0,
        watch::body_bytes::<crate::cmd::tunnel::ControlIdentity>()
    );
    let global_value = {
        let value = lifecycle.shutdown.borrow();
        std::ptr::from_ref(&*value) as usize
    };
    let scan_value = {
        let value = lifecycle.scan.borrow();
        std::ptr::from_ref(&*value) as usize
    };
    assert!(observed[0].1 <= global_value && global_value < observed[0].1 + observed[0].0);
    assert!(observed[1].1 <= scan_value && scan_value < observed[1].1 + observed[1].0);
    assert_ne!(observed[0].1, observed[1].1);
    drop(observed);
    bank.close();
    let (global, scan) = super::super::metadata_vec::with_allocation_hook(
        |_, _| panic!("closed lifecycle view allocated"),
        || {
            let global = lifecycle.global_epoch().unwrap();
            let scan = lifecycle.scan_control(Duration::from_secs(1)).unwrap();
            lifecycle.close();
            assert!(global
                .check()
                .unwrap_err()
                .is::<ios_core::OperationCancelled>());
            (global, scan)
        },
    );
    drop(lifecycle);
    assert_eq!(bank.available_permits(), 0);
    drop(global);
    assert_eq!(bank.available_permits(), 0);
    drop(scan);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn coordinator_gate_owner_parallel_old_tickets_keep_original_key_and_closed_pool_body() {
    let key = "original-gate-key";
    let total = Lifecycle::initial_metadata_bytes(key) + Lifecycle::watch_bytes();
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
    use futures_util::FutureExt;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = seen.clone();
    let charged = bank.clone();
    let ticket = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            captured
                .lock()
                .unwrap()
                .push((layout, pointer, charged.available_permits()));
        },
        || lifecycle.ticket(key).now_or_never().unwrap().unwrap(),
    );
    let backing_pointer = lifecycle.devices.lock().await.rows.as_ptr() as usize;
    {
        let observed = seen.lock().unwrap();
        assert_eq!(observed.len(), 5);
        assert_eq!(
            observed[0].0,
            std::alloc::Layout::array::<u8>(key.len()).unwrap()
        );
        assert_eq!(observed[0].1, ticket.gate.udid.as_ptr() as usize);
        assert_eq!(observed[0].2, 2 * std::mem::size_of::<DeviceRow>());
        assert_eq!(
            observed[1].0,
            std::alloc::Layout::array::<DeviceRow>(2).unwrap()
        );
        assert_eq!(observed[1].2, 0);
        assert_eq!(observed[1].1, backing_pointer);
        assert_eq!(
            observed[2].0.size(),
            watch::body_bytes::<crate::cmd::tunnel::ControlIdentity>()
        );
        assert_eq!(
            observed[3].0.size(),
            watch::body_bytes::<crate::cmd::tunnel::ControlIdentity>()
        );
        assert_eq!(observed[4].0.size(), DeviceGate::body_bytes());
        for allocation in &observed[2..] {
            assert_eq!(allocation.2, 0);
        }
        let data = std::ptr::from_ref(&*ticket.gate) as usize;
        assert!(observed[4].1 <= data && data < observed[4].1 + observed[4].0.size());
        let generation = ticket.gate.generation.borrow();
        let generation_value = std::ptr::from_ref(&*generation) as usize;
        assert!(
            observed[2].1 <= generation_value
                && generation_value < observed[2].1 + observed[2].0.size()
        );
        let intent = ticket.gate.intent.subscribe();
        let desired = intent.borrow();
        let desired_value = std::ptr::from_ref(&*desired) as usize;
        assert!(
            observed[3].1 <= desired_value && desired_value < observed[3].1 + observed[3].0.size()
        );
        assert_ne!(observed[2].1, observed[3].1);
        assert_ne!(observed[3].1, observed[4].1);
        assert_eq!(
            ticket.gate.storage().num_permits(),
            DeviceGate::body_bytes() + key.len()
        );
        assert!(Arc::ptr_eq(ticket.gate.storage().semaphore(), &bank));
    }
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
    let backing = 2 * std::mem::size_of::<DeviceRow>()
        + watch::body_bytes::<crate::cmd::tunnel::ControlIdentity>();
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
    let bank_birth = crate::cmd::tunnel::birth_bank::BirthBank::new(total);
    let bank = bank_birth.raw().clone();
    let lifecycle = Lifecycle::with_metadata(bank_birth.clone()).unwrap();
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

use super::*;

fn build(bank: Arc<Semaphore>, source: Arc<FakeSource>) -> Result<TunnelAgentState> {
    TunnelAgentState::with_source_retry_metadata(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source,
        TUNNEL_CONNECT_TIMEOUT,
        reconnect::RetryPolicy::default(),
        bank,
    )
}

fn minimum(state: &TunnelAgentState) -> usize {
    TunnelAgentOwner::body_bytes()
        + state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentState::watch_bytes()
        + failure_slot::Counts::body_bytes()
}

fn no_source_calls(source: &FakeSource) {
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(A), 0);
}

#[test]
fn manager_init_body_and_children_short_refusal_refund_original_bank_before_source() {
    let source = FakeSource::new();
    let probe = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let total = minimum(&probe);
    drop(probe);
    for available in [
        0,
        TunnelAgentOwner::body_bytes() - 1,
        TunnelAgentOwner::body_bytes(),
        total - 1,
    ] {
        let bank = Arc::new(Semaphore::new(available));
        let error = build(bank.clone(), source.clone()).err().unwrap();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
        no_source_calls(&source);
    }
    let bank = Arc::new(Semaphore::new(total));
    let manager = build(bank.clone(), source.clone()).unwrap();
    assert_eq!(bank.available_permits(), 0);
    assert!(Arc::ptr_eq(&manager.inner.global_metadata_bytes, &bank));
    let retained = manager.clone();
    assert!(manager.inner.ptr_eq(&retained.inner));
    drop(manager);
    assert_eq!(bank.available_permits(), 0);
    no_source_calls(&source);
    drop(retained);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn manager_init_last_strong_destroys_source_but_last_weak_retains_original_body() {
    let source = FakeSource::new();
    let source_weak = Arc::downgrade(&source);
    let bank = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let manager = build(bank.clone(), source.clone()).unwrap();
    let weak = manager.inner.downgrade();
    let second_weak = weak.clone();
    let retained = weak.upgrade().unwrap();
    assert!(manager.inner.ptr_eq(&retained));
    no_source_calls(&source);
    drop(source);
    drop(manager);
    assert!(source_weak.upgrade().is_some());
    drop(retained);
    assert!(
        source_weak.upgrade().is_none(),
        "source fields must end at last strong"
    );
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - TunnelAgentOwner::body_bytes()
    );
    drop(weak);
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - TunnelAgentOwner::body_bytes()
    );
    drop(second_weak);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[test]
fn manager_init_parallel_weak_upgrades_keep_same_root_bank_then_refund_once() {
    let source = FakeSource::new();
    let source_weak = Arc::downgrade(&source);
    let bank = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let manager = build(bank.clone(), source.clone()).unwrap();
    let pointer = &*manager.inner as *const TunnelAgentInner as usize;
    let weak = manager.inner.downgrade();
    let barrier = Arc::new(std::sync::Barrier::new(9));
    no_source_calls(&source);
    drop(source);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let weak = weak.clone();
            let barrier = barrier.clone();
            let bank = bank.clone();
            scope.spawn(move || {
                barrier.wait();
                for _ in 0..128 {
                    if let Some(inner) = weak.upgrade() {
                        assert_eq!(&*inner as *const TunnelAgentInner as usize, pointer);
                        assert!(Arc::ptr_eq(&inner.global_metadata_bytes, &bank));
                    }
                }
            });
        }
        barrier.wait();
        drop(manager);
    });
    assert!(source_weak.upgrade().is_none());
    assert!(weak.upgrade().is_none());
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - TunnelAgentOwner::body_bytes()
    );
    drop(weak);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[test]
fn manager_init_exact_configuration_preserves_root_and_transfers_body_to_original_new_bank() {
    let source = FakeSource::new();
    let mut manager = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let pointer = &*manager.inner as *const TunnelAgentInner;
    let old_bank = manager.inner.global_metadata_bytes.clone();
    let total = minimum(&manager);
    manager.set_global_metadata_limit(total);
    let bank = manager.inner.global_metadata_bytes.clone();
    assert_eq!(&*manager.inner as *const TunnelAgentInner, pointer);
    assert_eq!(
        old_bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
    assert_eq!(bank.available_permits(), 0);
    no_source_calls(&source);
    let weak = manager.inner.downgrade();
    drop(manager);
    assert_eq!(
        bank.available_permits(),
        total - TunnelAgentOwner::body_bytes()
    );
    assert!(weak.upgrade().is_none());
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

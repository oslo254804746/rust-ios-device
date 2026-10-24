use super::*;

#[tokio::test]
async fn metadata_lease_actual_manager_publishes_original_destinations_allocated_before_source() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    let allocations = Arc::new(StdMutex::new(Vec::new()));
    let captured = allocations.clone();
    let observed = source.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        captured
            .lock()
            .unwrap()
            .push((layout, pointer, observed.count(A)));
    });
    let mut work = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut work).is_pending());
    assert_eq!(source.count(A), 1);
    gate.add_permits(1);
    let record = work.await.unwrap();
    drop(hook);
    {
        let table = state.inner.tunnels.read().await;
        let original = &table[A];
        for owner in [&original._metadata, &original._global_metadata] {
            let pointer = &**owner as *const metadata_lease::Lease as usize;
            let allocated = allocations.lock().unwrap();
            let matching = allocated
                .iter()
                .filter(|(layout, base, calls)| {
                    layout.size() == metadata_lease::Lease::body_bytes()
                        && *calls == 0
                        && pointer >= *base
                        && pointer + std::mem::size_of::<metadata_lease::Lease>()
                            <= base + layout.size()
                })
                .count();
            assert_eq!(
                matching, 1,
                "published lease did not use its original pre-Source backing"
            );
        }
    }
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    drop(record);
    state.notify_shutdown(&mut control_input_1);
    drop(control_input_1);
}

#[tokio::test]
async fn metadata_lease_actual_manager_short_body_refuses_before_source_without_failure_write() {
    let source = FakeSource::new();
    let global = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let publication = Arc::new(Semaphore::new(MAX_PUBLISHED_METADATA_BYTES));
    let mut probe = publication_table::Table::<ManagedTunnel>::new(&global, &publication);
    let prepared = probe.prepare(A).unwrap();
    let destination = MAX_PUBLISHED_METADATA_BYTES - publication.available_permits();
    drop(prepared);
    drop(probe.take_all());
    let limit = destination + publication::Life::bytes() + metadata_lease::Lease::pair_bytes() - 1;
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut prepared_setup_0 = state.prepare_cutover::<2>().unwrap();
    let mut prepared_setup_1 = state.prepare_cutover::<1>().unwrap();

    let inner = state.inner.get_mut().unwrap();
    inner.metadata_bytes = Arc::new(Semaphore::new(limit));
    *inner.tunnels.get_mut() =
        publication_table::Table::new(&inner.global_metadata_bytes, &inner.metadata_bytes);
    let error = start(state.clone(), A).await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(error.is::<ownership::Admission>());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    assert!(state.inner.tunnels.read().await.is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    drop(error);
    state.clear(&mut prepared_setup_0).await;
    drop(prepared_setup_0);
    assert_eq!(state.inner.metadata_bytes.available_permits(), limit);
    state.notify_shutdown(&mut prepared_setup_1);
    drop(prepared_setup_1);
}

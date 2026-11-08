use super::*;

#[tokio::test]
async fn publication_ticket_actual_manager_refusal_is_before_source_and_keeps_original_empty_row() {
    let source = FakeSource::new();
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut prepared_setup_0 = state.prepare_cutover::<1>().unwrap();
    let mut prepared_setup_1 = state.prepare_cutover::<1>().unwrap();

    let global = state.inner.global_metadata_bytes.clone();
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let probe_p = Arc::new(Semaphore::new(MAX_PUBLISHED_METADATA_BYTES));
    let mut probe = publication_table::Table::<ManagedTunnel>::new(&global, &probe_p);
    let destination = probe.prepare(A).unwrap();
    let node_bytes = MAX_PUBLISHED_METADATA_BYTES - probe_p.available_permits();
    drop((destination, probe));
    let total = node_bytes
        + publication::Life::bytes()
        + metadata_lease::Lease::pair_bytes()
        + publication::TicketOwner::body_bytes()
        - 1;
    let published = Arc::new(Semaphore::new(total));
    let inner = state.inner.get_mut().unwrap();
    inner.metadata_bytes = published.clone();
    *inner.tunnels.get_mut() = publication_table::Table::new(&global, &published);
    let error = state
        .connect_and_store(A, Route::Wifi(None), &ticket, None, None)
        .await
        .unwrap_err();
    assert!(error.is::<publication::Admission>() && error.is::<ownership::Admission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(state.inner.tunnels.read().await.is_empty());
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert_eq!(state.inner.retry.receipt_count(), 0);
    state.notify_shutdown(&mut prepared_setup_0);
    drop(prepared_setup_0);
    state.clear(&mut prepared_setup_1).await;
    drop(prepared_setup_1);
    drop((ticket, state));
    assert_eq!(published.available_permits(), total);
    assert_eq!(global.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn publication_ticket_actual_manager_preserves_captured_ticket_through_clear_and_root_observer(
) {
    let source = FakeSource::new();
    source.script(A, [Behavior::Success]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    let global = state.inner.global_metadata_bytes.clone();
    let published = state.inner.metadata_bytes.clone();
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    state
        .connect_and_store(A, Route::Wifi(None), &ticket, None, None)
        .await
        .unwrap();
    let (root, key) = {
        let table = state.inner.tunnels.read().await;
        let row = table.get(A).unwrap();
        assert!(row._ticket.same_incarnation(&ticket));
        (row._ticket.clone(), row._ticket.udid().as_ptr())
    };
    assert_eq!(source.count(A), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(root.udid().as_ptr(), key);
    assert!(!root.is_current());
    state.notify_shutdown(&mut control_input_1);
    drop(control_input_1);
    drop((ticket, state));
    assert_eq!(
        published.available_permits(),
        MAX_PUBLISHED_METADATA_BYTES - publication::TicketOwner::body_bytes()
    );
    assert!(global.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(root);
    assert_eq!(published.available_permits(), MAX_PUBLISHED_METADATA_BYTES);
    assert_eq!(global.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

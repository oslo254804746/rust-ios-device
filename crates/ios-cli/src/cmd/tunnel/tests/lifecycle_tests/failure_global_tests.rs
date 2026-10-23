use super::*;

#[tokio::test]
async fn failure_global_root_actual_candidate_refusal_keeps_slot_source_and_failure_table_untouched(
) {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let bank = state.inner.global_metadata_bytes.clone();
    let leave = failure_table::Destination::minimum_bytes()
        + failure_table::Node::input_bytes(&Route::Usbmux)
        + A.len()
        + lifecycle::TicketOwner::body_bytes()
        + marker_identity::Owner::body_bytes()
        - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - leave) as u32)
        .unwrap();
    assert!(state
        .refresh_connect_deferred(A, Route::Usbmux, &ticket, Some(1))
        .await
        .is_none());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert!(state.inner.tunnels.read().await.is_empty());
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert_eq!(bank.available_permits(), leave);
    drop(held);
    state.notify_shutdown();
    state.clear().await;
    drop((ticket, state));
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

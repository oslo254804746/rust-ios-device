use super::*;
use futures_util::FutureExt;

const WRITE_BUDGET: Duration = Duration::from_millis(40);
const CROSS: Duration = Duration::from_millis(100);

fn bounded_state(source: Arc<FakeSource>, budget: Duration) -> TunnelAgentState {
    let mut state = state(source, TUNNEL_CONNECT_TIMEOUT);
    state.inner.get_mut().unwrap().failure_write_timeout = budget;
    state
}

async fn intent(state: &TunnelAgentState, udid: &str) -> FailureIntent {
    let ticket = state.inner.lifecycle.ticket(udid).await.unwrap();
    let slot = state.reserve_failure_slot(udid).await.unwrap();
    let mut node = failure_table::Node::owned(
        udid.into(),
        AttemptIdentity {
            route: Route::Usbmux,
            native_id: Some(1),
        },
        &state.inner.global_metadata_bytes,
    )
    .unwrap();
    node.row.slot = Some(slot);
    node.prepare_global(&ticket, &state.inner.global_metadata_bytes)
        .unwrap();
    FailureIntent::new(DeferredFailure {
        ticket: Some(
            lifecycle::TicketOwner::new(&ticket, &state.inner.global_metadata_bytes).unwrap(),
        ),
        node,
        receipt: None,
        destination: None,
    })
}

fn elapsed(error: &anyhow::Error) {
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Timeout
    );
}

async fn refused(budget: Duration) -> anyhow::Error {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Failure(std::io::ErrorKind::BrokenPipe)]);
    let state = bounded_state(source.clone(), budget);
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let error = state.refresh_once().await.unwrap_err();
    let unchanged = state.inner.failed_attempts.read().await.is_empty();
    let not_ready = !state.is_ready().await;
    let slots = state.inner.failure_slots.available_permits();
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    assert!(unchanged && not_ready);
    assert_eq!(source.count(A), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
    let originals = error
        .downcast_ref::<bookkeeping::SourceFailuresContext>()
        .unwrap();
    assert_eq!(originals.receipts().len(), 1);
    assert_eq!(
        originals.receipts()[0]
            .primary()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS - 1,
        "returned dual-cause receipt must retain its original admission owner"
    );
    error
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_zero_does_not_write_or_mark_ready() {
    elapsed(&refused(Duration::ZERO).await);
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_overflow_preserves_input_category_and_slots() {
    let error = refused(Duration::MAX).await;
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::InvalidInput
    );
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_deadline_crossing_before_commit_does_not_create_row() {
    let source = FakeSource::new();
    let state = bounded_state(source.clone(), WRITE_BUDGET);
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let pending = intent(&state, A).await;
    state
        .inner
        .lifecycle
        .on_next_failure_record(|_| std::thread::sleep(CROSS));
    let error = state
        .flush_failure_intents(vec![pending])
        .await
        .unwrap_err();
    let unchanged = state.inner.failed_attempts.read().await.is_empty();
    let slots = state.inner.failure_slots.available_permits();
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    elapsed(&error);
    assert!(unchanged && source.calls.lock().unwrap().is_empty());
    assert_eq!(slots, MAX_FAILURE_RECORDS);
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_late_insert_rolls_back_new_unobserved_row() {
    let source = FakeSource::new();
    let state = bounded_state(source.clone(), WRITE_BUDGET);
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let pending = intent(&state, A).await;
    *state.inner.failure_after_insert.lock().unwrap() =
        Some(Box::new(|| std::thread::sleep(CROSS)));
    let error = state
        .flush_failure_intents(vec![pending])
        .await
        .unwrap_err();
    let unchanged = state.inner.failed_attempts.read().await.is_empty();
    let slots = state.inner.failure_slots.available_permits();
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    elapsed(&error);
    assert!(unchanged && source.calls.lock().unwrap().is_empty());
    assert_eq!(slots, MAX_FAILURE_RECORDS);
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_late_update_restores_original_backoff_and_permit() {
    let source = FakeSource::new();
    let state = bounded_state(source, WRITE_BUDGET);
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let first = intent(&state, A).await;
    state
        .record_prepared_failure(
            first.failed.node,
            failure_table::Destination::new(&state.inner.global_metadata_bytes).unwrap(),
            &first.ticket,
        )
        .await;
    let before = state.inner.failed_attempts.read().await[A].clone();
    let pending = intent(&state, A).await;
    *state.inner.failure_after_insert.lock().unwrap() =
        Some(Box::new(|| std::thread::sleep(CROSS)));
    let error = state
        .flush_failure_intents(vec![pending])
        .await
        .unwrap_err();
    let after = state.inner.failed_attempts.read().await[A].clone();
    let slots = state.inner.failure_slots.available_permits();
    let same_permit = before
        .slot
        .as_ref()
        .unwrap()
        .ptr_eq(after.slot.as_ref().unwrap());
    drop(before.slot);
    drop(after.slot);
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    elapsed(&error);
    assert!(same_permit && before.identity == after.identity);
    assert_eq!(
        (before.last_attempt, before.fail_count),
        (after.last_attempt, after.fail_count)
    );
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_stop_a_skips_only_a_failure_intent() {
    let source = FakeSource::new();
    let state = bounded_state(source, Duration::from_secs(1));
    let retained_stop = Arc::new(StdMutex::new(state.prepare_cutover::<3>().unwrap()));
    let callback_stop = retained_stop.clone();

    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let a = intent(&state, A).await;
    let b = intent(&state, B).await;
    state
        .inner
        .lifecycle
        .on_next_failure_record(move |lifecycle| {
            lifecycle
                .stop(A, &mut callback_stop.lock().unwrap())
                .now_or_never()
                .expect("uncontended stop must complete");
        });
    state.flush_failure_intents(vec![a, b]).await.unwrap();
    let failed = state.inner.failed_attempts.read().await;
    let only_b = failed.len() == 1 && failed[B].fail_count == 1;
    let slots = state.inner.failure_slots.available_permits();
    drop(failed);
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    assert!(only_b);
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_timely_prefix_survives_later_row_rollback() {
    let source = FakeSource::new();
    let state = bounded_state(source, WRITE_BUDGET);
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let a = intent(&state, A).await;
    let b = intent(&state, B).await;
    let weak = state.inner.downgrade();
    *state.inner.failure_after_insert.lock().unwrap() = Some(Box::new(move || {
        *weak.upgrade().unwrap().failure_after_insert.lock().unwrap() =
            Some(Box::new(|| std::thread::sleep(CROSS)));
    }));
    let error = state.flush_failure_intents(vec![a, b]).await.unwrap_err();
    let failed = state.inner.failed_attempts.read().await;
    let prefix = failed.len() == 1 && failed[A].fail_count == 1;
    let slots = state.inner.failure_slots.available_permits();
    drop(failed);
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    elapsed(&error);
    assert!(prefix);
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_bookkeeping_two_writes_share_one_absolute_deadline() {
    let source = FakeSource::new();
    let state = bounded_state(source, Duration::from_millis(400));
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    let a = intent(&state, A).await;
    let b = intent(&state, B).await;
    let weak = state.inner.downgrade();
    *state.inner.failure_after_insert.lock().unwrap() = Some(Box::new(move || {
        std::thread::sleep(Duration::from_millis(150));
        *weak.upgrade().unwrap().failure_after_insert.lock().unwrap() =
            Some(Box::new(|| std::thread::sleep(Duration::from_millis(300))));
    }));
    let error = state.flush_failure_intents(vec![a, b]).await.unwrap_err();
    let failed = state.inner.failed_attempts.read().await;
    let only_first = failed.len() == 1 && failed[A].fail_count == 1;
    let slots = state.inner.failure_slots.available_permits();
    drop(failed);
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    elapsed(&error);
    assert!(
        only_first,
        "the second write reset the shared bookkeeping deadline"
    );
    assert_eq!(slots, MAX_FAILURE_RECORDS - 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

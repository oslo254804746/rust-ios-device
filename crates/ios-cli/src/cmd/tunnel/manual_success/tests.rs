use super::super::{AttemptIdentity, Lifecycle, Route};
use super::*;
use crate::access::{AccessControl, Permission, Upstreams};
use std::time::Duration;

fn state() -> TunnelAgentState {
    TunnelAgentState::with_source(
        ios_core::TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        Arc::new(super::super::source::NoDevices),
        Duration::from_secs(1),
    )
}

fn authority(state: &TunnelAgentState, key: &str) -> DeviceAuthority {
    state
        .inner
        .access
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap()
        .device_authority(Permission::ManageTunnels, key)
        .ok()
        .unwrap()
}

async fn manual(state: &TunnelAgentState, key: &str) -> (Ticket, Arc<()>, FailureVersion) {
    manual_authorized(state, key, authority(state, key)).await
}

async fn manual_authorized(
    state: &TunnelAgentState,
    key: &str,
    authority: DeviceAuthority,
) -> (Ticket, Arc<()>, FailureVersion) {
    let ticket = state.inner.lifecycle.ticket(key).await.unwrap();
    let slot = state.reserve_failure_slot(key).await.unwrap();
    state
        .record_failed_attempt(
            key,
            AttemptIdentity {
                route: Route::Wifi(None),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let failure = state.inner.failed_attempts.read().await[key]
        .version
        .clone();
    ticket
        .commit(|| {
            ticket.establish_manual(
                super::super::intent::RouteIntent::Wifi(None),
                authority,
                Some(failure.clone()),
                None,
            )
        })
        .unwrap();
    let desired = ticket.desired().version;
    (ticket, desired, failure)
}

#[tokio::test]
async fn canonical_coordinator_key_buffer_and_fee_follow_last_ticket_after_table_drop() {
    let bank = Arc::new(Semaphore::new(65536));
    let lifecycle = Lifecycle::with_metadata(Arc::clone(&bank)).unwrap();
    let ticket = lifecycle.ticket("original-canonical-key").await.unwrap();
    let pointer = ticket.udid().as_ptr();
    let same = lifecycle.ticket(ticket.udid()).await.unwrap();
    assert!(std::ptr::eq(same.udid().as_ptr(), pointer));
    drop(lifecycle);
    drop(same);
    assert!(std::ptr::eq(ticket.udid().as_ptr(), pointer));
    assert!(bank.available_permits() < 65536);
    drop(ticket);
    assert_eq!(bank.available_permits(), 65536);
}

#[tokio::test]
async fn manual_preflight_revoked_authority_keeps_original_failure_and_exact_pending_tuple() {
    use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
    let policy = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::ManageServer, Permission::ManageTunnels],
    )]);
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let principal = access.authenticate(&headers).unwrap();
    let authorized = principal
        .device_authority(Permission::ManageTunnels, DEVICE)
        .ok()
        .unwrap();
    let state = TunnelAgentState::with_source(
        ios_core::TunMode::Userspace,
        access.clone(),
        Upstreams::default(),
        Arc::new(super::super::source::NoDevices),
        Duration::from_secs(1),
    );
    let mut plan = Preflight::new(&state.inner.global_metadata_bytes).unwrap();
    let (ticket, desired, failure) = manual_authorized(&state, DEVICE, authorized).await;
    state.inner.lifecycle.fill_manual_successes(&mut plan).await;
    assert_eq!(plan.selected(), 1);
    policy.replace(vec![grant(TOKEN, vec![Permission::ManageServer])]);
    access.reload(&principal).await.unwrap();
    plan.into_inner()
        .flush_collected(
            &state,
            &super::super::scan::Deadline::new(Duration::from_secs(1)).unwrap(),
        )
        .await
        .unwrap();
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[DEVICE].version,
        &failure
    ));
    let pending = ticket.pending_success().unwrap();
    assert!(
        Arc::ptr_eq(&pending.0, &desired) && FailureVersion::ptr_eq(&pending.1.version, &failure)
    );
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn manual_preflight_stop_skips_old_writer_without_restarting_backoff() {
    let state = state();
    let mut plan = Preflight::new(&state.inner.global_metadata_bytes).unwrap();
    let (ticket, _, failure) = manual(&state, "stopped-before-cleanup").await;
    state.inner.lifecycle.fill_manual_successes(&mut plan).await;
    state.inner.lifecycle.stop(ticket.udid()).await;
    plan.into_inner()
        .flush_collected(
            &state,
            &super::super::scan::Deadline::new(Duration::from_secs(1)).unwrap(),
        )
        .await
        .unwrap();
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[ticket.udid()].version,
        &failure
    ));
    assert!(ticket.pending_success().is_none());
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test(start_paused = true)]
async fn manual_preflight_expired_authority_keeps_failure_and_unprocessed_pending() {
    use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
    let policy = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::ManageTunnels])]);
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let principal = access.authenticate(&headers).unwrap();
    let authorized = principal
        .device_authority(Permission::ManageTunnels, DEVICE)
        .ok()
        .unwrap();
    let state = TunnelAgentState::with_source(
        ios_core::TunMode::Userspace,
        access,
        Upstreams::default(),
        Arc::new(super::super::source::NoDevices),
        Duration::from_secs(1),
    );
    let mut plan = Preflight::new(&state.inner.global_metadata_bytes).unwrap();
    let (ticket, desired, failure) = manual_authorized(&state, DEVICE, authorized).await;
    state.inner.lifecycle.fill_manual_successes(&mut plan).await;
    assert_eq!(plan.selected(), 1);
    tokio::time::advance(Duration::from_secs(3601)).await;
    plan.into_inner()
        .flush_collected(
            &state,
            &super::super::scan::Deadline::new(Duration::from_secs(1)).unwrap(),
        )
        .await
        .unwrap();
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[DEVICE].version,
        &failure
    ));
    let pending = ticket.pending_success().unwrap();
    assert!(
        Arc::ptr_eq(&pending.0, &desired) && FailureVersion::ptr_eq(&pending.1.version, &failure)
    );
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn actual_pending_clear_late_clock_restores_same_desired_and_failure_arcs() {
    let state = state();
    let (ticket, desired, failure) = manual(&state, "pending-clear-clock").await;
    let deadline = super::super::scan::Deadline::new(Duration::from_millis(40)).unwrap();
    let checks = std::sync::atomic::AtomicUsize::new(0);
    let fenced = ticket.fenced();
    let error = fenced
        .commit(|| {
            fenced.clear_success_checked(&desired, &failure, || {
                if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 1 {
                    std::thread::sleep(Duration::from_millis(100));
                }
                deadline.check()
            })
        })
        .unwrap_err();
    assert!(error.is::<super::super::scan::DeadlineReached>());
    assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 2);
    let pending = ticket.pending_success().unwrap();
    assert!(
        Arc::ptr_eq(&pending.0, &desired) && FailureVersion::ptr_eq(&pending.1.version, &failure)
    );
    state.notify_shutdown();
    state.clear().await;
}

#[test]
fn manual_preflight_actual_maximum_backings_exact_and_one_byte_refusal_refund() {
    let bytes = Preflight::bytes();
    for limit in [bytes - 1, bytes] {
        let bank = Arc::new(Semaphore::new(limit));
        match Preflight::new(&bank) {
            Ok(plan) => {
                assert_eq!(limit, bytes);
                assert_eq!(bank.available_permits(), 0);
                drop(plan);
            }
            Err(error) => {
                assert_eq!(limit, bytes - 1);
                assert_eq!(
                    crate::output::operation_error_class(error.as_ref()),
                    ios_core::ErrorClass::ResourceLimit
                );
            }
        }
        assert_eq!(bank.available_permits(), limit);
    }
}

#[tokio::test]
async fn manual_preflight_without_manual_captures_new_gate_and_latest_version_at_zero_quota() {
    let state = state();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    assert_eq!(state.inner.lifecycle.desired().await.unwrap().len(), 0);
    let mut plan = Preflight::new(&bank).unwrap();
    let (ticket, old_desired, failure) = manual(&state, "source-created-manual").await;
    ticket
        .commit(|| {
            ticket.establish_manual(
                super::super::intent::RouteIntent::Wifi(Some("current-host".into())),
                authority(&state, ticket.udid()),
                Some(failure.clone()),
                None,
            )
        })
        .unwrap();
    let new_desired = ticket.desired().version;
    assert!(!Arc::ptr_eq(&new_desired, &old_desired));
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    state.inner.lifecycle.fill_manual_successes(&mut plan).await;
    assert_eq!(plan.selected(), 1);
    assert!(Arc::ptr_eq(
        &plan.completed.as_slice()[0].desired,
        &new_desired
    ));
    assert!(std::ptr::eq(
        plan.completed.as_slice()[0].ticket.udid().as_ptr(),
        ticket.udid().as_ptr()
    ));
    plan.into_inner()
        .flush_collected(
            &state,
            &super::super::scan::Deadline::new(Duration::from_secs(1)).unwrap(),
        )
        .await
        .unwrap();
    assert!(!state
        .inner
        .failed_attempts
        .read()
        .await
        .contains_key(ticket.udid()));
    assert!(ticket.pending_success().is_none());
    drop(held);
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn manual_preflight_does_not_pin_idle_automatic_namespace() {
    let lifecycle = Lifecycle::new();
    let bank = Arc::new(Semaphore::new(Preflight::bytes()));
    let plan = Preflight::new(&bank).unwrap();
    for index in 0..super::super::lifecycle::MAX_DEVICE_GATES {
        drop(lifecycle.ticket(&format!("idle-{index}")).await.unwrap());
    }
    assert_eq!(
        lifecycle.coordinator_count().await,
        super::super::lifecycle::MAX_DEVICE_GATES
    );
    let replacement = lifecycle.ticket("new-after-source").await.unwrap();
    assert_eq!(replacement.udid(), "new-after-source");
    assert_eq!(lifecycle.coordinator_count().await, 1);
    assert_eq!(plan.selected(), 0);
    drop(plan);
    assert_eq!(bank.available_permits(), Preflight::bytes());
}

#[tokio::test]
async fn manual_preflight_stale_writer_cannot_clear_replacement_pending_tuple() {
    let state = state();
    let mut plan = Preflight::new(&state.inner.global_metadata_bytes).unwrap();
    let (ticket, old_desired, old_failure) = manual(&state, "changing-version").await;
    state.inner.lifecycle.fill_manual_successes(&mut plan).await;
    let slot = state.reserve_failure_slot(ticket.udid()).await.unwrap();
    state
        .record_failed_attempt(
            ticket.udid(),
            AttemptIdentity {
                route: Route::Wifi(None),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let new_failure = state.inner.failed_attempts.read().await[ticket.udid()]
        .version
        .clone();
    ticket
        .commit(|| {
            ticket.establish_manual(
                super::super::intent::RouteIntent::Wifi(None),
                authority(&state, ticket.udid()),
                Some(new_failure.clone()),
                None,
            )
        })
        .unwrap();
    let new_desired = ticket.desired().version;
    assert!(!Arc::ptr_eq(&old_desired, &new_desired));
    assert!(!FailureVersion::ptr_eq(&old_failure, &new_failure));
    plan.into_inner()
        .flush_collected(
            &state,
            &super::super::scan::Deadline::new(Duration::from_secs(1)).unwrap(),
        )
        .await
        .unwrap();
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[ticket.udid()].version,
        &new_failure
    ));
    let (pending_desired, pending) = ticket.pending_success().unwrap();
    assert!(Arc::ptr_eq(&pending_desired, &new_desired));
    assert!(FailureVersion::ptr_eq(&pending.version, &new_failure));
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn manual_preflight_actual_late_writer_restores_failure_stamp_and_pending_tuple() {
    let state = state();
    let plan = Preflight::new(&state.inner.global_metadata_bytes).unwrap();
    let (ticket, desired, failure) = manual(&state, "late-manual-success").await;
    ticket.fenced().remember_failure(Some(failure.clone()));
    let original_stamp = ticket.failure_stamp().unwrap();
    *state.inner.success_after_remove.lock().unwrap() =
        Some(Box::new(|| std::thread::sleep(Duration::from_millis(100))));
    let deadline = super::super::scan::Deadline::new(Duration::from_millis(40)).unwrap();
    let error = plan
        .into_inner()
        .flush(&state, &deadline)
        .await
        .unwrap_err();
    assert!(error.is::<super::super::scan::DeadlineReached>());
    assert!(FailureVersion::ptr_eq(
        &state.inner.failed_attempts.read().await[ticket.udid()].version,
        &failure
    ));
    let stamp = ticket.failure_stamp().unwrap();
    assert!(
        FailureVersion::ptr_eq(&stamp.0, &original_stamp.0)
            && Arc::ptr_eq(&stamp.1, &original_stamp.1)
    );
    let pending = ticket.pending_success().unwrap();
    assert!(
        Arc::ptr_eq(&pending.0, &desired) && FailureVersion::ptr_eq(&pending.1.version, &failure)
    );
    state.notify_shutdown();
    state.clear().await;
}

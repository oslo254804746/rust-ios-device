//! Registry contract fixtures. Sender-only TunnelHandle::new has no native
//! task; these establish registry identity/capacity/fencing, not worker cleanup.
//! Real userspace/proxy/Core activation fixtures are required separately.

use super::super::lifecycle::Lifecycle;
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::watch;

const OBSERVE: Duration = Duration::from_secs(1);

fn registry(capacity: usize) -> Arc<Registry> {
    Registry::small(NonZeroUsize::new(capacity).unwrap())
}

fn stamp() -> OwnershipStamp {
    OwnershipStamp::new(Arc::new(()), Arc::new(()))
}

fn handle() -> (Arc<TunnelHandle>, watch::Receiver<()>) {
    let (handle, receiver) = TunnelHandle::new(
        "synthetic-private-native-target".into(),
        ios_core::TunnelInfo {
            server_address: "fd00::2".into(),
            server_rsd_port: 58783,
            client_address: "fd00::1".into(),
            client_mtu: 1280,
        },
        None,
    );
    (Arc::new(handle), receiver)
}

#[tokio::test]
async fn combined_capacity_covers_every_phase_before_a_source_can_be_polled() {
    let registry = registry(5);
    let stamp = stamp();
    let reserved = registry.reserve(stamp.clone()).unwrap();
    let inflight = registry.reserve(stamp.clone()).unwrap();
    inflight.start().unwrap();
    let (unpublished_handle, _unpublished_rx) = handle();
    let unpublished = registry
        .reserve(stamp.clone())
        .unwrap()
        .attach(unpublished_handle.clone());
    let (active_handle, _active_rx) = handle();
    let active = registry
        .reserve(stamp.clone())
        .unwrap()
        .attach(active_handle.clone());
    active.publish(&stamp).unwrap();
    let (retired_handle, _retired_rx) = handle();
    let retired = registry
        .reserve(stamp.clone())
        .unwrap()
        .attach(retired_handle.clone());
    drop(retired);
    let snapshot = registry.snapshot();
    assert_eq!(snapshot.occupied(), 5);
    assert_eq!(
        (
            snapshot.reserved,
            snapshot.inflight,
            snapshot.unpublished,
            snapshot.active,
            snapshot.retired
        ),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(snapshot.native_handles, 3);
    let source_polls = AtomicUsize::new(0);
    let result = async {
        let admission = registry.reserve(stamp.clone())?;
        admission.start()?;
        source_polls.fetch_add(1, Ordering::SeqCst);
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let Err(error) = result else {
        panic!("full owner registry admitted another source")
    };
    assert!(error
        .to_string()
        .contains("native tunnel owner capacity exhausted"));
    assert_eq!(source_polls.load(Ordering::SeqCst), 0);
    assert!(!retired_handle.is_alive());
    assert!(active_handle.is_alive());
    assert!(unpublished_handle.is_alive());
    drop(reserved);
    drop(inflight);
    drop(unpublished);
    drop(active);
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(report.selected, 3);
    assert_eq!(report.joined, 3);
    assert_eq!(report.snapshot.occupied(), 0);
}

#[tokio::test]
async fn original_arc_survives_unpublished_guard_drop_and_is_the_joined_owner() {
    let registry = registry(1);
    let (native, _receiver) = handle();
    let lease = registry.reserve(stamp()).unwrap().attach(native.clone());
    let key = lease.key.clone();
    drop(lease);
    assert!(!native.is_alive());
    let SelectedOwner::Native(original) = registry.selected_owner(&key) else {
        panic!("unpublished owner was lost")
    };
    assert!(Arc::ptr_eq(&native, &original));
    assert_eq!(registry.snapshot().retired, 1);
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(report.joined, 1);
    assert_eq!(report.snapshot.native_handles, 0);
    assert!(
        Arc::ptr_eq(&native, &original),
        "completion used a replacement handle"
    );
}

#[tokio::test]
async fn caller_cancelled_drain_keeps_original_arc_and_capacity_for_retry() {
    let registry = registry(1);
    let (native, _receiver) = handle();
    let lease = registry.reserve(stamp()).unwrap().attach(native.clone());
    lease.retire();
    let serial = registry.drains.lock().await;
    let mut drain = Box::pin(registry.drain_retired(CleanupClock::new(OBSERVE).unwrap()));
    assert!(futures_util::poll!(&mut drain).is_pending());
    drop(drain);
    let SelectedOwner::Native(retained) = registry.selected_owner(&lease.key) else {
        panic!("cancelled drain removed original owner")
    };
    assert!(Arc::ptr_eq(&retained, &native));
    assert_eq!(registry.snapshot().occupied(), 1);
    assert!(registry.reserve(stamp()).is_err());
    drop(serial);
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(report.joined, 1);
    assert_eq!(report.snapshot.occupied(), 0);
}

#[tokio::test]
async fn expired_clock_stops_synchronously_retains_actual_elapsed_and_retries_same_owner() {
    let registry = registry(1);
    let (native, _receiver) = handle();
    let lease = registry.reserve(stamp()).unwrap().attach(native.clone());
    lease.retire();
    let drain = registry.drain_retired(CleanupClock::new(Duration::ZERO).unwrap());
    assert!(
        !native.is_alive(),
        "stop depended on polling the cleanup future"
    );
    let failure = drain.await.unwrap_err();
    let before = failure.snapshot();
    assert_eq!(
        ios_core::classify_error(failure.primary()),
        ErrorClass::Timeout
    );
    assert!(failure.elapsed().is_some());
    assert_eq!(before.native_handles, 1);
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(report.joined, 1);
    assert_eq!(report.snapshot.occupied(), 0);
    assert_eq!(
        failure.snapshot().native_handles,
        before.native_handles,
        "old deadline result changed after retry"
    );
    assert!(failure.elapsed().is_some());
}

#[tokio::test]
async fn stale_lease_drop_cannot_retire_a_reused_slot_incarnation() {
    let registry = registry(1);
    let old_stamp = stamp();
    let (old, _old_rx) = handle();
    let delayed_old_lease = registry.reserve(old_stamp).unwrap().attach(old.clone());
    delayed_old_lease.retire();
    registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    let fresh_stamp = stamp();
    let (fresh, _fresh_rx) = handle();
    let fresh_lease = registry
        .reserve(fresh_stamp.clone())
        .unwrap()
        .attach(fresh.clone());
    fresh_lease.publish(&fresh_stamp).unwrap();
    drop(delayed_old_lease);
    assert!(fresh.is_alive());
    assert_eq!(registry.snapshot().active, 1);
    drop(fresh_lease);
    registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn terminal_close_stops_active_owner_independently_of_a_held_external_table() {
    let registry = registry(1);
    let stamp = stamp();
    let (native, _receiver) = handle();
    let lease = registry
        .reserve(stamp.clone())
        .unwrap()
        .attach(native.clone());
    lease.publish(&stamp).unwrap();
    let table = tokio::sync::RwLock::new(());
    let held = table.write().await;
    registry.close_and_stop_all();
    assert!(!native.is_alive());
    assert!(registry.snapshot().sealed);
    let Err(error) = registry.reserve(stamp.clone()) else {
        panic!("sealed registry admitted a source")
    };
    assert!(error.is::<ios_core::OperationCancelled>());
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert!(report.snapshot.closed_and_empty());
    drop(held);
    drop(lease);
    // This external lock fixture establishes registry independence only. The
    // actual serve-owner/real table/native worker fixture is still required.
}

#[tokio::test]
async fn delayed_post_close_attachment_retains_and_stops_the_original_runtime() {
    let registry = registry(1);
    let stamp = stamp();
    let reservation = registry.reserve(stamp.clone()).unwrap();
    reservation.start().unwrap();
    registry.close_and_stop_all();
    let mut waiting = Box::pin(registry.drain_retired(CleanupClock::new(OBSERVE).unwrap()));
    assert!(futures_util::poll!(&mut waiting).is_pending());
    assert_eq!(registry.snapshot().retired, 1);
    let (native, _receiver) = handle();
    let lease = reservation.attach(native.clone());
    assert!(!native.is_alive());
    assert!(lease.publish(&stamp).is_err());
    let report = waiting.await.unwrap();
    assert_eq!(report.joined, 1);
    assert!(report.snapshot.closed_and_empty());
}

#[tokio::test]
async fn empty_retired_reservation_drop_wakes_drain_without_claiming_native_join() {
    let registry = registry(1);
    let reservation = registry.reserve(stamp()).unwrap();
    reservation.start().unwrap();
    registry.close_and_stop_all();
    let mut drain = Box::pin(registry.drain_retired(CleanupClock::new(OBSERVE).unwrap()));
    assert!(futures_util::poll!(&mut drain).is_pending());
    drop(reservation);
    let report = drain.await.unwrap();
    assert_eq!(report.selected, 1);
    assert_eq!(report.joined, 0);
    assert!(report.snapshot.closed_and_empty());
}

#[tokio::test]
async fn reusable_clear_and_targeted_stop_retire_old_exact_ticket_epochs() {
    for global_clear in [true, false] {
        let lifecycle = Lifecycle::new();
        let registry = registry(2);
        let old_ticket = lifecycle.ticket("synthetic-private-target").await.unwrap();
        let old_stamp = old_ticket.ownership_stamp();
        let old = old_ticket
            .commit(|| registry.reserve(old_stamp.clone()))
            .unwrap();
        old.start().unwrap();
        let old_epoch = if global_clear {
            lifecycle.clear_owned().unwrap()
        } else {
            lifecycle
                .stop_owned("synthetic-private-target")
                .await
                .unwrap()
        };
        let fresh_ticket = lifecycle.ticket("synthetic-private-target").await.unwrap();
        let fresh_stamp = fresh_ticket.ownership_stamp();
        let fresh = fresh_ticket
            .commit(|| registry.reserve(fresh_stamp.clone()))
            .unwrap();
        fresh.start().unwrap();
        if global_clear {
            registry.retire_global(&old_epoch)
        } else {
            registry.retire_device(&old_epoch)
        }
        let (old_native, _old_rx) = handle();
        let old_lease = old.attach(old_native.clone());
        let (fresh_native, _fresh_rx) = handle();
        let fresh_lease = fresh.attach(fresh_native.clone());
        assert!(!old_native.is_alive());
        assert!(fresh_native.is_alive());
        assert!(old_ticket.commit(|| old_lease.publish(&old_stamp)).is_err());
        fresh_ticket
            .commit(|| fresh_lease.publish(&fresh_stamp))
            .unwrap();
        assert_eq!(registry.snapshot().active, 1);
        assert_eq!(registry.snapshot().retired, 1);
        registry
            .drain_retired(CleanupClock::new(OBSERVE).unwrap())
            .await
            .unwrap();
        assert!(
            fresh_native.is_alive(),
            "old epoch retirement hit a fresh admission"
        );
        drop(old_lease);
        drop(fresh_lease);
        registry
            .drain_retired(CleanupClock::new(OBSERVE).unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_thread_clear_or_stop_fences_old_reservation_then_preserves_new_attachment() {
    for global_clear in [true, false] {
        let lifecycle = Arc::new(Lifecycle::new());
        let registry = registry(2);
        let old_ticket = lifecycle.ticket("synthetic-private-target").await.unwrap();
        let old_stamp = old_ticket.ownership_stamp();
        let (release_commit, released_commit) = std::sync::mpsc::channel();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let registry_in_commit = registry.clone();
        let admission = std::thread::spawn(move || {
            let reservation = old_ticket
                .commit(|| {
                    let reservation = registry_in_commit.reserve(old_stamp.clone())?;
                    entered_tx.send(()).unwrap();
                    released_commit.recv_timeout(OBSERVE)?;
                    Ok(reservation)
                })
                .unwrap();
            (old_ticket, reservation)
        });
        let entered = entered_rx.recv_timeout(OBSERVE);
        let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
        let finished_fence = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let finished_in_fence = finished_fence.clone();
        let lifecycle_in_fence = lifecycle.clone();
        let runtime = tokio::runtime::Handle::current();
        let fence = std::thread::spawn(move || {
            attempted_tx.send(()).unwrap();
            let old = if global_clear {
                lifecycle_in_fence.clear_owned().unwrap()
            } else {
                runtime
                    .block_on(lifecycle_in_fence.stop_owned("synthetic-private-target"))
                    .unwrap()
            };
            finished_in_fence.store(true, Ordering::SeqCst);
            old
        });
        let attempted = attempted_rx.recv_timeout(OBSERVE);
        let premature_fence = finished_fence.load(Ordering::SeqCst);
        // Release all gates and join both threads before assertions or native
        // cleanup. A failure assertion cannot strand the commit watch borrows.
        let released = release_commit.send(());
        drop(release_commit);
        let (old_ticket, old) = admission.join().unwrap();
        let old_epoch = fence.join().unwrap();
        let fresh_ticket = lifecycle.ticket("synthetic-private-target").await.unwrap();
        let fresh_stamp = fresh_ticket.ownership_stamp();
        let fresh = fresh_ticket
            .commit(|| registry.reserve(fresh_stamp.clone()))
            .unwrap();
        if global_clear {
            registry.retire_global(&old_epoch)
        } else {
            registry.retire_device(&old_epoch)
        }
        let (old_native, _old_rx) = handle();
        let old_lease = old.attach(old_native.clone());
        let (fresh_native, _fresh_rx) = handle();
        let fresh_lease = fresh.attach(fresh_native.clone());
        let old_stopped = !old_native.is_alive();
        let fresh_survived = fresh_native.is_alive();
        let old_rejected = old_ticket
            .commit(|| old_lease.publish(&old_ticket.ownership_stamp()))
            .is_err();
        fresh_ticket
            .commit(|| fresh_lease.publish(&fresh_stamp))
            .unwrap();
        registry
            .drain_retired(CleanupClock::new(OBSERVE).unwrap())
            .await
            .unwrap();
        drop(old_lease);
        drop(fresh_lease);
        registry
            .drain_retired(CleanupClock::new(OBSERVE).unwrap())
            .await
            .unwrap();
        assert!(
            entered.is_ok() && attempted.is_ok() && released.is_ok(),
            "cross-thread fixture handoff failed"
        );
        assert!(
            !premature_fence,
            "lifecycle write bypassed the real Ticket.commit watch fence"
        );
        assert!(old_stopped && old_rejected);
        assert!(
            fresh_survived,
            "exact old epoch retirement stopped the new admission"
        );
    }
}

#[tokio::test]
async fn reservation_requires_the_same_ticket_commit_fence_as_publication() {
    let lifecycle = Lifecycle::new();
    let registry = registry(1);
    let old = lifecycle.ticket("synthetic-private-target").await.unwrap();
    let old_stamp = old.ownership_stamp();
    let old_global = lifecycle.clear_owned().unwrap();
    registry.retire_global(&old_global);
    let result = old.commit(|| registry.reserve(old_stamp.clone()));
    assert!(result.is_err());
    assert_eq!(registry.snapshot().occupied(), 0);
    // Negative control: the registry alone cannot consult private lifecycle
    // watches, and a direct stale reservation would miss the completed fence.
    let bypassed = registry.reserve(old_stamp).unwrap();
    assert_eq!(registry.snapshot().occupied(), 1);
    drop(bypassed);
    assert_eq!(registry.snapshot().occupied(), 0);
}

#[tokio::test]
async fn foreign_ticket_stamp_cannot_publish_an_acquired_owner() {
    let registry = registry(1);
    let correct = stamp();
    let (native, _receiver) = handle();
    let lease = registry
        .reserve(correct.clone())
        .unwrap()
        .attach(native.clone());
    assert!(lease.publish(&stamp()).is_err());
    assert_eq!(registry.snapshot().unpublished, 1);
    lease.publish(&correct).unwrap();
    assert_eq!(registry.snapshot().active, 1);
    drop(lease);
    registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
}

#[test]
fn invalid_cleanup_horizon_has_no_retirement_side_effect_and_snapshot_is_secret_safe() {
    let registry = registry(1);
    let stamp = stamp();
    let (native, _receiver) = handle();
    let lease = registry
        .reserve(stamp.clone())
        .unwrap()
        .attach(native.clone());
    lease.publish(&stamp).unwrap();
    assert!(CleanupClock::new(Duration::MAX).is_err());
    assert!(native.is_alive());
    assert_eq!(registry.snapshot().active, 1);
    assert!(!format!("{:?}", registry.snapshot()).contains("synthetic-private"));
    drop(lease);
    // Sender-only zero-task handle: final registry Drop is a stop fallback.
    // Actual native fixtures must explicitly drain and observe before final Drop.
}

#[test]
fn message_only_or_unrecognized_native_failure_cannot_certify_a_join() {
    for kind in [
        std::io::ErrorKind::Other,
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::ConnectionReset,
    ] {
        let error = TunnelError::Io(std::io::Error::new(
            kind,
            "pending_joins=0 joins_complete=true synthetic text",
        ));
        assert!(!shutdown_joins_certified(&error));
    }
    assert!(!shutdown_joins_certified(&TunnelError::Protocol(
        "joins_complete=true".into()
    )));
}

#[tokio::test]
async fn ordinal_selection_excludes_owners_retired_after_the_fixed_cutoff() {
    let registry = registry(2);
    let (a, _a_rx) = handle();
    let (b, _b_rx) = handle();
    let a_lease = registry.reserve(stamp()).unwrap().attach(a);
    let b_lease = registry.reserve(stamp()).unwrap().attach(b);
    drop(a_lease);
    let first = registry.drain_retired(CleanupClock::new(OBSERVE).unwrap());
    drop(b_lease);
    let report = first.await.unwrap();
    assert_eq!(
        (report.selected, report.joined, report.snapshot.retired),
        (1, 1, 1)
    );
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (report.selected, report.joined, report.snapshot.occupied()),
        (1, 1, 0)
    );
}

#[tokio::test]
async fn ordinal_selection_preserves_count_and_never_joins_a_reused_slot() {
    let registry = registry(1);
    let (a, _a_rx) = handle();
    let a_lease = registry.reserve(stamp()).unwrap().attach(a);
    drop(a_lease);
    let old = registry.drain_retired(CleanupClock::new(OBSERVE).unwrap());
    let joined = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(joined.joined, 1);
    let (b, _b_rx) = handle();
    let b_lease = registry.reserve(stamp()).unwrap().attach(Arc::clone(&b));
    drop(b_lease);
    let old = old.await.unwrap();
    assert_eq!((old.selected, old.joined, old.snapshot.retired), (1, 0, 1));
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!(
        (report.selected, report.joined, report.snapshot.occupied()),
        (1, 1, 0)
    );
}

#[tokio::test]
async fn ordinal_repeated_retirement_and_late_attachment_keep_original_selection() {
    let registry = registry(1);
    let owner = stamp();
    let reserved = registry.reserve(owner.clone()).unwrap();
    reserved.start().unwrap();
    registry.retire_global(&owner.global);
    let selected = registry.lock().selection();
    assert_eq!((selected.cutoff, selected.count), (1, 1));
    registry.retire_device(&owner.device);
    let pending = registry.drain_retired(CleanupClock::new(OBSERVE).unwrap());
    let (native, _rx) = handle();
    let lease = reserved.attach(Arc::clone(&native));
    assert!(
        !native.is_alive(),
        "late original handle was not synchronously stopped"
    );
    assert_eq!(registry.lock().retired, selected.cutoff);
    let report = pending.await.unwrap();
    assert_eq!(
        (report.selected, report.joined, report.snapshot.occupied()),
        (1, 1, 0)
    );
    drop(lease);
}

#[tokio::test]
async fn ordinal_admission_overflow_never_wraps_or_blocks_final_cleanup() {
    let registry = registry(1);
    {
        let mut state = registry.lock();
        state.issued = u64::MAX - 1;
        state.retired = u64::MAX - 1;
    }
    let owner = stamp();
    let reserved = registry.reserve(owner.clone()).unwrap();
    let (native, _rx) = handle();
    let lease = reserved.attach(native);
    drop(lease);
    assert_eq!(registry.lock().retired, u64::MAX);
    registry.retire_global(&owner.global);
    let report = registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
    assert_eq!((report.selected, report.joined), (1, 1));
    let polls = AtomicUsize::new(0);
    let result = async {
        let admission = registry.reserve(stamp())?;
        admission.start()?;
        polls.fetch_add(1, Ordering::SeqCst);
        Ok::<(), anyhow::Error>(())
    }
    .await;
    let error = result.expect_err("exhausted incarnation ID admitted another Source");
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ErrorClass::ResourceLimit
    );
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(registry.snapshot().occupied(), 0);
    let state = registry.lock();
    assert_eq!((state.issued, state.retired), (u64::MAX, u64::MAX));
}

#[test]
fn ordinal_empty_reservation_release_does_not_refund_lifetime_admission() {
    let registry = registry(1);
    drop(registry.reserve(stamp()).unwrap());
    {
        let state = registry.lock();
        assert_eq!((state.issued, state.retired), (1, 0));
    }
    let next = registry.reserve(stamp()).unwrap();
    assert_eq!(registry.lock().issued, 2);
    drop(next);
    assert_eq!(registry.lock().issued, 2);
}

#[tokio::test]
async fn ordinal_namespace_stop_does_not_repeat_stop_on_unrelated_retired_owner() {
    let registry = registry(2);
    let target = stamp();
    let (a, mut a_rx) = handle();
    let a_lease = registry.reserve(target.clone()).unwrap().attach(a);
    let (b, mut b_rx) = handle();
    let b_lease = registry.reserve(stamp()).unwrap().attach(b);
    drop(b_lease);
    b_rx.borrow_and_update();
    registry.retire_global(&target.global);
    assert!(a_rx.has_changed().unwrap());
    a_rx.borrow_and_update();
    assert!(
        !b_rx.has_changed().unwrap(),
        "old namespace stopped another retired owner"
    );
    drop(a_lease);
    registry
        .drain_retired(CleanupClock::new(OBSERVE).unwrap())
        .await
        .unwrap();
}

#[test]
fn native_census_backing_refusal_keeps_original_history_and_last_root_grant() {
    let registry = registry(1);
    let original = Arc::new(TunnelError::Protocol("synthetic-native-census".into()));
    registry.lock().first_failure = Some(Arc::clone(&original));
    let bank = Arc::clone(&registry.byte_bank);
    let before = bank.available_permits();
    let held = Arc::clone(&bank)
        .try_acquire_many_owned(before.try_into().unwrap())
        .unwrap();
    assert!(registry.payload_outcomes().is_err());
    assert!(Arc::ptr_eq(
        registry.lock().first_failure.as_ref().unwrap(),
        &original
    ));
    drop(held);
    let (unjoined, roots) = registry.payload_outcomes().unwrap();
    assert_eq!(unjoined, 0);
    assert!(Arc::ptr_eq(&roots.as_slice()[0], &original));
    assert!(bank.available_permits() < before);
    drop(roots);
    assert_eq!(bank.available_permits(), before);
}

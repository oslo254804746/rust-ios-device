use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn authority() -> DeviceAuthority {
    crate::access::AccessControl::trusted_test_local()
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap()
        .device_authority(crate::access::Permission::ManageTunnels, "synthetic-route")
        .ok()
        .unwrap()
}

fn route(capacity: usize, text: &str) -> RouteIntent {
    let mut host = String::with_capacity(capacity);
    host.push_str(text);
    RouteIntent::Wifi(Some(host))
}

fn manual(cell: &Cell, route: RouteIntent) -> anyhow::Result<Arc<()>> {
    cell.checked_replace(|| cell.manual(route, authority()), None, || Ok(()))
}

#[test]
fn manual_route_bytes_capacity_shared_snapshots_and_last_owner_after_stop() {
    let route = route(8192, "synthetic-route-host");
    let RouteIntent::Wifi(Some(host)) = &route else {
        unreachable!()
    };
    let original = host.as_ptr();
    let bytes = RetainedRoute::bytes(&route);
    let bank = Arc::new(Semaphore::new(bytes));
    let cell = Cell::new(Arc::new(()), Arc::clone(&bank));
    let version = manual(&cell, route).unwrap();
    assert_eq!(bank.available_permits(), 0);
    let snapshot = cell.snapshot();
    let copied = snapshot.clone();
    let Desired::Manual { route: first, .. } = &snapshot.desired else {
        panic!("manual missing")
    };
    let Desired::Manual { route: second, .. } = &copied.desired else {
        panic!("manual missing")
    };
    assert!(Arc::ptr_eq(first, second));
    let RouteIntent::Wifi(Some(host)) = &first.route else {
        unreachable!()
    };
    assert_eq!(host.as_ptr(), original);
    assert_eq!(host.capacity(), 8192);
    assert_eq!(
        bank.available_permits(),
        0,
        "snapshot clones double charged"
    );
    assert!(Arc::ptr_eq(&version, &snapshot.version));
    cell.replace(Desired::Stopped);
    assert_eq!(
        bank.available_permits(),
        0,
        "stop released live snapshot storage"
    );
    drop(cell);
    drop(snapshot);
    assert_eq!(bank.available_permits(), 0);
    drop(copied);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn manual_route_bytes_rejected_replacement_preserves_original_version_watch_and_pending() {
    let original = route(4096, "original-private-host");
    let bytes = RetainedRoute::bytes(&original);
    let bank = Arc::new(Semaphore::new(bytes));
    let cell = Cell::new(Arc::new(()), Arc::clone(&bank));
    manual(&cell, original).unwrap();
    let before = cell.snapshot();
    let watch = cell.changes.subscribe();
    let error = manual(&cell, route(32, "candidate-host")).err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let after = cell.snapshot();
    assert!(Arc::ptr_eq(&before.version, &after.version));
    let (Desired::Manual { route: old, .. }, Desired::Manual { route: current, .. }) =
        (&before.desired, &after.desired)
    else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(old, current));
    assert!(!watch.has_changed().unwrap());
    assert!(cell.pending_success().is_none());
    assert_eq!(bank.available_permits(), 0);
    drop(before);
    drop(after);
    drop(cell);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn manual_route_bytes_late_candidate_rolls_back_original_arc_and_refunds_only_new_owner() {
    let old = route(4096, "original-host");
    let new = route(2048, "new-host");
    let old_bytes = RetainedRoute::bytes(&old);
    let new_bytes = RetainedRoute::bytes(&new);
    let bank = Arc::new(Semaphore::new(old_bytes + new_bytes));
    let cell = Cell::new(Arc::new(()), Arc::clone(&bank));
    manual(&cell, old).unwrap();
    let before = cell.snapshot();
    let calls = AtomicUsize::new(0);
    let error = cell
        .checked_replace(
            || cell.manual(new, authority()),
            Some(Arc::new(())),
            || {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    Ok(())
                } else {
                    Err(super::super::lifecycle::cancelled())
                }
            },
        )
        .err()
        .unwrap();
    assert!(error.is::<ios_core::OperationCancelled>());
    let after = cell.snapshot();
    assert!(Arc::ptr_eq(&before.version, &after.version));
    let (Desired::Manual { route: old, .. }, Desired::Manual { route: current, .. }) =
        (&before.desired, &after.desired)
    else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(old, current));
    assert!(cell.pending_success().is_none());
    assert_eq!(bank.available_permits(), new_bytes);
    drop(before);
    drop(after);
    drop(cell);
    assert_eq!(bank.available_permits(), old_bytes + new_bytes);
}

#[test]
fn manual_route_bytes_clear_keeps_original_epoch_snapshot_and_zero_bank_stop_works() {
    let selected = route(2048, "selected-host");
    let bytes = RetainedRoute::bytes(&selected);
    let bank = Arc::new(Semaphore::new(bytes));
    let epoch = Arc::new(());
    let cell = Cell::new(Arc::clone(&epoch), Arc::clone(&bank));
    manual(&cell, selected).unwrap();
    let before = cell.snapshot();
    cell.reset(Arc::new(()));
    assert!(matches!(cell.snapshot().desired, Desired::Automatic));
    assert!(Arc::ptr_eq(&before.epoch, &epoch));
    assert_eq!(bank.available_permits(), 0);
    cell.replace(Desired::Stopped);
    assert_eq!(bank.available_permits(), 0);
    drop(before);
    assert_eq!(bank.available_permits(), bytes);
    let zero = Cell::new(Arc::new(()), Arc::new(Semaphore::new(0)));
    zero.replace(Desired::Stopped);
    zero.reset(Arc::new(()));
    assert!(matches!(zero.snapshot().desired, Desired::Automatic));
}

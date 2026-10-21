use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn shared_route_actual_text_is_paid_before_copy_and_last_snapshot_owns_it() {
    let host = "manual-filter-雪";
    let bytes = RetainedRoute::body_bytes() + host.len();
    let bank = Arc::new(Semaphore::new(bytes));
    let charged = bank.clone();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let captured = observed.clone();
    let route = super::super::metadata_vec::with_allocation_hook(
        move |layout, pointer| {
            assert_eq!(charged.available_permits(), 0);
            captured.lock().unwrap().push((layout, pointer));
        },
        || {
            RetainedRoute::established(
                &Route::Mobdev2("observed-address".into()),
                Some(host),
                &bank,
            )
            .unwrap()
        },
    );
    let RouteIntent::Mobdev2(Some(text)) = &route.route else {
        unreachable!()
    };
    assert_eq!(text, host);
    assert_eq!(text.capacity(), host.len());
    let pointer = text.as_ptr();
    {
        let allocated = observed.lock().unwrap();
        assert_eq!(allocated.len(), 2);
        assert_eq!(
            allocated[0],
            (
                std::alloc::Layout::array::<u8>(host.len()).unwrap(),
                pointer as usize
            )
        );
        assert_eq!(allocated[1].0.size(), RetainedRoute::body_bytes());
        let value = std::ptr::from_ref(&*route) as usize;
        assert!(allocated[1].1 <= value && value < allocated[1].1 + allocated[1].0.size());
    }
    let cell = Cell::new(Arc::new(()), bank.clone());
    cell.checked_replace(
        || cell.manual_shared(route.clone(), authority()),
        None,
        || Ok(()),
    )
    .unwrap();
    let snapshot = cell.snapshot();
    let Desired::Manual {
        route: retained, ..
    } = &snapshot.desired
    else {
        unreachable!()
    };
    assert!(route.ptr_eq(retained));
    cell.replace(Desired::Stopped);
    drop(cell);
    drop(route);
    let RouteIntent::Mobdev2(Some(text)) = &retained.route else {
        unreachable!()
    };
    assert_eq!(text.as_ptr(), pointer);
    assert_eq!(bank.available_permits(), 0);
    drop(snapshot);
    assert_eq!(bank.available_permits(), bytes);
}

#[test]
fn shared_route_body_and_text_last_byte_refusal_refund_without_destination_allocation() {
    let host = "selected-host";
    let body = RetainedRoute::body_bytes();
    for limit in [body - 1, body + host.len() - 1] {
        let bank = Arc::new(Semaphore::new(limit));
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("route refusal allocated destination text"),
            || {
                RetainedRoute::established(&Route::Wifi(Some(host.into())), None, &bank)
                    .err()
                    .unwrap()
            },
        );
        assert!(error.is::<RouteAdmission>());
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), limit);
    }
    let bank = Arc::new(Semaphore::new(2 * body));
    let absent =
        RetainedRoute::established(&Route::Wifi(None), Some("ignored-filter"), &bank).unwrap();
    let empty = RetainedRoute::established(&Route::Usb(Some(String::new())), None, &bank).unwrap();
    assert!(matches!(absent.route, RouteIntent::Wifi(None)));
    assert!(matches!(&empty.route, RouteIntent::Usb(Some(host)) if host.is_empty()));
    assert_eq!(bank.available_permits(), 0);
    drop(absent);
    drop(empty);
    assert_eq!(bank.available_permits(), 2 * body);
}

#[test]
fn shared_route_foreign_bank_rejection_keeps_original_version_watch_and_fees() {
    let original_bank = Arc::new(Semaphore::new(4096));
    let route =
        RetainedRoute::established(&Route::Wifi(Some("original".into())), None, &original_bank)
            .unwrap();
    let before = original_bank.available_permits();
    let bank = Arc::new(Semaphore::new(0));
    let cell = Cell::new(Arc::new(()), bank.clone());
    let version = cell.snapshot().version;
    let watch = cell.changes.subscribe();
    let error = cell
        .checked_replace(
            || cell.manual_shared(route.clone(), authority()),
            None,
            || Ok(()),
        )
        .err()
        .unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(Arc::ptr_eq(&version, &cell.snapshot().version));
    assert!(!watch.has_changed().unwrap());
    assert!(matches!(cell.snapshot().desired, Desired::Automatic));
    assert_eq!(original_bank.available_permits(), before);
    assert_eq!(bank.available_permits(), 0);
    drop(cell);
    drop(route);
    assert_eq!(original_bank.available_permits(), 4096);
}

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
    assert!(first.ptr_eq(second));
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
    assert!(old.ptr_eq(current));
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
    assert!(old.ptr_eq(current));
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

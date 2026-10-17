use super::*;
use std::sync::Barrier;

#[test]
fn route_body_original_filter_snapshot_replacement_releases_text_before_last_weak_body() {
    let filter = "原-manual-filter";
    let total = RetainedRoute::body_bytes() + filter.len();
    let bank = Arc::new(Semaphore::new(total));
    let route = RetainedRoute::established(
        RouteRef::Mobdev2("actual-discovered-address"),
        Some(filter),
        &bank,
    )
    .unwrap();
    let RouteIntent::Mobdev2(Some(host)) = &route.route else {
        panic!("desired original filter missing")
    };
    assert_eq!(host, filter);
    let pointer = host.as_ptr();
    let weak = route.downgrade();
    let cell = Cell::new(Arc::new(()), bank.clone());
    let authority = crate::access::AccessControl::trusted_test_local()
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap()
        .device_authority(
            crate::access::Permission::ManageTunnels,
            "route-body-fixture",
        )
        .ok()
        .unwrap();
    cell.checked_replace(
        || cell.manual_shared(route.clone(), authority),
        None,
        || Ok(()),
    )
    .unwrap();
    let snapshot = cell.snapshot();
    let Desired::Manual {
        route: captured, ..
    } = &snapshot.desired
    else {
        panic!("manual snapshot missing")
    };
    assert!(captured.ptr_eq(&route));
    let RouteIntent::Mobdev2(Some(host)) = &captured.route else {
        unreachable!()
    };
    assert_eq!(host.as_ptr(), pointer);
    cell.replace(Desired::Stopped);
    drop((cell, route));
    assert!(weak.upgrade().is_some());
    assert_eq!(bank.available_permits(), 0);
    drop(snapshot);
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), filter.len());
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn route_body_parallel_last_clones_keep_original_host_and_independent_weak_body_fee() {
    let host = "original-route-host";
    let total = RetainedRoute::body_bytes() + host.len();
    let bank = Arc::new(Semaphore::new(total));
    let route = RetainedRoute::established(RouteRef::Wifi(Some(host)), None, &bank).unwrap();
    let pointer = match &route.route {
        RouteIntent::Wifi(Some(host)) => host.as_ptr() as usize,
        _ => unreachable!(),
    };
    let weak = route.downgrade();
    let mut owners = (0..7).map(|_| route.clone()).collect::<Vec<_>>();
    owners.push(route);
    let barrier = Arc::new(Barrier::new(8));
    std::thread::scope(|scope| {
        for owner in owners {
            let barrier = barrier.clone();
            scope.spawn(move || {
                let RouteIntent::Wifi(Some(host)) = &owner.route else {
                    unreachable!()
                };
                assert_eq!(host.as_ptr() as usize, pointer);
                barrier.wait();
                drop(owner);
            });
        }
    });
    assert!(weak.upgrade().is_none());
    assert_eq!(bank.available_permits(), host.len());
    drop(weak);
    assert_eq!(bank.available_permits(), total);
}

#[test]
fn route_body_guard_preflight_short_and_partial_text_refusal_keep_original_quota_and_bank() {
    let host = "required-route-host";
    for available in [
        0,
        RetainedRoute::body_bytes() - 1,
        RetainedRoute::body_bytes() + host.len() - 1,
    ] {
        let bank = Arc::new(Semaphore::new(available));
        let error = super::super::metadata_vec::with_allocation_hook(
            |_, _| panic!("refused route allocated host"),
            || {
                RetainedRoute::established(RouteRef::Wifi(Some(host)), None, &bank)
                    .err()
                    .unwrap()
            },
        );
        assert!(error.is::<RouteAdmission>());
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), available);
    }
    let bank = Arc::new(Semaphore::new(RetainedRoute::body_bytes()));
    let route =
        RetainedRoute::established(RouteRef::Wifi(None), Some("ignored-filter"), &bank).unwrap();
    assert!(matches!(route.route, RouteIntent::Wifi(None)));
    assert!(Arc::ptr_eq(route._storage.semaphore(), &bank));
    assert_eq!(route._storage.num_permits(), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(route);
    assert_eq!(bank.available_permits(), RetainedRoute::body_bytes());
}

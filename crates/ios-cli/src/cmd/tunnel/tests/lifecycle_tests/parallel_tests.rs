use super::*;
use crate::cmd::tunnel::record_owner::PaidRecord;

fn scan_job(state: &TunnelAgentState) -> tokio::task::JoinHandle<Result<()>> {
    let state = state.clone();
    tokio::spawn(async move { state.refresh_once().await })
}

async fn record_within_budget(
    state: &TunnelAgentState,
    udid: &str,
) -> std::result::Result<PaidRecord, tokio::time::error::Elapsed> {
    tokio::time::timeout(LOCAL_BUDGET, async {
        loop {
            if let Some(record) = state.get_record(udid).await.unwrap() {
                return record;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
}

async fn abort_scan(scan: tokio::task::JoinHandle<Result<()>>) {
    scan.abort();
    assert!(scan.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_healthy_b_publishes_while_a_remains_stalled() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), B.into()];
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan = scan_job(&state);
    source.entered(A, 1).await;
    let observed = record_within_budget(&state, B).await;
    let active = source.active.load(Ordering::SeqCst);
    let a_record = state.get_record(A).await.unwrap();
    abort_scan(scan).await;
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    state.clear().await;
    assert!(observed.is_ok(), "healthy B waited behind stalled A");
    assert_eq!(active, 1);
    assert!(a_record.is_none());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_duplicate_a_candidates_do_not_occupy_b_window() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), A.into(), A.into(), A.into(), B.into()];
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan = scan_job(&state);
    source.entered(A, 1).await;
    let observed = record_within_budget(&state, B).await;
    abort_scan(scan).await;
    state.clear().await;
    assert!(
        observed.is_ok(),
        "duplicate A candidates occupied the connection window"
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_sixteen_candidates_start_four_and_restore_all_permits() {
    let source = FakeSource::new();
    let devices = (0..16)
        .map(|i| format!("synthetic-parallel-{i}"))
        .collect::<Vec<_>>();
    for udid in &devices {
        source.script(udid, [Behavior::Pending]);
    }
    *source.usb.lock().unwrap() = devices.clone();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan = scan_job(&state);
    let observed = tokio::time::timeout(LOCAL_BUDGET, async {
        while source.active.load(Ordering::SeqCst) != lifecycle::MAX_CONNECTS {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let active = source.active.load(Ordering::SeqCst);
    abort_scan(scan).await;
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert!(
        observed.is_ok(),
        "scan did not fill the bounded connection window"
    );
    assert_eq!(active, lifecycle::MAX_CONNECTS);
    assert_eq!(source.peak.load(Ordering::SeqCst), lifecycle::MAX_CONNECTS);
    for udid in &devices[..4] {
        assert_eq!(source.count(udid), 1);
    }
    for udid in &devices[4..] {
        assert_eq!(source.count(udid), 0);
    }
    assert_eq!(source.exited.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_caller_drop_owns_every_active_connection_future() {
    let source = FakeSource::new();
    let devices = (0..4)
        .map(|i| format!("synthetic-owned-{i}"))
        .collect::<Vec<_>>();
    for udid in &devices {
        source.script(udid, [Behavior::Pending]);
    }
    *source.usb.lock().unwrap() = devices;
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    let active = source.active.load(Ordering::SeqCst);
    drop(scan);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(active, lifecycle::MAX_CONNECTS);
    assert_eq!(source.exited.load(Ordering::SeqCst), 4);
    state.refresh_once().await.unwrap();
    assert_eq!(state.list_records().await.unwrap().len(), 4);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_stop_a_does_not_cancel_healthy_b_connection() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), B.into()];
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Pending]);
    source.script(B, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan = scan_job(&state);
    let b_entered = tokio::time::timeout(LOCAL_BUDGET, async {
        while source.count(B) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    if b_entered.is_err() {
        abort_scan(scan).await;
        panic!("B never entered while A stalled");
    }
    assert!(!state.remove_tunnel(A).await);
    gate.add_permits(1);
    tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(state.get_record(A).await.unwrap().is_none());
    assert!(state.get_record(B).await.unwrap().is_some());
    assert!(!state.inner.failed_attempts.read().await.contains_key(A));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_mobdev2_b_is_not_behind_stalled_usb_a() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: "192.0.2.42".into(),
    });
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let scan = scan_job(&state);
    source.entered(A, 1).await;
    let observed = record_within_budget(&state, B).await;
    abort_scan(scan).await;
    state.clear().await;
    assert!(observed.is_ok(), "mobdev2 B waited for the USB batch");
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    assert_eq!(
        source.calls.lock().unwrap().last().unwrap().1,
        Route::Mobdev2("192.0.2.42".into())
    );
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_paired_discovery_failure_keeps_usb_result_and_native_error() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.paired_failure.store(true, Ordering::SeqCst);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(!crate::output::error_message(&error).contains("synthetic-private"));
    assert_eq!(source.count(A), 1);
    assert!(state.get_record(A).await.unwrap().is_some());
    assert!(!state.is_ready().await);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_parallel_cross_family_duplicate_keeps_first_usb_route() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    *source.paired.lock().unwrap() = vec![
        PairedMobdev2Device {
            udid: A.into(),
            host: "192.0.2.41".into(),
        },
        PairedMobdev2Device {
            udid: B.into(),
            host: "192.0.2.42".into(),
        },
        PairedMobdev2Device {
            udid: B.into(),
            host: "192.0.2.43".into(),
        },
    ];
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    let calls = source.calls.lock().unwrap().clone();
    assert_eq!(
        calls.iter().find(|(id, _)| id == A).unwrap().1,
        Route::Usbmux
    );
    assert_eq!(
        calls.iter().find(|(id, _)| id == B).unwrap().1,
        Route::Mobdev2("192.0.2.42".into())
    );
    assert_eq!(state.list_records().await.unwrap().len(), 2);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

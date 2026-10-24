use super::*;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};

#[tokio::test]
async fn lifecycle_tunnel_retention_complete_absent_snapshot_releases_past_failures() {
    let source = FakeSource::new();
    let devices = (0..12)
        .map(|i| format!("synthetic-retired-{i}"))
        .collect::<Vec<_>>();
    for udid in &devices {
        source.script(
            udid,
            [Behavior::Failure(std::io::ErrorKind::ConnectionReset)],
        );
    }
    *source.usb.lock().unwrap() = devices;
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    assert_eq!(state.inner.failed_attempts.read().await.len(), 12);
    source.usb.lock().unwrap().clear();
    state.refresh_once().await.unwrap();
    let retained = state.inner.failed_attempts.read().await.len();
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(
        retained, 0,
        "observed absence retained every historical UDID"
    );
    assert_eq!(source.calls.lock().unwrap().len(), 12);
}

#[tokio::test]
async fn lifecycle_tunnel_retention_failed_paired_snapshot_keeps_unknown_but_retires_absent_usb() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: "192.0.2.2".into(),
    });
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    source.script(B, [Behavior::Failure(std::io::ErrorKind::PermissionDenied)]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    assert_eq!(state.inner.failed_attempts.read().await.len(), 2);
    source.usb.lock().unwrap().clear();
    source.paired_failure.store(true, Ordering::SeqCst);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    let failed = state.inner.failed_attempts.read().await;
    let usb_retained = failed.contains_key(A);
    let paired_retained = failed.contains_key(B);
    drop(failed);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(!usb_retained, "authoritative USB absence kept old failure");
    assert!(
        paired_retained,
        "paired discovery error was mistaken for absence"
    );
    assert_eq!(source.count(B), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_retention_policy_reload_releases_no_longer_authorized_failure() {
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::ManageServer, Permission::ManageTunnels],
    )]);
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(DEVICE.into());
    source.script(
        DEVICE,
        [Behavior::Failure(std::io::ErrorKind::ConnectionReset)],
    );
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    assert_eq!(state.inner.failed_attempts.read().await.len(), 1);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    fixture.replace(vec![grant(
        TOKEN,
        vec![Permission::ManageServer, Permission::List],
    )]);
    let response = client()
        .post(format!("http://127.0.0.1:{port}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    state.refresh_once().await.unwrap();
    let retained = state.inner.failed_attempts.read().await.len();
    server.abort();
    server.await.unwrap_err();
    state.clear(&mut prepared_outer_0).await;
    drop(prepared_outer_0);
    assert_eq!(
        retained, 0,
        "list-only policy retained device failure metadata"
    );
    assert_eq!(source.count(DEVICE), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_retention_known_changed_id_removes_old_metadata_before_attempt_wait() {
    let source = FakeSource::new();
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: 71,
        usb: true,
    }]);
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate)]);
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: 72,
        usb: true,
    }]);
    source.connected_ids.lock().unwrap().insert(A.into(), 72);
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.count(A), 2);
    let retained = state.inner.failed_attempts.read().await.contains_key(A);
    drop(scan);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert!(
        !retained,
        "obsolete identity remained while replacement was pending"
    );
}

#[tokio::test]
async fn lifecycle_tunnel_retention_same_present_expired_identity_keeps_count_progression() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut(A)
        .unwrap()
        .last_attempt -= Duration::from_secs(31);
    state.refresh_once().await.unwrap();
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 2);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

// Populate through the manager's real reservation/recording path. This fixture
// exercises retained metadata, without simulating 4096 native connections.
async fn retained_paired_failures(state: &TunnelAgentState, count: usize) {
    for index in 0..count {
        let udid = format!("synthetic-cache-{index}");
        let ticket = state.inner.lifecycle.ticket(&udid).await.unwrap();
        let slot = state.reserve_failure_slot(&udid).await.unwrap();
        state
            .record_failed_attempt(
                &udid,
                AttemptIdentity {
                    route: Route::Mobdev2("192.0.2.9".into()),
                    native_id: None,
                },
                slot,
                &ticket,
            )
            .await;
    }
}

#[tokio::test]
async fn lifecycle_tunnel_retention_capacity_includes_pending_attempt_and_denies_new_native_io() {
    let source = FakeSource::new();
    *source.usb.lock().unwrap() = vec![A.into(), B.into()];
    source.paired_failure.store(true, Ordering::SeqCst);
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    retained_paired_failures(&state, MAX_FAILURE_RECORDS - 1).await;
    let mut scan = Box::pin(state.refresh_once());
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.count(A) == 0 {
            assert!(futures_util::poll!(&mut scan).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 0);
    assert_eq!(state.inner.failure_slots.available_permits(), 0);
    assert_eq!(
        state.inner.failed_attempts.read().await.len(),
        MAX_FAILURE_RECORDS - 1
    );
    // The initial USB snapshot already succeeded. Fail the later publication
    // query so the outstanding reservation becomes a retained failure.
    source.list_failure.store(true, Ordering::SeqCst);
    gate.add_permits(1);
    let error = scan.await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        state.inner.failed_attempts.read().await.len(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    source.list_failure.store(false, Ordering::SeqCst);
    state.refresh_once().await.unwrap_err();
    assert_eq!(source.count(B), 0, "full capacity allowed fresh native IO");
    let denied = state.reserve_failure_slot(B).await.unwrap_err();
    let message = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel",
        &denied,
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&message).unwrap();
    assert_eq!(value["error"]["class"], "resource_limit");
    assert!(!denied.chain().any(|cause| cause.is::<std::io::Error>()));
    source.usb.lock().unwrap().clear();
    source.paired_failure.store(false, Ordering::SeqCst);
    state.refresh_once().await.unwrap();
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

#[tokio::test]
async fn lifecycle_tunnel_retention_full_capacity_allows_existing_identity_retry() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    state.refresh_once().await.unwrap();
    retained_paired_failures(&state, MAX_FAILURE_RECORDS - 1).await;
    source.paired_failure.store(true, Ordering::SeqCst);
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut(A)
        .unwrap()
        .last_attempt -= Duration::from_secs(31);
    assert_eq!(state.inner.failure_slots.available_permits(), 0);
    state.refresh_once().await.unwrap_err();
    assert_eq!(source.count(A), 2);
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 2);
    assert_eq!(
        state.inner.failed_attempts.read().await.len(),
        MAX_FAILURE_RECORDS
    );
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_retention_caller_drop_releases_unrecorded_capacity_reservation() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    source.paired_failure.store(true, Ordering::SeqCst);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    retained_paired_failures(&state, MAX_FAILURE_RECORDS - 1).await;
    let mut scan = Box::pin(state.refresh_once());
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.count(A) == 0 {
            assert!(futures_util::poll!(&mut scan).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.count(A), 1);
    assert_eq!(state.inner.failure_slots.available_permits(), 0);
    drop(scan);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(state.inner.failure_slots.available_permits(), 1);
    assert_eq!(
        state.inner.failed_attempts.read().await.len(),
        MAX_FAILURE_RECORDS - 1
    );
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_retention_clear_queued_failure_cannot_resurrect_metadata() {
    let source = FakeSource::new();
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    source.list_failure.store(true, Ordering::SeqCst);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut clear_input = state.prepare_cutover::<2>().unwrap();
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let mut attempt = Box::pin(state.refresh_connect(A, Route::Usbmux, &ticket, Some(1)));
    assert!(futures_util::poll!(&mut attempt).is_pending());
    assert_eq!(source.count(A), 1);
    let failed = state.inner.failed_attempts.write().await;
    gate.add_permits(1);
    assert!(futures_util::poll!(&mut attempt).is_pending());
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    let mut clear = Box::pin(state.clear(&mut clear_input));
    assert!(futures_util::poll!(&mut clear).is_pending());
    assert!(!ticket.is_current());
    drop(failed);
    attempt.await;
    clear.await;
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_retention_success_releases_reserved_failure_capacity() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.paired_failure.store(true, Ordering::SeqCst);
    let gate = Arc::new(Semaphore::new(0));
    source.script(A, [Behavior::Held(gate.clone())]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    retained_paired_failures(&state, MAX_FAILURE_RECORDS - 1).await;
    let mut scan = Box::pin(state.refresh_once());
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.count(A) == 0 {
            assert!(futures_util::poll!(&mut scan).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(state.inner.failure_slots.available_permits(), 0);
    gate.add_permits(1);
    scan.await.unwrap_err();
    assert!(state.get_record(A).await.unwrap().is_some());
    assert_eq!(
        state.inner.failed_attempts.read().await.len(),
        MAX_FAILURE_RECORDS - 1
    );
    assert_eq!(state.inner.failure_slots.available_permits(), 1);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
}

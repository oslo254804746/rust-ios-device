//! Actual HTTP requests and production scans through the private Source seam.
use super::*;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};

async fn cleanup(state: &TunnelAgentState, source: &FakeSource) {
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert!(state.inner.native.snapshot().closed_and_empty());
}

async fn post(base: &str, udid: &str, transport: Option<&str>, host: Option<&str>) -> StatusCode {
    let mut body = json!({"udid":udid});
    if let Some(transport) = transport {
        body["connection_type"] = json!(transport);
    }
    if let Some(host) = host {
        body["ip"] = json!(host);
    }
    client()
        .post(format!("{base}/start-tunnel"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
}

fn route(state: &TunnelAgentState, ticket: &Ticket) -> Option<intent::RouteIntent> {
    let _ = state;
    match ticket.desired().desired {
        intent::Desired::Manual { route, .. } => Some(route.route.clone()),
        _ => None,
    }
}

#[tokio::test]
async fn intent_http_wifi_selector_survives_usb_and_partial_discovery_without_fallback() {
    for host in [None, Some("192.0.2.45")] {
        let source = FakeSource::new();
        let state = state(source.clone(), LOCAL_BUDGET);
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let base = format!("http://127.0.0.1:{port}");
        let initial = post(&base, A, Some("wifi"), host).await;
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let initial_route = route(&state, &ticket);
        // The native fixture covers is_alive loss and original runtime retirement.
        state.inner.tunnels.write().await.remove(A);
        source.usb.lock().unwrap().push(A.into());
        source.list_failure.store(true, Ordering::SeqCst);
        source.paired_failure.store(true, Ordering::SeqCst);
        let scan = state.refresh_once().await;
        let restored = state.get_record(A).await;
        let calls = source.calls.lock().unwrap().clone();
        let recovered_route = route(&state, &ticket);
        server.abort();
        let _ = server.await;
        cleanup(&state, &source).await;
        assert_eq!(initial, StatusCode::OK);
        assert!(initial_route == Some(intent::RouteIntent::Wifi(host.map(str::to_owned))));
        assert!(recovered_route == initial_route);
        assert!(scan.is_err(), "preserve the acquired discovery primary");
        assert!(restored.is_some());
        assert_eq!(
            calls,
            vec![
                (A.into(), Route::Wifi(host.map(str::to_owned))),
                (A.into(), Route::Wifi(host.map(str::to_owned)))
            ]
        );
    }
}

#[tokio::test]
async fn intent_http_actual_mobdev2_family_rediscovers_host_and_respects_original_pin() {
    for pin in [None, Some("192.0.2.50")] {
        let source = FakeSource::new();
        source.script(
            A,
            [
                Behavior::Failure(std::io::ErrorKind::BrokenPipe),
                Behavior::Success,
            ],
        );
        source.paired.lock().unwrap().push(PairedMobdev2Device {
            udid: A.into(),
            host: "192.0.2.50".into(),
        });
        let state = state(source.clone(), LOCAL_BUDGET);
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let base = format!("http://127.0.0.1:{port}");
        let initial = post(&base, A, None, pin).await;
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let established = route(&state, &ticket);
        state.inner.tunnels.write().await.remove(A);
        source.usb.lock().unwrap().push(A.into());
        source.paired.lock().unwrap()[0].host = "192.0.2.51".into();
        let scan = state.refresh_once().await;
        let record = state.get_record(A).await;
        let calls = source.calls.lock().unwrap().clone();
        let retained = route(&state, &ticket);
        server.abort();
        let _ = server.await;
        cleanup(&state, &source).await;
        assert_eq!(initial, StatusCode::OK);
        assert!(established == Some(intent::RouteIntent::Mobdev2(pin.map(str::to_owned))));
        assert!(retained == established);
        scan.unwrap();
        assert_eq!(record.is_some(), pin.is_none());
        assert_eq!(
            &calls[..2],
            &[
                (A.into(), Route::Usbmux),
                (A.into(), Route::Mobdev2("192.0.2.50".into()))
            ]
        );
        if pin.is_none() {
            assert_eq!(calls[2], (A.into(), Route::Mobdev2("192.0.2.51".into())));
        } else {
            assert_eq!(calls.len(), 2);
        }
    }
}

#[tokio::test]
async fn intent_http_delete_survives_scans_replug_failed_start_and_caller_drop() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let initial = post(&base, A, Some("wifi"), None).await;
    let authority = match state
        .inner
        .lifecycle
        .ticket(A)
        .await
        .unwrap()
        .desired()
        .desired
    {
        intent::Desired::Manual { authority, .. } => authority,
        _ => panic!("HTTP Source success did not establish manual intent"),
    };
    let stop = client()
        .delete(format!("{base}/tunnel/{A}"))
        .send()
        .await
        .unwrap()
        .status();
    let before = source.count(A);
    source
        .usb_details
        .lock()
        .unwrap()
        .replace(vec![source::UsbDevice {
            udid: A.into(),
            device_id: 71,
            usb: true,
        }]);
    state.refresh_once().await.unwrap();
    source.usb_details.lock().unwrap().replace(Vec::new());
    state.refresh_once().await.unwrap();
    source
        .usb_details
        .lock()
        .unwrap()
        .replace(vec![source::UsbDevice {
            udid: A.into(),
            device_id: 72,
            usb: true,
        }]);
    source.paired_failure.store(true, Ordering::SeqCst);
    let incomplete = state.refresh_once().await;
    source.paired_failure.store(false, Ordering::SeqCst);
    source.script(A, [Behavior::Failure(std::io::ErrorKind::BrokenPipe)]);
    let failed = post(&base, A, Some("wifi"), None).await;
    let stopped_after_failure = matches!(
        state
            .inner
            .lifecycle
            .ticket(A)
            .await
            .unwrap()
            .desired()
            .desired,
        intent::Desired::Stopped
    );
    source.script(A, [Behavior::Pending]);
    let mut pending = Box::pin(state.ensure_tunnel_with_intent(
        A,
        None,
        Some(StartTunnelConnectionType::Wifi),
        Some(intent::ManualRequest {
            authority,
            host_filter: None,
        }),
    ));
    assert!(futures_util::poll!(&mut pending).is_pending());
    drop(pending);
    let stopped_after_drop = matches!(
        state
            .inner
            .lifecycle
            .ticket(A)
            .await
            .unwrap()
            .desired()
            .desired,
        intent::Desired::Stopped
    );
    let after_attempts = source.count(A);
    state.refresh_once().await.unwrap();
    let after_scan = source.count(A);
    let resumed = post(&base, A, Some("wifi"), Some("192.0.2.60")).await;
    let resumed_route = route(&state, &state.inner.lifecycle.ticket(A).await.unwrap());
    server.abort();
    let _ = server.await;
    cleanup(&state, &source).await;
    assert_eq!(initial, StatusCode::OK);
    assert_eq!(stop, StatusCode::OK);
    assert_eq!(before, 1);
    assert!(incomplete.is_err());
    assert_eq!(failed, StatusCode::BAD_GATEWAY);
    assert!(stopped_after_failure && stopped_after_drop);
    assert_eq!(after_attempts, before + 2);
    assert_eq!(after_scan, after_attempts);
    assert_eq!(resumed, StatusCode::OK);
    assert!(resumed_route == Some(intent::RouteIntent::Wifi(Some("192.0.2.60".into()))));
}

#[tokio::test]
async fn intent_http_missing_stop_is_durable_and_4097_key_is_rejected_before_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    for index in 0..4096 {
        state
            .stop_tunnel(&format!("bounded-intent-{index}"))
            .await
            .unwrap();
    }
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let known = client()
        .delete(format!("{base}/tunnel/bounded-intent-0"))
        .send()
        .await
        .unwrap()
        .status();
    let rejected = post(&base, A, Some("wifi"), None).await;
    let rejected_stop = client()
        .delete(format!("{base}/tunnel/{A}"))
        .send()
        .await
        .unwrap();
    let rejection_class = rejected_stop.headers().get("x-ios-error-class").cloned();
    let rejection_status = rejected_stop.status();
    let count = state.inner.lifecycle.coordinator_count().await;
    let calls = source.calls.lock().unwrap().len();
    state.clear().await;
    let next = post(&base, A, Some("wifi"), None).await;
    let next_count = state.inner.lifecycle.coordinator_count().await;
    server.abort();
    let _ = server.await;
    cleanup(&state, &source).await;
    assert_eq!(known, StatusCode::NOT_FOUND);
    assert_eq!(
        (rejected, rejection_status),
        (StatusCode::BAD_GATEWAY, StatusCode::BAD_GATEWAY)
    );
    assert_eq!(rejection_class.unwrap(), "resource_limit");
    assert_eq!((count, calls), (4096, 0));
    assert_eq!(next, StatusCode::OK);
    assert_eq!(next_count, 1);
}

#[tokio::test]
async fn intent_actual_manual_source_success_clears_old_default_backoff_before_next_loss() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Failure(std::io::ErrorKind::BrokenPipe)]);
    let state = state(source.clone(), LOCAL_BUDGET);
    state.refresh_once().await.unwrap();
    let old = state.inner.failed_attempts.read().await[A].version.clone();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let started = post(&base, A, Some("usbmux"), None).await;
    let cleaned = !state.inner.failed_attempts.read().await.contains_key(A);
    state.inner.tunnels.write().await.remove(A);
    state.refresh_once().await.unwrap();
    let calls = source.count(A);
    let pending = state
        .inner
        .lifecycle
        .ticket(A)
        .await
        .unwrap()
        .pending_success();
    server.abort();
    let _ = server.await;
    cleanup(&state, &source).await;
    drop(old);
    assert_eq!(started, StatusCode::OK);
    assert!(cleaned && pending.is_none());
    assert_eq!(calls, 3, "no time advance to the old 30-second backoff");
}

#[tokio::test]
async fn intent_contested_manual_cleanup_never_removes_later_real_source_failure_version() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Failure(std::io::ErrorKind::BrokenPipe)]);
    let state = state(source.clone(), LOCAL_BUDGET);
    state.refresh_once().await.unwrap();
    let held = state.inner.failed_attempts.write().await;
    let old = held[A].version.clone();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let started = post(&base, A, Some("usbmux"), None).await;
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap().fenced();
    let queued = ticket.pending_success().unwrap().1.version;
    drop(held);
    state.inner.tunnels.write().await.remove(A);
    source.script(A, [Behavior::Failure(std::io::ErrorKind::ConnectionReset)]);
    let failed = state
        .refresh_connect_deferred(A, Route::Usbmux, &ticket, Some(1))
        .await
        .unwrap();
    state
        .flush_failure_intents(vec![FailureIntent {
            _input: None,
            udid: A.into(),
            ticket: ticket.clone(),
            failed,
        }])
        .await
        .unwrap();
    let newer = state.inner.failed_attempts.read().await[A].clone();
    let whole = scan::Deadline::new(LOCAL_BUDGET).unwrap();
    state.flush_manual_successes(&whole).await.unwrap();
    let retained = state.inner.failed_attempts.read().await[A].version.clone();
    let calls = source.count(A);
    let newer_fail_count = newer.fail_count;
    let newer_version = newer.version.clone();
    // This retained fixture clone owns the real failure permit. Release it
    // before requiring every original permit to recover during cleanup.
    drop(newer);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source).await;
    assert_eq!(started, StatusCode::OK);
    assert!(Arc::ptr_eq(&old, &queued));
    assert!(!Arc::ptr_eq(&old, &newer_version));
    assert!(Arc::ptr_eq(&newer_version, &retained));
    assert_eq!(newer_fail_count, 1);
    assert_eq!(calls, 3);
}

#[tokio::test]
async fn intent_original_grant_reload_cannot_borrow_another_active_grant_for_recovery() {
    let fixture = PolicyFixture::new(vec![
        grant(
            TOKEN,
            vec![
                Permission::ManageServer,
                Permission::ManageTunnels,
                Permission::List,
            ],
        ),
        grant(REPLACEMENT, vec![Permission::Connect]),
    ]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(DEVICE.into());
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        Upstreams::default(),
        source.clone(),
        LOCAL_BUDGET,
    );
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let started = client()
        .post(format!("{base}/start-tunnel"))
        .bearer_auth(TOKEN)
        .json(&json!({"udid":DEVICE,"connection_type":"wifi"}))
        .send()
        .await
        .unwrap()
        .status();
    fixture.replace(vec![grant(
        REPLACEMENT,
        vec![
            Permission::Connect,
            Permission::ManageTunnels,
            Permission::List,
        ],
    )]);
    let reload = client()
        .post(format!("{base}/auth/reload"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .status();
    state.refresh_once().await.unwrap();
    state.refresh_once().await.unwrap();
    let suppressed = matches!(
        state
            .inner
            .lifecycle
            .ticket(DEVICE)
            .await
            .unwrap()
            .desired()
            .desired,
        intent::Desired::Stopped
    );
    let before = source.count(DEVICE);
    let replacement = client()
        .post(format!("{base}/start-tunnel"))
        .bearer_auth(REPLACEMENT)
        .json(&json!({"udid":DEVICE,"connection_type":"wifi"}))
        .send()
        .await
        .unwrap()
        .status();
    let after = source.count(DEVICE);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source).await;
    assert_eq!(
        (started, reload, replacement),
        (StatusCode::OK, StatusCode::OK, StatusCode::OK)
    );
    assert!(suppressed);
    assert_eq!((before, after), (1, 2));
}

#[tokio::test]
async fn intent_live_http_start_reauthorizes_only_the_actual_published_selector() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let started = post(&base, A, Some("wifi"), Some("192.0.2.10")).await;
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let repeated = post(&base, A, Some("usb"), Some("192.0.2.99")).await;
    let retained = state.inner.tunnels.read().await[A].publication.clone();
    let actual = route(&state, &state.inner.lifecycle.ticket(A).await.unwrap());
    let count = source.count(A);
    server.abort();
    let _ = server.await;
    cleanup(&state, &source).await;
    assert_eq!((started, repeated), (StatusCode::OK, StatusCode::OK));
    assert_eq!(count, 1);
    assert!(Arc::ptr_eq(&publication, &retained));
    assert!(actual == Some(intent::RouteIntent::Wifi(Some("192.0.2.10".into()))));
}

#[tokio::test]
async fn intent_clear_does_not_evict_live_old_tickets_to_evade_4096_gate_limit() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let mut held = Vec::new();
    for index in 0..4096 {
        held.push(
            state
                .inner
                .lifecycle
                .ticket(&format!("held-gate-{index}"))
                .await
                .unwrap(),
        );
    }
    state.clear().await;
    let rejected = state
        .inner
        .lifecycle
        .ticket("new-epoch-unknown")
        .await
        .err();
    let old_cancelled = held.iter().all(|ticket| !ticket.is_current());
    let count = state.inner.lifecycle.coordinator_count().await;
    drop(held);
    let next = state.inner.lifecycle.ticket("new-epoch-unknown").await;
    let next_count = state.inner.lifecycle.coordinator_count().await;
    let calls = source.calls.lock().unwrap().len();
    cleanup(&state, &source).await;
    let rejected = rejected.unwrap();
    assert_eq!(
        crate::output::operation_error_class(rejected.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(old_cancelled && next.is_ok());
    assert_eq!((count, next_count, calls), (4096, 1, 0));
}

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn assert_cancelled(error: &anyhow::Error) {
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(!crate::output::error_message(error).contains(A));
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_all_routes_and_scan_stay_closed_after_clear() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.notify_shutdown();
    state.clear().await;
    for route in [
        None,
        Some(StartTunnelConnectionType::Usbmux),
        Some(StartTunnelConnectionType::Usb),
        Some(StartTunnelConnectionType::Wifi),
    ] {
        let result = state.ensure_tunnel(A, None, route).await;
        state.clear().await;
        assert_cancelled(&result.unwrap_err());
    }
    assert_cancelled(&state.refresh_once().await.unwrap_err());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(!state.is_ready().await);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_cancels_connection_before_locked_table_cleanup() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut attempt = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut attempt).is_pending());
    source.entered(A, 1).await;
    let table = state.inner.tunnels.write().await;
    state.notify_shutdown();
    let outcome = tokio::time::timeout(LOCAL_BUDGET, attempt).await;
    drop(table);
    state.clear().await;
    assert_cancelled(
        &outcome
            .expect("shutdown waited for table cleanup")
            .unwrap_err(),
    );
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
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
async fn lifecycle_tunnel_shutdown_admission_drops_completed_device_during_attachment_query() {
    let source = FakeSource::new();
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut attempt = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut attempt).is_pending());
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.notify_shutdown();
    let outcome = tokio::time::timeout(LOCAL_BUDGET, attempt).await;
    state.clear().await;
    assert_cancelled(
        &outcome
            .expect("attachment query ignored shutdown")
            .unwrap_err(),
    );
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_queued_same_device_never_polls_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let serial = ticket.serial().await;
    let mut queued = Box::pin(start(state.clone(), A));
    assert!(futures_util::poll!(&mut queued).is_pending());
    state.notify_shutdown();
    let outcome = tokio::time::timeout(LOCAL_BUDGET, queued).await;
    drop(serial);
    state.clear().await;
    assert_cancelled(&outcome.expect("serial queue ignored shutdown").unwrap_err());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_plain_clear_allows_fresh_manual_start() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), A).await.unwrap();
    state.clear().await;
    start(state.clone(), A).await.unwrap();
    assert_eq!(source.count(A), 2);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_discovery_cancel_drops_probe_without_ready() {
    let source = FakeSource::new();
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    state.notify_shutdown();
    tokio::time::timeout(LOCAL_BUDGET, scan)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(!state.is_ready().await);
    assert_cancelled(&state.refresh_once().await.unwrap_err());
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_server_caller_drop_rejects_retained_state_clone() {
    let source = FakeSource::new();
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut owner = Box::pin(server::serve(
        listener,
        agent_router(state.clone()),
        state.clone(),
        Duration::from_secs(10),
        server::CLEANUP_TIMEOUT,
        std::future::pending(),
    ));
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.usb_active.load(Ordering::SeqCst) == 0 {
            assert!(futures_util::poll!(&mut owner).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(owner);
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    state.clear().await;
    assert_cancelled(&start(state.clone(), A).await.unwrap_err());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert!(!state.is_ready().await);
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_admission_http_body_finishing_after_clear_cannot_publish() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), B).await.unwrap();
    let entered = Arc::new(Semaphore::new(0));
    let request_entered = entered.clone();
    let app = agent_router(state.clone()).layer(axum::middleware::from_fn(
        move |request: axum::extract::Request, next: axum::middleware::Next| {
            let entered = request_entered.clone();
            async move {
                if request.uri().path() == "/start-tunnel" {
                    entered.add_permits(1);
                }
                next.run(request).await
            }
        },
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let owner_state = state.clone();
    let owner = tokio::spawn(async move {
        server::serve(
            listener,
            app,
            owner_state,
            Duration::from_secs(10),
            server::CLEANUP_TIMEOUT,
            std::future::pending(),
        )
        .await
    });
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let body = serde_json::to_vec(&json!({"udid": A, "connection_type": "usbmux"})).unwrap();
    let headers = format!(
        "POST /start-tunnel HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await.unwrap();
    stream.write_all(&body[..1]).await.unwrap();
    tokio::time::timeout(LOCAL_BUDGET, entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let response = client()
        .post(format!("http://127.0.0.1:{port}/shutdown"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while !state.list_records().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    // Axum still owns this incomplete request during graceful drain.
    assert!(!owner.is_finished());
    stream.write_all(&body[1..]).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(LOCAL_BUDGET, stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(LOCAL_BUDGET, owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        source.count(A),
        0,
        "delegated request contacted source after shutdown"
    );
    assert!(
        state.list_records().await.is_empty(),
        "late handler recreated a tunnel after clear"
    );
    let response = String::from_utf8(response).unwrap();
    assert!(response.starts_with("HTTP/1.1 502"), "{response}");
    assert!(
        response.contains("x-ios-error-class: cancelled"),
        "{response}"
    );
    assert!(!response.contains(A));
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

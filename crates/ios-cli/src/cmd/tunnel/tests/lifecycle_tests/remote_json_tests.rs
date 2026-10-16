use super::*;

#[tokio::test]
async fn remote_json_escaped_wire_keeps_listing_target_and_original_global_error_without_source() {
    const BODY: &[u8] = br#"{"uid\u002d\u96ea":[{"interface":"quote\"\u96ea","upstream":"https:\/\/nested.invalid\/","extension":"line\nnext"}]}"#;
    const INVALID: &[u8] = b"{\n\"uid\\u0061\": [\n\"bad\\q\"]}";
    let invalid = Arc::new(AtomicBool::new(false));
    let captured = invalid.clone();
    let (port, server) = spawn_test_router(Router::new().route(
        "/",
        get(move || {
            let invalid = captured.clone();
            async move {
                if invalid.load(Ordering::SeqCst) {
                    INVALID
                } else {
                    BODY
                }
            }
        }),
    ))
    .await;
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let url = format!("http://127.0.0.1:{port}/");
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(url.clone())
        .unwrap();
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let grouped = state.list_federated(0).await.unwrap().prepare().unwrap();
    let output = serde_json::to_value(&grouped).unwrap();
    assert_eq!(output["uid-雪"][0]["interface"], "quote\"雪");
    assert_eq!(output["uid-雪"][0]["extension"], "line\nnext");
    assert_eq!(output["uid-雪"][0]["upstream"], url);
    drop(grouped);
    assert_eq!(bank.available_permits(), before);
    let target = state
        .remote_target(&ConnectQuery {
            udid: "uid-雪".into(),
            port: 12345,
            interface: Some("quote\"雪".into()),
            upstream: None,
            hop: 0,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(target.interface.as_deref(), Some("quote\"雪"));
    assert_eq!(
        target.next_upstream.as_deref(),
        Some("https://nested.invalid/")
    );
    drop(target);
    assert_eq!(bank.available_permits(), before);
    invalid.store(true, Ordering::SeqCst);
    let error = fetch_upstream_listing(&state.inner.upstream_access, &url, 1, &bank)
        .await
        .err()
        .unwrap();
    let expected = serde_json::from_slice::<serde_json::Value>(INVALID).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<serde_json::Error>()
            .unwrap()
            .to_string(),
        expected.to_string()
    );
    assert_eq!(bank.available_permits(), before);
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn remote_json_actual_root_preserves_extensions_overlay_and_original_target_selection() {
    let body = serde_json::json!({
        "synthetic-雪":[null,{"interface":"selected","upstream":"nested-original","extension":{"snow":"雪🦀","items":[0,true,null]}},17],
        "empty":[],"scalar-only":[false,3],"remote-only":[{"future-key":"kept"}]
    });
    let expected = body.clone();
    let (upstream_port, upstream_task) = spawn_test_router(Router::new().route(
        "/",
        get(move || {
            let body = body.clone();
            async move { Json(body) }
        }),
    ))
    .await;
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let url = format!("http://127.0.0.1:{upstream_port}/");
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(url.clone())
        .unwrap();
    let bank = state.inner.global_metadata_bytes.clone();
    let baseline = bank.available_permits();
    let query = ConnectQuery {
        udid: "synthetic-雪".into(),
        port: 12345,
        interface: None,
        upstream: Some(url.clone()),
        hop: 0,
    };
    let selected = state.remote_target(&query).await.unwrap().unwrap();
    assert!(
        selected.interface.is_none(),
        "legacy default selects first array entry even if scalar"
    );
    drop(selected);
    let selected = state
        .remote_target(&ConnectQuery {
            interface: Some("selected".into()),
            ..query
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(selected.interface.as_deref(), Some("selected"));
    assert_eq!(selected.next_upstream.as_deref(), Some("nested-original"));
    drop(selected);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let output = client()
        .get(format!("http://127.0.0.1:{port}/"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(output.as_object().unwrap().len(), 2);
    assert_eq!(output["synthetic-雪"].as_array().unwrap().len(), 1);
    assert_eq!(
        output["synthetic-雪"][0]["extension"],
        expected["synthetic-雪"][1]["extension"]
    );
    assert_eq!(output["synthetic-雪"][0]["upstream"], url);
    assert_eq!(output["remote-only"][0]["future-key"], "kept");
    assert!(output.get("empty").is_none() && output.get("scalar-only").is_none());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while bank.available_permits() != baseline {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    upstream_task.abort();
    assert!(upstream_task.await.unwrap_err().is_cancelled());
    state.clear().await;
}

#[tokio::test]
async fn remote_json_snapshot_refuses_before_http_and_body_copy_refuses_before_allocation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let (port, server) = spawn_test_router(Router::new().route(
        "/",
        get(move || {
            seen.fetch_add(1, Ordering::SeqCst);
            async { Json(serde_json::json!({"uid":[{"field":"private-fixture"}]})) }
        }),
    ))
    .await;
    let source = FakeSource::new();
    let state = state(source, TUNNEL_CONNECT_TIMEOUT);
    let url = format!("http://127.0.0.1:{port}/");
    state
        .inner
        .upstreams
        .write()
        .await
        .insert(url.clone())
        .unwrap();
    let bank = state.inner.global_metadata_bytes.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let error = state.list_federated(0).await.err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let allocated = Arc::new(AtomicUsize::new(0));
    let captured = allocated.clone();
    // Hook is thread-local; the current-thread fixture drives the entire HTTP
    // body collector here. Framework-owned receive chunks are a separate scope.
    let hook = super::super::super::metadata_vec::on_allocation(move |_, _| {
        captured.fetch_add(1, Ordering::SeqCst);
    });
    let error = fetch_upstream_listing(&state.inner.upstream_access, &url, 1, &bank)
        .await
        .err()
        .unwrap();
    drop(hook);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(allocated.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), 0);
    drop(held);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    state.clear().await;
}

use super::*;

#[tokio::test]
async fn upstream_registry_http_growth_refusal_keeps_state_and_original_resource_error_without_source(
) {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let url = "https://registered-original-fixture.invalid:23456/";
    let success_bytes = serde_json::to_vec(&serde_json::json!({
        "data":true,"operation":"add_upstream","url":url
    }))
    .unwrap()
    .len();
    let held = Arc::new(StdMutex::new(None));
    let captured = held.clone();
    let charged = bank.clone();
    let guard = metadata_vec::on_allocation(move |layout, _| {
        if layout.size() == success_bytes && captured.lock().unwrap().is_none() {
            let leave = url.len() + 2 * std::mem::size_of::<paid_json::Text>() - 1;
            *captured.lock().unwrap() = Some(
                charged
                    .clone()
                    .try_acquire_many_owned((charged.available_permits() - leave) as u32)
                    .unwrap(),
            );
        }
    });
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/upstream"))
        .header("connection", "close")
        .json(&serde_json::json!({"url":url}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("registration admission"));
    drop(guard);
    assert!(
        held.lock().unwrap().is_some(),
        "success reply must precede mutation"
    );
    held.lock().unwrap().take();
    assert_eq!(state.inner.upstreams.read().await.len(), 0);
    assert_eq!(bank.available_permits(), before);
    let response = client()
        .post(format!("http://127.0.0.1:{port}/upstream"))
        .header("connection", "close")
        .json(&serde_json::json!({"url":url}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.bytes().await.unwrap();
    let after = bank.available_permits();
    let pointer = state
        .inner
        .upstreams
        .read()
        .await
        .iter()
        .next()
        .unwrap()
        .as_str()
        .as_ptr();
    let response = client()
        .post(format!("http://127.0.0.1:{port}/upstream"))
        .header("connection", "close")
        .json(&serde_json::json!({"url":url}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.bytes().await.unwrap();
    assert_eq!(
        state
            .inner
            .upstreams
            .read()
            .await
            .iter()
            .next()
            .unwrap()
            .as_str()
            .as_ptr(),
        pointer
    );
    assert_eq!(bank.available_permits(), after);
    let response = client()
        .delete(format!("http://127.0.0.1:{port}/upstream"))
        .header("connection", "close")
        .json(&serde_json::json!({"url":url}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
    assert_eq!(body["data"], true);
    assert_eq!(bank.available_permits(), after + url.len());
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    drop(state);
    // Axum's accepted connection task observes Connection: close after the
    // last body flush; wait for that original owner, not just the listener.
    tokio::time::timeout(LOCAL_BUDGET, async {
        while bank.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

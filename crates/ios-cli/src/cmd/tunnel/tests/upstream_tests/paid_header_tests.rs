use super::*;

#[tokio::test]
async fn authorization_payload_http_actual_header_and_pre_send_refusal_preserve_original_token() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = calls.clone();
    let (port, server) = spawn_test_router(Router::new().route(
        "/",
        get(move |headers: axum::http::HeaderMap| {
            let calls = captured.clone();
            async move {
                assert_eq!(headers["authorization"], format!("Bearer {TOKEN}"));
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Json(json!({}))
            }
        }),
    ))
    .await;
    let root = format!("http://127.0.0.1:{port}/");
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let access = configured(
        &fixture,
        json!([{"url":root,"token_file":fixture.token_path}]),
    )
    .await;
    let bytes = 7 + TOKEN.len();
    let limited = Arc::new(Semaphore::new(bytes - 1));
    let hook = metadata_vec::on_allocation(|_, _| panic!("refused header or body allocated"));
    let error = fetch_upstream_listing(&access, &root, 1, &limited)
        .await
        .err()
        .unwrap();
    drop(hook);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(!format!("{error:?}").contains(TOKEN));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(limited.available_permits(), bytes - 1);
    let bank = Arc::new(Semaphore::new(8192));
    drop(
        fetch_upstream_listing(&access, &root, 1, &bank)
            .await
            .unwrap(),
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(1), async {
        while bank.available_permits() != 8192 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn authorization_payload_websocket_request_last_header_outlives_input_client_and_manager() {
    let root = "http://127.0.0.1:23456/";
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
    let access = configured(
        &fixture,
        json!([{"url":root,"token_file":fixture.token_path}]),
    )
    .await;
    let state = TunnelAgentState::with_upstreams(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        access,
    );
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let client = state.inner.upstream_access.client(root).unwrap();
    let url =
        reqwest::Url::parse("ws://127.0.0.1:23456/connect?udid=uid&port=12345&hop=1").unwrap();
    let bytes = 7 + TOKEN.len();
    let limited = Arc::new(Semaphore::new(bytes - 1));
    let error = client
        .websocket_request(&url, |token| authorization::header(token, &limited))
        .unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(limited.available_permits(), bytes - 1);
    let request = client
        .websocket_request(&url, |token| authorization::header(token, &bank))
        .unwrap();
    assert_eq!(bank.available_permits(), before - bytes);
    let last = request.headers()["authorization"].clone();
    assert!(last.is_sensitive());
    assert_eq!(
        last.as_bytes().as_ptr(),
        request.headers()["authorization"].as_bytes().as_ptr()
    );
    assert!(!format!("{request:?}").contains(TOKEN));
    drop(request);
    drop(client);
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(Duration::from_secs(1)).await.unwrap();
    drop(state);
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - bytes
    );
    assert_eq!(last.as_bytes(), format!("Bearer {TOKEN}").as_bytes());
    drop(last);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

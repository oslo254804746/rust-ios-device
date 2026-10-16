use super::*;

#[test]
fn remote_target_moves_paid_url_and_keeps_optional_route_text_with_original_url_contract() {
    let bank = Arc::new(Semaphore::new(4096));
    let root = "https://127.0.0.1:23456/";
    let upstream = paid_json::Text::copy(root, &bank).unwrap();
    let pointer = upstream.as_str().as_ptr();
    let target = RemoteTarget::new(
        upstream,
        Some("quote\"雪"),
        Some("https://nested.invalid/"),
        &bank,
    )
    .unwrap();
    assert_eq!(
        target.upstream.as_str().as_ptr(),
        pointer,
        "paid snapshot URL must move without another copy"
    );
    let bytes = root.len() + "quote\"雪".len() + "https://nested.invalid/".len();
    assert_eq!(bank.available_permits(), 4096 - bytes);
    let query = ConnectQuery {
        udid: "synthetic-雪".into(),
        port: 12345,
        interface: None,
        upstream: None,
        hop: 1,
    };
    let url = remote_connect_url(&target, &query).unwrap();
    assert_eq!(url.scheme(), "wss");
    assert_eq!(url.path(), "/connect");
    let pairs = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    assert_eq!(pairs["udid"], "synthetic-雪");
    assert_eq!(pairs["port"], "12345");
    assert_eq!(pairs["hop"], "2");
    assert_eq!(pairs["interface"], "quote\"雪");
    assert_eq!(pairs["upstream"], "https://nested.invalid/");
    drop(target);
    assert_eq!(bank.available_permits(), 4096);
    let target = RemoteTarget::new(
        paid_json::Text::copy(root, &bank).unwrap(),
        Some(""),
        None,
        &bank,
    )
    .unwrap();
    assert_eq!(target.interface.as_deref(), Some(""));
    assert!(target.next_upstream.is_none());
    drop(target);
    assert_eq!(bank.available_permits(), 4096);
    // The independently allocated URL is outside the target text's fee.
    assert_eq!(url.scheme(), "wss");
}

#[test]
fn remote_target_last_byte_refusal_refunds_partial_fields_and_preserves_resource_cause() {
    let root = "http://127.0.0.1:23456/";
    let interface = "selected-雪";
    let next = "nested-original";
    let total = root.len() + interface.len() + next.len() - 1;
    let bank = Arc::new(Semaphore::new(total));
    let upstream = paid_json::Text::copy(root, &bank).unwrap();
    let allocations = Arc::new(AtomicUsize::new(0));
    let captured = allocations.clone();
    let error = metadata_vec::with_allocation_hook(
        move |layout, _| {
            assert_eq!(layout.size(), interface.len());
            captured.fetch_add(1, Ordering::SeqCst);
        },
        || {
            RemoteTarget::new(upstream, Some(interface), Some(next), &bank)
                .err()
                .unwrap()
        },
    );
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(allocations.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), total);
}

#[tokio::test]
async fn remote_target_result_keeps_exact_text_fees_after_input_and_manager_drop() {
    let (port, server) = spawn_test_router(Router::new().route("/", get(|| async {
        Json(serde_json::json!({"uid":[{"interface":"selected-雪","upstream":"nested-original","extension":{"large":"unselected-payload"}}]}))
    }))).await;
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let url = format!("http://127.0.0.1:{port}/");
    state.inner.upstreams.write().await.insert(url.clone());
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let target = state
        .remote_target(&ConnectQuery {
            udid: "uid".into(),
            port: 12345,
            interface: Some("selected-雪".into()),
            upstream: None,
            hop: 0,
        })
        .await
        .unwrap()
        .unwrap();
    let bytes = url.len() + "selected-雪".len() + "nested-original".len();
    assert_eq!(
        bank.available_permits(),
        before - bytes,
        "raw body/tree/source backing must already be released"
    );
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    drop(state);
    assert_eq!(
        bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES - bytes
    );
    assert_eq!(target.upstream.as_str(), url);
    assert_eq!(target.interface.as_deref(), Some("selected-雪"));
    assert_eq!(target.next_upstream.as_deref(), Some("nested-original"));
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    drop(target);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

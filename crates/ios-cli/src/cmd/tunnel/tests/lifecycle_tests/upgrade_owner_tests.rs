use super::*;

#[tokio::test]
async fn websocket_owner_one_byte_refusal_precedes_locked_local_lookup_and_remote_http() {
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let (leaf_port, leaf) = spawn_test_router(Router::new().fallback(get(move || {
        let observed = observed.clone();
        async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Json(json!({}))
        }
    })))
    .await;
    for remote in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), LOCAL_BUDGET);
        let url = format!("http://127.0.0.1:{leaf_port}/");
        state
            .inner
            .upstreams
            .write()
            .await
            .insert(url.clone())
            .unwrap();
        let bank = state.inner.global_metadata_bytes.clone();
        let before = bank.available_permits();
        let owner_bytes = if remote {
            let prepared = upgrade_owner::prepare(&bank, upgrade_owner::remote).unwrap();
            let bytes = before - bank.available_permits();
            drop(prepared);
            bytes
        } else {
            let prepared = upgrade_owner::prepare(&bank, upgrade_owner::local).unwrap();
            let bytes = before - bank.available_permits();
            drop(prepared);
            bytes
        };
        let leave = 2048 + owner_bytes - 1;
        let held = bank
            .clone()
            .try_acquire_many_owned((before - leave) as u32)
            .unwrap();
        // If the default-local path gets as far as target lookup it waits on
        // this actual table lock. The admission response must precede it.
        let table = state.inner.tunnels.write().await;
        let allocations = Arc::new(StdMutex::new(Vec::new()));
        let observed = allocations.clone();
        let guard = metadata_vec::on_allocation(move |layout, _| {
            observed.lock().unwrap().push(layout.size());
        });
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let selector = if remote {
            format!("&upstream={url}")
        } else {
            String::new()
        };
        let error = tokio::time::timeout(
            LOCAL_BUDGET,
            tokio_tungstenite::connect_async(format!(
                "ws://127.0.0.1:{port}/connect?udid={A}&port=12345{selector}"
            )),
        )
        .await
        .expect("owner refusal before locked lookup")
        .unwrap_err();
        match error {
            tungstenite::Error::Http(response) => {
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
                let body = std::str::from_utf8(response.body().as_ref().unwrap()).unwrap();
                assert!(body.contains("WebSocket owner admission failed"));
            }
            other => panic!("expected owner admission response: {other}"),
        }
        drop(guard);
        assert_eq!(
            *allocations.lock().unwrap(),
            [2048],
            "no prefix of owner backings"
        );
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        assert_eq!(source.count(A), 0);
        assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
        drop(table);
        drop(held);
        tokio::time::timeout(LOCAL_BUDGET, async {
            while bank.available_permits() != before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        state.notify_shutdown();
        state.clear().await;
        state.drain_native(LOCAL_BUDGET).await.unwrap();
        drop(state);
        // Aborting the listener is not a join of Hyper's accepted connection.
        // The failed client has dropped its TCP stream; require the actual
        // final connection/router owner to end, rather than assuming it did.
        tokio::time::timeout(LOCAL_BUDGET, async {
            while bank.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual last HTTP/router owner releases the original manager");
        assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    }
    leaf.abort();
    assert!(leaf.await.unwrap_err().is_cancelled());
}

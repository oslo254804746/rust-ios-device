use super::*;

#[tokio::test]
async fn remote_epoch_local_stop_keeps_remote_but_global_clear_shutdown_close_original_proxy() {
    const UID: &str = "remote-epoch-fixture";
    let probe_bank = Arc::new(tokio::sync::Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let probe = upgrade_owner::prepare(&probe_bank, upgrade_owner::remote).unwrap();
    let future_bytes = MAX_GLOBAL_KNOWN_METADATA_BYTES
        - probe_bank.available_permits()
        - shared_weak_allocation::Strong::<std::sync::Mutex<Option<WebSocket>>, ()>::body_bytes();
    drop(probe);
    assert_eq!(
        probe_bank.available_permits(),
        MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
    let mut closed = Vec::new();
    for shutdown in [false, true] {
        let proxy = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy_port = proxy.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut tcp, _) = proxy.accept().await.unwrap();
            let mut preamble = [0; 20];
            tcp.read_exact(&mut preamble).await.unwrap();
            assert_eq!(
                u32::from_le_bytes(preamble[16..].try_into().unwrap()),
                12345
            );
            let mut bytes = [0; 128];
            loop {
                let count = tcp.read(&mut bytes).await.unwrap();
                if count == 0 {
                    break;
                }
                tcp.write_all(&bytes[..count]).await.unwrap();
            }
        });
        let mut record = fake_record(proxy_port);
        record.udid = UID.into();
        let listing = serde_json::to_value(&record).unwrap();
        let leaf = Router::new()
            .route(
                "/",
                get(move |Query(query): Query<FederationQuery>| {
                    let listing = listing.clone();
                    async move {
                        assert_eq!(query.hop, 1);
                        Json(json!({UID:[listing]}))
                    }
                }),
            )
            .route(
                "/connect",
                get(
                    move |Query(query): Query<ConnectQuery>, ws: WebSocketUpgrade| async move {
                        assert_eq!(query.udid, UID);
                        assert_eq!(query.port, 12345);
                        assert_eq!(query.hop, 1);
                        let tcp = connect_record_port(&record, query.port).await.unwrap();
                        ws.on_upgrade(move |socket| async move {
                            let _ = bridge_tcp_websocket(socket, tcp, &fixture_bridge_bank()).await;
                        })
                    },
                ),
            );
        let (leaf_port, leaf_task) = spawn_test_router(leaf).await;
        let state = TunnelAgentState::new(TunMode::Userspace);
        state
            .inner
            .upstreams
            .write()
            .await
            .insert(format!("http://127.0.0.1:{leaf_port}/"))
            .unwrap();
        let bank = state.inner.global_metadata_bytes.clone();
        let before = bank.available_permits();
        let mut gate_bytes = 0;
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let (mut websocket, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{port}/connect?udid={UID}&port=12345"
        ))
        .await
        .unwrap();
        assert_eq!(
            state.inner.lifecycle.coordinator_count().await,
            0,
            "remote lookup must not create a local device gate"
        );
        for bytes in [b"before-stop".as_slice(), b"after-local-stop".as_slice()] {
            if bytes == b"after-local-stop" {
                assert!(!state.stop_tunnel(UID).await.unwrap());
                // Stopping an unknown local UID still creates the original
                // canonical suppression gate, independently of the remote WS.
                gate_bytes = Lifecycle::initial_metadata_bytes(UID);
                assert_eq!(state.inner.lifecycle.coordinator_count().await, 1);
            }
            websocket
                .send(tungstenite::Message::Binary(bytes.to_vec()))
                .await
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), websocket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
                tungstenite::Message::Binary(bytes.to_vec())
            );
            assert_eq!(before - bank.available_permits(), future_bytes + gate_bytes,
                "actual active remote bridge owns its original concrete future fee after HTTP/target owners end");
        }
        if shutdown {
            state.notify_shutdown();
        } else {
            state.clear().await;
        }
        let terminal = tokio::time::timeout(Duration::from_millis(150), websocket.next()).await;
        closed.push(matches!(
            terminal,
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(tungstenite::Message::Close(_))))
        ));
        drop(websocket);
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while bank.available_permits() != before - gate_bytes {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("actual final bridge drop refunds its own backing");
        server.abort();
        leaf_task.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        assert!(leaf_task.await.unwrap_err().is_cancelled());
        state.notify_shutdown();
        state.clear().await;
        state.drain_native(Duration::from_secs(1)).await.unwrap();
    }
    assert_eq!(
        closed,
        [true, true],
        "root global lifetime must close both remote proxy hops"
    );
}

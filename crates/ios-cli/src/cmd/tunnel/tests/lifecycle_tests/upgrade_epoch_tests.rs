use super::*;

#[tokio::test]
async fn upgrade_epoch_captured_record_never_borrows_a_newer_restart_ticket() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    drop(start(state.clone(), A).await.unwrap());
    let captured = state.get_local_target(A).await.unwrap().unwrap();
    assert!(captured.ticket.is_current());
    state.stop_tunnel(A).await.unwrap();
    let newer = state.inner.lifecycle.ticket(A).await.unwrap();
    assert!(newer.is_current());
    assert!(!captured.ticket.same_incarnation(&newer));
    let polls = AtomicUsize::new(0);
    let error = captured
        .ticket
        .run(async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    drop(start(state.clone(), A).await.unwrap());
    let restarted = state.get_local_target(A).await.unwrap().unwrap();
    assert!(restarted.ticket.same_incarnation(&newer));
    assert!(!captured.ticket.is_current());
    assert_eq!(source.count(A), 2);
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
}

async fn echo(
    websocket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<TcpStream>,
    >,
    bytes: &[u8],
) {
    websocket
        .send(tungstenite::Message::Binary(bytes.to_vec()))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(LOCAL_BUDGET, websocket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        tungstenite::Message::Binary(bytes.to_vec())
    );
}

#[tokio::test]
async fn upgrade_epoch_manual_live_change_keeps_a_and_stop_a_keeps_b_original_connection() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let proxy_port = listener.local_addr().unwrap().port();
    let peers = tokio::spawn(async move {
        let mut owned = Vec::new();
        for expected_port in [12345, 12346] {
            let (mut tcp, _) = listener.accept().await.unwrap();
            owned.push(tokio::spawn(async move {
                let mut preamble = [0; 20];
                tcp.read_exact(&mut preamble).await.unwrap();
                assert_eq!(
                    u32::from_le_bytes(preamble[16..].try_into().unwrap()),
                    expected_port
                );
                let mut bytes = [0; 128];
                loop {
                    let count = tcp.read(&mut bytes).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    tcp.write_all(&bytes[..count]).await.unwrap();
                }
            }));
        }
        owned
    });
    let source = FakeSource::new();
    source
        .proxy_port
        .store(proxy_port as usize, Ordering::SeqCst);
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), LOCAL_BUDGET);
    state.refresh_once().await.unwrap();
    let old_selection = state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .unwrap()
        ._ticket
        .clone();
    drop(start(state.clone(), B).await.unwrap());
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let (mut a, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/connect?udid={A}&port=12345"
    ))
    .await
    .unwrap();
    let (mut b, _) = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/connect?udid={B}&port=12346"
    ))
    .await
    .unwrap();
    let peers = peers.await.unwrap();
    echo(&mut a, b"before-manual").await;
    echo(&mut b, b"original-B").await;
    let authority = state
        .inner
        .access
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap()
        .device_authority(Permission::ManageTunnels, A)
        .ok()
        .unwrap();
    drop(
        state
            .ensure_tunnel_with_intent(
                A,
                None,
                Some(StartTunnelConnectionType::Wifi),
                Some(intent::ManualRequest {
                    authority,
                    host_filter: None,
                }),
            )
            .await
            .unwrap(),
    );
    assert!(
        !old_selection.is_current(),
        "fixture must change the old desired fence"
    );
    echo(&mut a, b"same-runtime-after-manual").await;
    state.stop_tunnel(A).await.unwrap();
    let terminal = tokio::time::timeout(LOCAL_BUDGET, a.next()).await.unwrap();
    assert!(matches!(
        terminal,
        None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_)))
    ));
    echo(&mut b, b"B-after-stop-A").await;
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    state.notify_shutdown();
    let terminal = tokio::time::timeout(LOCAL_BUDGET, b.next()).await.unwrap();
    assert!(matches!(
        terminal,
        None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_)))
    ));
    drop(a);
    drop(b);
    for peer in peers {
        tokio::time::timeout(LOCAL_BUDGET, peer)
            .await
            .unwrap()
            .unwrap();
    }
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn upgrade_epoch_local_stop_clear_shutdown_closes_idle_original_tcp() {
    let mut closed = Vec::new();
    for action in 0..3 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let proxy_port = listener.local_addr().unwrap().port();
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut preamble = [0; 20];
            tcp.read_exact(&mut preamble).await.unwrap();
            assert_eq!(
                u32::from_le_bytes(preamble[16..].try_into().unwrap()),
                12345
            );
            ready_tx.send(()).unwrap();
            // Deliberately keep this original peer alive after FakeDevice Drop.
            // The manager epoch must close its own stream, not wait for us.
            let eof = tcp.read(&mut [0; 1]).await.unwrap();
            assert_eq!(eof, 0);
        });
        let source = FakeSource::new();
        source
            .proxy_port
            .store(proxy_port as usize, Ordering::SeqCst);
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        drop(start(state.clone(), A).await.unwrap());
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let (mut websocket, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{port}/connect?udid={A}&port=12345"
        ))
        .await
        .unwrap();
        ready_rx.await.unwrap();
        let usb = source.usb_calls.load(Ordering::SeqCst);
        match action {
            0 => assert!(state.stop_tunnel(A).await.unwrap()),
            1 => state.clear().await,
            _ => state.notify_shutdown(),
        }
        let terminal = tokio::time::timeout(Duration::from_millis(150), websocket.next()).await;
        closed.push(matches!(
            terminal,
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(tungstenite::Message::Close(_))))
        ));
        drop(websocket);
        tokio::time::timeout(LOCAL_BUDGET, peer)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source.count(A), 1);
        assert_eq!(source.usb_calls.load(Ordering::SeqCst), usb);
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        state.notify_shutdown();
        state.clear().await;
        state.drain_native(LOCAL_BUDGET).await.unwrap();
    }
    assert_eq!(
        closed,
        [true, true, true],
        "stop/clear/shutdown must cancel the original upgrade"
    );
}

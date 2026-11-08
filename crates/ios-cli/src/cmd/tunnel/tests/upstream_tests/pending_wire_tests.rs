use super::*;

async fn headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 512];
    while !bytes.ends_with(b"\r\n\r\n") {
        let count = stream.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0, "original request ended before its headers");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 8192, "fixture header bound exceeded");
    }
    bytes
}

fn authorization(bytes: &[u8], persistent: bool) {
    let text = std::str::from_utf8(bytes).unwrap();
    let value = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.trim());
    assert_eq!(value.is_some(), persistent);
    if let Some(value) = value {
        assert!(value == format!("Bearer {TOKEN}"));
    }
}

#[tokio::test]
async fn pending_remote_wire_clear_shutdown_close_original_listing_and_handshake_without_retry() {
    const UID: &str = "pending-remote-wire";
    let mut observed = Vec::new();
    for handshake in [false, true] {
        for shutdown in [false, true] {
            for persistent in [false, true] {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
                let upstream_port = listener.local_addr().unwrap().port();
                let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
                let mut peer = tokio::spawn(async move {
                    let (mut original, _) = listener.accept().await.unwrap();
                    let request = headers(&mut original).await;
                    assert!(request.starts_with(b"GET /?hop=1 HTTP/1.1\r\n"));
                    authorization(&request, persistent);
                    if handshake {
                        let body = format!(r#"{{"{UID}":[{{"interface":"selected"}}]}}"#);
                        let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                        original.write_all(response.as_bytes()).await.unwrap();
                        original.shutdown().await.unwrap();
                        drop(original);
                        let (mut socket, _) = listener.accept().await.unwrap();
                        let request = headers(&mut socket).await;
                        assert!(request.starts_with(b"GET /connect?"));
                        authorization(&request, persistent);
                        original = socket;
                    }
                    // Keep this exact request pending: no listing response or no101.
                    ready_tx.send(()).unwrap();
                    let eof = original.read(&mut [0; 1]).await;
                    let strict_eof = matches!(eof, Ok(0));
                    let no_retry =
                        tokio::time::timeout(Duration::from_millis(60), listener.accept())
                            .await
                            .is_err();
                    (strict_eof, no_retry)
                });
                let root = format!("http://127.0.0.1:{upstream_port}/");
                let policy = persistent
                    .then(|| PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]));
                let access = match &policy {
                    Some(policy) => {
                        configured(policy, json!([{"url":root,"token_file":policy.token_path}]))
                            .await
                    }
                    None => Upstreams::default(),
                };
                let state = TunnelAgentState::with_upstreams(
                    TunMode::Userspace,
                    AccessControl::trusted_test_local(),
                    access,
                );
                let mut signal_input = PreparedSignal::new(&state, shutdown);
                let mut prepared_outer_0 = state.prepare_cutover::<1>().unwrap();
                let mut prepared_outer_1 = state.prepare_cutover::<1>().unwrap();

                if !persistent {
                    state.inner.upstreams.write().await.insert(&root).unwrap();
                }
                let bank = state.inner.global_metadata_bytes.clone();
                let before = bank.available_permits();
                let (port, server) = spawn_test_router(agent_router(state.clone())).await;
                let mut request = tokio::spawn(tokio_tungstenite::connect_async(format!(
                    "ws://127.0.0.1:{port}/connect?udid={UID}&port=12345"
                )));
                tokio::time::timeout(Duration::from_secs(1), ready_rx)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
                signal_input.notify_or_clear(&state).await;
                let cancelled =
                    match tokio::time::timeout(Duration::from_secs(1), &mut request).await {
                        Ok(result) => match result.unwrap() {
                            Err(tungstenite::Error::Http(response)) => {
                                response.status() == StatusCode::BAD_GATEWAY
                                    && response.headers().get("x-ios-error-class").is_some_and(
                                        |value| {
                                            value
                                                == crate::output::operation_error_class(
                                                    lifecycle::cancelled().as_ref(),
                                                )
                                                .as_str()
                                        },
                                    )
                            }
                            _ => false,
                        },
                        Err(_) => {
                            request.abort();
                            assert!(request.await.unwrap_err().is_cancelled());
                            false
                        }
                    };
                let (strict_eof, no_retry) =
                    match tokio::time::timeout(Duration::from_secs(1), &mut peer).await {
                        Ok(result) => result.unwrap(),
                        Err(_) => {
                            // Failing fixture cleanup only; this is NOT counted as EOF
                            // or a successful original IO lifetime/driver join.
                            peer.abort();
                            assert!(peer.await.unwrap_err().is_cancelled());
                            (false, false)
                        }
                    };
                observed.push((
                    handshake, shutdown, persistent, cancelled, strict_eof, no_retry,
                ));
                tokio::time::timeout(Duration::from_secs(1), async {
                    while bank.available_permits() != before {
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .unwrap();
                assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
                server.abort();
                assert!(server.await.unwrap_err().is_cancelled());
                state.notify_shutdown(&mut prepared_outer_0);
                drop(prepared_outer_0);
                state.clear(&mut prepared_outer_1).await;
                drop(prepared_outer_1);
                state.drain_native(Duration::from_secs(1)).await.unwrap();
            }
        }
    }
    assert!(
        observed
            .iter()
            .all(|(_, _, _, cancelled, eof, no_retry)| *cancelled && *eof && *no_retry),
        "original listing/handshake IO observations: {observed:?}"
    );
}

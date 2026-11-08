use super::*;
use tokio::sync::oneshot;

fn connector(address: std::net::SocketAddr) -> ForwardConnector {
    ForwardConnector {
        device_id: 1,
        tunnel_proxy: None,
        _native: None,
        setup_failure: None,
        direct_test_target: Some(address),
    }
}

#[tokio::test]
async fn direct_port_wins_without_sending_business_data_to_proxy() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut connector = connector(direct.local_addr().unwrap());
    connector.tunnel_proxy = Some(TunnelProxyTarget {
        server_addr: "fd00::1".into(),
        proxy_port: proxy.local_addr().unwrap().port(),
    });
    let mut stream = connector.connect(62078).await.unwrap();
    stream.write_all(b"one-request").await.unwrap();
    let mut peer = direct.accept().await.unwrap().0;
    let mut data = [0; 11];
    peer.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"one-request");
    assert!(
        tokio::time::timeout(Duration::from_millis(30), proxy.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn refused_direct_port_falls_back_before_any_business_bytes() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut connector = connector(direct.local_addr().unwrap());
    drop(direct);
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    connector.tunnel_proxy = Some(TunnelProxyTarget {
        server_addr: "fd00::1".into(),
        proxy_port: proxy.local_addr().unwrap().port(),
    });
    let opened = tokio::spawn(async move { connector.connect(8100).await });
    let mut peer = proxy.accept().await.unwrap().0;
    let mut preamble = [0; 20];
    peer.read_exact(&mut preamble).await.unwrap();
    assert_eq!(
        &preamble[..16],
        &"fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets()
    );
    assert_eq!(
        &preamble[16..],
        &(8100u32 | ios_core::USERSPACE_PROXY_CONNECT_ACK).to_le_bytes()
    );
    peer.write_all(&[0]).await.unwrap();
    let mut stream = opened.await.unwrap().unwrap();
    stream.write_all(b"one-request").await.unwrap();
    let mut data = [0; 11];
    peer.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"one-request");
}

#[tokio::test]
async fn proxy_remote_failure_is_a_connect_error_before_business_copy() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut connector = connector(direct.local_addr().unwrap());
    drop(direct);
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    connector.tunnel_proxy = Some(TunnelProxyTarget {
        server_addr: "fd00::1".into(),
        proxy_port: proxy.local_addr().unwrap().port(),
    });
    let opened = tokio::spawn(async move { connector.connect(8100).await });
    let mut peer = proxy.accept().await.unwrap().0;
    let mut preamble = [0; 20];
    peer.read_exact(&mut preamble).await.unwrap();
    peer.write_all(&[1]).await.unwrap();
    let result = opened.await.unwrap();
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("remote refusal was accepted"),
    };
    assert!(error.chain().any(|cause| cause
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::ConnectionAborted)));
    let mut data = Vec::new();
    peer.read_to_end(&mut data).await.unwrap();
    assert!(data.is_empty());
}

#[tokio::test]
async fn legitimate_empty_eof_does_not_retry_on_another_route() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut connector = connector(direct.local_addr().unwrap());
    connector.tunnel_proxy = Some(TunnelProxyTarget {
        server_addr: "fd00::1".into(),
        proxy_port: proxy.local_addr().unwrap().port(),
    });
    let mut stream = connector.connect(8100).await.unwrap();
    let mut peer = direct.accept().await.unwrap().0;
    peer.shutdown().await.unwrap();
    assert_eq!(
        stream.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), proxy.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn once_forward_copies_partial_chunks_then_joins_both_original_tcp_eof() {
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_address = remote_listener.local_addr().unwrap();
    let remote = tokio::spawn(async move {
        let mut stream = remote_listener.accept().await.unwrap().0;
        let mut data = Vec::new();
        stream.read_to_end(&mut data).await.unwrap();
        assert_eq!(data, b"synthetic-forward-payload");
        stream.write_all(b"synthetic-reply").await.unwrap();
        stream.shutdown().await.unwrap();
        let mut extra = [0u8; 1];
        assert_eq!(stream.read(&mut extra).await.unwrap(), 0);
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(serve(
        listener,
        connector(remote_address),
        8100,
        true,
        std::future::pending(),
    ));
    let mut local = TcpStream::connect(address).await.unwrap();
    for chunk in b"synthetic-forward-payload".chunks(3) {
        local.write_all(chunk).await.unwrap();
    }
    local.shutdown().await.unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), local.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, b"synthetic-reply");
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    remote.await.unwrap();
    assert!(
        TcpStream::connect(address).await.is_err(),
        "original listener must be closed"
    );
}

#[tokio::test]
async fn forward_stop_after_readiness_wakes_idle_copy_and_joins_original_client() {
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_address = remote_listener.local_addr().unwrap();
    let (ready, observed) = oneshot::channel();
    let remote = tokio::spawn(async move {
        let mut stream = remote_listener.accept().await.unwrap().0;
        ready.send(()).unwrap();
        let mut data = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut data))
            .await
            .unwrap()
            .unwrap();
        assert!(data.is_empty());
        stream.shutdown().await.unwrap();
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(
        listener,
        connector(remote_address),
        8100,
        false,
        async {
            stopped.await.unwrap();
            Ok(())
        },
    ));
    let mut local = TcpStream::connect(address).await.unwrap();
    observed.await.unwrap();
    stop.send(()).unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), local.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    local.shutdown().await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    remote.await.unwrap();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn forward_client_limit_keeps_seventeenth_connection_out_of_owned_tasks() {
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_address = remote_listener.local_addr().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(
        listener,
        connector(remote_address),
        8100,
        false,
        async {
            stopped.await.unwrap();
            Ok(())
        },
    ));
    let mut locals = Vec::new();
    let mut remotes = Vec::new();
    for _ in 0..MAX_CLIENTS {
        locals.push(TcpStream::connect(address).await.unwrap());
        remotes.push(
            tokio::time::timeout(Duration::from_secs(2), remote_listener.accept())
                .await
                .unwrap()
                .unwrap()
                .0,
        );
    }
    let seventeenth = TcpStream::connect(address).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), remote_listener.accept())
            .await
            .is_err()
    );
    drop(seventeenth); // Never accepted by the bounded owner table.
    stop.send(()).unwrap();
    let readers = tokio::spawn(async move {
        let mut jobs = JoinSet::new();
        for mut stream in locals.into_iter().chain(remotes) {
            jobs.spawn(async move {
                let mut bytes = Vec::new();
                tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
                assert!(bytes.is_empty());
                stream.shutdown().await.unwrap();
            });
        }
        while let Some(result) = jobs.join_next().await {
            result.unwrap();
        }
    });
    let error = tokio::time::timeout(Duration::from_secs(4), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    readers.await.unwrap();
}

#[tokio::test]
async fn original_tcp_reset_is_never_a_clean_drain_receipt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut local = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let peer = listener.accept().await.unwrap().0;
    local
        .write_all(b"synthetic-unread-reset-marker")
        .await
        .unwrap();
    let mut byte = [0u8; 1];
    assert_eq!(peer.peek(&mut byte).await.unwrap(), 1);
    drop(peer); // Native RST: unread receive bytes are deliberately retained.
    let error = retire_socket(&mut local, Instant::now() + Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(error
        .chain()
        .any(|cause| cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|native| matches!(
                native.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
            ))));
}

#[tokio::test]
async fn preferred_and_fallback_real_dial_failures_keep_both_original_errors() {
    let preferred = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let preferred_address = preferred.local_addr().unwrap();
    let fallback = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let fallback_address = fallback.local_addr().unwrap();
    drop(preferred);
    drop(fallback);
    let connector = ForwardConnector {
        device_id: 1,
        tunnel_proxy: Some(TunnelProxyTarget {
            server_addr: "fd00::1".into(),
            proxy_port: fallback_address.port(),
        }),
        _native: None,
        setup_failure: None,
        direct_test_target: Some(preferred_address),
    };
    let error = match connector.connect(8100).await {
        Err(error) => error,
        Ok(_) => panic!("both actual refused endpoints must fail"),
    };
    let failure = error
        .downcast_ref::<ForwardFallbackFailure>()
        .expect("original failures must be retained in a typed owner");
    assert!(failure.primary.chain().any(|cause| cause
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::ConnectionRefused)));
    assert!(failure.fallback.chain().any(|cause| cause
        .downcast_ref::<std::io::Error>()
        .is_some_and(|native| native.kind() == std::io::ErrorKind::ConnectionRefused)));
}

#[tokio::test]
async fn one_reset_client_is_retained_without_stopping_a_second_original_client() {
    let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_address = remote_listener.local_addr().unwrap();
    let (reset, reset_seen) = oneshot::channel();
    let remote = tokio::spawn(async move {
        let first = remote_listener.accept().await.unwrap().0;
        let mut marker = [0u8; 1];
        assert_eq!(first.peek(&mut marker).await.unwrap(), 1);
        drop(first);
        reset.send(()).unwrap();
        let mut second = tokio::time::timeout(Duration::from_secs(3), remote_listener.accept())
            .await
            .unwrap()
            .unwrap()
            .0;
        let mut request = Vec::new();
        second.read_to_end(&mut request).await.unwrap();
        assert_eq!(request, b"second-survives");
        second.write_all(b"second-completed").await.unwrap();
        second.shutdown().await.unwrap();
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(serve(
        listener,
        connector(remote_address),
        8100,
        false,
        async {
            stopped.await.unwrap();
            Ok(())
        },
    ));
    let mut first = TcpStream::connect(address).await.unwrap();
    first.write_all(b"unread-reset-marker").await.unwrap();
    first.shutdown().await.unwrap();
    reset_seen.await.unwrap();
    let mut first_bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), first.read_to_end(&mut first_bytes))
        .await
        .unwrap()
        .unwrap();
    let mut second = TcpStream::connect(address).await.unwrap();
    second.write_all(b"second-survives").await.unwrap();
    second.shutdown().await.unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), second.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply, b"second-completed");
    remote.await.unwrap();
    stop.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let retained = error
        .downcast_ref::<ServeFailure>()
        .expect("original client failure must survive the later stop");
    assert_eq!(retained.clients.count, 1);
    assert!(retained
        .clients
        .first
        .as_ref()
        .unwrap()
        .chain()
        .any(|cause| cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|native| matches!(
                native.kind(),
                std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::NotConnected
            ))));
}

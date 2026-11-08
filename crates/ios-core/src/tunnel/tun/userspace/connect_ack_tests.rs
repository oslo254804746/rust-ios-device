use super::*;

#[tokio::test]
async fn preamble_ack_is_opt_in_and_invalid_ports_still_fail() {
    for (port, ack) in [(62078u32, false), (62078 | LOCAL_PROXY_CONNECT_ACK, true)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let mut peer = listener.accept().await.unwrap().0;
        let mut bytes = [0; 20];
        bytes[..16].copy_from_slice(&std::net::Ipv6Addr::LOCALHOST.octets());
        bytes[16..].copy_from_slice(&port.to_le_bytes());
        client.write_all(&bytes).await.unwrap();
        let (_, actual, confirmed) = read_proxy_preamble(&mut peer).await.unwrap();
        assert_eq!(actual, 62078);
        assert_eq!(confirmed, ack);
    }
}

#[tokio::test]
async fn acknowledged_proxy_waits_for_original_remote_connect_and_preserves_legacy_bytes() {
    for ack in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let peer = listener.accept().await.unwrap().0;
        let (to_stack, _reads) = mpsc::with_bank::<OwnedBuffer>(1, None).unwrap();
        let (writes, from_stack) = mpsc::with_bank::<OwnedBuffer>(1, None).unwrap();
        let (connected, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(proxy_local_client(
            peer,
            to_stack,
            from_stack,
            ready,
            RuntimeStorage::unmetered(),
            ack,
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), client.read_u8())
                .await
                .is_err()
        );
        connected.send(()).unwrap();
        if ack {
            assert_eq!(client.read_u8().await.unwrap(), 0);
        }
        let mut payload = OwnedBuffer::zeroed(5, None).unwrap();
        payload.copy_from_slice(b"hello");
        writes.send(payload).await.unwrap();
        let mut bytes = [0; 5];
        client.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"hello");
        drop(client);
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn aborted_remote_connect_reports_failure_instead_of_clean_business_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let peer = listener.accept().await.unwrap().0;
    let (to_stack, _reads) = mpsc::with_bank::<OwnedBuffer>(1, None).unwrap();
    let (_writes, from_stack) = mpsc::with_bank::<OwnedBuffer>(1, None).unwrap();
    let (connected, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(proxy_local_client(
        peer,
        to_stack,
        from_stack,
        ready,
        RuntimeStorage::unmetered(),
        true,
    ));
    drop(connected);
    assert_eq!(client.read_u8().await.unwrap(), 1);
    let mut bytes = Vec::new();
    client.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty());
    task.await.unwrap();
}

#[tokio::test]
async fn natural_forward_completion_delivers_bytes_then_clean_eof() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let peer = listener.accept().await.unwrap().0;
    let (to_stack, _reads) = mpsc::with_bank::<OwnedBuffer>(1, None).unwrap();
    let (writes, from_stack) = mpsc::with_bank::<OwnedBuffer>(1, None).unwrap();
    let (connected, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(proxy_local_client(
        peer,
        to_stack,
        from_stack,
        ready,
        RuntimeStorage::unmetered(),
        false,
    ));
    connected.send(()).unwrap();
    let mut payload = OwnedBuffer::zeroed(5, None).unwrap();
    payload.copy_from_slice(b"hello");
    writes.send(payload).await.unwrap();
    let mut bytes = [0; 5];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes, b"hello");
    drop(writes);
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    let eof = tokio::time::timeout(Duration::from_secs(2), client.read(&mut [0; 1])).await;
    assert!(
        matches!(eof, Ok(Ok(0))),
        "natural proxy completion: {eof:?}"
    );
}

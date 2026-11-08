use super::*;
use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    command: DebugCmd,
}

#[test]
fn parses_debug_command_and_checks_version() {
    let cmd = TestCli::parse_from([
        "debug",
        "com.example.App",
        "--local-app",
        "C:/build/Demo.app",
    ]);
    assert_eq!(cmd.command.bundle_id, "com.example.App");
    assert_eq!(
        cmd.command.local_app,
        Some(PathBuf::from("C:/build/Demo.app"))
    );
    assert!(ensure_rsd_debugproxy_supported(&Version::new(15, 5, 0)).is_err());
    assert!(ensure_rsd_debugproxy_supported(&Version::new(17, 0, 0)).is_ok());
}

#[test]
fn service_is_required_before_any_debug_bytes() {
    assert!(require_debugproxy_port(None)
        .unwrap_err()
        .to_string()
        .contains("missing"));
    assert!(require_debugproxy_port(Some(0)).is_err());
    assert_eq!(require_debugproxy_port(Some(1234)).unwrap(), 1234);
}

#[test]
fn script_arguments_escape_paths_and_reject_injection() {
    let path = r#"C:\build folder\say"hello`1+1`.app"#;
    let argument = lldb_argument(path).unwrap();
    // Independently decode LLDB's quoted argument escape form.
    let mut decoded = String::new();
    let mut chars = argument[1..argument.len() - 1].chars();
    while let Some(ch) = chars.next() {
        decoded.push(if ch == '\\' {
            chars.next().unwrap()
        } else {
            ch
        });
    }
    assert_eq!(decoded, path);
    let remote = "/app/quoted\"path`1+1`.app";
    let encoded_remote = python_string(remote).unwrap();
    assert!(!encoded_remote.contains('`'));
    assert_eq!(
        serde_json::from_str::<String>(&encoded_remote).unwrap(),
        remote
    );
    assert!(lldb_argument("a\nprocess launch").is_err());
    assert!(python_string("a\nprocess launch").is_err());
    assert!(python_string("/app/with\\backslash.app").is_err());
}

#[tokio::test]
async fn original_proxy_ack_then_loopback_transfers_both_ways_and_releases() {
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = proxy.accept().await.unwrap();
        let mut header = [0; 20];
        stream.read_exact(&mut header).await.unwrap();
        assert_eq!(
            &header[..16],
            &"fd00::1".parse::<Ipv6Addr>().unwrap().octets()
        );
        assert_eq!(
            u32::from_le_bytes(header[16..].try_into().unwrap()),
            1234 | ios_core::USERSPACE_PROXY_CONNECT_ACK
        );
        stream.write_all(&[0]).await.unwrap();
        let mut request = [0; 4];
        stream.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        stream.write_all(b"pong").await.unwrap();
        stream.shutdown().await.unwrap();
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
    });
    let remote = connect_proxy(proxy_port, "fd00::1".parse().unwrap(), 1234)
        .await
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = tokio::spawn(relay_one(listener, remote));
    let mut client = TcpStream::connect(address).await.unwrap();
    client.write_all(b"ping").await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert_eq!(response, b"pong");
    relay.await.unwrap().unwrap();
    peer.await.unwrap();
    assert!(TcpStream::connect(address).await.is_err());
}

#[tokio::test]
async fn remote_failure_is_propagated_before_business_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut header = [0; 20];
        stream.read_exact(&mut header).await.unwrap();
        stream.write_all(&[1]).await.unwrap();
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
    });
    let error = connect_proxy(port, "fd00::1".parse().unwrap(), 1234)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("remote connection failed"));
    peer.await.unwrap();
}

#[tokio::test]
async fn invalid_or_missing_ack_is_reported() {
    for ack in [Some(2), None] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = [0; 20];
            stream.read_exact(&mut header).await.unwrap();
            if let Some(value) = ack {
                stream.write_all(&[value]).await.unwrap();
            }
        });
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            connect_proxy(port, "fd00::1".parse().unwrap(), 1234),
        )
        .await
        .unwrap()
        .unwrap_err();
        let io = error.downcast_ref::<std::io::Error>().unwrap();
        assert_eq!(
            io.kind(),
            if ack.is_some() {
                std::io::ErrorKind::InvalidData
            } else {
                std::io::ErrorKind::UnexpectedEof
            }
        );
        tokio::time::timeout(Duration::from_secs(2), peer)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn cancellation_during_transfer_releases_both_accepted_sockets() {
    let peer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = TcpStream::connect(peer_listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = peer_listener.accept().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(until_cancelled(relay_one(listener, remote), async {
        stopped.await.unwrap();
        Ok(())
    }));
    let mut client = TcpStream::connect(address).await.unwrap();
    client.write_all(b"ping").await.unwrap();
    let mut received = [0; 4];
    tokio::time::timeout(Duration::from_secs(2), peer.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"ping");
    stop.send(()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error
        .chain()
        .any(|source| source.is::<ios_core::OperationCancelled>()));
    assert!(TcpStream::connect(address).await.is_err());
    for stream in [&mut client, &mut peer] {
        let mut rest = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut rest))
            .await
            .unwrap()
            .unwrap();
        assert!(rest.is_empty());
    }
}

#[tokio::test]
async fn cancellation_drops_original_listener_and_remote_socket() {
    let peer_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote = TcpStream::connect(peer_listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = peer_listener.accept().await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(until_cancelled(relay_one(listener, remote), async {
        stopped.await.unwrap();
        Ok(())
    }));
    stop.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert!(error
        .chain()
        .any(|source| source.is::<ios_core::OperationCancelled>()));
    assert!(TcpStream::connect(address).await.is_err());
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), peer.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(bytes.is_empty());
}

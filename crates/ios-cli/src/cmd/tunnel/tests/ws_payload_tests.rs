use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn original_bridge(
    bank: Arc<Semaphore>,
    write_limit: Option<usize>,
) -> ws_payload::Completion {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let tcp_port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        tcp.write_all(b"x").await.unwrap();
        let eof = tokio::time::timeout(Duration::from_secs(2), tcp.read(&mut [0; 1]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(eof, 0, "original TCP endpoint must reach strict EOF");
    });
    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let app = Router::new().route(
        "/connect",
        get(move |ws: WebSocketUpgrade| {
            let bank = bank.clone();
            let tx = tx.clone();
            async move {
                let tcp = TcpStream::connect(("127.0.0.1", tcp_port)).await.unwrap();
                let ws = bounded_websocket_upgrade(ws);
                let ws = match write_limit {
                    Some(limit) => ws.write_buffer_size(0).max_write_buffer_size(limit),
                    None => ws,
                };
                ws.on_upgrade(move |socket| async move {
                    let result = bridge_tcp_websocket(socket, tcp, &bank).await;
                    tx.lock().unwrap().take().unwrap().send(result).ok();
                })
            }
        }),
    );
    let (port, server) = spawn_test_router(app).await;
    let (mut client, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/connect"))
            .await
            .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .unwrap()
        .unwrap();
    peer.await.unwrap();
    let terminal = tokio::time::timeout(Duration::from_secs(2), client.next())
        .await
        .unwrap();
    assert!(matches!(
        terminal,
        None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_)))
    ));
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    result
}

#[tokio::test]
async fn websocket_payload_actual_quota_refusal_allocates_nothing_and_closes_original_tcp() {
    let bank = Arc::new(Semaphore::new(0));
    let hook = metadata_vec::on_allocation(|_, _| panic!("refused Binary Vec was allocated"));
    let completion = original_bridge(bank.clone(), None).await;
    drop(hook);
    let error = completion.result().as_ref().unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(error
        .chain()
        .any(|cause| cause.to_string() == "tunnel global known metadata byte limit exceeded"));
    assert_eq!(bank.available_permits(), 0);
    drop(completion);
    assert_eq!(bank.available_permits(), 0);
}

#[tokio::test]
async fn websocket_payload_actual_write_buffer_full_retains_original_frame_pointer_and_fee() {
    let bank = Arc::new(Semaphore::new(1));
    let pointer = Arc::new(AtomicUsize::new(0));
    let captured = pointer.clone();
    let charged = bank.clone();
    let hook = metadata_vec::on_allocation(move |layout, original| {
        assert_eq!(layout.size(), 1);
        assert_eq!(charged.available_permits(), 0);
        captured.store(original, Ordering::SeqCst);
    });
    // A one-byte payload plus its frame header exceeds this original
    // tungstenite write-buffer limit, so the Err owns that same Frame Vec.
    let completion = original_bridge(bank.clone(), Some(1)).await;
    drop(hook);
    let error = completion.result().as_ref().unwrap_err();
    let original = error
        .chain()
        .find_map(|source| source.downcast_ref::<tungstenite::Error>())
        .unwrap();
    let tungstenite::Error::WriteBufferFull(tungstenite::Message::Frame(frame)) = original else {
        panic!("original payload-owning frame error missing");
    };
    assert_eq!(frame.payload(), b"x");
    assert_eq!(
        frame.payload().as_ptr() as usize,
        pointer.load(Ordering::SeqCst)
    );
    assert_ne!(pointer.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(completion);
    assert_eq!(bank.available_permits(), 1);
}

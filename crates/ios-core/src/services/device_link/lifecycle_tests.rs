use super::*;
use tokio::io::{duplex, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufStream, DuplexStream};

fn frame(value: &plist::Value) -> Vec<u8> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value).unwrap();
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(payload);
    bytes
}

fn process(value: plist::Value) -> plist::Value {
    plist::Value::Array(vec!["DLMessageProcessMessage".into(), value])
}

fn dictionary() -> plist::Dictionary {
    plist::Dictionary::from_iter([("Owned", plist::Value::Boolean(true))])
}

fn version() -> plist::Value {
    plist::Value::Array(vec!["DLMessageVersionExchange".into(), 300u64.into()])
}

fn ready() -> plist::Value {
    plist::Value::Array(vec!["DLMessageDeviceReady".into()])
}

async fn read_frame(peer: &mut DuplexStream) -> plist::Value {
    let len = peer.read_u32().await.unwrap() as usize;
    assert!(len <= MAX_PLIST_SIZE);
    let mut payload = vec![0; len];
    peer.read_exact(&mut payload).await.unwrap();
    plist::from_bytes(&payload).unwrap()
}

fn timed_out(error: DeviceLinkError, issued: bool) {
    let DeviceLinkError::Io(error) = error else {
        panic!("expected IO timeout");
    };
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(
        error
            .get_ref()
            .and_then(|source| source.downcast_ref::<Elapsed>())
            .is_some(),
        issued
    );
}

async fn rejected(client: &mut DeviceLinkClient<DuplexStream>) {
    tokio::time::timeout(Duration::from_secs(1), async {
        client.set_operation_timeout(None);
        assert!(!client.is_usable());
        assert!(matches!(
            client.version_exchange().await,
            Err(DeviceLinkError::Protocol(_))
        ));
        assert!(matches!(
            client.send_message(&true).await,
            Err(DeviceLinkError::Protocol(_))
        ));
        assert!(matches!(
            client.recv_message().await,
            Err(DeviceLinkError::Protocol(_))
        ));
        assert!(matches!(
            client.send_process_message(&dictionary()).await,
            Err(DeviceLinkError::Protocol(_))
        ));
        assert!(matches!(
            client.recv_process_message().await,
            Err(DeviceLinkError::Protocol(_))
        ));
        assert!(matches!(
            client.call_process_message(&dictionary()).await,
            Err(DeviceLinkError::Protocol(_))
        ));
        assert!(matches!(
            client.disconnect().await,
            Err(DeviceLinkError::Protocol(_))
        ));
    })
    .await
    .expect("invalid client must reject before waiting on transport");
}

#[tokio::test]
async fn lifecycle_device_link_preflight_and_unpolled_operations_do_no_io() {
    struct Refused;
    impl Serialize for Refused {
        fn serialize<T: serde::Serializer>(&self, _: T) -> Result<T::Ok, T::Error> {
            Err(serde::ser::Error::custom("local serialization refused"))
        }
    }
    let (stream, mut peer) = duplex(4096);
    let mut client = DeviceLinkClient::new(stream);
    client.set_operation_timeout(Some(Duration::ZERO));
    timed_out(client.version_exchange().await.unwrap_err(), false);
    timed_out(client.send_message(&true).await.unwrap_err(), false);
    timed_out(client.recv_message().await.unwrap_err(), false);
    timed_out(
        client
            .send_process_message(&dictionary())
            .await
            .unwrap_err(),
        false,
    );
    timed_out(client.recv_process_message().await.unwrap_err(), false);
    timed_out(
        client
            .call_process_message(&dictionary())
            .await
            .unwrap_err(),
        false,
    );
    timed_out(client.disconnect().await.unwrap_err(), false);
    assert!(client.is_usable());
    client.set_operation_timeout(Some(Duration::MAX));
    let DeviceLinkError::Io(error) = client.recv_message().await.unwrap_err() else {
        panic!("expected invalid deadline");
    };
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    drop(client.version_exchange());
    drop(client.send_message(&true));
    drop(client.recv_message());
    drop(client.send_process_message(&dictionary()));
    drop(client.recv_process_message());
    drop(client.call_process_message(&dictionary()));
    drop(client.disconnect());
    client.set_operation_timeout(None);
    assert!(matches!(
        client.send_message(&Refused).await,
        Err(DeviceLinkError::Plist(_))
    ));
    assert!(matches!(
        client.send_message(&"x".repeat(MAX_PLIST_SIZE)).await,
        Err(DeviceLinkError::Protocol(_))
    ));
    assert!(client.is_usable());
    let mut byte = [0];
    assert!(
        tokio::time::timeout(Duration::from_millis(5), peer.read(&mut byte))
            .await
            .is_err()
    );
    let (sent, received) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.send_message(&true), read_frame(&mut peer))
    })
    .await
    .unwrap();
    sent.unwrap();
    assert_eq!(received, plist::Value::Boolean(true));
}

#[tokio::test]
async fn lifecycle_device_link_empty_idle_cancellation_and_timeout_allow_reuse() {
    let (stream, mut peer) = duplex(4096);
    let mut client = DeviceLinkClient::new(stream);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), client.recv_message())
            .await
            .is_err()
    );
    assert!(client.is_usable());
    client.set_operation_timeout(Some(Duration::from_millis(5)));
    timed_out(client.recv_process_message().await.unwrap_err(), true);
    assert!(client.is_usable());
    client.set_operation_timeout(None);
    peer.write_all(&frame(&process(plist::Value::Dictionary(dictionary()))))
        .await
        .unwrap();
    assert_eq!(client.recv_process_message().await.unwrap(), dictionary());
    assert!(client.is_usable());
}

#[tokio::test]
async fn lifecycle_device_link_partial_and_buffered_reads_close_reuse() {
    for caller_cancel in [false, true] {
        let (stream, mut peer) = duplex(4096);
        let mut client = DeviceLinkClient::new(stream);
        let bytes = frame(&plist::Value::Boolean(true));
        peer.write_all(&bytes[..6]).await.unwrap();
        if caller_cancel {
            assert!(
                tokio::time::timeout(Duration::from_millis(5), client.recv_message())
                    .await
                    .is_err()
            );
        } else {
            client.set_operation_timeout(Some(Duration::from_millis(5)));
            timed_out(client.recv_message().await.unwrap_err(), true);
        }
        peer.write_all(&bytes[6..]).await.unwrap();
        rejected(&mut client).await;
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    }
    let (stream, mut peer) = duplex(4096);
    let mut stream = BufStream::new(stream);
    peer.write_all(&100u32.to_be_bytes()).await.unwrap();
    assert_eq!(stream.fill_buf().await.unwrap(), 100u32.to_be_bytes());
    let mut client = DeviceLinkClient::new(stream);
    client.set_operation_timeout(Some(Duration::from_millis(5)));
    timed_out(client.recv_message().await.unwrap_err(), true);
    assert!(!client.is_usable());
}

#[tokio::test]
async fn lifecycle_device_link_blocked_send_distinguishes_empty_and_partial_progress() {
    let (stream, mut peer) = duplex(64);
    let mut client = DeviceLinkClient::new(stream);
    client.stream_mut().write_all(&[0x5a; 64]).await.unwrap();
    client.set_operation_timeout(Some(Duration::from_millis(5)));
    timed_out(client.send_message(&true).await.unwrap_err(), true);
    assert!(client.is_usable());
    let mut prior = [0; 64];
    peer.read_exact(&mut prior).await.unwrap();
    assert_eq!(prior, [0x5a; 64]);
    timed_out(client.send_message(&true).await.unwrap_err(), true);
    rejected(&mut client).await;
    drop(client);
    let mut prefix = Vec::new();
    peer.read_to_end(&mut prefix).await.unwrap();
    assert_eq!(prefix, frame(&plist::Value::Boolean(true))[..64]);
}

#[tokio::test]
async fn lifecycle_device_link_process_call_cancellation_rejects_late_reply_without_io() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let (stream, mut peer) = duplex(4096);
        let mut client = DeviceLinkClient::new(stream);
        let request = dictionary();
        {
            let mut pending = Box::pin(client.call_process_message(&request));
            let request = tokio::select! {
                value = read_frame(&mut peer) => value,
                _ = &mut pending => panic!("call must await a reply"),
            };
            assert_eq!(request, process(plist::Value::Dictionary(dictionary())));
        }
        peer.write_all(&frame(&process(plist::Value::Dictionary(dictionary()))))
            .await
            .unwrap();
        rejected(&mut client).await;
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_device_link_process_call_shares_send_and_receive_budget() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let (stream, mut peer) = duplex(512);
        let mut client = DeviceLinkClient::new(stream);
        client.set_operation_timeout(Some(Duration::from_millis(200)));
        let request =
            plist::Dictionary::from_iter([("Payload", plist::Value::String("x".repeat(4096)))]);
        let server = async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            assert_eq!(
                read_frame(&mut peer).await,
                process(plist::Value::Dictionary(request.clone()))
            );
            tokio::time::sleep(Duration::from_millis(125)).await;
            let reply = frame(&process(plist::Value::Dictionary(dictionary())));
            assert!(reply.len() <= 512);
            peer.write_all(&reply).await.unwrap();
        };
        let (result, ()) = tokio::join!(client.call_process_message(&request), server);
        timed_out(result.unwrap_err(), true);
        rejected(&mut client).await;
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_device_link_version_exchange_retains_ownership_between_messages() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let (stream, mut peer) = duplex(4096);
        let mut client = DeviceLinkClient::new(stream);
        peer.write_all(&frame(&version())).await.unwrap();
        {
            let mut pending = Box::pin(client.version_exchange());
            let ack = tokio::select! {
                value = read_frame(&mut peer) => value,
                _ = &mut pending => panic!("exchange must await ready"),
            };
            assert_eq!(
                ack,
                plist::Value::Array(vec![
                    "DLMessageVersionExchange".into(),
                    "DLVersionsOk".into(),
                    300u64.into()
                ])
            );
        }
        peer.write_all(&frame(&ready())).await.unwrap();
        rejected(&mut client).await;
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_device_link_version_exchange_has_one_total_budget() {
    tokio::time::timeout(Duration::from_secs(1), async {
        let (stream, mut peer) = duplex(4096);
        let mut client = DeviceLinkClient::new(stream);
        client.set_operation_timeout(Some(Duration::from_millis(200)));
        let server = async {
            tokio::time::sleep(Duration::from_millis(125)).await;
            peer.write_all(&frame(&version())).await.unwrap();
            assert_eq!(
                read_frame(&mut peer).await,
                plist::Value::Array(vec![
                    "DLMessageVersionExchange".into(),
                    "DLVersionsOk".into(),
                    300u64.into()
                ])
            );
            tokio::time::sleep(Duration::from_millis(125)).await;
            peer.write_all(&frame(&ready())).await.unwrap();
        };
        let (result, ()) = tokio::join!(client.version_exchange(), server);
        timed_out(result.unwrap_err(), true);
        rejected(&mut client).await;
        drop(client);
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lifecycle_device_link_complete_envelope_errors_and_transport_failures_keep_policy() {
    let (stream, mut peer) = duplex(4096);
    let mut client = DeviceLinkClient::new(stream);
    peer.write_all(&frame(&process("wrong-payload".into())))
        .await
        .unwrap();
    assert!(matches!(
        client.recv_process_message().await,
        Err(DeviceLinkError::Protocol(_))
    ));
    assert!(client.is_usable());
    let server = async {
        assert_eq!(
            read_frame(&mut peer).await,
            process(plist::Value::Dictionary(dictionary()))
        );
        peer.write_all(&frame(&process("wrong-payload".into())))
            .await
            .unwrap();
    };
    let request = dictionary();
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.call_process_message(&request), server)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(DeviceLinkError::Protocol(_))));
    assert!(client.is_usable());
    peer.write_all(&frame(&process(plist::Value::Dictionary(dictionary()))))
        .await
        .unwrap();
    assert_eq!(client.recv_process_message().await.unwrap(), dictionary());

    for bytes in [
        vec![],
        [4u32.to_be_bytes().as_slice(), b"xxxx"].concat(),
        (MAX_PLIST_SIZE as u32 + 1).to_be_bytes().to_vec(),
    ] {
        let (stream, mut peer) = duplex(4096);
        let mut client = DeviceLinkClient::new(stream);
        peer.write_all(&bytes).await.unwrap();
        drop(peer);
        let error = client.recv_message().await.unwrap_err();
        match bytes.len() {
            0 => assert!(
                matches!(error, DeviceLinkError::Io(ref error) if error.kind() == std::io::ErrorKind::UnexpectedEof)
            ),
            8 => assert!(matches!(error, DeviceLinkError::Plist(_))),
            _ => assert!(matches!(error, DeviceLinkError::Protocol(_))),
        }
        rejected(&mut client).await;
    }
    let (stream, peer) = duplex(4096);
    let mut client = DeviceLinkClient::new(stream);
    drop(peer);
    assert!(
        matches!(client.send_message(&true).await, Err(DeviceLinkError::Io(ref error)) if error.kind() == std::io::ErrorKind::BrokenPipe)
    );
    rejected(&mut client).await;
    let (stream, mut peer) = duplex(4096);
    let mut client = DeviceLinkClient::new(stream);
    peer.write_all(&frame(&ready())).await.unwrap();
    assert!(matches!(
        client.version_exchange().await,
        Err(DeviceLinkError::Protocol(_))
    ));
    rejected(&mut client).await;
}

#[tokio::test]
async fn lifecycle_device_link_disconnect_is_terminal_and_partial_cancel_is_uncertain() {
    let (stream, mut peer) = duplex(4096);
    let mut client = DeviceLinkClient::new(stream);
    // The cleanup cap also safely clamps an otherwise unrepresentable budget.
    client.set_operation_timeout(Some(Duration::MAX));
    let (result, message) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.disconnect(), read_frame(&mut peer))
    })
    .await
    .unwrap();
    result.unwrap();
    assert_eq!(
        message,
        plist::Value::Array(vec![
            "DLMessageDisconnect".into(),
            "___EmptyParameterString___".into()
        ])
    );
    rejected(&mut client).await;
    drop(client);
    assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    for caller_cancel in [false, true] {
        let (stream, mut peer) = duplex(64);
        let mut client = DeviceLinkClient::new(stream);
        if caller_cancel {
            assert!(
                tokio::time::timeout(Duration::from_millis(5), client.disconnect())
                    .await
                    .is_err()
            );
        } else {
            client.set_operation_timeout(Some(Duration::from_millis(5)));
            timed_out(client.disconnect().await.unwrap_err(), true);
        }
        rejected(&mut client).await;
        drop(client);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(
            bytes,
            frame(&plist::Value::Array(vec![
                "DLMessageDisconnect".into(),
                "___EmptyParameterString___".into()
            ]))[..64]
        );
    }
}

#[tokio::test]
async fn lifecycle_device_link_default_disconnect_cap_expires_on_blocked_transport() {
    tokio::time::timeout(Duration::from_secs(6), async {
        let (stream, mut peer) = duplex(64);
        let mut client = DeviceLinkClient::new(stream);
        timed_out(client.disconnect().await.unwrap_err(), true);
        rejected(&mut client).await;
        drop(client);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes.len(), 64);
    })
    .await
    .unwrap();
}

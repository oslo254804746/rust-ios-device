use super::*;
use tokio::io::{AsyncReadExt, DuplexStream};

async fn read(peer: &mut DuplexStream) -> Dictionary {
    read_plist_frame(peer, MAX_FRAME).await.unwrap()
}
async fn write(peer: &mut DuplexStream, dict: Dictionary) {
    write_xml_plist_frame(peer, &dict, MAX_FRAME).await.unwrap();
}
async fn handshake(peer: &mut DuplexStream, ecid: u64) {
    assert_eq!(read(peer).await["Request"].as_string(), Some("QueryType"));
    write(
        peer,
        Dictionary::from_iter([
            (
                "Type".to_owned(),
                Value::String("com.apple.mobile.restored".into()),
            ),
            (
                "RestoreProtocolVersion".to_owned(),
                Value::Integer(15.into()),
            ),
        ]),
    )
    .await;
    assert_eq!(
        read(peer).await["QueryKey"].as_string(),
        Some("HardwareInfo")
    );
    write(
        peer,
        Dictionary::from_iter([(
            "HardwareInfo".to_owned(),
            Value::Dictionary(Dictionary::from_iter([(
                "UniqueChipID".to_owned(),
                Value::Integer(ecid.into()),
            )])),
        )]),
    )
    .await;
}

#[tokio::test]
async fn actual_start_and_firmware_chunks_reach_owned_peer() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let firmware = Dictionary::from_iter([
        ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
        (
            "DataType".to_owned(),
            Value::String("FirmwareUpdaterData".into()),
        ),
        (
            "Arguments".to_owned(),
            Value::Dictionary(Dictionary::from_iter([(
                "DataChunkSize".to_owned(),
                Value::Integer(2.into()),
            )])),
        ),
    ]);
    let incoming = firmware.clone();
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let start = read(&mut peer).await;
        assert_eq!(start["Request"].as_string(), Some("StartRestore"));
        assert_eq!(
            start["RestoreProtocolVersion"].as_unsigned_integer(),
            Some(15)
        );
        assert_eq!(
            start["RestoreOptions"].as_dictionary().unwrap()["RestoreBehavior"].as_string(),
            Some("Update")
        );
        write(&mut peer, incoming).await;
        let header = read(&mut peer).await;
        assert_eq!(header["DataSize"].as_unsigned_integer(), Some(5));
        assert_eq!(
            header["FirmwareResponseData"].as_dictionary().unwrap()["Ticket"].as_data(),
            Some(&[9][..])
        );
        let mut got = Vec::new();
        loop {
            let chunk = read(&mut peer).await;
            got.extend_from_slice(chunk["FirmwareResponseData"].as_data().unwrap());
            if chunk.get("DataDone").and_then(Value::as_boolean) == Some(true) {
                break;
            }
        }
        assert_eq!(got, b"12345");
        assert_eq!(
            peer.read(&mut [0]).await.unwrap(),
            0,
            "original client must reach EOF after Drop"
        );
    });
    let mut client =
        RestoredClient::connect(stream, 42, Duration::from_secs(1), OperationCancel::new())
            .await
            .unwrap();
    assert!(client.next_message().await.is_err());
    client
        .start_update(
            &Value::Integer(15.into()),
            Dictionary::from_iter([("RestoreBehavior".to_owned(), Value::String("Update".into()))]),
        )
        .await
        .unwrap();
    let actual = client.next_message().await.unwrap();
    assert_eq!(actual, firmware);
    client
        .send_firmware_response(
            &actual,
            Dictionary::from_iter([("Ticket".to_owned(), Value::Data(vec![9]))]),
            b"12345",
        )
        .await
        .unwrap();
    assert_eq!(client.firmware_bytes_written(), 5);
    drop(client);
    tokio::time::timeout(Duration::from_secs(1), remote)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancellation_interrupts_idle_receive_and_reuse_is_terminal() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        assert_eq!(
            read(&mut peer).await["Request"].as_string(),
            Some("StartRestore")
        );
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    });
    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(stream, 42, Duration::from_secs(10), cancel.clone())
        .await
        .unwrap();
    client
        .start_update(
            &Value::Integer(15.into()),
            Dictionary::from_iter([("RestoreBehavior".to_owned(), Value::String("Update".into()))]),
        )
        .await
        .unwrap();
    let trigger = tokio::spawn(async move {
        tokio::task::yield_now().await;
        cancel.cancel();
    });
    let error = tokio::time::timeout(Duration::from_secs(1), client.next_message())
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Cancelled);
    assert!(client
        .next_message()
        .await
        .unwrap_err()
        .to_string()
        .contains("terminal"));
    drop(client);
    trigger.await.unwrap();
    remote.await.unwrap();
}

#[tokio::test]
async fn wrong_ecid_and_timeout_do_not_start_restore() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 99).await;
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    });
    assert!(
        RestoredClient::connect(stream, 42, Duration::from_secs(1), OperationCancel::new())
            .await
            .is_err()
    );
    remote.await.unwrap();
    let (stream, _peer) = tokio::io::duplex(32768);
    let result = RestoredClient::connect(
        stream,
        42,
        Duration::from_millis(10),
        OperationCancel::new(),
    )
    .await;
    assert_eq!(
        crate::classify_error(&result.err().unwrap()),
        crate::ErrorClass::Timeout
    );
}

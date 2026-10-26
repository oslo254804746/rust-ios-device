use super::*;

#[test]
fn review_regression_missing_status_is_not_success() {
    let event = restore_event_from_plist(&Dictionary::from_iter([(
        "MsgType".to_owned(),
        Value::String("StatusMsg".into()),
    )]));
    assert!(!matches!(
        event,
        RestoreLifecycleEvent::Status {
            code: 0,
            finished: true,
            ..
        }
    ));
}

#[tokio::test]
async fn review_regression_unserved_data_port_never_completes() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let _ = read(&mut peer).await;
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                (
                    "DataType".to_owned(),
                    Value::String("SystemImageData".into()),
                ),
                ("DataPort".to_owned(), Value::Integer(1234.into())),
            ]),
        )
        .await;
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("StatusMsg".into())),
                ("Status".to_owned(), Value::Integer(0.into())),
            ]),
        )
        .await;
    });
    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(stream, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();
    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1]), None).unwrap();
    let result = run_restored_update_loop(
        &mut client,
        sm,
        Dictionary::from_iter([("RestoreBehavior".to_owned(), Value::String("Update".into()))]),
        |_, _| panic!("unsupported route must not call firmware provider"),
        &cancel,
    )
    .await;
    remote.await.unwrap();
    assert!(
        result.is_err(),
        "unserved data request must not count as a successful update"
    );
}
use tokio::io::{AsyncReadExt, DuplexStream};

#[tokio::test]
async fn system_image_data_port_uses_actual_asr_transport_and_does_not_finish_restore() {
    use tokio::io::AsyncWriteExt;
    let (stream, mut peer) = tokio::io::duplex(8192);
    let control = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        assert_eq!(
            read(&mut peer).await["Request"].as_string(),
            Some("StartRestore")
        );
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType", Value::String("DataRequestMsg".into())),
                ("DataType", Value::String("SystemImageData".into())),
                ("DataPort", Value::Integer(4321u64.into())),
            ]),
        )
        .await;
        peer
    });
    let (data, mut data_peer) = tokio::io::duplex(8192);
    let asr = tokio::spawn(async move {
        data_peer.write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Initiate</string></dict></plist>\n").await.unwrap();
        let mut offer = Vec::new();
        while !offer.ends_with(b"</plist>\n") {
            offer.push(data_peer.read_u8().await.unwrap());
        }
        let offer: Dictionary = plist::from_bytes(&offer).unwrap();
        assert_eq!(
            offer["Payload"].as_dictionary().unwrap()["Size"].as_unsigned_integer(),
            Some(3)
        );
        data_peer.write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Payload</string></dict></plist>\n").await.unwrap();
        let mut bytes = [0; 3];
        data_peer.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"OS!");
        assert_eq!(data_peer.read(&mut [0; 1]).await.unwrap(), 0);
    });
    let mut client =
        RestoredClient::connect(stream, 42, Duration::from_secs(2), OperationCancel::new())
            .await
            .unwrap();
    client
        .start_update(
            &Value::Integer(15u64.into()),
            Dictionary::from_iter([("RestoreBehavior", Value::String("Update".into()))]),
        )
        .await
        .unwrap();
    let incoming = client.next_message().await.unwrap();
    let receipt = client
        .send_system_image(
            &incoming,
            &mut std::io::Cursor::new(b"OS!".to_vec()),
            |port| async move {
                assert_eq!(port, 4321);
                Ok(data)
            },
        )
        .await
        .unwrap();
    assert_eq!(receipt.image_bytes_sent, 3);
    assert!(!client.terminal);
    drop(client);
    drop(control.await.unwrap());
    asr.await.unwrap();
}

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

#[tokio::test]
async fn restored_update_loop_executes_full_lifecycle_and_verifies_terminal_status() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let start = read(&mut peer).await;
        assert_eq!(start["Request"].as_string(), Some("StartRestore"));
        assert_eq!(
            start["RestoreOptions"].as_dictionary().unwrap()["RestoreBehavior"].as_string(),
            Some("Update")
        );

        // 1. ProgressMsg
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("ProgressMsg".into())),
                (
                    "Operation".to_owned(),
                    Value::String("Updating baseband".into()),
                ),
                ("Progress".to_owned(), Value::Integer(25.into())),
            ]),
        )
        .await;

        // 2. CheckpointMsg
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("CheckpointMsg".into())),
                (
                    "Checkpoint".to_owned(),
                    Value::String("CHECKPOINT_FLASH_START".into()),
                ),
            ]),
        )
        .await;

        // 3. DataRequestMsg for firmware
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                (
                    "DataType".to_owned(),
                    Value::String("FirmwareUpdaterData".into()),
                ),
                (
                    "Arguments".to_owned(),
                    Value::Dictionary(Dictionary::from_iter([
                        ("UpdaterName".to_owned(), Value::String("Rose".into())),
                        ("DataChunkSize".to_owned(), Value::Integer(2.into())),
                    ])),
                ),
            ]),
        )
        .await;

        // Consume header and chunks
        let header = read(&mut peer).await;
        assert_eq!(header["DataSize"].as_unsigned_integer(), Some(5));
        let mut data = Vec::new();
        loop {
            let chunk = read(&mut peer).await;
            data.extend_from_slice(chunk["FirmwareResponseData"].as_data().unwrap());
            if chunk.get("DataDone").and_then(Value::as_boolean) == Some(true) {
                break;
            }
        }
        assert_eq!(data, b"12345");

        // 4. Final ProgressMsg
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("ProgressMsg".into())),
                (
                    "Operation".to_owned(),
                    Value::String("Verifying filesystem".into()),
                ),
                ("Progress".to_owned(), Value::Integer(95.into())),
            ]),
        )
        .await;

        // 5. Terminal StatusMsg (status 0)
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("StatusMsg".into())),
                ("Status".to_owned(), Value::Integer(0.into())),
                (
                    "Log".to_owned(),
                    Value::String("Restore finished successfully".into()),
                ),
            ]),
        )
        .await;
    });

    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(stream, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();

    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1, 2, 3]), Some(vec![4, 5, 6]))
        .unwrap();

    let options =
        Dictionary::from_iter([("RestoreBehavior".to_owned(), Value::String("Update".into()))]);
    let report = run_restored_update_loop(
        &mut client,
        sm,
        options,
        |name, _req| {
            assert_eq!(name, "Rose");
            Ok((
                Dictionary::from_iter([("Ticket".to_owned(), Value::Data(vec![0xAA]))]),
                b"12345".to_vec(),
            ))
        },
        &cancel,
    )
    .await
    .unwrap();

    assert_eq!(report.final_state, RestoreState::Completed);
    assert_eq!(report.progress_percent, Some(95));
    assert!(report
        .checkpoints_reached
        .contains(&"CHECKPOINT_FLASH_START".to_string()));
    assert_eq!(report.total_bytes_transferred, 5);
    remote.await.unwrap();
}

#[tokio::test]
async fn restored_update_loop_handles_device_failure_status() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let _start = read(&mut peer).await;

        // Terminal StatusMsg with non-zero failure code
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("StatusMsg".into())),
                ("Status".to_owned(), Value::Integer(14.into())),
                ("Log".to_owned(), Value::String("NAND write error".into())),
            ]),
        )
        .await;
    });

    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(stream, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();

    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1, 2, 3]), None).unwrap();

    let options =
        Dictionary::from_iter([("RestoreBehavior".to_owned(), Value::String("Update".into()))]);
    let err = run_restored_update_loop(
        &mut client,
        sm,
        options,
        |_name, _req| Ok((Dictionary::new(), Vec::new())),
        &cancel,
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("StatusCode") || msg.contains("14") || msg.contains("failed"));
    remote.await.unwrap();
}

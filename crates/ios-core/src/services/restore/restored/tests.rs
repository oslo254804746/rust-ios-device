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

#[tokio::test]
async fn system_image_dispatch_uses_data_port_and_status_zero_is_not_update_completion() {
    use tokio::io::AsyncWriteExt;
    let (control, mut peer) = tokio::io::duplex(8192);
    let (data, mut asr_peer) = tokio::io::duplex(8192);
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let remote = tokio::spawn(async move {
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
        done_rx.await.unwrap();
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType", Value::String("StatusMsg".into())),
                ("Status", Value::Integer(0u64.into())),
            ]),
        )
        .await;
    });
    let asr = tokio::spawn(async move {
        asr_peer.write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Initiate</string></dict></plist>\n").await.unwrap();
        let mut offer = Vec::new();
        while !offer.ends_with(b"</plist>\n") {
            offer.push(asr_peer.read_u8().await.unwrap());
        }
        let offer: Dictionary = plist::from_bytes(&offer).unwrap();
        assert_eq!(
            offer["Payload"].as_dictionary().unwrap()["Size"].as_unsigned_integer(),
            Some(3)
        );
        asr_peer.write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Payload</string></dict></plist>\n").await.unwrap();
        let mut received = [0; 3];
        asr_peer.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"OS!");
        assert_eq!(asr_peer.read(&mut [0; 1]).await.unwrap(), 0);
        done_tx.send(()).unwrap();
    });
    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(control, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();
    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1]), None).unwrap();
    let mut image = std::io::Cursor::new(b"OS!".to_vec());
    let mut data = Some(data);
    let mut connector = |port| {
        assert_eq!(port, 4321);
        std::future::ready(Ok(data.take().unwrap()))
    };
    let error = run_restored_update_loop_with_system_image(
        &mut client,
        sm,
        Dictionary::from_iter([("RestoreBehavior", Value::String("Update".into()))]),
        |_, _| panic!("firmware was not requested"),
        &cancel,
        &mut image,
        &mut connector,
    )
    .await
    .unwrap_err();
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    let report = loop {
        let current = source.expect("partial execution report");
        if let Some(execution) =
            current.downcast_ref::<super::super::state_machine::RestoreExecutionError>()
        {
            break &execution.report;
        }
        source = current.source();
    };
    assert_eq!(report.total_bytes_transferred, 3);
    assert_eq!(report.data_requests_served, vec!["SystemImageData"]);
    assert_ne!(report.final_state, RestoreState::Completed);
    assert!(client.terminal);
    asr.await.unwrap();
    remote.await.unwrap();
}

#[tokio::test]
async fn system_image_dispatch_cancel_drops_data_port_and_preserves_cancel_cause() {
    let (control, mut peer) = tokio::io::duplex(8192);
    let (data, mut asr_peer) = tokio::io::duplex(8192);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let _ = read(&mut peer).await;
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType", Value::String("DataRequestMsg".into())),
                ("DataType", Value::String("SystemImageData".into())),
            ]),
        )
        .await;
    });
    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(control, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();
    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1]), None).unwrap();
    let mut image = std::io::Cursor::new(b"OS!".to_vec());
    let mut data = Some(data);
    let trigger = cancel.clone();
    let mut connector = |port| {
        assert_eq!(port, super::super::asr::DEFAULT_PORT);
        let data = data.take().unwrap();
        let trigger = trigger.clone();
        async move {
            trigger.cancel();
            Ok(data)
        }
    };
    let error = run_restored_update_loop_with_system_image(
        &mut client,
        sm,
        Dictionary::from_iter([("RestoreBehavior", Value::String("Update".into()))]),
        |_, _| panic!("firmware was not requested"),
        &cancel,
        &mut image,
        &mut connector,
    )
    .await
    .unwrap_err();
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Cancelled);
    assert!(has_owned_cancel(&error));
    assert!(client.terminal);
    assert_eq!(asr_peer.read(&mut [0; 1]).await.unwrap(), 0);
    remote.await.unwrap();
}

#[tokio::test]
async fn system_image_async_request_is_not_silently_served_synchronously() {
    let (control, mut peer) = tokio::io::duplex(8192);
    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let _ = read(&mut peer).await;
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType", Value::String("AsyncDataRequestMsg".into())),
                ("DataType", Value::String("SystemImageData".into())),
                ("DataPort", Value::Integer(4321u64.into())),
            ]),
        )
        .await;
    });
    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(control, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();
    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1]), None).unwrap();
    let mut image = std::io::Cursor::new(b"OS!".to_vec());
    let mut connector = |_port| {
        std::future::ready(Err::<tokio::io::DuplexStream, _>(RestoreError::Protocol(
            "unexpected connection".into(),
        )))
    };
    let error = run_restored_update_loop_with_system_image(
        &mut client,
        sm,
        Dictionary::from_iter([("RestoreBehavior", Value::String("Update".into()))]),
        |_, _| panic!("firmware was not requested"),
        &cancel,
        &mut image,
        &mut connector,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, RestoreError::Io(ref io) if io.kind() == std::io::ErrorKind::Unsupported)
    );
    assert!(client.terminal);
    remote.await.unwrap();
}

fn partial_report(error: &RestoreError) -> &RestoreExecutionReport {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(execution) =
            current.downcast_ref::<super::super::state_machine::RestoreExecutionError>()
        {
            return &execution.report;
        }
        source = current.source();
    }
    panic!("original partial report is missing")
}

fn has_owned_cancel(error: &RestoreError) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    for _ in 0..32 {
        let Some(current) = source else { break };
        if current.is::<crate::OperationCancelled>() {
            return true;
        }
        source = if let Some(io) = current.downcast_ref::<std::io::Error>() {
            // std::io::Error::source can forward past the owned custom payload.
            io.get_ref()
                .map(|error| error as &(dyn std::error::Error + 'static))
        } else {
            current.source()
        };
    }
    false
}

#[tokio::test]
async fn independent_loop_cancel_interrupts_start_and_firmware_send() {
    for during_firmware in [false, true] {
        let (control, mut peer) = tokio::io::duplex(1);
        let loop_cancel = OperationCancel::new();
        let trigger = loop_cancel.clone();
        let remote = tokio::spawn(async move {
            handshake(&mut peer, 42).await;
            if during_firmware {
                assert_eq!(
                    read(&mut peer).await["Request"].as_string(),
                    Some("StartRestore")
                );
                write(
                    &mut peer,
                    Dictionary::from_iter([
                        ("MsgType", Value::String("DataRequestMsg".into())),
                        ("DataType", Value::String("FirmwareUpdaterData".into())),
                        (
                            "Arguments",
                            Value::Dictionary(Dictionary::from_iter([(
                                "UpdaterName",
                                Value::String("Rose".into()),
                            )])),
                        ),
                    ]),
                )
                .await;
            }
            let _first_submitted_byte = peer.read_u8().await.unwrap();
            trigger.cancel();
            let mut remainder = Vec::new();
            peer.read_to_end(&mut remainder).await.unwrap();
        });
        let mut client =
            RestoredClient::connect(control, 42, Duration::from_secs(5), OperationCancel::new())
                .await
                .unwrap();
        let mut state = RestoreStateMachine::new();
        state.receive_nonces(Some(vec![1]), None).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            run_restored_update_loop(
                &mut client,
                state,
                Dictionary::from_iter([("RestoreBehavior", Value::String("Update".into()))]),
                |_, _| Ok((Dictionary::new(), vec![7; 1024])),
                &loop_cancel,
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(has_owned_cancel(&result));
        assert!(matches!(
            partial_report(&result).final_state,
            RestoreState::Failed(super::super::state_machine::RestoreFailure::Cancelled)
        ));
        assert!(client.terminal);
        assert!(client.next_message().await.is_err());
        drop(client);
        tokio::time::timeout(Duration::from_secs(2), remote)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn rejected_requests_retain_report_and_cannot_consume_late_success() {
    for scenario in 0..5 {
        let (control, mut peer) = tokio::io::duplex(8192);
        let remote = tokio::spawn(async move {
            handshake(&mut peer, 42).await;
            let _ = read(&mut peer).await;
            let mut request = Dictionary::from_iter([
                ("MsgType", Value::String("DataRequestMsg".into())),
                ("DataType", Value::String("FirmwareUpdaterData".into())),
            ]);
            if scenario == 0 {
                request.insert("DataType".into(), Value::String("SystemImageData".into()));
                request.insert("DataPort".into(), Value::Integer(0u64.into()));
            } else if scenario == 4 {
                request.insert("MsgType".into(), Value::String("UnknownMessage".into()));
            } else if scenario != 1 {
                request.insert(
                    "Arguments".into(),
                    Value::Dictionary(Dictionary::from_iter([(
                        "UpdaterName",
                        Value::String("Rose".into()),
                    )])),
                );
            }
            write(&mut peer, request).await;
            write(
                &mut peer,
                Dictionary::from_iter([
                    ("MsgType", Value::String("StatusMsg".into())),
                    ("Status", Value::Integer(0u64.into())),
                ]),
            )
            .await;
            let mut remainder = Vec::new();
            peer.read_to_end(&mut remainder).await.unwrap();
            assert!(remainder.is_empty());
        });
        let cancel = OperationCancel::new();
        let mut client =
            RestoredClient::connect(control, 42, Duration::from_secs(5), cancel.clone())
                .await
                .unwrap();
        let mut state = RestoreStateMachine::new();
        state.receive_nonces(Some(vec![1]), None).unwrap();
        let mut image = std::io::Cursor::new(vec![1]);
        let mut connector = |_| {
            std::future::ready(Err::<tokio::io::DuplexStream, _>(RestoreError::Protocol(
                "must not connect".into(),
            )))
        };
        let error = run_restored_update_loop_with_system_image(
            &mut client,
            state,
            Dictionary::from_iter([("RestoreBehavior", Value::String("Update".into()))]),
            |_, _| {
                if scenario == 2 {
                    Err(RestoreError::Io(std::io::Error::from(
                        std::io::ErrorKind::PermissionDenied,
                    )))
                } else {
                    Ok((Dictionary::new(), Vec::new()))
                }
            },
            &cancel,
            &mut image,
            &mut connector,
        )
        .await
        .unwrap_err();
        assert_ne!(partial_report(&error).final_state, RestoreState::Completed);
        if scenario == 2 {
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::PermissionDenied
            );
        }
        assert!(client.terminal);
        assert!(client.next_message().await.is_err());
        drop(client);
        tokio::time::timeout(Duration::from_secs(2), remote)
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn full_restored_protocol_session_dispatches_all_upstream_data_requests_and_asr() {
    use tokio::io::AsyncWriteExt;

    let comp = |path: &str, fw: bool, fud: bool, ean: bool| {
        let mut info = Dictionary::from_iter([("Path".to_owned(), Value::String(path.into()))]);
        if fw {
            info.insert("IsFirmwarePayload".into(), Value::Boolean(true));
        }
        if fud {
            info.insert("IsFUDFirmware".into(), Value::Boolean(true));
        }
        if ean {
            info.insert("IsEarlyAccessFirmware".into(), Value::Boolean(true));
        }
        Value::Dictionary(Dictionary::from_iter([(
            "Info".to_owned(),
            Value::Dictionary(info),
        )]))
    };

    let identity = Dictionary::from_iter([
        ("ApBoardID".to_owned(), Value::Integer(0x0c.into())),
        ("ApChipID".to_owned(), Value::Integer(0x8110.into())),
        (
            "Manifest".to_owned(),
            Value::Dictionary(Dictionary::from_iter([
                (
                    "LLB".to_owned(),
                    comp("Firmware/all_flash/LLB.im4p", true, false, false),
                ),
                (
                    "iBoot".to_owned(),
                    comp("Firmware/all_flash/iBoot.im4p", true, false, false),
                ),
                (
                    "DeviceTree".to_owned(),
                    comp("Firmware/all_flash/DeviceTree.im4p", true, false, false),
                ),
                (
                    "RestoreSEP".to_owned(),
                    comp("Firmware/all_flash/RestoreSEP.im4p", false, false, false),
                ),
                (
                    "SEP".to_owned(),
                    comp("Firmware/all_flash/SEP.im4p", false, false, false),
                ),
                (
                    "KernelCache".to_owned(),
                    comp("kernelcache.release", false, false, false),
                ),
                (
                    "SystemVolume".to_owned(),
                    comp("090-root.dmg.trustcache", false, false, false),
                ),
                (
                    "Ap,SystemVolumeCanonicalMetadata".to_owned(),
                    comp("090-root.mtree", false, false, false),
                ),
                (
                    "AOP".to_owned(),
                    comp("Firmware/AOP/aop.im4p", false, true, true),
                ),
                (
                    "StaticTrustCache".to_owned(),
                    comp("Firmware/trustcache", false, false, false),
                ),
                (
                    "RBM".to_owned(),
                    comp("Firmware/rbm.im4p", false, false, false),
                ),
            ])),
        ),
    ]);

    let (control, mut peer) = tokio::io::duplex(65536);
    let (asr_stream, mut asr_peer) = tokio::io::duplex(16384);
    let (asr_done_tx, asr_done_rx) = tokio::sync::oneshot::channel();

    let remote = tokio::spawn(async move {
        handshake(&mut peer, 42).await;
        let start = read(&mut peer).await;
        assert_eq!(start["Request"].as_string(), Some("StartRestore"));

        // 1. BuildIdentityDict
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                (
                    "DataType".to_owned(),
                    Value::String("BuildIdentityDict".into()),
                ),
            ]),
        )
        .await;
        let reply = read(&mut peer).await;
        assert!(reply.contains_key("BuildIdentityDict"));

        // 2. RootTicket
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                ("DataType".to_owned(), Value::String("RootTicket".into())),
            ]),
        )
        .await;
        let reply = read(&mut peer).await;
        assert_eq!(
            reply["RootTicketData"].as_data(),
            Some(&b"AP_TICKET_DATA"[..])
        );

        // 3. NORData (array mode: iBoot must be first in NorImageData)
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                ("DataType".to_owned(), Value::String("NORData".into())),
            ]),
        )
        .await;
        let reply = read(&mut peer).await;
        let llb_parsed =
            super::super::img4::parse_img4_envelope(reply["LlbImageData"].as_data().unwrap())
                .unwrap();
        assert_eq!(&llb_parsed.fourcc, b"illb");
        let nor_arr = reply["NorImageData"].as_array().unwrap();
        assert_eq!(nor_arr.len(), 2);
        let first_nor =
            super::super::img4::parse_img4_envelope(nor_arr[0].as_data().unwrap()).unwrap();
        assert_eq!(&first_nor.fourcc, b"ibot");
        assert!(reply.contains_key("RestoreSEPImageData"));
        assert!(reply.contains_key("SEPImageData"));

        // 4. FDRTrustData
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                ("DataType".to_owned(), Value::String("FDRTrustData".into())),
            ]),
        )
        .await;
        let reply = read(&mut peer).await;
        assert!(reply.is_empty());

        // 5. KernelCache & DeviceTree
        for (dt, key, fourcc) in [
            ("KernelCache", "KernelCacheFile", *b"krnl"),
            ("DeviceTree", "DeviceTreeFile", *b"dtre"),
            ("SystemImageRootHash", "SystemImageRootHashFile", *b"isys"),
            (
                "SystemImageCanonicalMetadata",
                "SystemImageCanonicalMetadataFile",
                *b"msys",
            ),
            ("ReceiptManifest", "ReceiptManifest", *b"rmbt"),
        ] {
            write(
                &mut peer,
                Dictionary::from_iter([
                    ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                    ("DataType".to_owned(), Value::String(dt.into())),
                ]),
            )
            .await;
            let reply = read(&mut peer).await;
            let env =
                super::super::img4::parse_img4_envelope(reply[key].as_data().unwrap()).unwrap();
            assert_eq!(env.fourcc, fourcc);
        }

        // 6. SourceBootObjectV4 (chunked FileData + FileDataDone)
        write(
            &mut peer,
            Dictionary::from_iter([
                (
                    "MsgType".to_owned(),
                    Value::String("AsyncDataRequestMsg".into()),
                ),
                (
                    "DataType".to_owned(),
                    Value::String("SourceBootObjectV4".into()),
                ),
                (
                    "Arguments".to_owned(),
                    Value::Dictionary(Dictionary::from_iter([(
                        "ImageName".to_owned(),
                        Value::String("StaticTrustCache".into()),
                    )])),
                ),
            ]),
        )
        .await;
        let chunk_msg = read(&mut peer).await;
        let env = super::super::img4::parse_img4_envelope(chunk_msg["FileData"].as_data().unwrap())
            .unwrap();
        assert_eq!(&env.fourcc, b"trst");
        let done_msg = read(&mut peer).await;
        assert_eq!(done_msg["FileDataDone"].as_boolean(), Some(true));

        // 7. FUDData, PersonalizedData, EANData, HostSystemTime, RestoreAttestation, BBUpdateStatusMsg
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                ("DataType".to_owned(), Value::String("FUDData".into())),
                (
                    "Arguments".to_owned(),
                    Value::Dictionary(Dictionary::from_iter([(
                        "ImageList".to_owned(),
                        Value::Boolean(true),
                    )])),
                ),
            ]),
        )
        .await;
        let fud_list = read(&mut peer).await;
        assert_eq!(fud_list["FUDImageList"].as_array().unwrap().len(), 1);

        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("DataRequestMsg".into())),
                (
                    "DataType".to_owned(),
                    Value::String("HostSystemTime".into()),
                ),
            ]),
        )
        .await;
        let time_reply = read(&mut peer).await;
        assert!(time_reply.contains_key("SetHostTimeOnDevice"));

        write(
            &mut peer,
            Dictionary::from_iter([(
                "MsgType".to_owned(),
                Value::String("RestoreAttestation".into()),
            )]),
        )
        .await;
        let attest_reply = read(&mut peer).await;
        assert_eq!(
            attest_reply["RestoreShouldAttest"].as_boolean(),
            Some(false)
        );

        write(
            &mut peer,
            Dictionary::from_iter([
                (
                    "MsgType".to_owned(),
                    Value::String("BBUpdateStatusMsg".into()),
                ),
                ("Accepted".to_owned(), Value::Boolean(true)),
            ]),
        )
        .await;

        // 8. SystemImageData over ASR
        write(
            &mut peer,
            Dictionary::from_iter([
                (
                    "MsgType".to_owned(),
                    Value::String("AsyncDataRequestMsg".into()),
                ),
                (
                    "DataType".to_owned(),
                    Value::String("SystemImageData".into()),
                ),
                ("DataPort".to_owned(), Value::Integer(12345.into())),
            ]),
        )
        .await;
        asr_done_rx.await.unwrap();

        // 9. Final StatusMsg(0) and expect ReceivedFinalStatusMsg ack
        write(
            &mut peer,
            Dictionary::from_iter([
                ("MsgType".to_owned(), Value::String("StatusMsg".into())),
                ("Status".to_owned(), Value::Integer(0.into())),
            ]),
        )
        .await;
        let final_ack = read(&mut peer).await;
        assert_eq!(
            final_ack["MsgType"].as_string(),
            Some("ReceivedFinalStatusMsg")
        );
    });

    let asr_task = tokio::spawn(async move {
        asr_peer
            .write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Initiate</string></dict></plist>\n")
            .await
            .unwrap();
        let mut offer = Vec::new();
        while !offer.ends_with(b"</plist>\n") {
            offer.push(asr_peer.read_u8().await.unwrap());
        }
        asr_peer
            .write_all(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Command</key><string>Payload</string></dict></plist>\n")
            .await
            .unwrap();
        let mut received = [0u8; 6];
        asr_peer.read_exact(&mut received).await.unwrap();
        assert_eq!(&received, b"ROOTFS");
        asr_done_tx.send(()).unwrap();
    });

    let cancel = OperationCancel::new();
    let mut client = RestoredClient::connect(control, 42, Duration::from_secs(5), cancel.clone())
        .await
        .unwrap();
    let mut sm = RestoreStateMachine::new();
    sm.receive_nonces(Some(vec![1, 2, 3, 4]), None).unwrap();

    let assets = RestoredProtocolAssets::new(
        identity,
        b"AP_TICKET_DATA".to_vec(),
        Dictionary::new(),
        Some(vec![1, 2, 3, 4]),
    );
    let mut image = std::io::Cursor::new(b"ROOTFS".to_vec());
    let mut asr_stream_opt = Some(asr_stream);

    let report = run_restored_protocol_session(
        &mut client,
        sm,
        Dictionary::from_iter([("RestoreBehavior".to_owned(), Value::String("Update".into()))]),
        &assets,
        |comp_name| Ok(format!("PAYLOAD_{comp_name}").into_bytes()),
        |updater, _req| {
            Ok((
                Dictionary::from_iter([("Updater".to_owned(), Value::String(updater.to_owned()))]),
                b"FW_BYTES".to_vec(),
            ))
        },
        &mut image,
        |port| {
            assert_eq!(port, 12345);
            std::future::ready(Ok(asr_stream_opt.take().unwrap()))
        },
        &cancel,
    )
    .await
    .unwrap();

    assert_eq!(report.final_state, RestoreState::Completed);
    assert!(report
        .data_requests_served
        .contains(&"BuildIdentityDict".to_string()));
    assert!(report.data_requests_served.contains(&"NORData".to_string()));
    assert!(report
        .data_requests_served
        .contains(&"KernelCache".to_string()));
    assert!(report
        .data_requests_served
        .contains(&"SystemImageData".to_string()));
    assert!(report.total_bytes_transferred > 0);

    asr_task.await.unwrap();
    remote.await.unwrap();
}

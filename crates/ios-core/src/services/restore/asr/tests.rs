use super::*;
use std::io::Cursor;

fn xml(dict: Dictionary) -> Vec<u8> {
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &dict).unwrap();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes
}
fn command(name: &str) -> Dictionary {
    Dictionary::from_iter([("Command", Value::String(name.into()))])
}

#[tokio::test]
async fn real_unframed_validation_oob_and_chunk_checksums_use_the_os_image() {
    let image: Vec<u8> = (0..CHUNK + 3).map(|i| (i % 251) as u8).collect();
    let expected = image.clone();
    let (stream, mut peer) = tokio::io::duplex(CHUNK * 2);
    let server = tokio::spawn(async move {
        let mut initiate = command("Initiate");
        initiate.insert("Checksum Chunks".into(), true.into());
        peer.write_all(&xml(initiate)).await.unwrap();
        let offer = read_command(&mut peer).await.unwrap();
        assert_eq!(offer["Version"].as_unsigned_integer(), Some(1));
        assert_eq!(
            offer["Packet Payload Size"].as_unsigned_integer(),
            Some(1450)
        );
        assert_eq!(
            offer["Checksum Chunk Size"].as_unsigned_integer(),
            Some(CHUNK as u64)
        );
        assert_eq!(
            offer["Payload"].as_dictionary().unwrap()["Size"].as_unsigned_integer(),
            Some(expected.len() as u64)
        );
        let mut oob = command("OOBData");
        oob.insert(
            "OOB Offset".into(),
            Value::Integer((CHUNK as u64 - 2).into()),
        );
        oob.insert("OOB Length".into(), Value::Integer(5u64.into()));
        peer.write_all(&xml(oob)).await.unwrap();
        let mut validation = [0; 5];
        peer.read_exact(&mut validation).await.unwrap();
        assert_eq!(validation, expected[CHUNK - 2..]);
        peer.write_all(&xml(command("Payload"))).await.unwrap();
        for part in expected.chunks(CHUNK) {
            let mut payload = vec![0; part.len()];
            peer.read_exact(&mut payload).await.unwrap();
            assert_eq!(payload, part);
            let mut checksum = [0; 20];
            peer.read_exact(&mut checksum).await.unwrap();
            assert_eq!(checksum.as_slice(), Sha1::digest(part).as_slice());
        }
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    });
    let receipt = send_image(
        stream,
        &mut Cursor::new(image),
        Duration::from_secs(2),
        OperationCancel::new(),
    )
    .await
    .unwrap();
    assert_eq!(receipt.image_bytes_sent, CHUNK as u64 + 3);
    assert_eq!(receipt.validation_bytes_sent, 5);
    assert!(receipt.checksum_chunks);
    server.await.unwrap();
}

#[tokio::test]
async fn invalid_oob_range_fails_without_sending_substitute_data() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let server = tokio::spawn(async move {
        peer.write_all(&xml(command("Initiate"))).await.unwrap();
        read_command(&mut peer).await.unwrap();
        let mut oob = command("OOBData");
        oob.insert("OOB Offset".into(), Value::Integer(u64::MAX.into()));
        oob.insert("OOB Length".into(), Value::Integer(2u64.into()));
        peer.write_all(&xml(oob)).await.unwrap();
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    });
    let error = send_image(
        stream,
        &mut Cursor::new(vec![1, 2, 3]),
        Duration::from_secs(1),
        OperationCancel::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, RestoreError::Protocol(_)));
    server.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn partial_command_timeout_preserves_elapsed_and_closes_owned_port() {
    let (stream, mut peer) = tokio::io::duplex(1024);
    peer.write_all(b"<?xml ").await.unwrap();
    let started = Instant::now();
    let error = send_image(
        stream,
        &mut Cursor::new(vec![1]),
        Duration::from_millis(100),
        OperationCancel::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_millis(100));
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    let mut elapsed = false;
    while let Some(error) = source {
        elapsed |= error.is::<tokio::time::error::Elapsed>();
        source = error.source();
    }
    assert!(elapsed);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn cancellation_before_initiate_closes_port_and_retains_cancel_cause() {
    let (stream, mut peer) = tokio::io::duplex(1);
    let cancel = OperationCancel::new();
    cancel.cancel();
    let error = send_image(
        stream,
        &mut Cursor::new(vec![1]),
        Duration::from_secs(1),
        cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Cancelled);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

use super::*;
use crate::xpc::message::{decode_message, encode_message, flags};
use bytes::Bytes;
use plist::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn response(arguments: IndexMap<String, XpcValue>) -> XpcMessage {
    XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id: 1,
        body: Some(XpcValue::Dictionary(arguments)),
    }
}

pub(super) fn fixture_identity() -> plist::Dictionary {
    let mut identity = plist::Dictionary::from_iter([
        ("Cryptex1,ChipID".to_string(), Value::String("0x0".into())),
        ("Cryptex1,Type".to_string(), Value::Integer(1.into())),
        ("Cryptex1,SubType".to_string(), Value::Integer(2.into())),
        (
            "Cryptex1,ProductClass".to_string(),
            Value::String("0x3".into()),
        ),
        ("Cryptex1,UseProductClass".to_string(), Value::Boolean(true)),
        ("Cryptex1,NonceDomain".to_string(), Value::Integer(4.into())),
        (
            "Cryptex1,Version".to_string(),
            Value::String("39.1.0".into()),
        ),
        (
            "Cryptex1,PreauthorizationVersion".to_string(),
            Value::String("39.1.0".into()),
        ),
        (
            "Info".to_string(),
            Value::Dictionary(plist::Dictionary::from_iter([(
                "Variant".to_string(),
                Value::String("iOS Customer Developer Disk Image Cryptex".into()),
            )])),
        ),
    ]);
    let mut manifest = plist::Dictionary::new();
    for (index, name) in [
        "GenericDmg",
        "GenericTrustCache",
        "CryptexInfoPlist",
        "GenericVolume",
    ]
    .iter()
    .enumerate()
    {
        manifest.insert(
            format!("Cryptex1,{name}"),
            Value::Dictionary(plist::Dictionary::from_iter([
                ("Digest".to_string(), Value::Data(vec![index as u8; 48])),
                (
                    "Info".to_string(),
                    Value::Dictionary(plist::Dictionary::from_iter([
                        ("Personalize".to_string(), Value::Boolean(index != 0)),
                        (
                            "Path".to_string(),
                            Value::String(format!("Firmware/{name}")),
                        ),
                    ])),
                ),
            ])),
        );
    }
    identity.insert("Manifest".to_string(), Value::Dictionary(manifest));
    identity
}

fn chip() -> plist::Dictionary {
    plist::Dictionary::from_iter([
        ("img4_chip_chip".to_string(), Value::Integer(0x8030.into())),
        (
            "img4_chip_ecid".to_string(),
            Value::Integer(0x0102030405060708u64.into()),
        ),
        ("img4_chip_cpro".to_string(), Value::Boolean(true)),
    ])
}

#[test]
fn signing_uses_cryptex_identity_and_only_personalized_digests() {
    let request = build_cryptex_tss_request(&fixture_identity(), &chip(), &[7; 48]).unwrap();
    assert_eq!(
        request["Cryptex1,UDID"].as_data().unwrap(),
        &hex::decode("00000000000080300102030405060708").unwrap()
    );
    assert_eq!(request["Cryptex1,Nonce"].as_data().unwrap(), &[7; 48]);
    assert_eq!(
        request["Cryptex1,ProductClass"].as_unsigned_integer(),
        Some(3)
    );
    assert_eq!(request["@Cryptex1,Ticket"].as_boolean(), Some(true));
    assert_eq!(
        request["@VersionInfo"].as_string(),
        Some("libauthinstall-1104.0.9")
    );
    assert!(!request.contains_key("Cryptex1,GenericDmg"));
    assert!(!request
        .keys()
        .any(|key| key.starts_with("Ap") || key == "@ApImg4Ticket"));
    let component = request["Cryptex1,GenericTrustCache"]
        .as_dictionary()
        .unwrap();
    assert_eq!(component.len(), 1);
    assert_eq!(component["Digest"].as_data().unwrap(), &[1; 48]);
}

#[test]
fn signing_refuses_missing_device_or_nonce_inputs() {
    for key in ["img4_chip_ecid", "img4_chip_chip", "img4_chip_cpro"] {
        let mut identifiers = chip();
        identifiers.remove(key);
        assert!(build_cryptex_tss_request(&fixture_identity(), &identifiers, &[7; 48]).is_err());
    }
    assert!(build_cryptex_tss_request(&fixture_identity(), &chip(), &[]).is_err());
    assert!(build_cryptex_tss_request(&fixture_identity(), &chip(), &[7; 65]).is_err());
}

#[test]
fn nonce_uses_handle_and_validates_wrapper_lengths() {
    let nonce: Vec<u8> = (0..48).collect();
    let mut blob = vec![0, 0];
    blob.extend_from_slice(&nonce);
    blob.extend_from_slice(&[0, 0]);
    blob.extend_from_slice(&48u32.to_le_bytes());
    assert_eq!(unwrap_nonce(&blob).unwrap(), nonce);
    assert_eq!(
        NonceDomain::Handle(4).arguments(),
        IndexMap::from([("nonce-domain-handle".to_string(), XpcValue::Uint64(4))])
    );
    for length in [0u32, 51, u32::MAX] {
        blob[52..].copy_from_slice(&length.to_le_bytes());
        assert!(unwrap_nonce(&blob).is_err());
    }
    assert!(unwrap_nonce(&[0; 5]).is_err());
}

#[test]
fn install_request_preserves_signed_and_unsigned_wire_types() {
    let arguments = install_arguments(XpcValue::Dictionary(IndexMap::new()), [1, 2, 3, 4, 5]);
    assert_eq!(arguments["image-type-index"], XpcValue::Int64(10));
    for key in ["auth", "client-version", "persistence", "nonce-persistence"] {
        assert!(matches!(arguments[key], XpcValue::Uint64(_)));
    }
    for (index, key) in ["image", "trustcache", "im4m", "info", "volumehash"]
        .iter()
        .enumerate()
    {
        let (id, payload) = arguments[*key].as_file_transfer().unwrap();
        assert_eq!(id, index as u64 + 1);
        assert_eq!(payload.as_dict().unwrap()["s"], XpcValue::Uint64(id));
    }
    assert!(!arguments.contains_key("remote-cryptex-identifier"));
}

#[test]
fn daemon_failures_and_missing_status_are_not_success() {
    assert!(unwrap_response("install", response(IndexMap::new())).is_err());
    assert!(unwrap_response(
        "install",
        response(IndexMap::from([("error".to_string(), XpcValue::Int64(2))]))
    )
    .is_err());
    let cferr = XpcValue::Dictionary(IndexMap::from([(
        "cferr_userinfo".to_string(),
        XpcValue::Dictionary(IndexMap::from([(
            "NSLocalizedDescription".to_string(),
            XpcValue::String("nonce expired".into()),
        )])),
    )]));
    let error = unwrap_response(
        "install",
        response(IndexMap::from([("cferr".to_string(), cferr)])),
    )
    .unwrap_err();
    assert!(error.to_string().contains("nonce expired"));
    assert!(unwrap_response(
        "read-personalization-id",
        response(IndexMap::from([(
            "argv".to_string(),
            XpcValue::Dictionary(IndexMap::new())
        )]))
    )
    .is_ok());
}

#[test]
fn installed_list_preserves_versions_and_rejects_malformed_entries() {
    let entry = XpcValue::Dictionary(IndexMap::from([
        (
            "remote-cryptex-identifier".into(),
            XpcValue::String(DDI_IDENTIFIER.into()),
        ),
        (
            "remote-cryptex-version".into(),
            XpcValue::String("27.1.fixture".into()),
        ),
    ]));
    let arguments = IndexMap::from([("remote-cryptex-array".into(), XpcValue::Array(vec![entry]))]);
    assert_eq!(
        parse_installed(&arguments).unwrap(),
        vec![InstalledCryptex {
            identifier: DDI_IDENTIFIER.into(),
            version: "27.1.fixture".into()
        }]
    );
    let invalid = IndexMap::from([(
        "remote-cryptex-array".into(),
        XpcValue::Array(vec![XpcValue::Null]),
    )]);
    assert!(parse_installed(&invalid).is_err());
}

fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let size = payload.len() as u32;
    let mut bytes = size.to_be_bytes()[1..].to_vec();
    bytes.extend_from_slice(&[kind, flags]);
    bytes.extend_from_slice(&stream.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

async fn read_frame(stream: &mut tokio::io::DuplexStream) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    stream.read_exact(&mut header).await.unwrap();
    let size = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
    assert!(size <= 16 * 1024);
    let mut payload = vec![0; size];
    stream.read_exact(&mut payload).await.unwrap();
    (
        header[3],
        header[4],
        u32::from_be_bytes(header[5..].try_into().unwrap()),
        payload,
    )
}

async fn fake_peer(
    mut stream: tokio::io::DuplexStream,
    expected: &'static str,
    install_payloads: Option<Vec<Vec<u8>>>,
    reply: bool,
) {
    let mut preface = [0; 24];
    stream.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
    stream.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    let empty = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })
    .unwrap();
    for id in [1, 1, 3] {
        stream.write_all(&frame(0, 0, id, &empty)).await.unwrap();
    }
    let request = loop {
        let (kind, _, id, payload) = read_frame(&mut stream).await;
        if kind != 0 {
            continue;
        }
        assert!(matches!(id, 1 | 3));
        let message = decode_message(Bytes::from(payload)).unwrap();
        if let Some(XpcValue::Dictionary(body)) = message.body {
            if body.contains_key("routine") {
                assert_eq!(body["routine"].as_str(), Some(expected));
                assert_ne!(message.flags & flags::WANTING_REPLY, 0);
                break body;
            }
        }
    };
    if let Some(payloads) = install_payloads {
        let arguments = request["argv"].as_dict().unwrap();
        for (index, expected) in payloads.iter().enumerate() {
            let id = 5 + index as u32 * 2;
            let (kind, flags, stream_id, bytes) = read_frame(&mut stream).await;
            assert_eq!((kind, flags, stream_id, bytes.len()), (1, 4, id, 0));
            let (kind, _, stream_id, bytes) = read_frame(&mut stream).await;
            assert_eq!((kind, stream_id), (0, id));
            let preamble = decode_message(Bytes::from(bytes.clone())).unwrap();
            assert_eq!(preamble.msg_id, index as u64 + 1);
            assert_eq!(
                preamble.flags,
                flags::ALWAYS_SET | flags::FILE_TX_STREAM_REQUEST
            );
            for stream_id in [0, id] {
                stream
                    .write_all(&frame(8, 0, stream_id, &(bytes.len() as u32).to_be_bytes()))
                    .await
                    .unwrap();
            }
            let mut received = Vec::new();
            loop {
                let (kind, flags, stream_id, bytes) = read_frame(&mut stream).await;
                assert_eq!((kind, stream_id), (0, id));
                if flags & 1 != 0 {
                    assert!(bytes.is_empty());
                    break;
                }
                received.extend_from_slice(&bytes);
                for stream_id in [0, id] {
                    stream
                        .write_all(&frame(8, 0, stream_id, &(bytes.len() as u32).to_be_bytes()))
                        .await
                        .unwrap();
                }
            }
            assert_eq!(&received, expected);
            let key = ["image", "trustcache", "im4m", "info", "volumehash"][index];
            assert_eq!(
                arguments[key]
                    .as_file_transfer()
                    .unwrap()
                    .1
                    .as_dict()
                    .unwrap()["s"],
                XpcValue::Uint64(received.len() as u64)
            );
            let ack = encode_message(&XpcMessage {
                flags: flags::ALWAYS_SET | flags::FILE_TX_STREAM_RESPONSE,
                msg_id: index as u64 + 1,
                body: None,
            })
            .unwrap();
            stream.write_all(&frame(0, 1, id, &ack)).await.unwrap();
        }
    }
    if !reply {
        std::future::pending::<()>().await;
    }
    let reply = encode_message(&response(IndexMap::from([
        ("error".to_string(), XpcValue::Uint64(0)),
        ("argv".to_string(), XpcValue::Dictionary(IndexMap::new())),
    ])))
    .unwrap();
    stream.write_all(&frame(0, 0, 1, &reply)).await.unwrap();
}

#[tokio::test]
async fn install_streams_five_payloads_with_backpressure_and_final_ack() {
    let root = std::env::temp_dir().join(format!("ios-cryptex-test-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(root.join("Firmware"))
        .await
        .unwrap();
    let image = vec![0x55; 160 * 1024];
    let payloads = [
        image.clone(),
        b"trustcache".to_vec(),
        b"info".to_vec(),
        b"volumehash".to_vec(),
    ];
    for (name, payload) in [
        "GenericDmg",
        "GenericTrustCache",
        "CryptexInfoPlist",
        "GenericVolume",
    ]
    .iter()
    .zip(&payloads)
    {
        tokio::fs::write(root.join("Firmware").join(name), payload)
            .await
            .unwrap();
    }
    let mut manifest = Vec::new();
    plist::to_writer_xml(
        &mut manifest,
        &plist::Dictionary::from_iter([
            (
                "ProductBuildVersion".to_string(),
                Value::String("fixture".into()),
            ),
            (
                "BuildIdentities".to_string(),
                Value::Array(vec![Value::Dictionary(fixture_identity())]),
            ),
        ]),
    )
    .unwrap();
    tokio::fs::write(root.join("BuildManifest.plist"), manifest)
        .await
        .unwrap();
    let assets = CryptexDdiAssets::load(&root).await.unwrap();
    assert_eq!(assets.nonce_domain_handle().unwrap(), 4);
    let properties = assets.properties().unwrap();
    assert_eq!(
        properties.as_dict().unwrap()["Cryptex1,NonceDomain"],
        XpcValue::Uint64(4)
    );
    let (local, remote) = tokio::io::duplex(4096);
    let task = tokio::spawn(fake_peer(
        remote,
        "install",
        Some(vec![
            image,
            payloads[1].clone(),
            b"signed-ticket".to_vec(),
            payloads[2].clone(),
            payloads[3].clone(),
        ]),
        true,
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        let xpc = XpcClient::connect_stream(local).await.unwrap();
        let mut client = CryptexClient::new(xpc, vec![FEATURE_INSTALL.into()]);
        client.install(&assets, b"signed-ticket").await.unwrap();
        task.await.unwrap();
    })
    .await
    .unwrap();
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn repeated_queries_open_a_fresh_connection() {
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let reconnect_count = count.clone();
    let (local, remote) = tokio::io::duplex(4096);
    let first = tokio::spawn(fake_peer(remote, "copy-installed", None, true));
    let xpc = XpcClient::connect_stream(local)
        .await
        .unwrap()
        .with_reconnector(move || {
            reconnect_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async {
                let (local, remote) = tokio::io::duplex(4096);
                tokio::spawn(fake_peer(remote, "copy-installed", None, true));
                XpcClient::connect_stream(local).await
            }
        });
    let mut client = CryptexClient::new(xpc, vec![FEATURE_IDENTIFIERS.into()]);
    assert!(client.copy_installed().await.unwrap().is_empty());
    assert!(client.copy_installed().await.unwrap().is_empty());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(
        client.uninstall(DDI_IDENTIFIER, None).await,
        Err(CryptexError::Unsupported(FEATURE_INSTALL))
    ));
    first.await.unwrap();
}

#[tokio::test]
async fn cancelled_routine_reconnects_on_next_call() {
    let reconnects = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let next = reconnects.clone();
    let (local, remote) = tokio::io::duplex(4096);
    let stuck = tokio::spawn(fake_peer(remote, "copy-installed", None, false));
    let xpc = XpcClient::connect_stream(local)
        .await
        .unwrap()
        .with_reconnector(move || {
            next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async {
                let (local, remote) = tokio::io::duplex(4096);
                tokio::spawn(fake_peer(remote, "copy-installed", None, true));
                XpcClient::connect_stream(local).await
            }
        });
    let mut client = CryptexClient::new(xpc, Vec::new());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), client.copy_installed())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(2), client.copy_installed())
            .await
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert_eq!(reconnects.load(std::sync::atomic::Ordering::SeqCst), 1);
    stuck.abort();
}

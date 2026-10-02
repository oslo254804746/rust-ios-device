use super::*;
use crate::{classify_error, test_util::MockStream, ErrorClass, PairRecord};
use openssl::{error::ErrorStack, x509::store::X509StoreBuilder};
use std::{error::Error, io, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn source(error: &LockdownError, kind: io::ErrorKind, class: ErrorClass) -> &io::Error {
    let LockdownError::Io(io) = error else {
        panic!("native source flattened")
    };
    assert_eq!(io.kind(), kind);
    assert_eq!(classify_error(error), class);
    assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
    io
}

fn stack(error: &LockdownError, kind: io::ErrorKind, class: ErrorClass) -> &ErrorStack {
    let stack = source(error, kind, class)
        .source()
        .unwrap()
        .downcast_ref::<ErrorStack>()
        .unwrap();
    assert!(!stack.errors().is_empty());
    stack
}

fn identity() -> (Vec<u8>, X509) {
    // Ephemeral synthetic organization identity; no real credential store.
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let cert = build_root_cert(&key).unwrap();
    let bundle = Pkcs12::builder()
        .name("owned-organization")
        .pkey(&key)
        .cert(&cert)
        .build2("owned-password")
        .unwrap()
        .to_der()
        .unwrap();
    (bundle, cert)
}

#[tokio::test]
async fn actual_p12_parse_and_password_errors_keep_stack_without_writes() {
    let (valid, _) = identity();
    for (bytes, password) in [
        (b"owned-private invalid DER".as_slice(), "owned-password"),
        (valid.as_slice(), "owned-private wrong-password"),
    ] {
        let mut stream = MockStream::eof();
        let error = pair_supervised(&mut stream, bytes, password, "owned-buid")
            .await
            .unwrap_err();
        stack(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput,
        );
        assert!(stream.written.is_empty());
    }
}

#[tokio::test]
async fn actual_nul_password_returns_native_error_instead_of_backend_panic() {
    let (valid, _) = identity();
    let mut stream = MockStream::eof();
    let error = pair_supervised(&mut stream, &valid, "owned-private\0password", "owned-buid")
        .await
        .unwrap_err();
    let native = source(
        &error,
        io::ErrorKind::InvalidInput,
        ErrorClass::InvalidInput,
    )
    .source()
    .unwrap()
    .downcast_ref::<std::ffi::NulError>()
    .unwrap();
    assert_eq!(native.nul_position(), 13);
    assert!(stream.written.is_empty());
}

#[tokio::test]
async fn actual_received_device_public_key_failure_keeps_stack_and_request() {
    let (bundle, _) = identity();
    let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "Value",
        plist::Value::Data(b"owned-private invalid public key".to_vec()),
    )]));
    let mut stream = MockStream::with_response(response);
    let error = pair_supervised(&mut stream, &bundle, "owned-password", "owned-buid")
        .await
        .unwrap_err();
    stack(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol);
    let request: plist::Value = plist::from_bytes(&stream.written[4..]).unwrap();
    let dict = request.as_dictionary().unwrap();
    assert_eq!(dict["Request"].as_string(), Some("GetValue"));
    assert_eq!(dict["Key"].as_string(), Some("DevicePublicKey"));
    assert_eq!(dict["Label"].as_string(), Some("ios-rs"));
    assert!(!dict.contains_key("Domain"));
}

#[tokio::test]
async fn actual_pkcs7_backend_failure_keeps_stack_after_first_pair_request() {
    let key = PKey::generate_ed25519().unwrap();
    let name = X509NameBuilder::new().unwrap().build();
    let mut cert = X509Builder::new().unwrap();
    cert.set_version(2).unwrap();
    let serial = BigNum::from_u32(0).unwrap().to_asn1_integer().unwrap();
    cert.set_serial_number(&serial).unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(&name).unwrap();
    cert.set_pubkey(&key).unwrap();
    cert.set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    cert.sign(&key, MessageDigest::null()).unwrap();
    let cert = cert.build();
    let bundle = Pkcs12::builder()
        .name("owned-organization")
        .pkey(&key)
        .cert(&cert)
        .build2("owned-password")
        .unwrap()
        .to_der()
        .unwrap();
    let device_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut stream = MockStream::with_responses(vec![
        plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Value",
            plist::Value::Data(device_key.public_key_to_pem().unwrap()),
        )])),
        plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Error", "MCChallengeRequired".into()),
            (
                "ExtendedResponse",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "PairingChallenge",
                    plist::Value::Data(b"owned-challenge".to_vec()),
                )])),
            ),
        ])),
    ]);
    let error = crate::pair_supervised(&mut stream, &bundle, "owned-password", "owned-buid")
        .await
        .unwrap_err();
    stack(&error, io::ErrorKind::Other, ErrorClass::Unknown);
    let mut sent = MockStream::new(stream.written);
    let get: plist::Value = recv_lockdown(&mut sent).await.unwrap();
    assert_eq!(
        get.as_dictionary().unwrap()["Request"].as_string(),
        Some("GetValue")
    );
    let pair: plist::Value = recv_lockdown(&mut sent).await.unwrap();
    assert_eq!(
        pair.as_dictionary().unwrap()["Request"].as_string(),
        Some("Pair")
    );
    assert_eq!(sent.read_pos, sent.read_buf.len());
}

#[test]
fn actual_root_certificate_backend_failure_keeps_opaque_stack() {
    // This tests the real builder boundary, not production RSA generation fault injection.
    let incompatible = PKey::generate_ed25519().unwrap();
    let error = build_root_cert(&incompatible).unwrap_err();
    stack(&error, io::ErrorKind::Other, ErrorClass::Unknown);
}

#[test]
fn actual_signed_certificate_backend_failure_keeps_opaque_stack() {
    let rsa = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let root = build_root_cert(&rsa).unwrap();
    let incompatible = PKey::generate_ed25519().unwrap();
    let error = build_signed_cert(&rsa, &root, &incompatible).unwrap_err();
    stack(&error, io::ErrorKind::Other, ErrorClass::Unknown);
}

struct OwnedRecords(PathBuf);
impl OwnedRecords {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("ios-supervised-source-{}", uuid::Uuid::new_v4()));
        crate::secret_file::create_secret_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedRecords {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn record() -> FullPairRecord {
    FullPairRecord {
        device_certificate: vec![1],
        host_certificate: vec![2],
        host_private_key: vec![3],
        root_certificate: vec![4],
        root_private_key: vec![5],
        host_id: "owned-host".into(),
        system_buid: "owned-buid".into(),
    }
}

#[test]
fn actual_record_save_failures_keep_io_and_preserve_owned_targets() {
    let owned = OwnedRecords::new();
    let obstruction = owned.0.join("owned-private-parent");
    std::fs::write(&obstruction, b"previous-owned-data").unwrap();
    let error = save_pair_record(
        &record(),
        b"escrow",
        None,
        &obstruction.join("record.plist"),
    )
    .unwrap_err();
    let LockdownError::Io(io) = &error else {
        panic!("native filesystem cause lost")
    };
    let original = io.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(io.kind(), original.kind());
    assert_eq!(classify_error(&error), classify_error(original));
    assert!(original.raw_os_error().is_some());
    assert!(!error.to_string().contains("owned-private"));
    assert_eq!(std::fs::read(&obstruction).unwrap(), b"previous-owned-data");

    let directory = owned.0.join("owned-private-directory");
    std::fs::create_dir(&directory).unwrap();
    let error = save_pair_record(&record(), b"escrow", None, &directory).unwrap_err();
    let LockdownError::Io(io) = &error else {
        panic!("native rename cause lost")
    };
    let original = io.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(io.kind(), original.kind());
    assert!(directory.is_dir());
    assert_eq!(std::fs::read_dir(&owned.0).unwrap().count(), 2);

    let path = owned.0.join("owned-private\0.plist");
    let error = save_pair_record(&record(), b"escrow", None, &path).unwrap_err();
    let native = source(
        &error,
        io::ErrorKind::InvalidInput,
        ErrorClass::InvalidInput,
    )
    .source()
    .unwrap()
    .downcast_ref::<io::Error>()
    .unwrap();
    assert_eq!(native.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read_dir(&owned.0).unwrap().count(), 2);
}

#[test]
fn actual_record_save_success_keeps_payload_permissions_and_replacement() {
    let owned = OwnedRecords::new();
    let path = owned.0.join("record.plist");
    std::fs::write(&path, b"previous-owned-data").unwrap();
    save_pair_record(&record(), b"escrow", Some("00:11:22:33:44:55"), &path).unwrap();
    let saved = PairRecord::load_from_path(&path, "unused-owned-identifier").unwrap();
    assert_eq!(saved.host_id, "owned-host");
    assert_eq!(saved.system_buid, "owned-buid");
    assert_eq!(*saved.host_private_key, [3]);
    assert_eq!(saved.wifi_mac_address.as_deref(), Some("00:11:22:33:44:55"));
    let value: plist::Value = plist::from_bytes(&std::fs::read(&path).unwrap()).unwrap();
    let fields = value.as_dictionary().unwrap();
    assert_eq!(fields["RootPrivateKey"].as_data(), Some([5].as_slice()));
    assert_eq!(fields["EscrowBag"].as_data(), Some(b"escrow".as_slice()));
    assert_eq!(fields.len(), 9);
    assert_eq!(std::fs::read_dir(&owned.0).unwrap().count(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&owned.0).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}

async fn receive(peer: &mut tokio::io::DuplexStream) -> plist::Value {
    let length = peer.read_u32().await.unwrap() as usize;
    assert!(length <= 64 * 1024);
    let mut payload = vec![0; length];
    peer.read_exact(&mut payload).await.unwrap();
    plist::from_bytes(&payload).unwrap()
}

async fn reply(peer: &mut tokio::io::DuplexStream, value: plist::Value) {
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &value).unwrap();
    assert!(bytes.len() <= 64 * 1024);
    peer.write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    peer.write_all(&bytes).await.unwrap();
}

#[tokio::test]
async fn actual_public_supervised_exchange_keeps_requests_chain_and_signed_challenge() {
    let (bundle, supervisor) = identity();
    let device_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let device_public = device_key.public_key_to_pem().unwrap();
    let (mut host, mut peer) = tokio::io::duplex(64 * 1024);
    let device = tokio::spawn(async move {
        let request = receive(&mut peer).await;
        assert_eq!(
            request.as_dictionary().unwrap()["Key"].as_string(),
            Some("DevicePublicKey")
        );
        reply(
            &mut peer,
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Value",
                plist::Value::Data(device_public),
            )])),
        )
        .await;
        let first = receive(&mut peer).await;
        let fields = first.as_dictionary().unwrap();
        assert_eq!(fields["Request"].as_string(), Some("Pair"));
        assert_eq!(fields["Label"].as_string(), Some("ios-rs"));
        assert_eq!(fields["ProtocolVersion"].as_string(), Some("2"));
        let payload = fields["PairRecord"].as_dictionary().unwrap();
        assert_eq!(payload.len(), 5);
        assert!(!payload.contains_key("HostPrivateKey"));
        assert!(!payload.contains_key("RootPrivateKey"));
        assert_eq!(payload["SystemBUID"].as_string(), Some("owned-buid"));
        let root = X509::from_pem(payload["RootCertificate"].as_data().unwrap()).unwrap();
        let root_key = root.public_key().unwrap();
        for field in ["RootCertificate", "HostCertificate", "DeviceCertificate"] {
            let cert = X509::from_pem(payload[field].as_data().unwrap()).unwrap();
            assert_eq!(cert.version(), 2);
            assert_eq!(
                cert.serial_number()
                    .to_bn()
                    .unwrap()
                    .to_dec_str()
                    .unwrap()
                    .to_string(),
                "0"
            );
            assert_eq!(cert.subject_name().entries().count(), 0);
            assert!(cert.verify(&root_key).unwrap());
            assert_eq!(
                cert.signature_algorithm().object().nid(),
                openssl::nid::Nid::SHA1WITHRSAENCRYPTION
            );
            if field == "DeviceCertificate" {
                assert!(cert.public_key().unwrap().public_eq(&device_key));
            }
        }
        let options = fields["PairingOptions"].as_dictionary().unwrap();
        assert_eq!(options.len(), 2);
        assert_eq!(options["ExtendedPairingErrors"].as_boolean(), Some(true));
        assert_eq!(
            options["SupervisorCertificate"].as_data().unwrap(),
            supervisor.to_der().unwrap()
        );
        let challenge = b"owned-pairing-challenge\0\xff\r\n";
        reply(
            &mut peer,
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("Error", "MCChallengeRequired".into()),
                (
                    "ExtendedResponse",
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "PairingChallenge",
                        plist::Value::Data(challenge.to_vec()),
                    )])),
                ),
            ])),
        )
        .await;
        let second = receive(&mut peer).await;
        let second_fields = second.as_dictionary().unwrap();
        assert_eq!(second_fields["PairRecord"], fields["PairRecord"]);
        assert_eq!(second_fields["Request"], fields["Request"]);
        let options = second_fields["PairingOptions"].as_dictionary().unwrap();
        assert_eq!(options.len(), 1);
        let signed = Pkcs7::from_der(options["ChallengeResponse"].as_data().unwrap()).unwrap();
        let mut certs = Stack::new().unwrap();
        certs.push(supervisor).unwrap();
        let store = X509StoreBuilder::new().unwrap().build();
        let mut plaintext = Vec::new();
        // Mathematical signature verification against the known synthetic signer;
        // trust-chain validation is outside this fixture and production pairing.
        signed
            .verify(
                &certs,
                &store,
                None,
                Some(&mut plaintext),
                Pkcs7Flags::NOVERIFY | Pkcs7Flags::NOINTERN,
            )
            .unwrap();
        assert_eq!(&plaintext, challenge);
        reply(
            &mut peer,
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "EscrowBag",
                plist::Value::Data(b"owned-escrow".to_vec()),
            )])),
        )
        .await;
        peer.shutdown().await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(15), async {
        let (record, escrow) =
            crate::pair_supervised(&mut host, &bundle, "owned-password", "owned-buid")
                .await
                .unwrap();
        assert_eq!(escrow, b"owned-escrow");
        assert_eq!(record.system_buid, "owned-buid");
        let host_key = PKey::private_key_from_pem(&record.host_private_key).unwrap();
        assert!(X509::from_pem(&record.host_certificate)
            .unwrap()
            .public_key()
            .unwrap()
            .public_eq(&host_key));
        device.await.unwrap();
        drop(host);
    })
    .await
    .expect("bounded owned supervised exchange");
}

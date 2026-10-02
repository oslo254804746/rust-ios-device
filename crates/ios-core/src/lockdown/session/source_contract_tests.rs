use super::tests::{dummy_pair_record, TEST_CERT_PEM, TEST_KEY_PEM};
use super::*;
use crate::{classify_error, test_util::MockStream, ErrorClass, PairRecordError};
use std::{error::Error, io, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn native<'a, T: Error + 'static>(error: &'a (dyn Error + 'static)) -> &'a T {
    let mut current = Some(error);
    for _ in 0..32 {
        let Some(error) = current else { break };
        if let Some(native) = error.downcast_ref::<T>() {
            return native;
        }
        current = if let Some(io) = error.downcast_ref::<io::Error>() {
            io.get_ref().map(|source| source as &(dyn Error + 'static))
        } else {
            error.source()
        };
    }
    panic!("native source lost");
}

fn outer(error: &LockdownError, kind: io::ErrorKind, class: ErrorClass) -> &io::Error {
    let LockdownError::Io(io) = error else {
        panic!("native source flattened");
    };
    assert_eq!(io.kind(), kind);
    assert_eq!(classify_error(error), class);
    assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
    io
}

#[tokio::test]
async fn actual_plist_serialization_failure_keeps_source_and_writes_nothing() {
    struct Refused;
    impl serde::Serialize for Refused {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("owned-private timeout cancelled"))
        }
    }
    let mut stream = MockStream::eof();
    let error = send_lockdown(&mut stream, &Refused).await.unwrap_err();
    outer(
        &error,
        io::ErrorKind::InvalidInput,
        ErrorClass::InvalidInput,
    );
    assert!(native::<plist::Error>(&error)
        .to_string()
        .contains("owned-private"));
    assert!(stream.written.is_empty());
}

#[tokio::test]
async fn actual_received_plist_failure_and_roundtrip_keep_framing_contract() {
    let mut stream = MockStream::with_raw_frames(vec![b"<plist><dict>".to_vec()]);
    let error = recv_lockdown::<_, plist::Value>(&mut stream)
        .await
        .unwrap_err();
    outer(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol);
    native::<plist::Error>(&error);

    let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("Request", "GetValue"),
        ("Label", "ios-rs"),
    ]));
    let mut stream = MockStream::eof();
    send_lockdown(&mut stream, &value).await.unwrap();
    let bytes = stream.written;
    let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    assert_eq!(length, bytes.len() - 4);
    assert_eq!(
        plist::from_bytes::<plist::Value>(&bytes[4..]).unwrap(),
        value
    );
    let mut stream = MockStream::new(bytes);
    assert_eq!(
        recv_lockdown::<_, plist::Value>(&mut stream).await.unwrap(),
        value
    );
}

struct OwnedRecords(PathBuf);
impl OwnedRecords {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("ios-lockdown-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedRecords {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn actual_pair_record_lookup_and_parse_retain_sources_and_safe_diagnostics() {
    let owned = OwnedRecords::new();
    let path = owned.0.join("owned-private.plist");
    let error = PairRecord::load_from_path(&path, "owned-private").unwrap_err();
    let PairRecordError::Read {
        path: retained,
        source,
    } = &error
    else {
        panic!("filesystem cause lost");
    };
    assert_eq!(retained, &path);
    assert_eq!(source.kind(), io::ErrorKind::NotFound);
    assert!(source.raw_os_error().is_some());
    assert_eq!(classify_error(&error), ErrorClass::NotFound);
    assert!(!format!("{error} {error:?}").contains("owned-private"));

    std::fs::write(&path, b"<plist><dict><key>owned-private</key>").unwrap();
    let error = PairRecord::load_from_path(&path, "owned-private").unwrap_err();
    let PairRecordError::Read {
        path: retained,
        source,
    } = &error
    else {
        panic!("plist cause lost");
    };
    assert_eq!(retained, &path);
    assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    native::<plist::Error>(&error);
    assert!(!format!("{error} {error:?} {:?}", source.get_ref()).contains("owned-private"));
}

#[test]
fn actual_pair_record_success_keeps_bytes_identity_and_optional_mac() {
    let owned = OwnedRecords::new();
    let path = owned.0.join("record.plist");
    let fields = plist::Dictionary::from_iter([
        ("DeviceCertificate", plist::Value::Data(vec![1])),
        ("HostCertificate", plist::Value::Data(vec![2])),
        ("HostPrivateKey", plist::Value::Data(vec![3, 4])),
        ("RootCertificate", plist::Value::Data(vec![5])),
        ("HostID", "owned-host".into()),
        ("SystemBUID", "owned-buid".into()),
        ("WiFiMACAddress", "00:11:22:33:44:55".into()),
    ]);
    let mut data = Vec::new();
    plist::to_writer_xml(&mut data, &fields).unwrap();
    std::fs::write(&path, data).unwrap();
    let record = PairRecord::load_from_path(&path, "unused-owned-identifier").unwrap();
    assert_eq!(record.device_certificate, [1]);
    assert_eq!(record.host_certificate, [2]);
    assert_eq!(*record.host_private_key, [3, 4]);
    assert_eq!(record.root_certificate, [5]);
    assert_eq!(record.host_id, "owned-host");
    assert_eq!(record.system_buid, "owned-buid");
    assert_eq!(
        record.wifi_mac_address.as_deref(),
        Some("00:11:22:33:44:55")
    );
    assert!(!format!("{record:?}").contains("owned-host"));
}

#[test]
fn actual_pem_and_client_auth_failures_retain_native_errors() {
    for record in [
        dummy_pair_record(
            b"-----BEGIN CERTIFICATE-----\nowned-private!\n-----END CERTIFICATE-----",
            TEST_KEY_PEM,
        ),
        dummy_pair_record(
            TEST_CERT_PEM,
            b"-----BEGIN PRIVATE KEY-----\nowned-private!\n-----END PRIVATE KEY-----",
        ),
    ] {
        let error = build_rustls_config(&record).unwrap_err();
        let original = outer(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput,
        )
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
        assert_eq!(original.kind(), io::ErrorKind::InvalidData);
    }
    let record = dummy_pair_record(
        TEST_CERT_PEM,
        b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----",
    );
    let error = build_rustls_config(&record).unwrap_err();
    outer(&error, io::ErrorKind::Other, ErrorClass::Unknown);
    native::<rustls::Error>(&error);
}

#[tokio::test]
async fn actual_invalid_server_name_returns_native_input_error_before_io() {
    let record = dummy_pair_record(TEST_CERT_PEM, TEST_KEY_PEM);
    let (stream, peer) = tokio::io::duplex(64);
    let error = build_rustls_connection(stream, &record, "owned-private invalid/name")
        .await
        .unwrap_err();
    outer(
        &error,
        io::ErrorKind::InvalidInput,
        ErrorClass::InvalidInput,
    );
    native::<rustls::pki_types::InvalidDnsNameError>(&error);
    assert!(!error.to_string().contains("owned-private"));
    drop(peer);
}

#[tokio::test]
async fn actual_tls_closed_write_and_bad_peer_keep_io_kind_and_backend_source() {
    let record = dummy_pair_record(TEST_CERT_PEM, TEST_KEY_PEM);
    let (stream, peer) = tokio::io::duplex(4096);
    drop(peer);
    let error = wrap_service_tls(stream, &record).await.unwrap_err();
    let original = outer(&error, io::ErrorKind::BrokenPipe, ErrorClass::Disconnected)
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(original.kind(), io::ErrorKind::BrokenPipe);

    let (stream, mut peer) = tokio::io::duplex(4096);
    let device = tokio::spawn(async move {
        let mut header = [0; 5];
        peer.read_exact(&mut header).await.unwrap();
        assert_eq!(header[0], 22);
        let length = u16::from_be_bytes([header[3], header[4]]) as usize;
        assert!(length <= 4096);
        peer.read_exact(&mut vec![0; length]).await.unwrap();
        // A complete record header with an invalid TLS content type. A short
        // handshake payload would legitimately wait for the remaining header.
        peer.write_all(&[255, 3, 3, 0, 0]).await.unwrap();
        let mut alert = [0; 64];
        let _ = peer.read(&mut alert).await;
    });
    let error = tokio::time::timeout(Duration::from_secs(5), wrap_service_tls(stream, &record))
        .await
        .expect("bounded TLS failure")
        .unwrap_err();
    let original = outer(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol)
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(original.kind(), io::ErrorKind::InvalidData);
    native::<rustls::Error>(original);
    tokio::time::timeout(Duration::from_secs(5), device)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn actual_service_tls_success_keeps_existing_verifier_and_application_io() {
    let record = dummy_pair_record(TEST_CERT_PEM, TEST_KEY_PEM);
    let certs = rustls_pemfile::certs(&mut Cursor::new(TEST_CERT_PEM))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let key = rustls_pemfile::private_key(&mut Cursor::new(TEST_KEY_PEM))
        .unwrap()
        .unwrap();
    let _ = rustls::crypto::ring::default_provider().install_default();
    let server = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let (stream, peer) = tokio::io::duplex(4096);
    let device = tokio::spawn(async move {
        let mut peer = acceptor.accept(peer).await.unwrap();
        let mut request = [0; 4];
        peer.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        peer.write_all(b"pong").await.unwrap();
        peer.flush().await.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut client = wrap_service_tls(stream, &record).await.unwrap();
        client.write_all(b"ping").await.unwrap();
        client.flush().await.unwrap();
        let mut response = [0; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        device.await.unwrap();
    })
    .await
    .expect("bounded successful TLS handshake");
}

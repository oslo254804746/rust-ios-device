use super::*;
use crate::xpc::message::{decode_message, encode_message, flags, XpcMessage, XpcValue};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const UID: &str = "prepared-rsd-local-fixture";
const SERVICE: &str = "fixture.current.service";
fn directory(port: u16) -> IndexMap<String, XpcValue> {
    IndexMap::from([
        ("MessageType".into(), XpcValue::String("Handshake".into())),
        (
            "Properties".into(),
            XpcValue::Dictionary(IndexMap::from([(
                "UniqueDeviceID".into(),
                XpcValue::String(UID.into()),
            )])),
        ),
        (
            "Services".into(),
            XpcValue::Dictionary(IndexMap::from([(
                SERVICE.into(),
                XpcValue::Dictionary(IndexMap::from([
                    ("Port".into(), XpcValue::Uint64(u64::from(port))),
                    (
                        "Properties".into(),
                        XpcValue::Dictionary(IndexMap::from([(
                            "Features".into(),
                            XpcValue::Array(vec![XpcValue::String("new-feature".into())]),
                        )])),
                    ),
                ])),
            )])),
        ),
    ])
}

#[test]
fn prepared_rsd_exact_backing_and_last_catalogue_owner_release() {
    let bank = Arc::new(Semaphore::new(65536));
    let original = directory(12345);
    let catalogue = PreparedRsdCatalogue::decode(&original, &bank).unwrap();
    let bytes = catalogue.known_bytes();
    assert_eq!(bank.available_permits(), 65536 - bytes);
    assert_eq!(catalogue.rows.capacity(), 1);
    assert_eq!(catalogue.udid.text.capacity(), UID.len());
    assert_eq!(catalogue.rows[0].name.text.capacity(), SERVICE.len());
    assert_eq!(catalogue.rows[0].features.as_ref().unwrap().capacity(), 1);
    assert_eq!(catalogue.service_port(SERVICE), Some(12345));
    assert_eq!(
        catalogue
            .service_features(SERVICE)
            .unwrap()
            .collect::<Vec<_>>(),
        ["new-feature"]
    );
    let owner = Arc::new(catalogue);
    let last = owner.clone();
    drop(owner);
    assert_eq!(bank.available_permits(), 65536 - bytes);
    drop(last);
    assert_eq!(bank.available_permits(), 65536);
    assert_eq!(
        original["Properties"].as_dict().unwrap()["UniqueDeviceID"].as_str(),
        Some(UID)
    );
}

#[test]
fn prepared_rsd_one_byte_refusal_refunds_every_partial_owner_without_mutating_input() {
    let original = directory(12345);
    let probe_bank = Arc::new(Semaphore::new(65536));
    let probe = PreparedRsdCatalogue::decode(&original, &probe_bank).unwrap();
    let needed = probe.known_bytes();
    drop(probe);
    for total in [
        0,
        std::mem::size_of::<PreparedRsdCatalogue>() - 1,
        needed - 1,
    ] {
        let bank = Arc::new(Semaphore::new(total));
        let error = PreparedRsdCatalogue::decode(&original, &bank).unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), total);
    }
    let bank = Arc::new(Semaphore::new(needed));
    let exact = PreparedRsdCatalogue::decode(&original, &bank).unwrap();
    assert_eq!(bank.available_permits(), 0);
    drop(exact);
    assert_eq!(bank.available_permits(), needed);
}

#[test]
fn prepared_rsd_missing_port_and_identity_are_incomplete_before_prefix_ownership() {
    for missing in ["UniqueDeviceID", "Port"] {
        let bank = Arc::new(Semaphore::new(65536));
        let mut native = directory(12345);
        if missing == "UniqueDeviceID" {
            if let XpcValue::Dictionary(properties) = native.get_mut("Properties").unwrap() {
                properties.shift_remove(missing);
            }
        } else if let XpcValue::Dictionary(services) = native.get_mut("Services").unwrap() {
            if let XpcValue::Dictionary(service) = services.get_mut(SERVICE).unwrap() {
                service.shift_remove(missing);
            }
        }
        assert!(PreparedRsdCatalogue::decode(&native, &bank).is_err());
        assert_eq!(bank.available_permits(), 65536);
    }
}

fn frame(kind: u8, flag: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    assert!(body.len() <= 0xff_ffff);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    bytes.extend_from_slice(&[kind, flag]);
    bytes.extend_from_slice(&stream.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}
async fn read_frame(peer: &mut tokio::net::TcpStream) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let n = (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
    assert!(n <= 4096);
    let mut bytes = vec![0; n];
    peer.read_exact(&mut bytes).await.unwrap();
    (
        header[3],
        header[4],
        u32::from_be_bytes(header[5..].try_into().unwrap()),
        bytes,
    )
}
async fn producer(peer: &mut tokio::net::TcpStream, port: u16) {
    // Same production bootstrap assertions as existing_rsd/tests.rs. No echo-only mock.
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
    assert_eq!(read_frame(peer).await.0, 4);
    assert_eq!(read_frame(peer).await.0, 8);
    peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
        let incoming = read_frame(peer).await;
        assert_eq!((incoming.0, incoming.2), (kind, stream));
    }
    let ack = read_frame(peer).await;
    assert_eq!((ack.0, ack.1, ack.2), (4, 1, 0));
    let handshake = read_frame(peer).await;
    assert_eq!((handshake.0, handshake.2), (0, 1));
    let decoded = decode_message(bytes::Bytes::from(handshake.3)).unwrap();
    assert_eq!(
        decoded.body.unwrap().as_dict().unwrap()["MessageType"].as_str(),
        Some("Handshake")
    );
    let wire = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(directory(port))),
    })
    .unwrap();
    peer.write_all(&frame(0, 0, 1, &wire)).await.unwrap();
    let mut tail = Vec::new();
    peer.read_to_end(&mut tail).await.unwrap();
    while !tail.is_empty() {
        assert!(tail.len() >= 13);
        assert_eq!(tail[3], 8); // Flow credit only; no extra request.
        let n = (usize::from(tail[0]) << 16) | (usize::from(tail[1]) << 8) | usize::from(tail[2]);
        assert_eq!(n, 4);
        tail.drain(..9 + n);
    }
}

#[tokio::test]
async fn prepared_rsd_same_address_changed_port_uses_new_real_service_socket_and_preserves_initial()
{
    let bank = Arc::new(Semaphore::new(1024 * 1024));
    let rsd = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let old_service = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let new_service = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let old_port = old_service.local_addr().unwrap().port();
    let new_port = new_service.local_addr().unwrap().port();
    let std::net::SocketAddr::V6(endpoint) = rsd.local_addr().unwrap() else {
        unreachable!()
    };
    let directory_peer = tokio::spawn(async move {
        for port in [old_port, new_port] {
            let (mut peer, _) = rsd.accept().await.unwrap();
            producer(&mut peer, port).await;
        }
    });
    let service_peer = tokio::spawn(async move {
        let (mut peer, _) = new_service.accept().await.unwrap();
        let mut body = Vec::new();
        peer.read_to_end(&mut body).await.unwrap();
        assert_eq!(body, b"readonly-fixture");
    });
    let route = super::super::ExistingRsdRoute::direct(endpoint).unwrap();
    let first = super::super::prepare_existing_rsd_with_identity_at(
        route,
        tokio::time::Instant::now() + Duration::from_secs(3),
        &bank,
        uuid::Uuid::nil(),
    )
    .await
    .unwrap();
    let second = super::super::prepare_existing_rsd_with_identity_at(
        route,
        tokio::time::Instant::now() + Duration::from_secs(3),
        &bank,
        uuid::Uuid::nil(),
    )
    .await
    .unwrap();
    assert_eq!(first.service_port(SERVICE), Some(old_port));
    assert_eq!(second.service_port(SERVICE), Some(new_port));
    let (handle, _control) = crate::TunnelHandle::new(
        UID.into(),
        crate::TunnelInfo {
            server_address: "::1".into(),
            server_rsd_port: endpoint.port(),
            client_address: "::1".into(),
            client_mtu: 1280,
        },
        None,
    );
    let runtime = Arc::new(handle);
    let initial_failure = crate::CoreError::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "original-initial-observation",
    ));
    let (legacy, initial_rsd_probe) = super::super::rsd_observation::RsdProbeOutcome::<
        crate::RsdHandshake,
    >::Completed(Err(initial_failure))
    .into_parts();
    let mut device = super::super::ConnectedDevice {
        info: crate::DeviceInfo {
            udid: UID.into(),
            device_id: 42,
            connection_type: "USB".into(),
            product_id: 0,
        },
        tunnel: Some(runtime.clone()),
        rsd: legacy,
        owned_initial_rsd: None,
        owned_initial_bank: None,
        prepared_rsd: None,
        initial_rsd_probe,
        pair_record: None,
        lockdown_transport: super::super::LockdownTransport::Usbmux { device_id: 42 },
    };
    let initial = device.initial_rsd_probe().error().unwrap() as *const CoreError;
    assert!(device
        .swap_current_rsd_catalogue(&runtime, Some(first))
        .unwrap()
        .is_none());
    let old = device
        .swap_current_rsd_catalogue(&runtime, Some(second))
        .unwrap()
        .unwrap();
    assert_eq!(old.service_port(SERVICE), Some(old_port));
    assert_eq!(
        device.initial_rsd_probe().error().unwrap() as *const CoreError,
        initial
    );
    assert!(device.rsd().is_none(), "initial API stays historical");
    let mut socket = device.connect_rsd_service(SERVICE).await.unwrap();
    socket.write_all(b"readonly-fixture").await.unwrap();
    drop(socket);
    assert!(
        tokio::time::timeout(Duration::from_millis(20), old_service.accept())
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(3), directory_peer)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), service_peer)
        .await
        .unwrap()
        .unwrap();
    drop(device);
    drop(old);
    drop(runtime);
    assert_eq!(bank.available_permits(), 1024 * 1024);
    // Legacy control constructor owns no native tasks; this checks original
    // directory/service sockets only. Userspace native joins need separate CLI gate.
}

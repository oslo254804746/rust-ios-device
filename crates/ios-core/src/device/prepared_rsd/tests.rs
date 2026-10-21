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
        #[cfg(feature = "tunnel")]
        owned_prepared_rsd: None,
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

fn owned_prepared(bank: &Arc<Semaphore>) -> PreparedRsdOwner {
    let catalogue = PreparedRsdCatalogue::decode(&directory(12345), bank).unwrap();
    let (fee, mut modes) = exact_vec(bank, 1).unwrap();
    modes.push(super::super::existing_rsd::RsdHandshakeMode::Direct);
    catalogue
        .finish_owned(
            RsdSelectionReceipt {
                handshake_mode: super::super::existing_rsd::RsdHandshakeMode::Direct,
                attempted_modes: modes,
            },
            fee,
        )
        .unwrap()
}
#[test]
fn prepared_owned_root_exports_keep_original_body_rows_features_and_selection_until_last_owner() {
    let bank = Arc::new(Semaphore::new(65536));
    let root = owned_prepared(&bank);
    let paid = root.known_bytes();
    let body = std::ptr::from_ref(&*root) as usize;
    let row = root.rows.as_ptr() as usize;
    let uid = root.udid().as_ptr() as usize;
    let feature = root
        .service_features(SERVICE)
        .unwrap()
        .next()
        .unwrap()
        .as_ptr() as usize;
    let modes = root.selection().unwrap().attempted_modes.as_ptr() as usize;
    let last = root.clone();
    assert!(last.ptr_eq(&root));
    drop(root);
    bank.close();
    assert_eq!(std::ptr::from_ref(&*last) as usize, body);
    assert_eq!(last.rows.as_ptr() as usize, row);
    assert_eq!(last.udid().as_ptr() as usize, uid);
    assert_eq!(
        last.service_features(SERVICE)
            .unwrap()
            .next()
            .unwrap()
            .as_ptr() as usize,
        feature
    );
    assert_eq!(
        last.selection().unwrap().attempted_modes.as_ptr() as usize,
        modes
    );
    assert_eq!(bank.available_permits(), 65536 - paid);
    assert_eq!(last.service_port(SERVICE), Some(12345));
    drop(last);
    assert_eq!(bank.available_permits(), 65536);
}
#[test]
fn prepared_owned_parallel_last_roots_share_original_payload_and_refund_exactly_once() {
    let bank = Arc::new(Semaphore::new(65536));
    let root = owned_prepared(&bank);
    let paid = root.known_bytes();
    let pointer = std::ptr::from_ref(&*root) as usize;
    let barrier = Arc::new(std::sync::Barrier::new(9));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let original = root.clone();
            let barrier = barrier.clone();
            let bank = bank.clone();
            scope.spawn(move || {
                barrier.wait();
                assert_eq!(std::ptr::from_ref(&*original) as usize, pointer);
                assert_eq!(original.service_port(SERVICE), Some(12345));
                assert_eq!(bank.available_permits(), 65536 - paid);
                barrier.wait();
                drop(original);
            });
        }
        bank.close();
        drop(root);
        barrier.wait();
        assert_eq!(bank.available_permits(), 65536 - paid);
        barrier.wait();
    });
    assert_eq!(bank.available_permits(), 65536);
}

fn owned_catalogue_addresses(root: &PreparedRsdOwner) -> (usize, usize, usize, usize, usize) {
    (
        std::ptr::from_ref(&**root) as usize,
        root.rows.as_ptr() as usize,
        root.udid().as_ptr() as usize,
        root.service_features(SERVICE)
            .unwrap()
            .next()
            .unwrap()
            .as_ptr() as usize,
        root.selection().unwrap().attempted_modes.as_ptr() as usize,
    )
}

async fn owned_service_peer(
    listener: tokio::net::TcpListener,
    request: &'static [u8],
    reply: &'static [u8],
) {
    let (mut peer, _) = listener.accept().await.unwrap();
    let mut body = vec![0; request.len()];
    peer.read_exact(&mut body).await.unwrap();
    assert_eq!(body, request);
    peer.write_all(reply).await.unwrap();
    let mut trailing = [0; 1];
    assert_eq!(peer.read(&mut trailing).await.unwrap(), 0);
    // The client initiated shutdown. End the accepted peer and the actual old
    // listener before the fixture can bind the replacement service generation.
    drop(peer);
    drop(listener);
}

async fn owned_service_exchange(
    device: &super::super::OwnedConnectedDevice,
    request: &[u8],
    expected_reply: &[u8],
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut socket = device.connect_rsd_service(SERVICE).await.unwrap();
        socket.write_all(request).await.unwrap();
        let mut reply = vec![0; expected_reply.len()];
        socket.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, expected_reply);
        socket.shutdown().await.unwrap();
        let mut trailing = [0; 1];
        assert_eq!(socket.read(&mut trailing).await.unwrap(), 0);
    })
    .await
    .expect("owned current service did not complete real IO and strict EOF");
}

async fn owned_current_service_restart(changed_port: bool) {
    const QUOTA: usize = 1024 * 1024;
    const BUDGET: Duration = Duration::from_secs(3);
    let bank = Arc::new(Semaphore::new(QUOTA));
    let rsd = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let std::net::SocketAddr::V6(endpoint) = rsd.local_addr().unwrap() else {
        unreachable!()
    };
    let route = super::super::ExistingRsdRoute::direct(endpoint).unwrap();
    let (ports, mut advertised) = tokio::sync::mpsc::channel(2);
    let directory_peer = tokio::spawn(async move {
        for _ in 0..2 {
            let port = advertised
                .recv()
                .await
                .expect("two explicit service generations");
            let (mut peer, _) = rsd.accept().await.unwrap();
            producer(&mut peer, port).await;
        }
    });
    let first_listener = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let old_address = first_listener.local_addr().unwrap();
    ports.send(old_address.port()).await.unwrap();
    let first_deadline = tokio::time::Instant::now() + BUDGET;
    let first = tokio::time::timeout_at(
        first_deadline,
        super::super::prepare_owned_existing_rsd_with_identity_at(
            route,
            first_deadline,
            &bank,
            uuid::Uuid::nil(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(first.belongs_to(&bank));
    assert_eq!(first.service_port(SERVICE), Some(old_address.port()));
    let first_paid = first.known_bytes();
    let first_addresses = owned_catalogue_addresses(&first);
    assert_eq!(bank.available_permits(), QUOTA - first_paid);
    let first_export = first.clone();

    let (handle, control) = crate::TunnelHandle::new(
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
    let (legacy, initial_rsd_probe) =
        super::super::rsd_observation::RsdProbeOutcome::<crate::RsdHandshake>::Completed(Err(
            crate::CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "owned-original-initial-observation",
            )),
        ))
        .into_parts();
    let mut device = super::super::OwnedConnectedDevice::new(super::super::ConnectedDevice {
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
        owned_prepared_rsd: None,
        initial_rsd_probe,
        pair_record: None,
        lockdown_transport: super::super::LockdownTransport::Usbmux { device_id: 42 },
    });
    let initial = device.initial_rsd_probe().error().unwrap() as *const CoreError;
    let initial_io = match device.initial_rsd_probe().error().unwrap() {
        CoreError::Io(error) => error as *const std::io::Error,
        _ => panic!("original typed initial IO cause missing"),
    };
    assert!(device
        .swap_current_rsd_catalogue(&runtime, Some(first))
        .unwrap()
        .is_none());
    assert!(device.prepared_rsd_owner().unwrap().ptr_eq(&first_export));
    assert_eq!(
        std::ptr::from_ref(device.current_rsd_catalogue().unwrap()) as usize,
        first_addresses.0
    );
    let first_peer = tokio::spawn(owned_service_peer(
        first_listener,
        b"owned-before-restart",
        b"first-peer",
    ));
    owned_service_exchange(&device, b"owned-before-restart", b"first-peer").await;
    tokio::time::timeout(BUDGET, first_peer)
        .await
        .unwrap()
        .unwrap();

    // This binding occurs only after the old service's real socket and listener
    // have ended. The RSD listener/endpoint stays unchanged for both observations.
    let second_listener = if changed_port {
        let candidate = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
            .await
            .unwrap();
        if candidate.local_addr().unwrap().port() == old_address.port() {
            // Keep that port occupied while requesting another ephemeral port.
            let different = tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
                .await
                .unwrap();
            drop(candidate);
            different
        } else {
            candidate
        }
    } else {
        tokio::net::TcpListener::bind(old_address).await.unwrap()
    };
    let new_address = second_listener.local_addr().unwrap();
    assert_eq!(new_address.port() != old_address.port(), changed_port);
    let old_endpoint_trap = if changed_port {
        Some(tokio::net::TcpListener::bind(old_address).await.unwrap())
    } else {
        None
    };
    ports.send(new_address.port()).await.unwrap();
    let second_deadline = tokio::time::Instant::now() + BUDGET;
    let second = tokio::time::timeout_at(
        second_deadline,
        super::super::prepare_owned_existing_rsd_with_identity_at(
            route,
            second_deadline,
            &bank,
            uuid::Uuid::nil(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(second.belongs_to(&bank));
    assert_eq!(second.service_port(SERVICE), Some(new_address.port()));
    assert_eq!(
        second.selection().unwrap().attempted_modes,
        [super::super::existing_rsd::RsdHandshakeMode::Direct]
    );
    let second_paid = second.known_bytes();
    let second_addresses = owned_catalogue_addresses(&second);
    assert_ne!(first_addresses.0, second_addresses.0);
    assert_eq!(bank.available_permits(), QUOTA - first_paid - second_paid);
    let current_export = second.clone();
    let displaced = device
        .swap_current_rsd_catalogue(&runtime, Some(second))
        .unwrap()
        .unwrap();
    assert!(displaced.ptr_eq(&first_export));
    assert_eq!(owned_catalogue_addresses(&displaced), first_addresses);
    assert!(device.prepared_rsd_owner().unwrap().ptr_eq(&current_export));
    assert_eq!(
        std::ptr::from_ref(device.current_rsd_catalogue().unwrap()) as usize,
        second_addresses.0
    );
    assert!(Arc::ptr_eq(device.tunnel_handle().unwrap(), &runtime));
    assert_eq!(device.server_address(), Some("::1"));
    assert_eq!(device.rsd_port(), Some(endpoint.port()));
    assert_eq!(
        device.initial_rsd_probe().error().unwrap() as *const CoreError,
        initial
    );
    let CoreError::Io(still_initial_io) = device.initial_rsd_probe().error().unwrap() else {
        panic!("refresh replaced original typed initial IO cause");
    };
    assert_eq!(still_initial_io as *const std::io::Error, initial_io);
    assert!(device.rsd().is_none(), "initial owned API stays historical");
    tokio::time::timeout(BUDGET, directory_peer)
        .await
        .unwrap()
        .unwrap();
    drop(ports);

    // A successful current lookup must work after quota is closed. This does
    // not claim that legacy SDK service-output allocations are byte-admitted.
    bank.close();
    let restarted_peer = tokio::spawn(owned_service_peer(
        second_listener,
        b"owned-after-restart",
        b"restarted-peer",
    ));
    owned_service_exchange(&device, b"owned-after-restart", b"restarted-peer").await;
    tokio::time::timeout(BUDGET, restarted_peer)
        .await
        .unwrap()
        .unwrap();
    if let Some(trap) = old_endpoint_trap {
        assert!(
            tokio::time::timeout(Duration::from_millis(20), trap.accept())
                .await
                .is_err()
        );
    }
    assert_eq!(bank.available_permits(), QUOTA - first_paid - second_paid);
    drop(device);
    assert_eq!(owned_catalogue_addresses(&first_export), first_addresses);
    assert_eq!(owned_catalogue_addresses(&current_export), second_addresses);
    assert_eq!(bank.available_permits(), QUOTA - first_paid - second_paid);
    drop(displaced);
    assert_eq!(bank.available_permits(), QUOTA - first_paid - second_paid);
    drop(first_export);
    assert_eq!(bank.available_permits(), QUOTA - second_paid);
    assert_eq!(
        current_export.service_port(SERVICE),
        Some(new_address.port())
    );
    drop(current_export);
    assert_eq!(bank.available_permits(), QUOTA);
    drop((runtime, control));
    // Channel-only TunnelHandle: no native worker or shutdown certificate is
    // invented here. Original native joins stay covered by the CLI fixtures.
}

#[tokio::test]
async fn prepared_owned_same_port_service_restart_uses_current_root_and_real_socket() {
    owned_current_service_restart(false).await;
}

#[tokio::test]
async fn prepared_owned_changed_port_service_restart_uses_current_root_and_real_socket() {
    owned_current_service_restart(true).await;
}

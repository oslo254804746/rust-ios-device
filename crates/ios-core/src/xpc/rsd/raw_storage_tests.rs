//! Isolated duplex and IPv6 localhost protocol fixtures; no physical devices.
use super::*;
use crate::device::existing_rsd::RsdHandshakeMode;
use crate::xpc::message::encode_message;
use indexmap::IndexMap;
use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::Semaphore,
};

fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut out = vec![(len >> 16) as u8, (len >> 8) as u8, len as u8, kind, flags];
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<(u8, u32, Vec<u8>)> {
    let mut header = [0u8; 9];
    stream.read_exact(&mut header).await?;
    let len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
    let mut payload = vec![0; len];
    stream.read_exact(&mut payload).await?;
    Ok((
        header[3],
        u32::from_be_bytes(header[5..].try_into().unwrap()) & 0x7fff_ffff,
        payload,
    ))
}

fn handshake(port: XpcValue) -> Bytes {
    let service = XpcValue::Dictionary(IndexMap::from_iter([
        ("Port".into(), port),
        (
            "Properties".into(),
            XpcValue::Dictionary(IndexMap::from_iter([(
                "Features".into(),
                XpcValue::Array(vec![XpcValue::String("fixture-feature".into())]),
            )])),
        ),
    ]));
    encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(IndexMap::from_iter([
            ("MessageType".into(), XpcValue::String("Handshake".into())),
            (
                "Properties".into(),
                XpcValue::Dictionary(IndexMap::from_iter([(
                    "UniqueDeviceID".into(),
                    XpcValue::String("fixture-only-udid".into()),
                )])),
            ),
            (
                "Services".into(),
                XpcValue::Dictionary(IndexMap::from_iter([("fixture.service".into(), service)])),
            ),
        ]))),
    })
    .unwrap()
}

async fn send_directory<S: AsyncWrite + Unpin>(peer: &mut S, message: &[u8]) -> io::Result<()> {
    peer.write_all(&frame(1, 4, 1, &[])).await?;
    // Interleave an auxiliary stream and split the XPC wrapper across frames.
    peer.write_all(&frame(1, 4, 6, &[])).await?;
    peer.write_all(&frame(0, 0, 6, b"auxiliary")).await?;
    peer.write_all(&frame(0, 0, 1, &message[..13])).await?;
    peer.write_all(&frame(4, 1, 0, &[])).await?;
    peer.write_all(&frame(0, 1, 1, &message[13..])).await?;
    peer.flush().await
}

async fn fixture<S: AsyncRead + AsyncWrite + Unpin>(
    mut peer: S,
    mode: RsdHandshakeMode,
    message: Bytes,
) -> io::Result<()> {
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await?;
    assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
    assert_eq!(read_frame(&mut peer).await?.0, 4);
    assert_eq!(read_frame(&mut peer).await?.0, 8);
    peer.write_all(&frame(4, 0, 0, &[])).await?;
    peer.flush().await?;
    if mode == RsdHandshakeMode::Passive {
        return send_and_drain_directory(&mut peer, &message).await;
    }
    let empty = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })
    .unwrap();
    loop {
        let (kind, stream_id, payload) = read_frame(&mut peer).await?;
        if kind != 0 {
            continue;
        }
        let outgoing = decode_message(Bytes::from(payload)).map_err(io::Error::other)?;
        match mode {
            RsdHandshakeMode::Direct => {
                let host_handshake = outgoing
                    .body
                    .as_ref()
                    .and_then(XpcValue::as_dict)
                    .and_then(|dictionary| dictionary.get("MessageType"))
                    .and_then(XpcValue::as_str)
                    == Some("Handshake");
                if host_handshake {
                    return send_and_drain_directory(&mut peer, &message).await;
                }
            }
            RsdHandshakeMode::Queued => {
                if stream_id == 3 {
                    return send_and_drain_directory(&mut peer, &message).await;
                }
            }
            RsdHandshakeMode::Legacy => {
                peer.write_all(&frame(1, 4, stream_id, &[])).await?;
                peer.write_all(&frame(0, 0, stream_id, &empty)).await?;
                peer.flush().await?;
                if stream_id == 3 {
                    return send_and_drain_directory(&mut peer, &message).await;
                }
            }
            RsdHandshakeMode::Passive => unreachable!(),
        }
    }
}

async fn send_and_drain_directory<S: AsyncRead + AsyncWrite + Unpin>(
    peer: &mut S,
    message: &Bytes,
) -> io::Result<()> {
    send_directory(peer, message).await?;
    // Keep the same peer alive while the client sends its remaining bootstrap
    // and flow credit. Only the owned client close ends this strict EOF read.
    let mut tail = [0; 1024];
    while peer.read(&mut tail).await? != 0 {}
    Ok(())
}

async fn checked_duplex(
    mode: RsdHandshakeMode,
    message: Bytes,
) -> (
    Result<crate::device::PreparedRsdCatalogue, XpcError>,
    Arc<Semaphore>,
    usize,
) {
    let target = [
        RsdHandshakeMode::Direct,
        RsdHandshakeMode::Queued,
        RsdHandshakeMode::Legacy,
        RsdHandshakeMode::Passive,
    ]
    .iter()
    .position(|candidate| *candidate == mode)
    .unwrap();
    let bank = Arc::new(Semaphore::new(128 * 1024 * 1024));
    let before = bank.available_permits();
    let mut clients = VecDeque::new();
    let mut peers = Vec::new();
    for attempt in 0..=target {
        let (client, peer) = tokio::io::duplex(64 * 1024);
        clients.push_back(client);
        let message = message.clone();
        peers.push(tokio::spawn(async move {
            if attempt == target {
                fixture(peer, mode, message).await
            } else {
                drop(peer);
                Ok(())
            }
        }));
    }
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        checked_handshake_prepared(
            || std::future::ready(Ok(clients.pop_front().expect("exact fallback count"))),
            uuid::Uuid::from_bytes([9; 16]),
            tokio::time::Instant::now() + Duration::from_secs(4),
            &bank,
        ),
    )
    .await;
    for peer in peers {
        peer.await.unwrap().unwrap();
    }
    let result = result
        .expect("fixture must not wait for the normal response timeouts")
        .map(|(catalogue, receipt, selection_fee)| {
            assert_eq!(receipt.handshake_mode, mode);
            assert_eq!(receipt.attempted_modes.len(), target + 1);
            drop(selection_fee);
            catalogue
        });
    (result, bank, before)
}

#[tokio::test]
async fn all_four_real_raw_h2_modes_keep_catalogue_and_release_transient_storage() {
    for mode in [
        RsdHandshakeMode::Direct,
        RsdHandshakeMode::Queued,
        RsdHandshakeMode::Legacy,
        RsdHandshakeMode::Passive,
    ] {
        let (result, bank, before) = checked_duplex(mode, handshake(XpcValue::Uint64(12345))).await;
        let catalogue = result.unwrap_or_else(|error| panic!("mode {mode:?}: {error:?}"));
        assert_eq!(catalogue.udid(), "fixture-only-udid");
        assert_eq!(catalogue.service_port("fixture.service"), Some(12345));
        assert_eq!(
            bank.available_permits(),
            before - catalogue.known_bytes(),
            "framer/body/arena must be gone; selection fee was explicitly dropped"
        );
        drop(catalogue);
        assert_eq!(bank.available_permits(), before);
    }
}

#[tokio::test]
async fn owned_prepare_all_four_success_modes_fill_the_original_pre_dial_root() {
    use crate::device::{
        existing_rsd::{prepare_owned_existing_rsd_with_identity_at, ExistingRsdRoute},
        prepared_rsd::{allocation_observer, OwnedDestination},
        PreparedRsdCatalogue,
    };
    use std::{alloc::Layout, net::SocketAddrV6, sync::Mutex};
    use tokio::net::TcpListener;

    let modes = [
        RsdHandshakeMode::Direct,
        RsdHandshakeMode::Queued,
        RsdHandshakeMode::Legacy,
        RsdHandshakeMode::Passive,
    ];
    for (target, mode) in modes.into_iter().enumerate() {
        let listener = TcpListener::bind("[::1]:0").await.unwrap();
        let std::net::SocketAddr::V6(endpoint) = listener.local_addr().unwrap() else {
            panic!("IPv6 localhost fixture");
        };
        assert_eq!(*endpoint.ip(), std::net::Ipv6Addr::LOCALHOST);
        let route =
            ExistingRsdRoute::direct(SocketAddrV6::new(*endpoint.ip(), endpoint.port(), 0, 0))
                .unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let accepted = accepts.clone();
        let peer = tokio::spawn(async move {
            for attempt in 0..=target {
                let (stream, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, Ordering::SeqCst);
                if attempt == target {
                    // Returns only on the owned client's strict Ok(0) close.
                    fixture(stream, mode, handshake(XpcValue::Uint64(12345)))
                        .await
                        .unwrap();
                } else {
                    drop(stream);
                }
            }
        });
        let bank = Arc::new(Semaphore::new(128 * 1024 * 1024));
        let before = bank.available_permits();
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let observed = allocations.clone();
        let observed_bank = bank.clone();
        let observed_accepts = accepts.clone();
        let observer = allocation_observer(move |layout, pointer| {
            observed.lock().unwrap().push((
                layout,
                pointer,
                observed_bank.available_permits(),
                observed_accepts.load(Ordering::SeqCst),
            ));
        });
        let catalogue = tokio::time::timeout(
            Duration::from_secs(5),
            prepare_owned_existing_rsd_with_identity_at(
                route,
                tokio::time::Instant::now() + Duration::from_secs(4),
                &bank,
                uuid::Uuid::from_bytes([9; 16]),
            ),
        )
        .await
        .expect("local fixture must finish within the original deadline")
        .unwrap_or_else(|error| panic!("owned mode {mode:?}: {error:?}"));
        drop(observer);
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .expect("same socket must reach strict EOF and join")
            .unwrap();
        assert_eq!(accepts.load(Ordering::SeqCst), target + 1);
        let allocations = allocations.lock().unwrap();
        let (root_layout, root, available, accepts_before_root) = allocations[0];
        assert_eq!(root_layout.size(), OwnedDestination::body_bytes());
        assert!(root_layout.size() > Layout::new::<PreparedRsdCatalogue>().size());
        assert_eq!(available, before - root_layout.size());
        assert_eq!(accepts_before_root, 0, "root must exist before first dial");
        assert_eq!(
            allocations
                .iter()
                .filter(|(layout, ..)| *layout == root_layout)
                .count(),
            1,
            "failed wire modes must keep the same actual destination"
        );
        let catalogue_pointer = (&*catalogue as *const PreparedRsdCatalogue) as usize;
        assert!((root..root + root_layout.size()).contains(&catalogue_pointer));
        assert_eq!(catalogue.udid(), "fixture-only-udid");
        assert_eq!(catalogue.service_port("fixture.service"), Some(12345));
        let selection = catalogue.selection().unwrap();
        assert_eq!(selection.handshake_mode, mode);
        assert_eq!(selection.attempted_modes, modes[..=target]);
        assert_eq!(selection.attempted_modes.capacity(), 4);
        assert!(catalogue.belongs_to(&bank));
        assert_eq!(bank.available_permits(), before - catalogue.known_bytes());
        let uid = catalogue.udid().as_ptr();
        let feature = catalogue
            .service_features("fixture.service")
            .unwrap()
            .next()
            .unwrap();
        assert_eq!(feature, "fixture-feature");
        let feature_pointer = feature.as_ptr();
        let modes_pointer = selection.attempted_modes.as_ptr();
        let rest = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        let retained = catalogue.clone();
        assert!(catalogue.ptr_eq(&retained));
        drop(catalogue);
        assert_eq!(retained.udid().as_ptr(), uid);
        assert_eq!(
            retained
                .service_features("fixture.service")
                .unwrap()
                .next()
                .unwrap()
                .as_ptr(),
            feature_pointer
        );
        assert_eq!(
            retained.selection().unwrap().attempted_modes.as_ptr(),
            modes_pointer
        );
        assert_eq!(bank.available_permits(), 0);
        let bytes = retained.known_bytes();
        drop(retained);
        assert_eq!(bank.available_permits(), bytes);
        drop(rest);
        assert_eq!(bank.available_permits(), before);
    }
}

#[tokio::test]
async fn malformed_port_keeps_native_parse_int_source_and_drops_all_charged_storage() {
    use std::error::Error;
    let (result, bank, before) = checked_duplex(
        RsdHandshakeMode::Direct,
        handshake(XpcValue::String("invalid-private-port".into())),
    )
    .await;
    let error = result.expect_err("invalid port accepted");
    let mut current: Option<&(dyn Error + 'static)> = Some(&error);
    let mut native = false;
    for _ in 0..32 {
        let Some(error) = current else {
            break;
        };
        native |= error.is::<std::num::ParseIntError>();
        current = if let Some(io) = error.downcast_ref::<io::Error>() {
            io.get_ref().map(|source| source as &(dyn Error + 'static))
        } else {
            error.source()
        };
    }
    assert!(native);
    assert_eq!(bank.available_permits(), before);
}

#[tokio::test]
async fn expired_absolute_deadline_never_dials_and_returns_all_selection_grants() {
    let bank = Arc::new(Semaphore::new(1024));
    let dials = Arc::new(AtomicUsize::new(0));
    let result = checked_handshake_prepared(
        || {
            dials.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err::<tokio::io::DuplexStream, _>(XpcError::NotConnected))
        },
        uuid::Uuid::from_bytes([9; 16]),
        tokio::time::Instant::now(),
        &bank,
    )
    .await;
    assert_eq!(
        crate::classify_error(&result.expect_err("expired budget accepted")),
        crate::ErrorClass::Timeout
    );
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 1024);
}

#[tokio::test]
async fn ipv6_localhost_fixture_uses_actual_production_existing_route_connector() {
    use crate::device::{prepare_existing_rsd_with_identity_at, ExistingRsdRoute};
    use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
    let listener = tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let SocketAddr::V6(address) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    let route =
        ExistingRsdRoute::direct(SocketAddrV6::new(Ipv6Addr::LOCALHOST, address.port(), 0, 0))
            .unwrap();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        fixture(
            socket,
            RsdHandshakeMode::Direct,
            handshake(XpcValue::Uint64(12345)),
        )
        .await
    });
    let bank = Arc::new(Semaphore::new(128 * 1024 * 1024));
    let result = prepare_existing_rsd_with_identity_at(
        route,
        tokio::time::Instant::now() + Duration::from_secs(2),
        &bank,
        uuid::Uuid::from_bytes([9; 16]),
    )
    .await;
    peer.await.unwrap().unwrap();
    let catalogue = result.unwrap();
    let clone = catalogue.clone();
    assert!(catalogue.belongs_to(&bank));
    assert_eq!(
        catalogue.selection().unwrap().handshake_mode,
        RsdHandshakeMode::Direct
    );
    drop(catalogue);
    assert!(bank.available_permits() < 128 * 1024 * 1024);
    drop(clone);
    assert_eq!(bank.available_permits(), 128 * 1024 * 1024);
}

#[tokio::test]
async fn raw_h2_returned_bytes_keep_the_grant_after_framer_drop() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        peer.write_all(&frame(1, 4, 1, &[])).await.unwrap();
        peer.write_all(&frame(0, 1, 1, b"kept")).await.unwrap();
    });
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let mut framer = H2Framer::connect_with_bank(client, Some(&bank))
        .await
        .unwrap();
    let bytes = framer.read_client_server(4).await.unwrap();
    let last = bytes.slice(1..2);
    drop(bytes);
    drop(framer);
    task.await.unwrap();
    assert_eq!(bank.available_permits(), quota - 4);
    assert_eq!(&last[..], b"e");
    drop(last);
    assert_eq!(bank.available_permits(), quota);
}

#[tokio::test]
async fn cancelled_partial_raw_frame_releases_payload_and_invalidates_original_framer() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let (finish, finished) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        let partial = frame(0, 0, 1, &[0; 32]);
        peer.write_all(&partial[..10]).await.unwrap();
        finished.await.unwrap();
        let mut byte = [0];
        assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
    });
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let admitted_payload = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = Arc::clone(&admitted_payload);
    crate::xpc::raw_storage::PAYLOAD_ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |len| {
            if len == 32 {
                observed.store(true, Ordering::SeqCst);
            }
        }))
    });
    let mut framer = H2Framer::connect_with_bank(client, Some(&bank))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), framer.read_client_server(32))
            .await
            .is_err()
    );
    assert!(!framer.is_usable());
    crate::xpc::raw_storage::PAYLOAD_ADMISSION.with(|hook| hook.borrow_mut().take());
    assert!(
        admitted_payload.load(Ordering::SeqCst),
        "did not reach the original partial payload read"
    );
    assert_eq!(bank.available_permits(), quota);
    drop(framer);
    finish.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn raw_payload_bank_denial_is_final_without_another_connector_call() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
        // The fixed checked selection vector fits this bank, this raw frame
        // does not. Supplying only its header proves denial before payload IO.
        let header = frame(0, 0, 1, &[0; 4096]);
        peer.write_all(&header[..9]).await.unwrap();
        let mut drain = [0; 1024];
        while peer.read(&mut drain).await.unwrap() != 0 {}
    });
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let held = Arc::new(std::sync::Mutex::new(None));
    let hold = Arc::clone(&held);
    let target_bank = Arc::clone(&bank);
    crate::xpc::raw_storage::PAYLOAD_ADMISSION.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |len| {
            if len == 4096 {
                let take = target_bank.available_permits() - (len - 1);
                *hold.lock().unwrap() = Some(
                    Arc::clone(&target_bank)
                        .try_acquire_many_owned(take.try_into().unwrap())
                        .unwrap(),
                );
            }
        }))
    });
    let mut client = Some(client);
    let calls = AtomicUsize::new(0);
    let result = checked_handshake_prepared(
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(client.take().expect("admission failure must be final")))
        },
        uuid::Uuid::from_bytes([9; 16]),
        tokio::time::Instant::now() + Duration::from_secs(1),
        &bank,
    )
    .await;
    crate::xpc::raw_storage::PAYLOAD_ADMISSION.with(|hook| hook.borrow_mut().take());
    task.await.unwrap();
    assert_eq!(
        crate::classify_error(&result.expect_err("raw payload admission bypassed")),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        held.lock().unwrap().is_some(),
        "quota did not target actual raw payload creation"
    );
    drop(held.lock().unwrap().take());
    assert_eq!(bank.available_permits(), quota);
}

async fn initial_duplex(
    target: RsdHandshakeMode,
    messages: Vec<Bytes>,
) -> Result<RsdHandshake, XpcError> {
    match initial_duplex_selected(target, messages, false).await? {
        (InitialDirectory::Legacy(directory), _) => Ok(directory),
        _ => unreachable!("legacy fixture selected legacy output"),
    }
}

async fn initial_duplex_selected(
    target: RsdHandshakeMode,
    messages: Vec<Bytes>,
    owned: bool,
) -> Result<(InitialDirectory, Arc<Semaphore>), XpcError> {
    let modes = [
        RsdHandshakeMode::Direct,
        RsdHandshakeMode::Queued,
        RsdHandshakeMode::Legacy,
        RsdHandshakeMode::Passive,
    ];
    let target_index = modes.iter().position(|mode| *mode == target).unwrap();
    assert!(messages.len() <= target_index + 1);
    let first_reply = target_index + 1 - messages.len();
    let mut messages = messages.into_iter();
    let mut clients = VecDeque::new();
    let mut peers = Vec::new();
    for (attempt, mode) in modes.into_iter().enumerate().take(target_index + 1) {
        let (client, peer) = tokio::io::duplex(64 * 1024);
        clients.push_back(client);
        let message = (attempt >= first_reply).then(|| messages.next().unwrap());
        peers.push(tokio::spawn(async move {
            match message {
                Some(message) => fixture(peer, mode, message).await,
                None => {
                    drop(peer);
                    Ok(())
                }
            }
        }));
    }
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let allocations = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = allocations.clone();
    let observed_bank = bank.clone();
    let observer = owned.then(|| {
        crate::device::prepared_rsd::allocation_observer(move |layout, pointer| {
            observed
                .lock()
                .unwrap()
                .push((layout, pointer, observed_bank.available_permits()));
        })
    });
    let directory = owned
        .then(|| OwnedRsdHandshake::prepare(&bank))
        .transpose()?;
    let original = directory
        .as_ref()
        .map(|directory| (directory.allocation_address(), directory.known_bytes()));
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        handshake_attempts_selected(
            || std::future::ready(Ok(clients.pop_front().expect("unexpected extra mode"))),
            uuid::Uuid::from_bytes([9; 16]),
            Some(&bank),
            directory,
        ),
    )
    .await;
    drop(observer);
    if let Some((pointer, bytes)) = original {
        let allocations = allocations.lock().unwrap();
        let (layout, allocated, available) = allocations[0];
        assert_eq!(
            (allocated, layout.size(), available),
            (pointer, bytes, quota - bytes)
        );
        assert_eq!(
            allocations
                .iter()
                .filter(|(other, ..)| *other == layout)
                .count(),
            1,
            "all initial modes must share one original root"
        );
        if let Ok(Ok(InitialDirectory::Owned(directory))) = &result {
            assert_eq!(directory.allocation_address(), pointer);
        }
    }
    let all_used = clients.is_empty();
    // An early-return regression must close the unused peers before joining;
    // otherwise their preface read would outlive the probe's outer timeout.
    drop(clients);
    let joined = tokio::time::timeout(
        Duration::from_secs(1),
        futures_util::future::join_all(peers.iter_mut()),
    )
    .await;
    let joined = match joined {
        Ok(joined) => joined,
        Err(_) => {
            for peer in &peers {
                peer.abort();
            }
            futures_util::future::join_all(peers).await;
            panic!("initial fixture peers did not close after the probe");
        }
    };
    assert!(
        all_used,
        "initial fallback failed to use the fresh transport"
    );
    for result in joined {
        result.unwrap().unwrap();
    }
    let result = result.expect("fixture must not need the normal response timeout");
    let retained = match &result {
        Ok(InitialDirectory::Owned(directory)) => directory.known_bytes(),
        _ => 0,
    };
    assert_eq!(bank.available_permits(), quota - retained);
    result.map(|directory| (directory, bank))
}

#[tokio::test]
async fn initial_all_four_modes_keep_legacy_zero_port_and_return_all_scratch_grants() {
    for mode in [
        RsdHandshakeMode::Direct,
        RsdHandshakeMode::Queued,
        RsdHandshakeMode::Legacy,
        RsdHandshakeMode::Passive,
    ] {
        let directory = initial_duplex(mode, vec![handshake(XpcValue::Uint64(0))])
            .await
            .unwrap();
        assert_eq!(directory.udid, "fixture-only-udid");
        assert_eq!(directory.services["fixture.service"].port, 0);
        assert_eq!(
            directory.services["fixture.service"].features,
            ["fixture-feature"]
        );
    }
}

#[cfg(feature = "mdns")]
#[tokio::test]
async fn initial_direct_tcp_uses_original_bank_and_keeps_bare_directory_usable() {
    let listener = tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        fixture(
            socket,
            RsdHandshakeMode::Direct,
            handshake(XpcValue::Uint64(0)),
        )
        .await
    });
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let directory = tokio::time::timeout(
        Duration::from_secs(2),
        handshake_with_bank(Ipv6Addr::LOCALHOST, port, Some(&bank)),
    )
    .await
    .unwrap()
    .unwrap();
    peer.await.unwrap().unwrap();
    assert_eq!(bank.available_permits(), quota);
    assert_eq!(directory.services["fixture.service"].port, 0);
}

fn initial_missing_uid() -> Bytes {
    encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(IndexMap::from_iter([(
            "MessageType".into(),
            XpcValue::String("Handshake".into()),
        )]))),
    })
    .unwrap()
}

#[tokio::test]
async fn initial_malformed_handshake_still_skips_to_valid_message_in_same_mode() {
    let malformed = initial_missing_uid();
    let mut messages = malformed.to_vec();
    messages.extend_from_slice(&handshake(XpcValue::Uint64(12345)));
    let directory = initial_duplex(RsdHandshakeMode::Direct, vec![Bytes::from(messages)])
        .await
        .unwrap();
    assert_eq!(directory.services["fixture.service"].port, 12345);
}

#[tokio::test]
async fn initial_six_semantic_failures_exhaust_mode_before_seventh_valid_message() {
    let malformed = initial_missing_uid();
    let mut messages = Vec::new();
    for _ in 0..6 {
        messages.extend_from_slice(&malformed);
    }
    messages.extend_from_slice(&handshake(XpcValue::Uint64(11111)));
    let directory = initial_duplex(
        RsdHandshakeMode::Queued,
        vec![Bytes::from(messages), handshake(XpcValue::Uint64(22222))],
    )
    .await
    .unwrap();
    assert_eq!(directory.services["fixture.service"].port, 22222);
}

#[tokio::test]
async fn initial_six_semantic_failures_retain_last_parser_error() {
    let malformed = initial_missing_uid();
    let mut messages = Vec::new();
    for _ in 0..6 {
        messages.extend_from_slice(&malformed);
    }
    let error = initial_duplex(RsdHandshakeMode::Passive, vec![Bytes::from(messages)])
        .await
        .unwrap_err();
    assert!(matches!(&error, XpcError::Tls(message) if message == "RSD: missing UniqueDeviceID"));
}

#[tokio::test]
async fn initial_wire_decode_failure_uses_fresh_fallback_without_semantic_skip() {
    let mut invalid = handshake(XpcValue::Uint64(12345)).to_vec();
    invalid[24..28].copy_from_slice(&0u32.to_le_bytes());
    let mut messages = invalid;
    messages.extend_from_slice(&handshake(XpcValue::Uint64(11111)));
    let directory = initial_duplex(
        RsdHandshakeMode::Queued,
        vec![Bytes::from(messages), handshake(XpcValue::Uint64(22222))],
    )
    .await
    .unwrap();
    assert_eq!(directory.services["fixture.service"].port, 22222);
}

#[tokio::test]
async fn initial_raw_pool_refusal_is_final_and_releases_the_original_transport() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).await.unwrap();
    });
    let bank = Arc::new(Semaphore::new(0));
    let mut client = Some(client);
    let calls = AtomicUsize::new(0);
    let error = handshake_attempts(
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(client.take().expect("quota refusal must be final")))
        },
        uuid::Uuid::from_bytes([9; 16]),
        Some(&bank),
    )
    .await
    .unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), 0);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn initial_cancelled_body_read_releases_admitted_body_and_original_socket() {
    let (client, mut peer) = tokio::io::duplex(4096);
    let (sent, ready) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        read_frame(&mut peer).await.unwrap();
        peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
        loop {
            let (kind, _, bytes) = read_frame(&mut peer).await.unwrap();
            if kind == 0 {
                let message = decode_message(Bytes::from(bytes)).unwrap();
                if message
                    .body
                    .as_ref()
                    .and_then(XpcValue::as_dict)
                    .and_then(|dictionary| dictionary.get("MessageType"))
                    .and_then(XpcValue::as_str)
                    == Some("Handshake")
                {
                    break;
                }
            }
        }
        let wire = handshake(XpcValue::Uint64(12345));
        peer.write_all(&frame(1, 4, 1, &[])).await.unwrap();
        peer.write_all(&frame(0, 0, 1, &wire[..25])).await.unwrap();
        sent.send(()).unwrap();
        let mut tail = [0; 1024];
        while peer.read(&mut tail).await.unwrap() != 0 {}
    });
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let mut client = Some(client);
    let mut probe = Box::pin(handshake_attempts(
        || std::future::ready(Ok(client.take().expect("single in-flight probe"))),
        uuid::Uuid::from_bytes([9; 16]),
        Some(&bank),
    ));
    tokio::select! {
        result = &mut probe => panic!("incomplete reply completed: {result:?}"),
        result = ready => result.unwrap(),
    }
    assert!(futures_util::poll!(&mut probe).is_pending());
    assert!(bank.available_permits() < quota);
    drop(probe);
    assert_eq!(bank.available_permits(), quota);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn owned_initial_all_four_modes_export_original_storage_after_each_fresh_fallback() {
    for mode in [
        RsdHandshakeMode::Direct,
        RsdHandshakeMode::Queued,
        RsdHandshakeMode::Legacy,
        RsdHandshakeMode::Passive,
    ] {
        let (directory, bank) =
            initial_duplex_selected(mode, vec![handshake(XpcValue::Uint64(0))], true)
                .await
                .unwrap();
        let InitialDirectory::Owned(directory) = directory else {
            panic!("owned selection lost")
        };
        assert_eq!(directory.udid(), "fixture-only-udid");
        let paid = directory.known_bytes();
        let name = directory.get_service("fixture.service").unwrap();
        let pointer = name.name().as_ptr();
        let exported = directory.clone().into_services().next().unwrap();
        drop((directory, name));
        bank.close();
        assert_eq!(exported.name().as_ptr(), pointer);
        assert_eq!(exported.port(), 0);
        assert_eq!(exported.features().collect::<Vec<_>>(), ["fixture-feature"]);
        assert_eq!(bank.available_permits(), 16 * 1024 * 1024 - paid);
        drop(exported);
        assert_eq!(bank.available_permits(), 16 * 1024 * 1024);
    }
}

#[tokio::test]
async fn owned_initial_root_refusal_precedes_identity_and_connector_poll() {
    let probe = Arc::new(Semaphore::new(1024 * 1024));
    let root = OwnedRsdHandshake::prepare(&probe).unwrap();
    let bytes = root.known_bytes();
    drop(root);
    for closed in [false, true] {
        let bank = Arc::new(Semaphore::new(if closed { bytes } else { bytes - 1 }));
        if closed {
            bank.close();
        }
        let before = bank.available_permits();
        let identity_polls = AtomicUsize::new(0);
        let dials = AtomicUsize::new(0);
        let observer = crate::device::prepared_rsd::allocation_observer(|_, _| {
            panic!("refused initial root allocated a backing");
        });
        let error = owned_handshake_with_identity_and_bank(
            || {
                dials.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err::<tokio::io::DuplexStream, _>(XpcError::NotConnected))
            },
            &bank,
            async {
                identity_polls.fetch_add(1, Ordering::SeqCst);
                Ok(uuid::Uuid::from_bytes([9; 16]))
            },
        )
        .await
        .unwrap_err();
        drop(observer);
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(identity_polls.load(Ordering::SeqCst), 0);
        assert_eq!(dials.load(Ordering::SeqCst), 0);
        assert_eq!(bank.available_permits(), before);
    }
}

#[tokio::test]
async fn owned_initial_pending_identity_drop_releases_the_actual_prepaid_root_without_dial() {
    use std::future::Future;
    let probe = Arc::new(Semaphore::new(1024 * 1024));
    let root = OwnedRsdHandshake::prepare(&probe).unwrap();
    let bytes = root.known_bytes();
    drop(root);
    let bank = Arc::new(Semaphore::new(bytes));
    let dials = AtomicUsize::new(0);
    let identity_polls = AtomicUsize::new(0);
    let allocations = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed = allocations.clone();
    let observed_bank = bank.clone();
    let observer = crate::device::prepared_rsd::allocation_observer(move |layout, pointer| {
        assert_eq!(observed_bank.available_permits(), 0);
        observed.lock().unwrap().push((layout, pointer));
    });
    let mut operation = Box::pin(owned_handshake_with_identity_and_bank(
        || {
            dials.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err::<tokio::io::DuplexStream, _>(XpcError::NotConnected))
        },
        &bank,
        async {
            identity_polls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(bank.available_permits(), 0);
            assert_eq!(allocations.lock().unwrap().len(), 1);
            std::future::pending::<Result<uuid::Uuid, XpcError>>().await
        },
    ));
    std::future::poll_fn(|cx| {
        assert!(operation.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(allocations.lock().unwrap()[0].0.size(), bytes);
    assert_eq!(identity_polls.load(Ordering::SeqCst), 1);
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    drop(operation);
    drop(observer);
    assert_eq!(bank.available_permits(), bytes);
}

#[tokio::test]
async fn owned_initial_pending_bootstrap_drop_closes_the_same_socket_and_joins_before_refund_check()
{
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let (client, mut peer) = tokio::io::duplex(64 * 1024);
    let (preface_sent, preface_seen) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
        preface_sent.send(()).unwrap();
        let mut tail = [0; 1024];
        loop {
            if peer.read(&mut tail).await.unwrap() == 0 {
                break;
            }
        }
    });
    let dials = AtomicUsize::new(0);
    let mut client = Some(client);
    let mut operation = Box::pin(owned_handshake_with_identity_and_bank(
        || {
            dials.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(client.take().expect("only the original socket")))
        },
        &bank,
        std::future::ready(Ok(uuid::Uuid::from_bytes([9; 16]))),
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            result = &mut operation => panic!("bootstrap completed before peer SETTINGS: {result:?}"),
            seen = preface_seen => seen.unwrap(),
        }
    }).await.unwrap();
    assert_eq!(dials.load(Ordering::SeqCst), 1);
    assert!(bank.available_permits() < quota);
    drop(operation);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bank.available_permits(), quota);
}

#[tokio::test]
async fn owned_initial_six_semantic_failures_keep_fresh_queued_transport_and_owned_export() {
    let mut bad = Vec::new();
    for _ in 0..6 {
        bad.extend_from_slice(&initial_missing_uid());
    }
    bad.extend_from_slice(&handshake(XpcValue::Uint64(1)));
    let (directory, bank) = initial_duplex_selected(
        RsdHandshakeMode::Queued,
        vec![Bytes::from(bad), handshake(XpcValue::Uint64(42))],
        true,
    )
    .await
    .unwrap();
    let InitialDirectory::Owned(directory) = directory else {
        panic!("owned selection lost")
    };
    assert_eq!(directory.get_port("fixture.service"), Some(42));
    drop(directory);
    assert_eq!(bank.available_permits(), 16 * 1024 * 1024);
}
#[cfg(feature = "mdns")]
#[tokio::test]
async fn owned_initial_direct_tcp_pre_tunnel_query_uses_original_bank_and_export() {
    let listener = tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        fixture(
            stream,
            RsdHandshakeMode::Direct,
            handshake(XpcValue::Uint64(0)),
        )
        .await
    });
    let bank = Arc::new(Semaphore::new(16 * 1024 * 1024));
    let directory = tokio::time::timeout(
        Duration::from_secs(2),
        initial_handshake_with_bank(Ipv6Addr::LOCALHOST, port, Some(&bank)),
    )
    .await
    .unwrap()
    .unwrap();
    peer.await.unwrap().unwrap();
    let InitialDirectory::Owned(directory) = directory else {
        panic!("pre-tunnel query did not select owned output")
    };
    assert!(directory.belongs_to(&bank));
    assert_eq!(directory.get_port("fixture.service"), Some(0));
    assert!(bank.available_permits() < 16 * 1024 * 1024);
    drop(directory);
    assert_eq!(bank.available_permits(), 16 * 1024 * 1024);
}

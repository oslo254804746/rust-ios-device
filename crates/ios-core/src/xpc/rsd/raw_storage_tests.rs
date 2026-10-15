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

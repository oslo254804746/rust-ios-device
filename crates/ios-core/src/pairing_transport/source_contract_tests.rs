use super::*;
use crate::xpc::{
    h2_raw::{H2Error, H2Framer, STREAM_CLIENT_SERVER},
    message::{decode_message, encode_message, flags, XpcMessage, XpcValue},
};
use crate::{classify_error, ErrorClass};
use std::{error::Error, io};
use tokio::{
    io::{duplex, AsyncReadExt, AsyncWriteExt, DuplexStream},
    time::{timeout, Duration},
};

async fn connected_pair() -> (H2Framer<DuplexStream>, DuplexStream) {
    let (client, mut server) = duplex(8192);
    let peer = tokio::spawn(async move {
        let mut preface = [0; 24];
        server.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);
        let mut setup = [0; 34];
        server.read_exact(&mut setup).await.unwrap();
        server
            .write_all(&super::tests::settings_frame())
            .await
            .unwrap();
        let mut ack = [0; 9];
        server.read_exact(&mut ack).await.unwrap();
        assert_eq!(ack, super::tests::settings_ack_frame().as_slice());
        server
    });
    let framer = timeout(Duration::from_secs(1), H2Framer::connect(client))
        .await
        .unwrap()
        .unwrap();
    (
        framer,
        timeout(Duration::from_secs(1), peer)
            .await
            .unwrap()
            .unwrap(),
    )
}

fn outer(error: &PairingTransportError, kind: io::ErrorKind, class: ErrorClass) -> &io::Error {
    assert_eq!(classify_error(error), class);
    let PairingTransportError::Io(source) = error else {
        panic!("manual pairing source lost")
    };
    assert_eq!(source.kind(), kind);
    assert!(source.source().is_some());
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    source
}

#[tokio::test]
async fn actual_send_on_closed_h2_retains_original_write_error() {
    let (mut framer, peer) = connected_pair().await;
    drop(peer);
    let error = timeout(
        Duration::from_secs(1),
        send_xpc(&mut framer, &xpc_dict(&[]), 1),
    )
    .await
    .unwrap()
    .unwrap_err();
    let native = outer(&error, io::ErrorKind::BrokenPipe, ErrorClass::Disconnected)
        .source()
        .unwrap()
        .downcast_ref::<H2Error>()
        .unwrap();
    let H2Error::Io(native) = native else {
        panic!("write cause lost")
    };
    assert_eq!(native.kind(), io::ErrorKind::BrokenPipe);
}

#[tokio::test]
async fn actual_receive_keeps_header_body_and_decoder_causes() {
    for mode in 0..3 {
        let (mut framer, mut peer) = connected_pair().await;
        if mode != 0 {
            let mut payload = encode_message(&XpcMessage {
                flags: flags::ALWAYS_SET | flags::DATA,
                msg_id: 1,
                body: Some(XpcValue::Null),
            })
            .unwrap()
            .to_vec();
            if mode == 1 {
                payload.truncate(24);
            } else {
                payload[0] ^= 1;
            }
            peer.write_all(&super::tests::data_frame(STREAM_CLIENT_SERVER, &payload))
                .await
                .unwrap();
        }
        peer.shutdown().await.unwrap();
        drop(peer);
        let error = timeout(Duration::from_secs(1), recv_xpc(&mut framer))
            .await
            .unwrap()
            .unwrap_err();
        if mode < 2 {
            let source = outer(&error, io::ErrorKind::UnexpectedEof, ErrorClass::Io);
            let H2Error::Io(native) = source.source().unwrap().downcast_ref::<H2Error>().unwrap()
            else {
                panic!("read cause lost")
            };
            assert_eq!(native.kind(), io::ErrorKind::UnexpectedEof);
        } else {
            assert!(outer(&error, io::ErrorKind::Other, ErrorClass::Protocol)
                .source()
                .unwrap()
                .is::<crate::XpcError>());
        }
    }
}

#[test]
fn short_received_header_returns_native_length_error_without_panicking() {
    for length in [0, 7, 8, 15] {
        let error = xpc_body_len(&vec![0; length]).unwrap_err();
        assert!(
            outer(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol)
                .source()
                .unwrap()
                .is::<std::array::TryFromSliceError>()
        );
    }
    assert_eq!(xpc_body_len(&[0; 24]).unwrap(), 0);
}

#[tokio::test]
async fn actual_connect_and_bootstrap_faults_keep_h2_and_xpc_sources() {
    let (stream, peer) = duplex(64);
    drop(peer);
    let native = H2Framer::connect(stream)
        .await
        .err()
        .expect("closed H2 connection");
    let error = error_context::h2("connect manual pairing H2", native);
    assert!(
        outer(&error, io::ErrorKind::BrokenPipe, ErrorClass::Disconnected)
            .source()
            .unwrap()
            .is::<H2Error>()
    );

    let (mut framer, peer) = connected_pair().await;
    drop(peer);
    let error = timeout(Duration::from_secs(1), bootstrap_remote_xpc(&mut framer))
        .await
        .unwrap()
        .unwrap_err();
    let source = outer(&error, io::ErrorKind::BrokenPipe, ErrorClass::Disconnected);
    assert!(source.source().unwrap().is::<crate::XpcError>());
    let mut current: Option<&(dyn Error + 'static)> = source.source();
    let mut h2_found = false;
    for _ in 0..32 {
        let Some(error) = current else { break };
        h2_found |= error.is::<H2Error>();
        current = error.source();
    }
    assert!(h2_found);
}

#[test]
fn inner_m6_and_private_seed_errors_survive_the_public_error_adapter() {
    use crate::lockdown::pairing::{HostIdentity, PairingError};
    let native = verify_device_info_response(&[3; 32], &[0; 16]).unwrap_err();
    let error = error_context::pairing("verify manual pairing device info", native);
    let source = outer(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol);
    let PairingError::Io(inner) = source
        .source()
        .unwrap()
        .downcast_ref::<PairingError>()
        .unwrap()
    else {
        panic!("inner pairing cause lost")
    };
    assert!(inner
        .source()
        .unwrap()
        .is::<chacha20poly1305::aead::Error>());

    let native = HostIdentity::from_private_key_bytes("owned-private", &[7; 31])
        .err()
        .expect("invalid seed");
    let error = error_context::pairing("owned identity adapter", native);
    let source = outer(
        &error,
        io::ErrorKind::InvalidInput,
        ErrorClass::InvalidInput,
    );
    let PairingError::Io(inner) = source
        .source()
        .unwrap()
        .downcast_ref::<PairingError>()
        .unwrap()
    else {
        panic!("inner seed cause lost")
    };
    assert!(inner
        .source()
        .unwrap()
        .is::<std::array::TryFromSliceError>());

    let native = SrpSession::new(&[1; 8], &[0])
        .err()
        .expect("invalid SRP public key");
    let error = error_context::pairing("compute manual pairing SRP", native);
    assert!(matches!(
        outer(&error, io::ErrorKind::Other, ErrorClass::Unknown)
            .source()
            .unwrap()
            .downcast_ref::<PairingError>(),
        Some(PairingError::Crypto(_))
    ));
}

#[test]
fn source_adapters_preserve_native_kinds_named_xpc_and_opaque_categories() {
    for (kind, class) in [
        (io::ErrorKind::TimedOut, ErrorClass::Timeout),
        (io::ErrorKind::BrokenPipe, ErrorClass::Disconnected),
        (
            io::ErrorKind::PermissionDenied,
            ErrorClass::PermissionDenied,
        ),
        (io::ErrorKind::InvalidInput, ErrorClass::InvalidInput),
        (io::ErrorKind::InvalidData, ErrorClass::Protocol),
        (io::ErrorKind::NotFound, ErrorClass::NotFound),
        (io::ErrorKind::Interrupted, ErrorClass::Io),
        (io::ErrorKind::UnexpectedEof, ErrorClass::Io),
        (io::ErrorKind::Other, ErrorClass::Io),
    ] {
        let error = error_context::xpc(
            "owned XPC adapter",
            crate::XpcError::Io(io::Error::new(kind, "owned-private cancelled timeout")),
        );
        let source = outer(&error, kind, class);
        let crate::XpcError::Io(native) = source
            .source()
            .unwrap()
            .downcast_ref::<crate::XpcError>()
            .unwrap()
        else {
            panic!("XPC IO cause lost")
        };
        assert_eq!(native.kind(), kind);
        assert_eq!(native.to_string(), "owned-private cancelled timeout");
    }
    for (native, class) in [
        (crate::XpcError::NotConnected, ErrorClass::Disconnected),
        (
            crate::XpcError::ServiceNotFound("owned-private".into()),
            ErrorClass::ServiceMissing,
        ),
        (
            crate::XpcError::Tls("owned-private".into()),
            ErrorClass::Protocol,
        ),
    ] {
        let error = error_context::xpc("owned XPC adapter", native);
        assert!(outer(&error, io::ErrorKind::Other, class)
            .source()
            .unwrap()
            .is::<crate::XpcError>());
    }
    let error = error_context::native(
        io::ErrorKind::Other,
        "encrypt manual pairing unlock request",
        chacha20poly1305::aead::Error,
    );
    assert!(outer(&error, io::ErrorKind::Other, ErrorClass::Unknown)
        .source()
        .unwrap()
        .is::<chacha20poly1305::aead::Error>());
}

#[tokio::test]
async fn real_owned_rsd_refusal_and_elapsed_keep_original_objects() {
    let socket = tokio::net::TcpSocket::new_v6().unwrap();
    socket.bind((Ipv6Addr::LOCALHOST, 0).into()).unwrap();
    let native = timeout(
        // Keep the original native refusal through all three RSD attempts;
        // Winsock delivers loopback refusal after ~2s per attempt.
        Duration::from_secs(if cfg!(windows) { 10 } else { 1 }),
        crate::xpc::rsd::handshake(Ipv6Addr::LOCALHOST, socket.local_addr().unwrap().port()),
    )
    .await
    .unwrap()
    .unwrap_err();
    let error = error_context::xpc("discover manual pairing service", native);
    let source = outer(&error, io::ErrorKind::ConnectionRefused, ErrorClass::Io);
    assert!(source.source().unwrap().is::<crate::XpcError>());
    let elapsed = timeout(Duration::from_millis(1), std::future::pending::<()>())
        .await
        .unwrap_err();
    let error = error_context::dial_timeout(elapsed);
    assert!(outer(&error, io::ErrorKind::TimedOut, ErrorClass::Timeout)
        .source()
        .unwrap()
        .is::<tokio::time::error::Elapsed>());
}

async fn read_request(peer: &mut DuplexStream) -> XpcMessage {
    for _ in 0..4 {
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let length =
            ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        assert!(length <= 4096);
        assert_eq!(
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            STREAM_CLIENT_SERVER
        );
        let mut body = vec![0; length];
        peer.read_exact(&mut body).await.unwrap();
        if header[3] == 0 {
            return decode_message(body.into()).unwrap();
        }
        assert_eq!(header[3], 1);
    }
    panic!("request missing");
}

#[tokio::test]
async fn actual_unlock_rpc_keeps_authentication_json_and_success_behavior() {
    for mode in 0..4 {
        let (mut framer, mut peer) = connected_pair().await;
        let remote = tokio::spawn(async move {
            let request = timeout(Duration::from_secs(1), read_request(&mut peer))
                .await
                .unwrap();
            assert_eq!(request.msg_id, 5);
            let body = request.body.unwrap();
            let value = body.as_dict().unwrap()["value"].as_dict().unwrap();
            assert_eq!(value["sequenceNumber"].as_uint64(), Some(9));
            let encrypted = decode_encrypted_response(body).unwrap();
            let nonce = make_encrypted_nonce(0);
            let cipher = ChaCha20Poly1305::new((&[1; 32]).into());
            let plaintext = cipher.decrypt(&nonce.into(), encrypted.as_slice()).unwrap();
            let request: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
            assert_eq!(
                request,
                serde_json::json!({"request":{"_0":{"createRemoteUnlockKey":{}}}})
            );
            let response = if mode == 1 {
                b"owned-private{".to_vec()
            } else {
                serde_json::to_vec(
                    &serde_json::json!({"response":{"_1":{"createRemoteUnlockKey":{
                        "hostKey": if mode == 2 { "owned-private" } else { "" }
                    }}}}),
                )
                .unwrap()
            };
            let cipher = ChaCha20Poly1305::new((&[2; 32]).into());
            let encrypted = if mode == 0 {
                vec![0; 16]
            } else {
                cipher.encrypt(&nonce.into(), response.as_slice()).unwrap()
            };
            let payload = encode_message(&XpcMessage {
                flags: flags::ALWAYS_SET | flags::DATA,
                msg_id: 5,
                body: Some(build_encrypted_request(&encrypted, 9)),
            })
            .unwrap();
            peer.write_all(&super::tests::data_frame(STREAM_CLIENT_SERVER, &payload))
                .await
                .unwrap();
            peer.shutdown().await.unwrap();
        });
        let mut sequence = 9;
        let result = timeout(
            Duration::from_secs(1),
            create_remote_unlock_key(&mut framer, &[1; 32], &[2; 32], &mut sequence),
        )
        .await
        .unwrap();
        timeout(Duration::from_secs(1), remote)
            .await
            .unwrap()
            .unwrap();
        drop(framer);
        assert_eq!(sequence, 10);
        if mode < 2 {
            let error = result.unwrap_err();
            let source = outer(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol)
                .source()
                .unwrap();
            if mode == 0 {
                assert!(source.is::<chacha20poly1305::aead::Error>());
            } else {
                assert!(source.is::<serde_json::Error>());
            }
        } else {
            assert_eq!(
                result.unwrap().as_deref(),
                if mode == 2 {
                    Some("owned-private")
                } else {
                    None
                }
            );
        }
    }
}

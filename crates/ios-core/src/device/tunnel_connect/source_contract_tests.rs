//! Owned native causes, without pairing or real credential-directory access.

use super::protocol::{extract_remote_pairing_stream_encrypted, extract_remote_pairing_tlv};

use std::error::Error;

#[test]
fn native_public_key_length_failure_stays_protocol_and_exact_length_succeeds() {
    for phase in [
        "decode direct pairing public key",
        "decode remote pairing public key",
    ] {
        for length in [0, 31, 33] {
            let error =
                super::pairing::decode_pairing_public_key(&vec![7; length], phase).unwrap_err();
            assert_eq!(error.class(), crate::ErrorClass::Protocol);
            let crate::CoreError::Io(outer) = error else {
                panic!("slice parser cause lost");
            };
            assert!(outer
                .source()
                .unwrap()
                .is::<std::array::TryFromSliceError>());
        }
        assert_eq!(
            super::pairing::decode_pairing_public_key(&[7; 32], phase).unwrap(),
            [7; 32]
        );
    }
}

#[test]
fn remote_base64_decode_retains_native_cause_and_protocol_category() {
    let tlv = serde_json::json!({
        "message": { "plain": { "_0": { "event": { "_0": {
            "pairingData": { "_0": { "data": "owned-private!" } }
        } } } } }
    });
    let encrypted = serde_json::json!({"message":{"streamEncrypted":{"_0":"owned-private!"}}});
    for error in [
        extract_remote_pairing_tlv(&tlv).unwrap_err(),
        extract_remote_pairing_stream_encrypted(&encrypted).unwrap_err(),
    ] {
        assert_eq!(error.class(), crate::ErrorClass::Protocol);
        let crate::CoreError::Io(outer) = &error else {
            panic!("native Base64 cause discarded");
        };
        assert!(std::error::Error::source(outer)
            .unwrap()
            .is::<base64::DecodeError>());
        assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    }
}

#[test]
fn actual_local_credential_loaders_keep_input_sources_and_key_mismatch_protocol() {
    use crate::{
        credentials::{PersistedCredentials, RemotePairingRecord},
        lockdown::pairing::{HostIdentity, PairingError},
    };
    let identity = HostIdentity::generate();
    let record = || RemotePairingRecord {
        public_key: identity.public_key_bytes(),
        private_key: identity.private_key_bytes(),
        remote_unlock_host_key: None,
        peer_alt_irk: None,
    };
    let persisted = || PersistedCredentials {
        remote_identifier: Some("owned-private".into()),
        host_identifier: identity.identifier.clone(),
        host_public_key_hex: hex::encode(identity.public_key_bytes()),
        host_private_key_hex: Some("owned-private!".into()),
        remote_unlock_host_key: None,
        device_address: "fd00::1".into(),
        rsd_port: 1,
    };
    let error = super::credentials::load_ios_rs_remote_pairing_credentials(
        "owned-private",
        record(),
        persisted(),
    )
    .err()
    .expect("invalid hex");
    assert_eq!(error.class(), crate::ErrorClass::InvalidInput);
    let crate::CoreError::Io(outer) = &error else {
        panic!("hex cause lost");
    };
    assert!(outer.source().unwrap().is::<hex::FromHexError>());
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    let mut invalid = record();
    invalid.private_key.pop();
    let first = super::credentials::load_ios_rs_remote_pairing_credentials(
        "owned-private",
        invalid.clone(),
        persisted(),
    )
    .err()
    .expect("bad identity");
    let second = super::credentials::load_pymobiledevice3_remote_pairing_credentials(
        "owned-private",
        "owned-private",
        invalid,
        std::path::Path::new("unused-owned-path"),
    )
    .err()
    .expect("bad identity");
    for error in [first, second] {
        assert_eq!(error.class(), crate::ErrorClass::InvalidInput);
        let crate::CoreError::Io(outer) = &error else {
            panic!("identity cause lost");
        };
        let PairingError::Io(identity_source) = outer
            .source()
            .unwrap()
            .downcast_ref::<PairingError>()
            .unwrap()
        else {
            panic!("inner identity cause lost");
        };
        assert_eq!(identity_source.kind(), std::io::ErrorKind::InvalidInput);
        assert!(identity_source
            .source()
            .unwrap()
            .is::<std::array::TryFromSliceError>());
        assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    }
    let mut mismatch = persisted();
    mismatch.host_private_key_hex = Some(hex::encode([0; 32]));
    let error = super::credentials::load_ios_rs_remote_pairing_credentials(
        "owned-private",
        record(),
        mismatch,
    )
    .err()
    .expect("mismatched key");
    assert!(matches!(error, crate::CoreError::Protocol(_)));
    assert_eq!(error.class(), crate::ErrorClass::Protocol);
}

#[tokio::test]
async fn actual_remote_listener_keeps_base64_or_aead_fault_and_valid_success() {
    use super::{
        pairing::RemotePairingControlChannel,
        protocol::{create_remote_pairing_tcp_listener, make_direct_encrypted_nonce},
    };
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use chacha20poly1305::{aead::Aead, KeyInit};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let session = crate::lockdown::pairing::VerifyPairSession {
        tlv: Vec::new(),
        encryption_key: [1; 32],
        client_key: [2; 32],
        server_key: [3; 32],
    };
    for fault in 0..3 {
        let response = match fault {
            0 => "owned-private!".to_owned(),
            1 => STANDARD.encode([0; 5]),
            _ => {
                let body = serde_json::to_vec(
                    &serde_json::json!({"response":{"_1":{"createListener":{"port":1234}}}}),
                )
                .unwrap();
                STANDARD.encode(
                    chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into())
                        .encrypt((&make_direct_encrypted_nonce(0)).into(), body.as_slice())
                        .unwrap(),
                )
            }
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut magic = [0; 9];
            stream.read_exact(&mut magic).await.unwrap();
            assert_eq!(&magic, b"RPPairing");
            let length = stream.read_u16().await.unwrap() as usize;
            assert!(length <= 4096);
            let mut request = vec![0; length];
            stream.read_exact(&mut request).await.unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["sequenceNumber"], 9);
            let response = serde_json::to_vec(
                &serde_json::json!({"message":{"streamEncrypted":{"_0":response}}}),
            )
            .unwrap();
            stream.write_all(b"RPPairing").await.unwrap();
            stream.write_u16(response.len() as u16).await.unwrap();
            stream.write_all(&response).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        let mut control = RemotePairingControlChannel::connect("127.0.0.1", address.port())
            .await
            .unwrap();
        let result = create_remote_pairing_tcp_listener(&mut control, &session, 9).await;
        drop(control);
        peer.await.unwrap();
        if fault == 2 {
            assert_eq!(result.unwrap(), 1234);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.class(), crate::ErrorClass::Protocol);
            let crate::CoreError::Io(outer) = &error else {
                panic!("decoder/authentication cause lost");
            };
            if fault == 0 {
                assert!(outer.source().unwrap().is::<base64::DecodeError>());
            } else {
                assert!(outer
                    .source()
                    .unwrap()
                    .is::<chacha20poly1305::aead::Error>());
            }
            assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
        }
    }
}

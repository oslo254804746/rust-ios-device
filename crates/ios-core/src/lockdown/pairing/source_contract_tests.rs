use super::*;
use crate::{classify_error, ErrorClass};
use std::{error::Error, io};

fn io_source(error: &PairingError, kind: io::ErrorKind, class: ErrorClass) -> &io::Error {
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<PairingError>();
    let family = match error {
        PairingError::Io(_) => "io",
        PairingError::Crypto(_) => "crypto",
        PairingError::Protocol(_) => "protocol",
    };
    assert_eq!(classify_error(error), class);
    assert_eq!(family, "io");
    let PairingError::Io(source) = error else {
        panic!("native pairing cause lost");
    };
    assert_eq!(source.kind(), kind);
    assert!(source.source().is_some());
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    source
}

#[test]
fn private_seed_length_retains_native_slice_cause_and_valid_identity() {
    for length in [0, 31, 33] {
        let error = HostIdentity::from_private_key_bytes("owned-private", &vec![7; length])
            .err()
            .expect("invalid private seed length");
        assert!(io_source(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput
        )
        .source()
        .unwrap()
        .is::<std::array::TryFromSliceError>());
    }
    let identity = HostIdentity::from_private_key_bytes("owned-private", &[7; 32]).unwrap();
    assert_eq!(identity.identifier, "owned-private");
    assert_eq!(identity.private_key_bytes(), [7; 32]);
    let public = VerifyingKey::from(&identity.signing_key);
    let signature = ed25519_dalek::Signature::from_slice(&identity.sign(b"owned message")).unwrap();
    public.verify_strict(b"owned message", &signature).unwrap();
}

#[test]
fn actual_m6_authentication_faults_retain_aead_cause_and_reject_partial_success() {
    let mut inner = TlvBuffer::new();
    let info = opack::encode(&opack::OpackValue::Dict(vec![(
        opack::OpackValue::String("altIRK".into()),
        opack::OpackValue::Bytes(vec![42; 16]),
    )]))
    .unwrap();
    inner.push_bytes(TYPE_INFO, &info);
    let plaintext = inner.into_bytes();
    let ciphertext = chacha_seal(&[3; 32], b"PS-Msg06", &plaintext).unwrap();
    assert_eq!(
        verify_device_info_response(&[3; 32], &ciphertext).unwrap(),
        Some(vec![42; 16])
    );
    let mut tampered = ciphertext.clone();
    tampered[0] ^= 1;
    let wrong_nonce = chacha_seal(&[3; 32], b"PS-Msg05", &plaintext).unwrap();
    for (key, body) in [
        ([4; 32], ciphertext.as_slice()),
        ([3; 32], tampered.as_slice()),
        ([3; 32], &ciphertext[..8]),
        ([3; 32], wrong_nonce.as_slice()),
    ] {
        let error = verify_device_info_response(&key, body).unwrap_err();
        assert!(
            io_source(&error, io::ErrorKind::InvalidData, ErrorClass::Protocol)
                .source()
                .unwrap()
                .is::<chacha20poly1305::aead::Error>()
        );
    }
}

#[test]
fn actual_device_info_encoding_retains_native_opack_error() {
    let identity =
        HostIdentity::from_private_key_bytes("owned-private".repeat(6000), &[7; 32]).unwrap();
    let error = build_device_info_tlv(&[42; 64], &identity).unwrap_err();
    let source = io_source(&error, io::ErrorKind::Other, ErrorClass::Protocol);
    assert!(matches!(
        source.source().unwrap().downcast_ref::<opack::OpackError>(),
        Some(opack::OpackError::Encode(_))
    ));
    assert!(!error.to_string().contains("owned-private"));
}

#[test]
fn native_crypto_adapter_keeps_input_errors_and_opaque_encryption_unknown() {
    use chacha20poly1305::aead::{AeadInPlace, Buffer};

    fn input<E: Error + Send + Sync + Copy + 'static>(phase: &'static str, native: E) {
        let error = native_error(io::ErrorKind::InvalidInput, phase, native);
        assert!(io_source(
            &error,
            io::ErrorKind::InvalidInput,
            ErrorClass::InvalidInput
        )
        .source()
        .unwrap()
        .is::<E>());
    }
    // These are real library errors passed through the production adapter.
    // The production 32-byte HKDF output/key cannot reach these length faults.
    let mut too_long = vec![0; 255 * 64 + 1];
    let native = Hkdf::<Sha512>::new(None, b"owned input")
        .expand(b"owned info", &mut too_long)
        .unwrap_err();
    input("expand pairing HKDF", native);
    let native = ChaCha20Poly1305::new_from_slice(&[3; 31])
        .err()
        .expect("invalid cipher key length");
    input("initialize pairing cipher", native);

    struct NoTagSpace([u8; 0]);
    impl AsRef<[u8]> for NoTagSpace {
        fn as_ref(&self) -> &[u8] {
            &self.0
        }
    }
    impl AsMut<[u8]> for NoTagSpace {
        fn as_mut(&mut self) -> &mut [u8] {
            &mut self.0
        }
    }
    impl Buffer for NoTagSpace {
        fn extend_from_slice(&mut self, _: &[u8]) -> chacha20poly1305::aead::Result<()> {
            Err(chacha20poly1305::aead::Error)
        }
        fn truncate(&mut self, _: usize) {}
    }
    let cipher = ChaCha20Poly1305::new_from_slice(&[3; 32]).unwrap();
    let nonce = chacha20poly1305::Nonce::from(chacha_nonce(b"PS-Msg05"));
    let native = cipher
        .encrypt_in_place(&nonce, b"", &mut NoTagSpace([]))
        .unwrap_err();
    // A bounded in-place buffer produces the library's opaque encryption error;
    // this does not inject failure into the production Vec encryption call.
    let error = native_error(io::ErrorKind::Other, "seal pairing payload", native);
    assert!(io_source(&error, io::ErrorKind::Other, ErrorClass::Unknown)
        .source()
        .unwrap()
        .is::<chacha20poly1305::aead::Error>());
}

#[test]
fn fixed_hkdf_nonce_and_authenticated_device_info_success_remain_unchanged() {
    // SHA512 extract/expand values independently computed with Python hmac.
    assert_eq!(
        hex::encode(hkdf_sha512(b"session_key", b"salt", b"ClientEncrypt-main").unwrap()),
        "689131765e3c621b0de6163d9d291a8e17324c9b8daa646b4b16cc2f3aa146c6"
    );
    let (client, server) = derive_cipher_keys(b"session_key").unwrap();
    assert_eq!(
        hex::encode(client),
        "94daf52029e840a8ebdd89132adf97248023585e18f70cb55770250e6a042d96"
    );
    assert_eq!(
        hex::encode(server),
        "6cd4c976feb352acbf79fef8365458c14c7a279da62a416b0c2656a204e00b0b"
    );
    assert_eq!(chacha_nonce(b"PS-Msg05"), *b"\0\0\0\0PS-Msg05");
    let identity = HostIdentity::from_private_key_bytes("owned-private", &[7; 32]).unwrap();
    let (tlv, key) = build_device_info_tlv(&[42; 64], &identity).unwrap();
    let outer = TlvBuffer::decode(&tlv);
    assert_eq!(outer[&TYPE_STATE].as_ref(), &[STATE_PHASE5]);
    let plaintext = chacha_open(&key, b"PS-Msg05", &outer[&TYPE_ENCRYPTED_DATA]).unwrap();
    let inner = TlvBuffer::decode(&plaintext);
    assert_eq!(
        inner[&TYPE_IDENTIFIER].as_ref(),
        identity.identifier.as_bytes()
    );
    assert_eq!(
        inner[&TYPE_PUBLIC_KEY].as_ref(),
        identity.public_key_bytes()
    );
    let mut signed = hkdf_sha512(
        &[42; 64],
        b"Pair-Setup-Controller-Sign-Salt",
        b"Pair-Setup-Controller-Sign-Info",
    )
    .unwrap()
    .to_vec();
    signed.extend_from_slice(identity.identifier.as_bytes());
    signed.extend_from_slice(&identity.public_key_bytes());
    let signature = ed25519_dalek::Signature::from_slice(&inner[&TYPE_SIGNATURE]).unwrap();
    VerifyingKey::from(&identity.signing_key)
        .verify_strict(&signed, &signature)
        .unwrap();
    let (info, consumed) = opack::decode(&inner[&TYPE_INFO]).unwrap();
    assert_eq!(consumed, inner[&TYPE_INFO].len());
    let opack::OpackValue::Dict(info) = info else {
        panic!("device info shape changed");
    };
    assert_eq!(info.len(), 7);
}

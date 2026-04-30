//! SRP (Secure Remote Password) pairing for new iOS 17+ devices.
//!
//! Implements the complete pairing handshake that runs when connecting to a new
//! (untrusted) device for the first time. The user must press "Trust" on the device.
//!
//! Service: `com.apple.internal.dt.coredevice.untrusted.tunnelservice`
//! Accessed via: RSD on the device's USB-Ethernet or mDNS IPv6 address (port 58783)
//!
//! Flow:
//! 1. Send handshake (wireProtocolVersion=19, attemptPairVerify=true)
//! 2. Try verifyPair() [if we already have keys] – skip if no keys
//! 3. setupManualPairing() – TLV{typeMethod=0, typeState=1}
//! 4. readDeviceKey() – receive device's SRP salt + public key
//! 5. setupSessionKey() – SRP-3072-SHA512, derive session key
//! 6. exchangeDeviceInfo() – HKDF → Ed25519 sign → ChaCha encrypt device info
//! 7. setupCiphers() – HKDF → two ChaCha20Poly1305 streams
//! 8. createUnlockKey() – final handshake to finish pairing

use crate::proto::{opack, tlv::TlvBuffer};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::rngs::OsRng;
use sha2::{Digest, Sha512};
use uuid::Uuid;

// ── TLV type codes (from go-ios tunnel/tlvbuffer.go) ──────────────────────────

const TYPE_METHOD: u8 = 0x00;
const TYPE_IDENTIFIER: u8 = 0x01;
const TYPE_PUBLIC_KEY: u8 = 0x03;
const TYPE_PROOF: u8 = 0x04;
const TYPE_ENCRYPTED_DATA: u8 = 0x05;
const TYPE_STATE: u8 = 0x06;
const TYPE_SIGNATURE: u8 = 0x0A;
const TYPE_INFO: u8 = 0x11;

// ── Pair state values ─────────────────────────────────────────────────────────

const STATE_START_REQUEST: u8 = 0x01;
const STATE_VERIFY_REQUEST: u8 = 0x03;
const STATE_PHASE5: u8 = 0x05;

// ── Identity (host keys, generated fresh each pairing) ────────────────────────

pub struct HostIdentity {
    pub identifier: String,
    pub signing_key: SigningKey,
}

impl HostIdentity {
    pub fn generate() -> Self {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        Self {
            identifier: Uuid::new_v4().to_string().to_uppercase(),
            signing_key,
        }
    }

    #[cfg(feature = "tunnel")]
    pub fn from_private_key_bytes(
        identifier: impl Into<String>,
        private_key: &[u8],
    ) -> Result<Self, PairingError> {
        let private_key: [u8; 32] = private_key.try_into().map_err(|_| {
            PairingError::Crypto(format!(
                "expected 32-byte Ed25519 private key seed, got {} bytes",
                private_key.len()
            ))
        })?;
        Ok(Self {
            identifier: identifier.into(),
            signing_key: SigningKey::from_bytes(&private_key),
        })
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        VerifyingKey::from(&self.signing_key).to_bytes().to_vec()
    }

    pub fn private_key_bytes(&self) -> Vec<u8> {
        self.signing_key.to_bytes().to_vec()
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        use ed25519_dalek::Signer;
        self.signing_key.sign(msg).to_bytes().to_vec()
    }
}

// ── SRP session ───────────────────────────────────────────────────────────────

/// SRP-3072-SHA512 session, matching go-ios srp.go.
///
/// Custom password hash: SHA512(salt || SHA512("Pair-Setup:000000"))
pub struct SrpSession {
    pub client_public: Vec<u8>,
    pub client_proof: Vec<u8>,
    pub session_key: Vec<u8>,
    // Internal SRP state for server proof verification
    verifier: SrpVerifier,
}

struct SrpVerifier {
    m2_expected: Vec<u8>,
}

impl SrpSession {
    /// Initialize SRP session from the device's salt and public key.
    pub fn new(salt: &[u8], device_public: &[u8]) -> Result<Self, PairingError> {
        // Custom password derivation: SHA512(salt || SHA512("Pair-Setup:000000"))
        let inner = {
            let mut h = Sha512::new();
            h.update(b"Pair-Setup:000000");
            h.finalize()
        };
        let x_hash = {
            let mut h = Sha512::new();
            h.update(salt);
            h.update(inner);
            h.finalize()
        };

        // RFC 5054 SRP-3072 with the custom x
        srp_compute(salt, device_public, &x_hash)
    }

    pub fn verify_server_proof(&self, server_proof: &[u8]) -> bool {
        server_proof == self.verifier.m2_expected.as_slice()
    }
}

/// Minimal SRP-3072 computation without the srp crate (uses BigUint arithmetic).
///
/// This implements SRP-6a as per RFC 5054, Group 3072.
///
/// Algorithm overview (SRP-3072-SHA512):
///   1. k = H(N || pad(g))           — multiplier parameter
///   2. x = H(salt || H(I || ":" || P))  — derived by caller as password hash
///   3. A = g^a mod N                — client ephemeral public key
///   4. u = H(pad(A) || pad(B))      — scrambling parameter
///   5. S = (B - k*g^x)^(a + u*x) mod N  — premaster secret
///   6. K = H(S)                     — session key
///   7. M1 = H(H(N) XOR H(g) || H(I) || salt || A || B || K)  — client proof
///   8. M2 = H(A || M1 || K)         — expected server proof
fn srp_compute(salt: &[u8], device_public_b: &[u8], x: &[u8]) -> Result<SrpSession, PairingError> {
    use num_bigint::BigUint;
    use num_traits::One;

    // RFC 5054 3072-bit group
    let n_hex = concat!(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD1",
        "29024E088A67CC74020BBEA63B139B22514A08798E3404DD",
        "EF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245",
        "E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3D",
        "C2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F",
        "83655D23DCA3AD961C62F356208552BB9ED529077096966D",
        "670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
        "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9",
        "DE2BCBF6955817183995497CEA956AE515D2261898FA0510",
        "15728E5A8AAAC42DAD33170D04507A33A85521ABDF1CBA64",
        "ECFB850458DBEF0A8AEA71575D060C7DB3970F85A6E1E4C7",
        "ABF5AE8CDB0933D71E8C94E04A25619DCEE3D2261AD2EE6B",
        "F12FFA06D98A0864D87602733EC86A64521F2B18177B200C",
        "BBE117577A615D6C770988C0BAD946E208E24FA074E5AB31",
        "43DB5BFCE0FD108E4B82D120A93AD2CAFFFFFFFFFFFFFFFF"
    );
    let g = BigUint::from(5u32);
    let n = BigUint::parse_bytes(n_hex.as_bytes(), 16)
        .ok_or(PairingError::Crypto("SRP: invalid N".into()))?;

    // Step 1: k = H(N || pad(g)) — SRP multiplier parameter
    let k = {
        let n_bytes = n.to_bytes_be();
        let mut g_bytes = vec![0u8; n_bytes.len()];
        let g_b = g.to_bytes_be();
        g_bytes[n_bytes.len() - g_b.len()..].copy_from_slice(&g_b);
        let mut h = Sha512::new();
        h.update(&n_bytes);
        h.update(&g_bytes);
        BigUint::from_bytes_be(&h.finalize())
    };

    // Step 2: Generate ephemeral private a (random 256-bit scalar)
    let a_secret: [u8; 32] = rand::random();
    let a = BigUint::from_bytes_be(&a_secret);
    // Step 3: A = g^a mod N — client ephemeral public key
    let big_a = g.modpow(&a, &n);
    let big_a_bytes = big_a.to_bytes_be();

    // B (device public)
    let big_b = BigUint::from_bytes_be(device_public_b);

    // Step 4: u = H(pad(A) || pad(B)) — scrambling parameter
    let n_len = n.to_bytes_be().len();
    let u = {
        let mut a_padded = vec![0u8; n_len.saturating_sub(big_a_bytes.len())];
        a_padded.extend_from_slice(&big_a_bytes);
        let b_bytes = big_b.to_bytes_be();
        let mut b_padded = vec![0u8; n_len.saturating_sub(b_bytes.len())];
        b_padded.extend_from_slice(&b_bytes);
        let mut h = Sha512::new();
        h.update(&a_padded);
        h.update(&b_padded);
        BigUint::from_bytes_be(&h.finalize())
    };

    // x already derived by caller: x = H(salt || H("Pair-Setup:000000"))
    let x_big = BigUint::from_bytes_be(x);

    // v = g^x mod N (password verifier, used to compute premaster secret)
    let v = g.modpow(&x_big, &n);

    // Step 5: S = (B - k*v)^(a + u*x) mod N — premaster secret
    let kv = (k * &v) % &n;
    let base = if big_b >= kv {
        (big_b - kv) % &n
    } else {
        return Err(PairingError::Crypto("SRP: B < k*v".into()));
    };
    let exp = (&a + &u * &x_big) % (&n - BigUint::one());
    let s = base.modpow(&exp, &n);
    let s_bytes = {
        let raw = s.to_bytes_be();
        let mut padded = vec![0u8; n_len.saturating_sub(raw.len())];
        padded.extend_from_slice(&raw);
        padded
    };

    // Step 6: K = H(S) — session key derived from premaster secret
    let session_key = {
        let mut h = Sha512::new();
        h.update(&s_bytes);
        h.finalize().to_vec()
    };

    // Step 7: M1 = H(H(N) XOR H(g) || H(I) || salt || A || B || K) — client proof
    let h_n = {
        let mut h = Sha512::new();
        h.update(n.to_bytes_be());
        h.finalize()
    };
    let h_g = {
        let mut h = Sha512::new();
        h.update(g.to_bytes_be());
        h.finalize()
    };
    let xor_ng: Vec<u8> = h_n.iter().zip(h_g.iter()).map(|(a, b)| a ^ b).collect();
    let h_i = {
        let mut h = Sha512::new();
        h.update(b"Pair-Setup");
        h.finalize()
    };

    let m1 = {
        let mut h = Sha512::new();
        h.update(&xor_ng);
        h.update(h_i);
        h.update(salt);
        h.update(&big_a_bytes);
        h.update(device_public_b);
        h.update(&session_key);
        h.finalize().to_vec()
    };

    // Step 8: M2 (expected server proof) = H(A || M1 || K) — for verifying device
    let m2 = {
        let mut h = Sha512::new();
        h.update(&big_a_bytes);
        h.update(&m1);
        h.update(&session_key);
        h.finalize().to_vec()
    };

    Ok(SrpSession {
        client_public: big_a_bytes,
        client_proof: m1,
        session_key,
        verifier: SrpVerifier { m2_expected: m2 },
    })
}

// ── HKDF helpers ──────────────────────────────────────────────────────────────

fn hkdf_sha512(ikm: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; 32], PairingError> {
    let h = Hkdf::<Sha512>::new(if salt.is_empty() { None } else { Some(salt) }, ikm);
    let mut out = [0u8; 32];
    h.expand(info, &mut out)
        .map_err(|e| PairingError::Crypto(format!("HKDF expand failed: {e}")))?;
    Ok(out)
}

// ── ChaCha20Poly1305 helpers ──────────────────────────────────────────────────

fn chacha_nonce(label: &[u8]) -> [u8; 12] {
    let mut n = [0u8; 12];
    let end = n.len();
    let start = end - label.len().min(8);
    n[start..end].copy_from_slice(&label[..label.len().min(8)]);
    n
}

fn chacha_seal(
    key: &[u8; 32],
    nonce_label: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, PairingError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| PairingError::Crypto(format!("ChaCha key init failed: {e}")))?;
    let nonce = chacha20poly1305::Nonce::from(chacha_nonce(nonce_label));
    cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| PairingError::Crypto(format!("ChaCha seal failed: {e}")))
}

fn chacha_open(
    key: &[u8; 32],
    nonce_label: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, PairingError> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| PairingError::Crypto(format!("ChaCha key init failed: {e}")))?;
    let nonce = chacha20poly1305::Nonce::from(chacha_nonce(nonce_label));
    cipher.decrypt(&nonce, ciphertext).map_err(|_| {
        PairingError::Crypto("ChaCha decrypt failed (wrong key or tampered data)".into())
    })
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PairingError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

// ── Pure crypto step functions (testable without network) ─────────────────────

/// Build the TLV bytes for the setupManualPairing initiation (State 1).
pub fn build_setup_tlv() -> Vec<u8> {
    let mut buf = TlvBuffer::new();
    buf.push_u8(TYPE_METHOD, 0x00);
    buf.push_u8(TYPE_STATE, STATE_START_REQUEST);
    buf.into_bytes()
}

/// Build the TLV bytes for the SRP proof message (State 3).
pub fn build_srp_proof_tlv(srp: &SrpSession) -> Vec<u8> {
    let mut buf = TlvBuffer::new();
    buf.push_u8(TYPE_STATE, STATE_VERIFY_REQUEST);
    buf.push_bytes(TYPE_PUBLIC_KEY, &srp.client_public);
    buf.push_bytes(TYPE_PROOF, &srp.client_proof);
    buf.into_bytes()
}

/// Build the TLV bytes for the device info exchange (State 5).
///
/// Returns (tlv_bytes, setup_key) where setup_key is needed to decrypt the response.
pub fn build_device_info_tlv(
    session_key: &[u8],
    identity: &HostIdentity,
) -> Result<(Vec<u8>, [u8; 32]), PairingError> {
    // 1. HKDF for controller sign
    let controller_salt = b"Pair-Setup-Controller-Sign-Salt";
    let controller_info = b"Pair-Setup-Controller-Sign-Info";
    let sign_key = hkdf_sha512(session_key, controller_salt, controller_info)?;

    // 2. Sign: H_controller || identifier || ed25519_public
    let mut sign_msg = sign_key.to_vec();
    sign_msg.extend_from_slice(identity.identifier.as_bytes());
    sign_msg.extend_from_slice(&identity.public_key_bytes());
    let signature = identity.sign(&sign_msg);

    // 3. Opack-encode device info
    // The altIRK, btAddr, mac, remotepairing_serial_number values below are
    // hardcoded dummy placeholders required by the Apple pairing protocol.
    // The device expects these fields to be present but does not validate their
    // actual content for a host-side pairing session.
    let device_info = opack::encode(&opack::OpackValue::Dict(vec![
        (
            opack::OpackValue::String("accountID".into()),
            opack::OpackValue::String(identity.identifier.clone()),
        ),
        (
            opack::OpackValue::String("altIRK".into()),
            opack::OpackValue::Bytes(vec![
                0x5e, 0xca, 0x81, 0x91, 0x92, 0x02, 0x82, 0x00, 0x11, 0x22, 0x33, 0x44, 0xbb, 0xf2,
                0x4a, 0xc8,
            ]),
        ),
        (
            opack::OpackValue::String("btAddr".into()),
            opack::OpackValue::String("FF:DD:99:66:BB:AA".into()),
        ),
        (
            opack::OpackValue::String("mac".into()),
            opack::OpackValue::Bytes(vec![0xff, 0x44, 0x88, 0x66, 0x33, 0x99]),
        ),
        (
            opack::OpackValue::String("model".into()),
            opack::OpackValue::String("ios-rs".into()),
        ),
        (
            opack::OpackValue::String("name".into()),
            opack::OpackValue::String("ios-rs-host".into()),
        ),
        (
            opack::OpackValue::String("remotepairing_serial_number".into()),
            opack::OpackValue::String("ios-rs-serial".into()),
        ),
    ]))
    .map_err(|e| PairingError::Protocol(e.to_string()))?;

    // 4. Build inner TLV
    let mut inner = TlvBuffer::new();
    inner.push_bytes(TYPE_SIGNATURE, &signature);
    inner.push_bytes(TYPE_PUBLIC_KEY, &identity.public_key_bytes());
    inner.push_bytes(TYPE_IDENTIFIER, identity.identifier.as_bytes());
    inner.push_bytes(TYPE_INFO, &device_info);
    let inner_bytes = inner.into_bytes();

    // 5. Derive encryption key + encrypt
    let setup_key = hkdf_sha512(
        session_key,
        b"Pair-Setup-Encrypt-Salt",
        b"Pair-Setup-Encrypt-Info",
    )?;
    let encrypted = chacha_seal(&setup_key, b"PS-Msg05", &inner_bytes)?;

    // 6. Outer TLV
    let mut outer = TlvBuffer::new();
    outer.push_u8(TYPE_STATE, STATE_PHASE5);
    outer.push_bytes(TYPE_ENCRYPTED_DATA, &encrypted);

    Ok((outer.into_bytes(), setup_key))
}

/// Verify the device info response (State 6).
///
/// Decrypts the device's encrypted TLV response using the setup key derived
/// during the device info exchange. A successful decryption proves the device
/// holds the same session key, completing mutual authentication.
pub fn verify_device_info_response(
    setup_key: &[u8; 32],
    encrypted_data: &[u8],
) -> Result<(), PairingError> {
    chacha_open(setup_key, b"PS-Msg06", encrypted_data)?;
    Ok(())
}

/// Derive the two session cipher keys from the SRP session key.
pub fn derive_cipher_keys(session_key: &[u8]) -> Result<([u8; 32], [u8; 32]), PairingError> {
    let client_key = hkdf_sha512(session_key, &[], b"ClientEncrypt-main")?;
    let server_key = hkdf_sha512(session_key, &[], b"ServerEncrypt-main")?;
    Ok((client_key, server_key))
}

// ── VerifyPair (for already-paired devices) ───────────────────────────────────

/// Build the TLV for the pair verify initiation.
#[cfg(feature = "tunnel")]
pub fn build_verify_start_tlv(x25519_pub: &[u8]) -> Vec<u8> {
    let mut buf = TlvBuffer::new();
    buf.push_u8(TYPE_STATE, STATE_START_REQUEST);
    buf.push_bytes(TYPE_PUBLIC_KEY, x25519_pub);
    buf.into_bytes()
}

/// Complete the pair verify handshake (step 2).
///
/// Returns the derived cipher keys (client_key, server_key) on success.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(feature = "tunnel")]
pub struct VerifyPairSession {
    pub tlv: Vec<u8>,
    pub encryption_key: [u8; 32],
    pub client_key: [u8; 32],
    pub server_key: [u8; 32],
}

/// Build the pair verify step-2 TLV and derive the keys required for the
/// subsequent encrypted control channel and TLS-PSK listener connection.
#[cfg(feature = "tunnel")]
pub fn build_verify_step2_tlv(
    our_secret: [u8; 32],     // x25519 secret scalar bytes
    our_public: &[u8; 32],    // x25519 public
    device_public: &[u8; 32], // from device TLV
    identity: &HostIdentity,
) -> Result<VerifyPairSession, PairingError> {
    // ECDH
    use x25519_dalek::{PublicKey as X25519Pub, StaticSecret};
    let our = StaticSecret::from(our_secret);
    let dev = X25519Pub::from(*device_public);
    let shared = our.diffie_hellman(&dev).to_bytes();

    // Derive encryption key
    let derived = hkdf_sha512(
        &shared,
        b"Pair-Verify-Encrypt-Salt",
        b"Pair-Verify-Encrypt-Info",
    )?;

    // Sign: our_public || identifier || device_public
    let mut sign_msg = our_public.to_vec();
    sign_msg.extend_from_slice(identity.identifier.as_bytes());
    sign_msg.extend_from_slice(device_public);
    let sig = identity.sign(&sign_msg);

    // Encrypt
    let mut inner = TlvBuffer::new();
    inner.push_bytes(TYPE_SIGNATURE, &sig);
    inner.push_bytes(TYPE_IDENTIFIER, identity.identifier.as_bytes());
    let inner_bytes = inner.into_bytes();
    let encrypted = chacha_seal(&derived, b"PV-Msg03", &inner_bytes)?;

    let mut outer = TlvBuffer::new();
    outer.push_u8(TYPE_STATE, STATE_VERIFY_REQUEST);
    outer.push_bytes(TYPE_ENCRYPTED_DATA, &encrypted);

    // Session keys from shared secret
    let client_key = hkdf_sha512(&shared, &[], b"ClientEncrypt-main")?;
    let server_key = hkdf_sha512(&shared, &[], b"ServerEncrypt-main")?;

    Ok(VerifyPairSession {
        tlv: outer.into_bytes(),
        encryption_key: shared,
        client_key,
        server_key,
    })
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[test]
    fn test_host_identity_generation() {
        let id = HostIdentity::generate();
        assert_eq!(id.identifier.len(), 36);
        assert_eq!(id.public_key_bytes().len(), 32);
        assert_eq!(id.private_key_bytes().len(), 32);
    }

    #[test]
    fn test_chacha_roundtrip() {
        let key = [0u8; 32];
        let plaintext = b"hello pairing world";
        let ct = chacha_seal(&key, b"PS-Msg05", plaintext).unwrap();
        let pt = chacha_open(&key, b"PS-Msg05", &ct).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn test_hkdf_sha512_deterministic() {
        let k1 = hkdf_sha512(b"session_key", b"salt", b"ClientEncrypt-main").unwrap();
        let k2 = hkdf_sha512(b"session_key", b"salt", b"ClientEncrypt-main").unwrap();
        assert_eq!(k1, k2);
        let k3 = hkdf_sha512(b"session_key", b"salt", b"ServerEncrypt-main").unwrap();
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_build_setup_tlv() {
        let tlv = build_setup_tlv();
        // Should have: [TYPE_METHOD=0, len=1, val=0, TYPE_STATE=6, len=1, val=1]
        assert!(tlv.len() >= 6);
        assert_eq!(tlv[0], TYPE_METHOD);
        assert_eq!(tlv[3], TYPE_STATE);
        assert_eq!(tlv[5], STATE_START_REQUEST);
    }

    #[test]
    fn test_derive_cipher_keys_different() {
        let (ck, sk) = derive_cipher_keys(b"test_session_key").unwrap();
        assert_ne!(ck, sk);
        assert_eq!(ck.len(), 32);
        assert_eq!(sk.len(), 32);
    }

    #[test]
    fn test_device_info_tlv() {
        let identity = HostIdentity::generate();
        let session_key = vec![0x42u8; 64];
        let (tlv, setup_key) = build_device_info_tlv(&session_key, &identity).unwrap();
        assert!(!tlv.is_empty());
        assert_eq!(setup_key.len(), 32);
    }

    #[test]
    fn test_build_verify_step2_tlv_returns_state_and_keys() {
        let identity = HostIdentity::generate();
        let our_secret = [0x11; 32];
        let our_static = x25519_dalek::StaticSecret::from(our_secret);
        let our_public = x25519_dalek::PublicKey::from(&our_static).to_bytes();
        let device_secret = [0x22; 32];
        let device_static = x25519_dalek::StaticSecret::from(device_secret);
        let device_public = x25519_dalek::PublicKey::from(&device_static).to_bytes();

        let session =
            build_verify_step2_tlv(our_secret, &our_public, &device_public, &identity).unwrap();

        let decoded = TlvBuffer::decode(&session.tlv);
        assert_eq!(
            decoded.get(&TYPE_STATE).map(Bytes::as_ref),
            Some(&[STATE_VERIFY_REQUEST][..])
        );
        assert!(decoded
            .get(&TYPE_ENCRYPTED_DATA)
            .is_some_and(|value| !value.is_empty()));
        assert_ne!(session.client_key, session.server_key);
        assert_ne!(session.encryption_key, [0u8; 32]);
    }
}

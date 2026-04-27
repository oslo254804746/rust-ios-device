//! SRP pairing XPC transport layer.
//!
//! Connects to the "untrusted" tunnel service on a new device and performs
//! the full SRP pairing handshake over XPC/H2.
//!
//! # Connection path
//! New device (not yet paired):
//!   1. Device's USB-Ethernet IPv6 address → RSD handshake (port 58783)
//!   2. RSD discovers "com.apple.internal.dt.coredevice.untrusted.tunnelservice"
//!   3. XPC/H2 connection to that port
//!   4. Send handshake, run SRP, receive pair record
//!
//! # Reference
//! go-ios/ios/tunnel/tunnel.go (ManualPairAndConnectToTunnel)
//! go-ios/ios/tunnel/untrusted.go (ManualPair, setupManualPairing, etc.)

use std::collections::HashMap;
use std::net::{Ipv6Addr, SocketAddr};

use crate::lockdown::pairing::{
    build_device_info_tlv, build_setup_tlv, build_srp_proof_tlv, derive_cipher_keys,
    verify_device_info_response, HostIdentity, SrpSession,
};
use crate::proto::tlv::TlvBuffer;
use bytes::{Bytes, BytesMut};
use chacha20poly1305::{aead::Aead, ChaCha20Poly1305, KeyInit};
use indexmap::IndexMap;
use tokio::net::TcpStream;

pub const UNTRUSTED_SERVICE_NAME: &str = "com.apple.internal.dt.coredevice.untrusted.tunnelservice";
const CONTROL_CHANNEL_ENVELOPE_TYPE: &str = "RemotePairing.ControlChannelMessageEnvelope";
const CONTROL_CHANNEL_ORIGIN: &str = "host";
const MAX_XPC_BODY_SIZE: usize = 1024 * 1024;

// TLV type codes
const TYPE_PUBLIC_KEY: u8 = 0x03;
const TYPE_PROOF: u8 = 0x04;
const TYPE_ENCRYPTED_DATA: u8 = 0x05;
const TYPE_SALT: u8 = 0x02;

/// Error type for pairing transport.
#[derive(Debug, thiserror::Error)]
pub enum PairingTransportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("XPC error: {0}")]
    Xpc(String),
    #[error("RSD error: {0}")]
    Rsd(String),
    #[error("SRP crypto error: {0}")]
    Crypto(String),
    #[error("pairing failed: {0}")]
    Failed(String),
    #[error("pairing rejected: {0}")]
    Rejected(String),
    #[error("missing required field: {0}")]
    MissingField(String),
    #[error("unexpected field type: {0}")]
    UnexpectedType(String),
    #[error("no untrusted tunnel service found in RSD")]
    ServiceNotFound,
}

/// Stored credentials after successful pairing.
#[derive(Debug, Clone)]
pub struct PairedCredentials {
    pub remote_identifier: String,
    pub host_identifier: String,
    pub host_public_key: Vec<u8>,
    pub host_private_key: Vec<u8>,
    pub remote_unlock_host_key: Option<String>,
    /// Cipher keys for subsequent sessions (client_key, server_key)
    pub session_keys: Option<([u8; 32], [u8; 32])>,
}

/// Perform the full SRP pairing handshake with a new (untrusted) device.
///
/// `device_addr` – device's USB-Ethernet or Wi-Fi IPv6 address
///
/// Returns `PairedCredentials` that should be persisted for future connections.
///
/// The user must press "Trust" on the device when prompted.
pub async fn pair_new_device(
    device_addr: Ipv6Addr,
) -> Result<PairedCredentials, PairingTransportError> {
    // 1. RSD handshake to find the untrusted service port
    let rsd = crate::xpc::rsd::handshake(device_addr, crate::xpc::rsd::RSD_PORT)
        .await
        .map_err(|e| PairingTransportError::Rsd(e.to_string()))?;

    let port = rsd
        .get_port(UNTRUSTED_SERVICE_NAME)
        .ok_or(PairingTransportError::ServiceNotFound)?;

    // 2. XPC connection to the untrusted service
    let sock_addr = SocketAddr::new(device_addr.into(), port);
    let stream = TcpStream::connect(sock_addr).await?;

    let mut framer = crate::xpc::h2_raw::H2Framer::connect(stream)
        .await
        .map_err(|e| PairingTransportError::Xpc(format!("H2: {e}")))?;

    // 3. RemoteXPC bootstrap + handshake request
    bootstrap_remote_xpc(&mut framer).await?;
    let mut sequence_number = 1;
    let handshake_body = build_handshake_request(next_sequence_number(&mut sequence_number));
    send_xpc(&mut framer, &handshake_body, 1).await?;
    let handshake = recv_handshake_response(&mut framer).await?;
    let remote_identifier = extract_remote_identifier(&handshake)?;

    // 4. Generate host identity
    let identity = HostIdentity::generate();

    // 5. setupManualPairing – send State 1
    let setup_tlv = build_setup_tlv();
    let pairing_event = build_pairing_event(
        &setup_tlv,
        "setupManualPairing",
        true,
        None,
        next_sequence_number(&mut sequence_number),
    );
    send_xpc(&mut framer, &pairing_event, 2).await?;
    recv_control_plain_message(&mut framer).await?; // ack

    // 6. Read device's SRP public key + salt
    let device_data = recv_xpc_pairing_data(&mut framer).await?;
    let device_tlv = parse_tlv(&device_data);

    let device_pub = device_tlv
        .get(&TYPE_PUBLIC_KEY)
        .ok_or_else(|| PairingTransportError::Failed("no public key from device".into()))?
        .to_vec();
    let salt = device_tlv
        .get(&TYPE_SALT)
        .ok_or_else(|| PairingTransportError::Failed("no salt from device".into()))?
        .to_vec();

    // 7. SRP computation
    let srp = SrpSession::new(&salt, &device_pub)
        .map_err(|e| PairingTransportError::Crypto(e.to_string()))?;

    // 8. Send SRP proof (State 3)
    let proof_tlv = build_srp_proof_tlv(&srp);
    let proof_event = build_pairing_event(
        &proof_tlv,
        "setupManualPairing",
        false,
        None,
        next_sequence_number(&mut sequence_number),
    );
    send_xpc(&mut framer, &proof_event, 3).await?;

    // 9. Read device server proof
    let server_data = recv_xpc_pairing_data(&mut framer).await?;
    let server_tlv = parse_tlv(&server_data);

    let server_proof = server_tlv
        .get(&TYPE_PROOF)
        .ok_or_else(|| PairingTransportError::Failed("no server proof".into()))?
        .to_vec();

    if !srp.verify_server_proof(&server_proof) {
        return Err(PairingTransportError::Failed(
            "server proof verification failed".into(),
        ));
    }

    // 10. Exchange device info (State 5)
    let (info_tlv, setup_key) = build_device_info_tlv(&srp.session_key, &identity)
        .map_err(|e| PairingTransportError::Crypto(e.to_string()))?;

    let info_event = build_pairing_event(
        &info_tlv,
        "setupManualPairing",
        false,
        Some("ios-rs-host"),
        next_sequence_number(&mut sequence_number),
    );
    send_xpc(&mut framer, &info_event, 4).await?;

    // 11. Read encrypted device info response (State 6)
    let enc_data = recv_xpc_pairing_data(&mut framer).await?;
    let enc_tlv = parse_tlv(&enc_data);
    let enc_payload = enc_tlv
        .get(&TYPE_ENCRYPTED_DATA)
        .ok_or_else(|| PairingTransportError::Failed("no encrypted data in response".into()))?;

    verify_device_info_response(&setup_key, enc_payload)
        .map_err(|e| PairingTransportError::Crypto(e.to_string()))?;

    // 12. Derive session cipher keys
    let (client_key, server_key) = derive_cipher_keys(&srp.session_key)
        .map_err(|e| PairingTransportError::Crypto(e.to_string()))?;
    let remote_unlock_host_key =
        create_remote_unlock_key(&mut framer, &client_key, &server_key, &mut sequence_number)
            .await?;

    Ok(PairedCredentials {
        remote_identifier,
        host_identifier: identity.identifier.clone(),
        host_public_key: identity.public_key_bytes(),
        host_private_key: identity.private_key_bytes(),
        remote_unlock_host_key,
        session_keys: Some((client_key, server_key)),
    })
}

// ── XPC message helpers ───────────────────────────────────────────────────────

fn xpc_dict(pairs: &[(&str, crate::xpc::message::XpcValue)]) -> crate::xpc::message::XpcValue {
    let mut map = IndexMap::new();
    for (k, v) in pairs {
        map.insert(k.to_string(), v.clone());
    }
    crate::xpc::message::XpcValue::Dictionary(map)
}

fn xpc_bool(b: bool) -> crate::xpc::message::XpcValue {
    crate::xpc::message::XpcValue::Bool(b)
}

fn xpc_int(n: i64) -> crate::xpc::message::XpcValue {
    crate::xpc::message::XpcValue::Int64(n)
}

fn xpc_uint(n: u64) -> crate::xpc::message::XpcValue {
    crate::xpc::message::XpcValue::Uint64(n)
}

fn xpc_data(d: &[u8]) -> crate::xpc::message::XpcValue {
    crate::xpc::message::XpcValue::Data(Bytes::copy_from_slice(d))
}

fn xpc_string(s: &str) -> crate::xpc::message::XpcValue {
    crate::xpc::message::XpcValue::String(s.to_string())
}

fn next_sequence_number(sequence_number: &mut u64) -> u64 {
    let current = *sequence_number;
    *sequence_number += 1;
    current
}

fn build_handshake_request(sequence_number: u64) -> crate::xpc::message::XpcValue {
    let request = xpc_dict(&[(
        "handshake",
        xpc_dict(&[(
            "_0",
            xpc_dict(&[
                (
                    "hostOptions",
                    xpc_dict(&[("attemptPairVerify", xpc_bool(true))]),
                ),
                ("wireProtocolVersion", xpc_int(19)),
            ]),
        )]),
    )]);
    build_plain_request(request, sequence_number)
}

fn build_plain_request(
    request: crate::xpc::message::XpcValue,
    sequence_number: u64,
) -> crate::xpc::message::XpcValue {
    build_control_channel_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[("_0", xpc_dict(&[("request", xpc_dict(&[("_0", request)]))]))]),
        )]),
        sequence_number,
    )
}

fn build_control_channel_envelope(
    message: crate::xpc::message::XpcValue,
    sequence_number: u64,
) -> crate::xpc::message::XpcValue {
    xpc_dict(&[
        ("mangledTypeName", xpc_string(CONTROL_CHANNEL_ENVELOPE_TYPE)),
        (
            "value",
            xpc_dict(&[
                ("message", message),
                ("originatedBy", xpc_string(CONTROL_CHANNEL_ORIGIN)),
                ("sequenceNumber", xpc_uint(sequence_number)),
            ]),
        ),
    ])
}

fn build_encrypted_request(
    encrypted_payload: &[u8],
    sequence_number: u64,
) -> crate::xpc::message::XpcValue {
    build_control_channel_envelope(
        xpc_dict(&[(
            "streamEncrypted",
            xpc_dict(&[("_0", xpc_data(encrypted_payload))]),
        )]),
        sequence_number,
    )
}

fn build_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> crate::xpc::message::XpcValue {
    let mut pairs = vec![
        ("data", xpc_data(tlv_data)),
        ("kind", xpc_string(kind)),
        ("startNewSession", xpc_bool(start_new_session)),
    ];
    if let Some(h) = sending_host {
        pairs.push(("sendingHost", xpc_string(h)));
    }
    build_control_channel_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "event",
                    xpc_dict(&[(
                        "_0",
                        xpc_dict(&[("pairingData", xpc_dict(&[("_0", xpc_dict(&pairs))]))]),
                    )]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

async fn bootstrap_remote_xpc<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
) -> Result<(), PairingTransportError> {
    crate::xpc::rsd::initialize_xpc_connection_on_framer(framer)
        .await
        .map_err(|e| PairingTransportError::Xpc(format!("RemoteXPC bootstrap: {e}")))
}

async fn send_xpc<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
    body: &crate::xpc::message::XpcValue,
    msg_id: u64,
) -> Result<(), PairingTransportError> {
    use crate::xpc::message::{encode_message, flags, XpcMessage};
    let msg = XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id,
        body: Some(body.clone()),
    };
    let bytes = encode_message(&msg).map_err(|e| PairingTransportError::Xpc(e.to_string()))?;
    framer
        .write_client_server(&bytes)
        .await
        .map_err(|e| PairingTransportError::Xpc(e.to_string()))
}

fn take_required_field(
    dict: &mut IndexMap<String, crate::xpc::message::XpcValue>,
    key: &str,
    path: &str,
) -> Result<crate::xpc::message::XpcValue, PairingTransportError> {
    dict.swap_remove(key)
        .ok_or_else(|| PairingTransportError::MissingField(path.to_string()))
}

fn take_required_dict(
    dict: &mut IndexMap<String, crate::xpc::message::XpcValue>,
    key: &str,
    path: &str,
) -> Result<IndexMap<String, crate::xpc::message::XpcValue>, PairingTransportError> {
    match take_required_field(dict, key, path)? {
        crate::xpc::message::XpcValue::Dictionary(value) => Ok(value),
        _ => Err(PairingTransportError::UnexpectedType(format!(
            "{path} must be a dictionary"
        ))),
    }
}

fn take_required_data(
    dict: &mut IndexMap<String, crate::xpc::message::XpcValue>,
    key: &str,
    path: &str,
) -> Result<Vec<u8>, PairingTransportError> {
    match take_required_field(dict, key, path)? {
        crate::xpc::message::XpcValue::Data(value) => Ok(value.to_vec()),
        _ => Err(PairingTransportError::UnexpectedType(format!(
            "{path} must be a data blob"
        ))),
    }
}

fn take_required_string(
    dict: &mut IndexMap<String, crate::xpc::message::XpcValue>,
    key: &str,
    path: &str,
) -> Result<String, PairingTransportError> {
    match take_required_field(dict, key, path)? {
        crate::xpc::message::XpcValue::String(value) => Ok(value),
        _ => Err(PairingTransportError::UnexpectedType(format!(
            "{path} must be a string"
        ))),
    }
}

fn decode_control_plain_message(
    body: crate::xpc::message::XpcValue,
) -> Result<IndexMap<String, crate::xpc::message::XpcValue>, PairingTransportError> {
    let mut envelope = match body {
        crate::xpc::message::XpcValue::Dictionary(value) => value,
        _ => {
            return Err(PairingTransportError::UnexpectedType(
                "control channel body must be a dictionary".into(),
            ));
        }
    };

    let mangled_type = take_required_string(&mut envelope, "mangledTypeName", "mangledTypeName")?;
    if mangled_type != CONTROL_CHANNEL_ENVELOPE_TYPE {
        return Err(PairingTransportError::Failed(format!(
            "unexpected control channel type: {mangled_type}"
        )));
    }

    let mut value = take_required_dict(&mut envelope, "value", "value")?;
    let mut message = take_required_dict(&mut value, "message", "value.message")?;
    let mut plain = take_required_dict(&mut message, "plain", "value.message.plain")?;
    take_required_dict(&mut plain, "_0", "value.message.plain._0")
}

fn decode_pairing_data_event(
    mut plain: IndexMap<String, crate::xpc::message::XpcValue>,
) -> Result<Vec<u8>, PairingTransportError> {
    let mut event = take_required_dict(&mut plain, "event", "value.message.plain._0.event")?;
    let mut event_body = take_required_dict(&mut event, "_0", "value.message.plain._0.event._0")?;

    if let Some(rejection) = event_body.get("pairingRejectedWithError") {
        return Err(PairingTransportError::Rejected(
            extract_pairing_rejection_message(rejection),
        ));
    }

    let mut pairing_data = take_required_dict(
        &mut event_body,
        "pairingData",
        "value.message.plain._0.event._0.pairingData",
    )?;
    let mut pairing_data_body = take_required_dict(
        &mut pairing_data,
        "_0",
        "value.message.plain._0.event._0.pairingData._0",
    )?;
    take_required_data(
        &mut pairing_data_body,
        "data",
        "value.message.plain._0.event._0.pairingData._0.data",
    )
}

fn extract_pairing_rejection_message(value: &crate::xpc::message::XpcValue) -> String {
    value
        .as_dict()
        .and_then(|wrapped| wrapped.get("wrappedError"))
        .and_then(crate::xpc::message::XpcValue::as_dict)
        .and_then(|user_info| user_info.get("userInfo"))
        .and_then(crate::xpc::message::XpcValue::as_dict)
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(crate::xpc::message::XpcValue::as_str)
        .unwrap_or("pairing rejected by device")
        .to_string()
}

async fn recv_xpc<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
) -> Result<crate::xpc::message::XpcValue, PairingTransportError> {
    use crate::xpc::message::decode_message;
    let header = framer
        .read_client_server(24)
        .await
        .map_err(|e| PairingTransportError::Xpc(e.to_string()))?;
    let body_len = xpc_body_len(&header)?;
    let body = if body_len > 0 {
        framer
            .read_client_server(body_len)
            .await
            .map_err(|e| PairingTransportError::Xpc(e.to_string()))?
    } else {
        Bytes::new()
    };
    let mut full = BytesMut::new();
    full.extend_from_slice(&header);
    full.extend_from_slice(&body);
    let msg =
        decode_message(full.freeze()).map_err(|e| PairingTransportError::Xpc(e.to_string()))?;
    msg.body
        .ok_or_else(|| PairingTransportError::MissingField("xpc message body".into()))
}

fn xpc_body_len(header: &[u8]) -> Result<usize, PairingTransportError> {
    let len = u64::from_le_bytes(
        header[8..16]
            .try_into()
            .map_err(|_| PairingTransportError::Xpc("bad header length field".into()))?,
    );
    let len = usize::try_from(len)
        .map_err(|_| PairingTransportError::Xpc("xpc body length exceeds usize".into()))?;
    if len > MAX_XPC_BODY_SIZE {
        return Err(PairingTransportError::Xpc(format!(
            "body too large: {len} bytes exceeds {MAX_XPC_BODY_SIZE}"
        )));
    }
    Ok(len)
}

async fn recv_control_plain_message<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
) -> Result<IndexMap<String, crate::xpc::message::XpcValue>, PairingTransportError> {
    decode_control_plain_message(recv_xpc(framer).await?)
}

async fn recv_handshake_response<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
) -> Result<IndexMap<String, crate::xpc::message::XpcValue>, PairingTransportError> {
    let mut plain = recv_control_plain_message(framer).await?;
    let mut response =
        take_required_dict(&mut plain, "response", "value.message.plain._0.response")?;
    let mut response_body =
        take_required_dict(&mut response, "_1", "value.message.plain._0.response._1")?;
    let mut handshake = take_required_dict(
        &mut response_body,
        "handshake",
        "value.message.plain._0.response._1.handshake",
    )?;
    take_required_dict(
        &mut handshake,
        "_0",
        "value.message.plain._0.response._1.handshake._0",
    )
}

async fn recv_xpc_pairing_data<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
) -> Result<Vec<u8>, PairingTransportError> {
    decode_pairing_data_event(recv_control_plain_message(framer).await?)
}

fn extract_remote_identifier(
    handshake: &IndexMap<String, crate::xpc::message::XpcValue>,
) -> Result<String, PairingTransportError> {
    handshake
        .get("peerDeviceInfo")
        .and_then(crate::xpc::message::XpcValue::as_dict)
        .and_then(|peer| peer.get("identifier"))
        .and_then(crate::xpc::message::XpcValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            PairingTransportError::MissingField(
                "value.message.plain._0.response._1.handshake._0.peerDeviceInfo.identifier".into(),
            )
        })
}

fn parse_tlv(data: &[u8]) -> HashMap<u8, Vec<u8>> {
    let map = TlvBuffer::decode(data);
    map.into_iter().map(|(k, v)| (k, v.to_vec())).collect()
}

fn make_encrypted_nonce(sequence_number: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&sequence_number.to_le_bytes());
    nonce
}

fn decode_encrypted_response(
    body: crate::xpc::message::XpcValue,
) -> Result<Vec<u8>, PairingTransportError> {
    let mut envelope = match body {
        crate::xpc::message::XpcValue::Dictionary(value) => value,
        _ => {
            return Err(PairingTransportError::UnexpectedType(
                "encrypted control channel body must be a dictionary".into(),
            ));
        }
    };

    let mangled_type = take_required_string(&mut envelope, "mangledTypeName", "mangledTypeName")?;
    if mangled_type != CONTROL_CHANNEL_ENVELOPE_TYPE {
        return Err(PairingTransportError::Failed(format!(
            "unexpected control channel type: {mangled_type}"
        )));
    }

    let mut value = take_required_dict(&mut envelope, "value", "value")?;
    let mut message = take_required_dict(&mut value, "message", "value.message")?;
    take_required_data(&mut message, "_0", "value.message.streamEncrypted._0").or_else(|_| {
        let mut stream_encrypted = take_required_dict(
            &mut message,
            "streamEncrypted",
            "value.message.streamEncrypted",
        )?;
        take_required_data(
            &mut stream_encrypted,
            "_0",
            "value.message.streamEncrypted._0",
        )
    })
}

fn extract_remote_unlock_host_key(
    response_body: &serde_json::Value,
) -> Result<Option<String>, PairingTransportError> {
    let Some(create_remote_unlock_key) = response_body.get("createRemoteUnlockKey") else {
        return Err(PairingTransportError::MissingField(
            "encrypted response.response._1.createRemoteUnlockKey".into(),
        ));
    };

    Ok(create_remote_unlock_key
        .get("hostKey")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .filter(|value| !value.is_empty()))
}

async fn create_remote_unlock_key<S>(
    framer: &mut crate::xpc::h2_raw::H2Framer<S>,
    client_key: &[u8; 32],
    server_key: &[u8; 32],
    sequence_number: &mut u64,
) -> Result<Option<String>, PairingTransportError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let client_cipher = ChaCha20Poly1305::new(client_key.into());
    let server_cipher = ChaCha20Poly1305::new(server_key.into());
    let nonce = make_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createRemoteUnlockKey": {}
            }
        }
    });
    let encrypted_request = client_cipher
        .encrypt(&nonce.into(), request.to_string().as_bytes())
        .map_err(|e| PairingTransportError::Crypto(e.to_string()))?;
    let body = build_encrypted_request(&encrypted_request, next_sequence_number(sequence_number));
    send_xpc(framer, &body, 5).await?;

    let encrypted_response = decode_encrypted_response(recv_xpc(framer).await?)?;
    let plaintext = server_cipher
        .decrypt(&nonce.into(), encrypted_response.as_ref())
        .map_err(|e| PairingTransportError::Crypto(e.to_string()))?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext)
        .map_err(|e| PairingTransportError::Xpc(format!("invalid encrypted JSON: {e}")))?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| {
            PairingTransportError::MissingField("encrypted response.response._1".into())
        })?;
    if let Some(error) = response_body.get("errorExtended") {
        return Err(PairingTransportError::Failed(format!(
            "createRemoteUnlockKey failed: {error:?}"
        )));
    }
    extract_remote_unlock_host_key(response_body)
}

#[cfg(test)]
mod tests {
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
    use tokio::time::{timeout, Duration};

    use super::*;

    const FRAME_DATA: u8 = 0x00;
    const FRAME_SETTINGS: u8 = 0x04;
    const FLAG_SETTINGS_ACK: u8 = 0x01;

    #[test]
    fn handshake_envelope_contains_required_control_fields() {
        let envelope = build_handshake_request(7);
        let outer = match envelope {
            crate::xpc::message::XpcValue::Dictionary(value) => value,
            other => panic!("expected envelope dictionary, got {other:?}"),
        };

        assert_eq!(
            outer
                .get("mangledTypeName")
                .and_then(crate::xpc::message::XpcValue::as_str),
            Some(CONTROL_CHANNEL_ENVELOPE_TYPE)
        );

        let value = outer
            .get("value")
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .expect("value dict");
        assert_eq!(
            value
                .get("originatedBy")
                .and_then(crate::xpc::message::XpcValue::as_str),
            Some(CONTROL_CHANNEL_ORIGIN)
        );
        assert_eq!(
            value
                .get("sequenceNumber")
                .and_then(crate::xpc::message::XpcValue::as_uint64),
            Some(7)
        );

        let handshake = value
            .get("message")
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .and_then(|message| message.get("plain"))
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .and_then(|plain| plain.get("_0"))
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .and_then(|plain| plain.get("request"))
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .and_then(|request| request.get("_0"))
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .and_then(|request| request.get("handshake"))
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .and_then(|handshake| handshake.get("_0"))
            .and_then(crate::xpc::message::XpcValue::as_dict)
            .expect("handshake payload");

        assert_eq!(
            handshake
                .get("hostOptions")
                .and_then(crate::xpc::message::XpcValue::as_dict)
                .and_then(|options| options.get("attemptPairVerify")),
            Some(&crate::xpc::message::XpcValue::Bool(true))
        );
        assert_eq!(
            handshake.get("wireProtocolVersion"),
            Some(&crate::xpc::message::XpcValue::Int64(19))
        );
    }

    #[tokio::test]
    async fn recv_xpc_reads_control_messages_from_client_server_stream() {
        let (client, mut server) = duplex(4096);

        let server_task = tokio::spawn(async move {
            let mut preface = [0u8; 24];
            server.read_exact(&mut preface).await.unwrap();
            assert_eq!(&preface, crate::xpc::h2_raw::H2_PREFACE);

            let mut settings = [0u8; 21];
            server.read_exact(&mut settings).await.unwrap();
            assert_eq!(settings[3], FRAME_SETTINGS);

            let mut window_update = [0u8; 13];
            server.read_exact(&mut window_update).await.unwrap();

            server.write_all(&settings_frame()).await.unwrap();
            server.flush().await.unwrap();

            let mut ack = [0u8; 9];
            server.read_exact(&mut ack).await.unwrap();
            assert_eq!(ack, settings_ack_frame().as_slice());

            let payload = crate::xpc::message::encode_message(&crate::xpc::message::XpcMessage {
                flags: crate::xpc::message::flags::ALWAYS_SET | crate::xpc::message::flags::DATA,
                msg_id: 1,
                body: Some(build_control_channel_envelope(
                    xpc_dict(&[("plain", xpc_dict(&[("_0", xpc_dict(&[]))]))]),
                    1,
                )),
            })
            .unwrap();

            server
                .write_all(&data_frame(
                    crate::xpc::h2_raw::STREAM_CLIENT_SERVER,
                    &payload,
                ))
                .await
                .unwrap();
            server.flush().await.unwrap();
        });

        let mut framer = crate::xpc::h2_raw::H2Framer::connect(client).await.unwrap();
        let plain = timeout(
            Duration::from_secs(1),
            recv_control_plain_message(&mut framer),
        )
        .await
        .expect("recv timed out")
        .unwrap();
        assert!(plain.is_empty());

        server_task.await.unwrap();
    }

    #[test]
    fn decode_pairing_data_event_extracts_inner_data() {
        let plain = dict_value(&[(
            "event",
            dict_value(&[(
                "_0",
                dict_value(&[(
                    "pairingData",
                    dict_value(&[(
                        "_0",
                        dict_value(&[
                            (
                                "data",
                                crate::xpc::message::XpcValue::Data(Bytes::from_static(
                                    b"\x01\x02",
                                )),
                            ),
                            ("kind", xpc_string("setupManualPairing")),
                            ("startNewSession", xpc_bool(false)),
                        ]),
                    )]),
                )]),
            )]),
        )]);

        let data = decode_pairing_data_event(unwrap_dict(plain)).unwrap();
        assert_eq!(data, vec![1, 2]);
    }

    #[test]
    fn decode_pairing_data_event_surfaces_rejection_reason() {
        let plain = dict_value(&[(
            "event",
            dict_value(&[(
                "_0",
                dict_value(&[(
                    "pairingRejectedWithError",
                    dict_value(&[(
                        "wrappedError",
                        dict_value(&[(
                            "userInfo",
                            dict_value(&[(
                                "NSLocalizedDescription",
                                xpc_string("Trust dialog denied"),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
        )]);

        let err = decode_pairing_data_event(unwrap_dict(plain)).unwrap_err();
        assert!(
            matches!(err, PairingTransportError::Rejected(message) if message == "Trust dialog denied")
        );
    }

    #[test]
    fn extract_remote_identifier_reads_peer_device_info() {
        let handshake = unwrap_dict(dict_value(&[(
            "peerDeviceInfo",
            dict_value(&[("identifier", xpc_string("00008150-000D6D6A1122401C"))]),
        )]));

        let remote_identifier = extract_remote_identifier(&handshake).unwrap();
        assert_eq!(remote_identifier, "00008150-000D6D6A1122401C");
    }

    #[test]
    fn xpc_body_len_rejects_oversized_body_before_allocation() {
        let mut header = [0u8; 24];
        header[8..16].copy_from_slice(&((MAX_XPC_BODY_SIZE as u64) + 1).to_le_bytes());

        let err = xpc_body_len(&header).unwrap_err();
        assert!(
            matches!(err, PairingTransportError::Xpc(message) if message.contains("body too large"))
        );
    }

    #[test]
    fn extract_remote_unlock_host_key_reads_host_key() {
        let response_body = serde_json::json!({
            "createRemoteUnlockKey": {
                "hostKey": "PcV5xhyuJBL7Qq9HOGeGVwtU4sJLe1jtl/vRy1tRKcI="
            }
        });

        let host_key = extract_remote_unlock_host_key(&response_body).unwrap();
        assert_eq!(
            host_key.as_deref(),
            Some("PcV5xhyuJBL7Qq9HOGeGVwtU4sJLe1jtl/vRy1tRKcI=")
        );
    }

    #[test]
    fn extract_remote_unlock_host_key_allows_missing_host_key() {
        let response_body = serde_json::json!({
            "createRemoteUnlockKey": {}
        });

        let host_key = extract_remote_unlock_host_key(&response_body).unwrap();
        assert!(host_key.is_none());
    }

    fn unwrap_dict(
        value: crate::xpc::message::XpcValue,
    ) -> IndexMap<String, crate::xpc::message::XpcValue> {
        match value {
            crate::xpc::message::XpcValue::Dictionary(dict) => dict,
            other => panic!("expected dictionary, got {other:?}"),
        }
    }

    fn dict_value(
        pairs: &[(&str, crate::xpc::message::XpcValue)],
    ) -> crate::xpc::message::XpcValue {
        xpc_dict(pairs)
    }

    fn settings_frame() -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0x03u16.to_be_bytes());
        payload.extend_from_slice(&100u32.to_be_bytes());
        payload.extend_from_slice(&0x04u16.to_be_bytes());
        payload.extend_from_slice(&1_048_576u32.to_be_bytes());
        frame(FRAME_SETTINGS, 0, 0, &payload)
    }

    fn settings_ack_frame() -> Vec<u8> {
        frame(FRAME_SETTINGS, FLAG_SETTINGS_ACK, 0, &[])
    }

    fn data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
        frame(FRAME_DATA, 0, stream_id, payload)
    }

    fn frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let len = payload.len();
        let mut out = Vec::with_capacity(9 + len);
        out.push(((len >> 16) & 0xFF) as u8);
        out.push(((len >> 8) & 0xFF) as u8);
        out.push((len & 0xFF) as u8);
        out.push(frame_type);
        out.push(flags);
        out.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }
}

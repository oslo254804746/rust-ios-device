//! Direct-RSD and remote-pairing control-channel message codecs.
//!
//! Keeping wire-shape construction and defensive response extraction together
//! makes them independently reviewable and keeps connection orchestration free
//! from nested protocol dictionaries.

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chacha20poly1305::{aead::Aead, KeyInit};
use indexmap::IndexMap;

use super::{CoreError, DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE, DIRECT_CONTROL_CHANNEL_ORIGIN};
use super::pairing::RemotePairingControlChannel;
use crate::lockdown::pairing::VerifyPairSession;
use crate::xpc::message::XpcValue;
use crate::xpc::XpcClient;

#[cfg(feature = "tunnel")]
pub(super) fn build_direct_handshake_request(sequence_number: u64) -> XpcValue {
    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "request",
                    xpc_dict(&[(
                        "_0",
                        xpc_dict(&[(
                            "handshake",
                            xpc_dict(&[(
                                "_0",
                                xpc_dict(&[
                                    (
                                        "hostOptions",
                                        xpc_dict(&[("attemptPairVerify", XpcValue::Bool(true))]),
                                    ),
                                    ("wireProtocolVersion", XpcValue::Int64(19)),
                                ]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

#[cfg(feature = "tunnel")]
pub(super) fn build_direct_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> XpcValue {
    let mut pairs = vec![
        (
            "data",
            XpcValue::Data(bytes::Bytes::copy_from_slice(tlv_data)),
        ),
        ("kind", XpcValue::String(kind.to_string())),
        ("startNewSession", XpcValue::Bool(start_new_session)),
    ];
    if let Some(host) = sending_host {
        pairs.push(("sendingHost", XpcValue::String(host.to_string())));
    }

    build_direct_control_envelope(
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

#[cfg(feature = "tunnel")]
pub(super) fn build_direct_pair_verify_failed_event(sequence_number: u64) -> XpcValue {
    build_direct_control_envelope(
        xpc_dict(&[(
            "plain",
            xpc_dict(&[(
                "_0",
                xpc_dict(&[(
                    "event",
                    xpc_dict(&[("_0", xpc_dict(&[("pairVerifyFailed", xpc_dict(&[]))]))]),
                )]),
            )]),
        )]),
        sequence_number,
    )
}

#[cfg(feature = "tunnel")]
pub(super) fn build_direct_control_envelope(message: XpcValue, sequence_number: u64) -> XpcValue {
    xpc_dict(&[
        (
            "mangledTypeName",
            XpcValue::String(DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE.to_string()),
        ),
        (
            "value",
            xpc_dict(&[
                ("message", message),
                (
                    "originatedBy",
                    XpcValue::String(DIRECT_CONTROL_CHANNEL_ORIGIN.to_string()),
                ),
                ("sequenceNumber", XpcValue::Uint64(sequence_number)),
            ]),
        ),
    ])
}

#[cfg(feature = "tunnel")]
pub(super) async fn create_direct_tcp_listener(
    client: &mut XpcClient,
    session: &VerifyPairSession,
    sequence_number: u64,
) -> Result<u16, CoreError> {
    let nonce = make_direct_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createListener": {
                    "key": BASE64_STANDARD.encode(session.encryption_key),
                    "peerConnectionsInfo": [{
                        "owningPID": std::process::id(),
                        "owningProcessName": "CoreDeviceService",
                    }],
                    "transportProtocolType": "tcp",
                }
            }
        }
    });
    let client_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.client_key).into());
    let encrypted = client_cipher
        .encrypt((&nonce).into(), request.to_string().as_bytes())
        .map_err(|e| CoreError::Other(format!("createListener encrypt failed: {e}")))?;

    client
        .send(build_direct_control_envelope(
            xpc_dict(&[(
                "streamEncrypted",
                xpc_dict(&[("_0", XpcValue::Data(bytes::Bytes::from(encrypted)))]),
            )]),
            sequence_number,
        ))
        .await?;

    let response = client.recv().await?;
    let encrypted_response = extract_direct_stream_encrypted(
        response
            .body
            .as_ref()
            .ok_or_else(|| CoreError::Protocol("createListener response missing body".into()))?,
    )?;
    let server_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into());
    let plaintext = server_cipher
        .decrypt((&nonce).into(), encrypted_response.as_ref())
        .map_err(|e| CoreError::Other(format!("createListener decrypt failed: {e}")))?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext)?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| CoreError::Protocol("createListener response missing response._1".into()))?;

    if let Some(message) = extract_direct_error_extended_message(response_body) {
        return Err(CoreError::Protocol(format!(
            "createListener returned errorExtended: {message}"
        )));
    }

    let port = response_body
        .get("createListener")
        .and_then(|value| value.get("port"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CoreError::Protocol("createListener response missing port".into()))?;
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| CoreError::Protocol(format!("invalid createListener port {port}")))
}

#[cfg(feature = "tunnel")]
pub(super) async fn create_remote_pairing_tcp_listener(
    control: &mut RemotePairingControlChannel,
    session: &VerifyPairSession,
    sequence_number: u64,
) -> Result<u16, CoreError> {
    let nonce = make_direct_encrypted_nonce(0);
    let request = serde_json::json!({
        "request": {
            "_0": {
                "createListener": {
                    "key": BASE64_STANDARD.encode(session.encryption_key),
                    "peerConnectionsInfo": [{
                        "owningPID": std::process::id(),
                        "owningProcessName": "CoreDeviceService",
                    }],
                    "transportProtocolType": "tcp",
                }
            }
        }
    });
    let client_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.client_key).into());
    let encrypted = client_cipher
        .encrypt((&nonce).into(), request.to_string().as_bytes())
        .map_err(|e| {
            CoreError::Other(format!("remote pairing createListener encrypt failed: {e}"))
        })?;

    control
        .send(&serde_json::json!({
            "message": {
                "streamEncrypted": {
                    "_0": BASE64_STANDARD.encode(encrypted),
                }
            },
            "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
            "sequenceNumber": sequence_number,
        }))
        .await?;

    let response = control.recv().await?;
    let encrypted_response = extract_remote_pairing_stream_encrypted(&response)?;
    let server_cipher = chacha20poly1305::ChaCha20Poly1305::new((&session.server_key).into());
    let plaintext = server_cipher
        .decrypt((&nonce).into(), encrypted_response.as_ref())
        .map_err(|e| {
            CoreError::Other(format!("remote pairing createListener decrypt failed: {e}"))
        })?;
    let response: serde_json::Value = serde_json::from_slice(&plaintext)?;
    let response_body = response
        .get("response")
        .and_then(|value| value.get("_1"))
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing createListener response missing response._1".into())
        })?;

    if let Some(message) = extract_direct_error_extended_message(response_body) {
        return Err(CoreError::Protocol(format!(
            "remote pairing createListener returned errorExtended: {message}"
        )));
    }

    let port = response_body
        .get("createListener")
        .and_then(|value| value.get("port"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing createListener response missing port".into())
        })?;
    u16::try_from(port)
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            CoreError::Protocol(format!("invalid remote pairing createListener port {port}"))
        })
}

#[cfg(feature = "tunnel")]
pub(super) fn xpc_dict(pairs: &[(&str, XpcValue)]) -> XpcValue {
    let mut map = IndexMap::new();
    for (key, value) in pairs {
        map.insert((*key).to_string(), value.clone());
    }
    XpcValue::Dictionary(map)
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_direct_remote_identifier(body: &XpcValue) -> Result<String, CoreError> {
    direct_plain_message(body)?
        .get("response")
        .and_then(XpcValue::as_dict)
        .and_then(|response| response.get("_1"))
        .and_then(XpcValue::as_dict)
        .and_then(|response| response.get("handshake"))
        .and_then(XpcValue::as_dict)
        .and_then(|handshake| handshake.get("_0"))
        .and_then(XpcValue::as_dict)
        .and_then(|handshake| handshake.get("peerDeviceInfo"))
        .and_then(XpcValue::as_dict)
        .and_then(|peer| peer.get("identifier"))
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| CoreError::Protocol("handshake missing peerDeviceInfo.identifier".into()))
}

#[cfg(feature = "tunnel")]
pub(super) fn build_remote_pairing_handshake_request(sequence_number: u64) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "request": {
                        "_0": {
                            "handshake": {
                                "_0": {
                                    "hostOptions": {
                                        "attemptPairVerify": true,
                                    },
                                    "wireProtocolVersion": 19,
                                }
                            }
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

#[cfg(feature = "tunnel")]
pub(super) fn build_remote_pairing_pairing_event(
    tlv_data: &[u8],
    kind: &str,
    start_new_session: bool,
    sending_host: Option<&str>,
    sequence_number: u64,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    body.insert(
        "data".into(),
        serde_json::Value::String(BASE64_STANDARD.encode(tlv_data)),
    );
    body.insert("kind".into(), serde_json::Value::String(kind.to_string()));
    body.insert(
        "startNewSession".into(),
        serde_json::Value::Bool(start_new_session),
    );
    if let Some(host) = sending_host {
        body.insert(
            "sendingHost".into(),
            serde_json::Value::String(host.to_string()),
        );
    }

    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "event": {
                        "_0": {
                            "pairingData": {
                                "_0": serde_json::Value::Object(body),
                            }
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

#[cfg(feature = "tunnel")]
pub(super) fn build_remote_pairing_pair_verify_failed_event(sequence_number: u64) -> serde_json::Value {
    serde_json::json!({
        "message": {
            "plain": {
                "_0": {
                    "event": {
                        "_0": {
                            "pairVerifyFailed": {}
                        }
                    }
                }
            }
        },
        "originatedBy": DIRECT_CONTROL_CHANNEL_ORIGIN,
        "sequenceNumber": sequence_number,
    })
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_direct_pairing_tlv(body: &XpcValue) -> Result<Vec<u8>, CoreError> {
    let event = direct_plain_message(body)?
        .get("event")
        .and_then(XpcValue::as_dict)
        .and_then(|event| event.get("_0"))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Protocol("pairing response missing event._0".into()))?;

    if let Some(message) = event
        .get("pairingRejectedWithError")
        .and_then(extract_direct_rejection_message)
    {
        return Err(CoreError::Protocol(format!("pairing rejected: {message}")));
    }

    event
        .get("pairingData")
        .and_then(XpcValue::as_dict)
        .and_then(|pairing| pairing.get("_0"))
        .and_then(XpcValue::as_dict)
        .and_then(|pairing| pairing.get("data"))
        .and_then(|value| match value {
            XpcValue::Data(bytes) => Some(bytes.to_vec()),
            _ => None,
        })
        .ok_or_else(|| CoreError::Protocol("pairing response missing pairingData._0.data".into()))
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_remote_pairing_tlv(body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
    let event = body
        .get("message")
        .and_then(|value| value.get("plain"))
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("event"))
        .and_then(|value| value.get("_0"))
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing response missing message.plain._0.event._0".into())
        })?;

    if let Some(message) = event
        .get("pairingRejectedWithError")
        .and_then(extract_remote_pairing_rejection_message)
    {
        return Err(CoreError::Protocol(format!(
            "remote pairing rejected: {message}"
        )));
    }

    let data = event
        .get("pairingData")
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("data"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CoreError::Protocol("remote pairing response missing pairingData._0.data".into())
        })?;
    BASE64_STANDARD
        .decode(data)
        .map_err(|e| CoreError::Other(format!("invalid remote pairing TLV base64: {e}")))
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_direct_stream_encrypted(body: &XpcValue) -> Result<Vec<u8>, CoreError> {
    direct_control_value(body)?
        .get("message")
        .and_then(XpcValue::as_dict)
        .and_then(|message| message.get("streamEncrypted"))
        .and_then(XpcValue::as_dict)
        .and_then(|encrypted| encrypted.get("_0"))
        .and_then(|value| match value {
            XpcValue::Data(bytes) => Some(bytes.to_vec()),
            _ => None,
        })
        .ok_or_else(|| {
            CoreError::Protocol("encrypted response missing message.streamEncrypted._0".into())
        })
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_remote_pairing_stream_encrypted(body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
    let encoded = body
        .get("message")
        .and_then(|value| value.get("streamEncrypted"))
        .and_then(|value| value.get("_0"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            CoreError::Protocol(
                "remote pairing encrypted response missing message.streamEncrypted._0".into(),
            )
        })?;
    BASE64_STANDARD.decode(encoded).map_err(|e| {
        CoreError::Other(format!(
            "invalid remote pairing encrypted payload base64: {e}"
        ))
    })
}

#[cfg(feature = "tunnel")]
pub(super) fn direct_control_value(body: &XpcValue) -> Result<&IndexMap<String, XpcValue>, CoreError> {
    let envelope = body.as_dict().ok_or_else(|| {
        CoreError::Protocol("direct control message body must be a dictionary".into())
    })?;
    let mangled_type = envelope
        .get("mangledTypeName")
        .and_then(XpcValue::as_str)
        .ok_or_else(|| {
            CoreError::Protocol("direct control message missing mangledTypeName".into())
        })?;
    if mangled_type != DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE {
        return Err(CoreError::Protocol(format!(
            "unexpected direct control channel type {mangled_type}"
        )));
    }
    envelope
        .get("value")
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| CoreError::Protocol("direct control message missing value".into()))
}

#[cfg(feature = "tunnel")]
pub(super) fn direct_plain_message(body: &XpcValue) -> Result<&IndexMap<String, XpcValue>, CoreError> {
    direct_control_value(body)?
        .get("message")
        .and_then(XpcValue::as_dict)
        .and_then(|message| message.get("plain"))
        .and_then(XpcValue::as_dict)
        .and_then(|plain| plain.get("_0"))
        .and_then(XpcValue::as_dict)
        .ok_or_else(|| {
            CoreError::Protocol("direct control message missing message.plain._0".into())
        })
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_direct_rejection_message(value: &XpcValue) -> Option<String> {
    value
        .as_dict()
        .and_then(|wrapped| wrapped.get("wrappedError"))
        .and_then(XpcValue::as_dict)
        .and_then(|wrapped| wrapped.get("userInfo"))
        .and_then(XpcValue::as_dict)
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(XpcValue::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_remote_pairing_rejection_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("wrappedError")
        .and_then(|wrapped| wrapped.get("userInfo"))
        .and_then(|user_info| user_info.get("NSLocalizedDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "tunnel")]
pub(super) fn extract_direct_error_extended_message(value: &serde_json::Value) -> Option<String> {
    value
        .get("errorExtended")
        .and_then(|value| value.get("_0"))
        .and_then(|value| value.get("userInfo"))
        .and_then(|value| value.get("NSLocalizedDescription"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

#[cfg(feature = "tunnel")]
pub(super) fn make_direct_encrypted_nonce(sequence_number: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[..8].copy_from_slice(&sequence_number.to_le_bytes());
    nonce
}

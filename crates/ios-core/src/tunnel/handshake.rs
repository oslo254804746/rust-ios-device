use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::tunnel::{TunnelError, ValidatedMtu};

/// CDTunnel packet magic.
///
/// The two references model the 10-byte header differently: go-ios writes the
/// literal `"CDTunnel\0"` followed by a *single* length byte
/// (`ios/tunnel/tunnel.go`, `exchangeCoreTunnelParameters`), while
/// pymobiledevice3 models it as this 8-byte magic plus a big-endian `u16` length
/// (`remote/tunnel_service.py`, `CDTunnelPacketData`). The two produce identical
/// bytes for any body shorter than 256, because the `u16`'s high byte is exactly
/// the NUL that terminates go-ios's magic — which is the only reason the `u16`
/// model here has ever interoperated.
///
/// We keep the permissive pymobiledevice3 model on read, but must obey the
/// stricter go-ios one on write: see [`MAX_BODY_LEN`].
const MAGIC: &[u8] = b"CDTunnel";
const HEADER_LEN: usize = 10;

/// Largest body a single length byte can express.
///
/// A device parsing the go-ios way reads only `header[9]` as the length, so a
/// body of 256 bytes or more would both truncate and put a non-zero byte where
/// that parser expects the magic's NUL terminator. Refuse to emit such a frame
/// rather than send something only half the ecosystem can read.
const MAX_BODY_LEN: usize = u8::MAX as usize;

/// Information returned by the CDTunnel handshake.
#[derive(Clone)]
pub struct TunnelInfo {
    pub server_address: String,
    pub server_rsd_port: u16,
    pub client_address: String,
    pub client_mtu: u32,
}

impl std::fmt::Debug for TunnelInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelInfo")
            .field(
                "server",
                &crate::redaction::diagnostic_id(&self.server_address),
            )
            .field("rsd_port", &self.server_rsd_port)
            .field(
                "client",
                &crate::redaction::diagnostic_id(&self.client_address),
            )
            .field("mtu", &self.client_mtu)
            .finish()
    }
}

fn parse_nonzero_u16(raw: &serde_json::Value, field: &str) -> Result<u16, TunnelError> {
    let value = raw
        .as_u64()
        .ok_or_else(|| TunnelError::Protocol(format!("missing {field}")))?;
    let port = u16::try_from(value).map_err(|source| {
        TunnelError::Io(crate::error::context::received_io(
            "decode CDTunnel port",
            Some(value),
            source,
        ))
    })?;
    if port == 0 {
        return Err(TunnelError::Protocol(format!(
            "invalid {field}: expected integer in 1..={}",
            u16::MAX,
        )));
    }
    Ok(port)
}

fn parse_supported_mtu(raw: &serde_json::Value, field: &str) -> Result<u32, TunnelError> {
    let raw_value = raw
        .as_u64()
        .ok_or_else(|| TunnelError::Protocol(format!("missing or invalid {field}")))?;
    let value = u32::try_from(raw_value).map_err(|source| {
        TunnelError::Io(crate::error::context::received_io(
            "decode CDTunnel MTU",
            Some(raw_value),
            source,
        ))
    })?;
    ValidatedMtu::new(value).map_err(|source| {
        TunnelError::Io(crate::error::context::received_io(
            "validate received CDTunnel MTU",
            Some(raw_value),
            source,
        ))
    })?;
    Ok(value)
}

pub fn encode_handshake_request(mtu: u32) -> Result<Vec<u8>, TunnelError> {
    let mtu = ValidatedMtu::new(mtu)?;
    let json = serde_json::json!({
        "type": "clientHandshakeRequest",
        "mtu": mtu.as_u16(),
    });
    let json_bytes = serde_json::to_vec(&json).map_err(|source| {
        TunnelError::Io(crate::error::context::retained_io(
            std::io::ErrorKind::InvalidInput,
            "encode CDTunnel request",
            source,
        ))
    })?;
    if json_bytes.len() > MAX_BODY_LEN {
        return Err(TunnelError::Protocol(format!(
            "handshake JSON exceeds {MAX_BODY_LEN} bytes"
        )));
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    let body_len = u16::try_from(json_bytes.len()).map_err(|source| {
        TunnelError::Io(crate::error::context::retained_io(
            std::io::ErrorKind::InvalidInput,
            "encode CDTunnel body length",
            source,
        ))
    })?;
    buf.extend_from_slice(&body_len.to_be_bytes());
    buf.extend_from_slice(&json_bytes);
    Ok(buf)
}

pub async fn exchange_tunnel_parameters_with_timeout<S>(
    stream: &mut S,
    timeout: Duration,
) -> Result<TunnelInfo, TunnelError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(timeout, exchange_tunnel_parameters_inner(stream))
        .await
        .map_err(|source| {
            TunnelError::Io(crate::error::context::deadline_io(
                "CDTunnel handshake",
                timeout,
                source,
            ))
        })?
}

async fn exchange_tunnel_parameters_inner<S>(stream: &mut S) -> Result<TunnelInfo, TunnelError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req = encode_handshake_request(1280)?;
    stream.write_all(&req).await?;
    stream.flush().await?;

    // Response: "CDTunnel" (8 bytes) + body_len (u16, big-endian) = 10 bytes header.
    // go-ios reads the same 10 bytes as `"CDTunnel\0"` + one length byte; see MAGIC.
    let mut header = [0u8; HEADER_LEN];
    stream.read_exact(&mut header).await?;

    if &header[..MAGIC.len()] != MAGIC {
        return Err(TunnelError::Protocol(format!(
            "invalid CDTunnel magic: {:?}",
            &header[..MAGIC.len()]
        )));
    }

    let body_len = u16::from_be_bytes([header[8], header[9]]) as usize;
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body).await?;

    let raw: serde_json::Value = serde_json::from_slice(&body).map_err(|source| {
        TunnelError::Io(crate::error::context::received_io(
            "decode CDTunnel response",
            None,
            source,
        ))
    })?;

    Ok(TunnelInfo {
        server_address: raw["serverAddress"]
            .as_str()
            .ok_or_else(|| TunnelError::Protocol("missing serverAddress".into()))?
            .to_string(),
        server_rsd_port: parse_nonzero_u16(&raw["serverRSDPort"], "serverRSDPort")?,
        client_address: raw["clientParameters"]["address"]
            .as_str()
            .ok_or_else(|| TunnelError::Protocol("missing clientParameters.address".into()))?
            .to_string(),
        client_mtu: parse_supported_mtu(&raw["clientParameters"]["mtu"], "clientParameters.mtu")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn connection_native_local_mtu_has_input_role_and_only_real_overflow_source() {
        use std::error::Error;
        for value in [0, 1279, 65536, u32::MAX] {
            let error = encode_handshake_request(value).unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::InvalidInput
            );
            let TunnelError::Io(io) = &error else {
                panic!("MTU role lost")
            };
            assert_eq!(io.kind(), std::io::ErrorKind::InvalidInput);
            assert_eq!(io.source().is_some(), value > u16::MAX as u32);
            if let Some(source) = io.source() {
                assert!(source.is::<std::num::TryFromIntError>());
            }
            assert!(error.to_string().contains(&value.to_string()));
        }
    }

    #[tokio::test]
    async fn connection_native_received_handshake_keeps_original_json_and_integer_chains() {
        use std::error::Error;
        let cases = [
            b"owned-private-invalid-json".to_vec(),
            br#"{"serverAddress":"owned-private","serverRSDPort":65536,"clientParameters":{"address":"owned-private","mtu":1280}}"#.to_vec(),
            br#"{"serverAddress":"owned-private","serverRSDPort":1234,"clientParameters":{"address":"owned-private","mtu":18446744073709551615}}"#.to_vec(),
            br#"{"serverAddress":"owned-private","serverRSDPort":1234,"clientParameters":{"address":"owned-private","mtu":65536}}"#.to_vec(),
            br#"{"serverAddress":"owned-private","serverRSDPort":1234,"clientParameters":{"address":"owned-private","mtu":0}}"#.to_vec(),
        ];
        for (index, body) in cases.into_iter().enumerate() {
            let (mut client, mut peer) = tokio::io::duplex(512);
            let caller = async move {
                let error =
                    exchange_tunnel_parameters_with_timeout(&mut client, Duration::from_secs(1))
                        .await
                        .unwrap_err();
                assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
                assert!(!format!("{error:?}").contains("owned-private"));
                let TunnelError::Io(io) = &error else {
                    panic!("received source lost")
                };
                assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
                assert!(!format!("{:?}", io.get_ref()).contains("owned-private"));
                let mut source = Some(&error as &(dyn Error + 'static));
                let mut json = false;
                let mut integer = false;
                let mut inner_input = false;
                for _ in 0..8 {
                    let Some(current) = source else { break };
                    json |= current.is::<serde_json::Error>();
                    integer |= current.is::<std::num::TryFromIntError>();
                    if let Some(io) = current.downcast_ref::<std::io::Error>() {
                        inner_input |= io.kind() == std::io::ErrorKind::InvalidInput;
                    }
                    source = current.source();
                }
                assert_eq!(json, index == 0);
                assert_eq!(integer, (1..=3).contains(&index));
                assert_eq!(inner_input, index >= 3);
                drop(client);
            };
            let server = async move {
                peer.write_all(b"CDTunnel").await.unwrap();
                peer.write_all(&(body.len() as u16).to_be_bytes())
                    .await
                    .unwrap();
                peer.write_all(&body).await.unwrap();
                peer.shutdown().await.unwrap();
                let mut request = Vec::new();
                peer.read_to_end(&mut request).await.unwrap();
                assert_eq!(request, encode_handshake_request(1280).unwrap());
            };
            tokio::time::timeout(Duration::from_secs(1), async {
                tokio::join!(caller, server);
            })
            .await
            .unwrap();
        }
    }

    #[tokio::test]
    async fn connection_native_handshake_timeout_keeps_issued_elapsed() {
        use std::error::Error;
        let (mut stream, mut peer) = tokio::io::duplex(512);
        let error = exchange_tunnel_parameters_with_timeout(&mut stream, Duration::ZERO)
            .await
            .unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let TunnelError::Io(io) = &error else {
            panic!("timer lost")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        assert!(io.source().unwrap().is::<tokio::time::error::Elapsed>());
        assert!(error.to_string().contains("budget=0ns"));
        drop(stream);
        let mut request = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut request))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request, encode_handshake_request(1280).unwrap());
    }

    #[test]
    fn tunnel_info_debug_hides_both_address_types_without_changing_selectors() {
        let secret = "synthetic-private-tunnel-address";
        let info = TunnelInfo {
            server_address: secret.into(),
            server_rsd_port: 1234,
            client_address: secret.into(),
            client_mtu: 1280,
        };
        let public_info = crate::tunnel::TunnelInfo {
            server_address: secret.into(),
            server_rsd_port: 1234,
            client_address: secret.into(),
            client_mtu: 1280,
        };
        for output in [
            format!("{info:?} {info:#?}"),
            format!("{public_info:?} {public_info:#?}"),
        ] {
            assert!(!output.contains(secret));
            assert!(output.contains("id:"));
            assert!(output.contains("1280"));
        }
        assert_eq!(info.server_address, secret);
        assert_eq!(public_info.client_address, secret);
    }

    async fn exchange_with_response_json(
        response_json: serde_json::Value,
    ) -> Result<TunnelInfo, TunnelError> {
        let response_bytes = serde_json::to_vec(&response_json).unwrap();
        let mut response = Vec::new();
        response.extend_from_slice(b"CDTunnel");
        response.extend_from_slice(&(response_bytes.len() as u16).to_be_bytes());
        response.extend_from_slice(&response_bytes);

        let (mut client, mut server) = tokio::io::duplex(4096);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 256];
            let _ = server.read(&mut buf).await;
            server.write_all(&response).await.unwrap();
        });

        exchange_tunnel_parameters_with_timeout(&mut client, Duration::from_secs(5)).await
    }

    #[test]
    fn test_encode_cdtunnel_request() {
        let bytes = encode_handshake_request(1280).unwrap();
        assert_eq!(&bytes[..8], b"CDTunnel");
        let json_len = u16::from_be_bytes([bytes[8], bytes[9]]) as usize;
        let json: serde_json::Value = serde_json::from_slice(&bytes[10..10 + json_len]).unwrap();
        assert_eq!(json["type"], "clientHandshakeRequest");
        assert_eq!(json["mtu"], 1280);
    }

    /// Requests must be byte-identical to what go-ios puts on the wire, since a
    /// device parsing that way reads only the last header byte as the length.
    #[test]
    fn test_encode_cdtunnel_request_matches_go_ios_framing() {
        let bytes = encode_handshake_request(1280).unwrap();
        let body = &bytes[HEADER_LEN..];
        assert!(body.len() <= MAX_BODY_LEN);

        let mut expected = Vec::new();
        expected.extend_from_slice(b"CDTunnel\0");
        expected.push(body.len() as u8);
        expected.extend_from_slice(body);

        assert_eq!(bytes, expected);
    }

    #[test]
    fn test_encode_handshake_request_validates_mtu_before_writing() {
        assert!(encode_handshake_request(0).is_err());
        assert!(encode_handshake_request(1279).is_err());
        assert!(encode_handshake_request(u16::MAX as u32 + 1).is_err());
        assert!(encode_handshake_request(u32::MAX).is_err());

        let bytes = encode_handshake_request(9000).unwrap();
        let body_len = u16::from_be_bytes([bytes[8], bytes[9]]) as usize;
        let json: serde_json::Value = serde_json::from_slice(&bytes[10..10 + body_len]).unwrap();
        assert_eq!(json["mtu"], 9000);
    }

    #[test]
    fn test_validate_mtu_boundaries() {
        for value in [0, 1279] {
            let error = ValidatedMtu::new(value).unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::InvalidInput
            );
            let TunnelError::Io(io) = error else {
                panic!("MTU input role lost")
            };
            assert!(io.to_string().contains("IPv6 requires at least 1280 bytes"));
            assert!(io.source().is_none());
        }

        assert_eq!(ValidatedMtu::new(1280).unwrap().as_u16(), 1280);
        assert_eq!(ValidatedMtu::new(1500).unwrap().as_u16(), 1500);
        assert_eq!(ValidatedMtu::new(9000).unwrap().as_u16(), 9000);
        assert_eq!(
            ValidatedMtu::new(u16::MAX as u32).unwrap().as_u16(),
            u16::MAX
        );

        for value in [u16::MAX as u32 + 1, u32::MAX] {
            let error = ValidatedMtu::new(value).unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::InvalidInput
            );
            let TunnelError::Io(io) = error else {
                panic!("MTU overflow lost")
            };
            assert!(io
                .to_string()
                .contains("exceeds the supported maximum 65535 bytes"));
            assert!(io.source().unwrap().is::<std::num::TryFromIntError>());
        }
    }

    /// Reads stay on the permissive pymobiledevice3 model (`Int16ub`), so a body
    /// past the single-length-byte limit is still accepted rather than rejected.
    #[tokio::test]
    async fn test_exchange_tunnel_parameters_with_16_bit_length() {
        let response_json = serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 58783u16,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": 1280u32,
                "padding": "x".repeat(300),
            }
        });
        let response_bytes = serde_json::to_vec(&response_json).unwrap();
        assert!(response_bytes.len() > 255);

        let params = exchange_with_response_json(response_json).await.unwrap();
        assert_eq!(params.server_address, "fd59:2381:6956::1");
        assert_eq!(params.server_rsd_port, 58783);
        assert_eq!(params.client_address, "fd59:2381:6956::2");
        assert_eq!(params.client_mtu, 1280);
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_rejects_zero_server_rsd_port() {
        let err = exchange_with_response_json(serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 0,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": 1280,
            }
        }))
        .await
        .unwrap_err();

        match err {
            TunnelError::Protocol(message) => {
                assert!(
                    message.contains("invalid serverRSDPort"),
                    "unexpected error: {message}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_rejects_out_of_range_server_rsd_port() {
        let err = exchange_with_response_json(serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 65536u64,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": 1280,
            }
        }))
        .await
        .unwrap_err();

        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Protocol);
        let TunnelError::Io(io) = err else {
            panic!("received port source lost")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
        assert!(io.source().unwrap().is::<std::num::TryFromIntError>());
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_rejects_zero_client_mtu() {
        let err = exchange_with_response_json(serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 58783,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": 0,
            }
        }))
        .await
        .unwrap_err();

        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Protocol);
        let TunnelError::Io(io) = err else {
            panic!("received MTU source lost")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::InvalidData);
        let source = io.source().unwrap().downcast_ref::<TunnelError>().unwrap();
        assert_eq!(
            crate::classify_error(source),
            crate::ErrorClass::InvalidInput
        );
        let TunnelError::Io(inner) = source else {
            panic!("MTU input role lost")
        };
        assert!(inner.source().is_none());
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_rejects_out_of_range_client_mtu() {
        let err = exchange_with_response_json(serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 58783,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": 65536u32,
            }
        }))
        .await
        .unwrap_err();

        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Protocol);
        let TunnelError::Io(io) = err else {
            panic!("received MTU source lost")
        };
        let source = io.source().unwrap().downcast_ref::<TunnelError>().unwrap();
        let TunnelError::Io(inner) = source else {
            panic!("MTU integer source lost")
        };
        assert!(inner.source().unwrap().is::<std::num::TryFromIntError>());
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_rejects_u32_max_client_mtu() {
        let err = exchange_with_response_json(serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 58783,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": u32::MAX,
            }
        }))
        .await
        .unwrap_err();

        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Protocol);
        let TunnelError::Io(io) = err else {
            panic!("received MTU source lost")
        };
        let source = io.source().unwrap().downcast_ref::<TunnelError>().unwrap();
        let TunnelError::Io(inner) = source else {
            panic!("MTU integer source lost")
        };
        assert!(inner
            .to_string()
            .contains("exceeds the supported maximum 65535 bytes"));
        assert!(inner.source().unwrap().is::<std::num::TryFromIntError>());
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_timeout() {
        let (mut client, _server) = tokio::io::duplex(4096);
        let err = exchange_tunnel_parameters_with_timeout(&mut client, Duration::from_millis(20))
            .await
            .unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        let TunnelError::Io(io) = err else {
            panic!("timer source lost")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        assert!(io.source().unwrap().is::<tokio::time::error::Elapsed>());
    }
}

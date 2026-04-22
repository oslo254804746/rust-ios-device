use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::TunnelError;

const MAGIC: &[u8] = b"CDTunnel";
const HEADER_LEN: usize = 10;
const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Information returned by the CDTunnel handshake.
#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub server_address: String,
    pub server_rsd_port: u16,
    pub client_address: String,
    pub client_mtu: u32,
}

fn parse_nonzero_u16(raw: &serde_json::Value, field: &str) -> Result<u16, TunnelError> {
    let value = raw
        .as_u64()
        .ok_or_else(|| TunnelError::Protocol(format!("missing {field}")))?;
    u16::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            TunnelError::Protocol(format!(
                "invalid {field}: expected integer in 1..={}",
                u16::MAX
            ))
        })
}

fn parse_nonzero_u32(raw: &serde_json::Value, field: &str) -> Result<u32, TunnelError> {
    let value = raw
        .as_u64()
        .ok_or_else(|| TunnelError::Protocol(format!("missing {field}")))?;
    u32::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            TunnelError::Protocol(format!(
                "invalid {field}: expected integer in 1..={}",
                u32::MAX
            ))
        })
}

pub fn encode_handshake_request(mtu: u32) -> Result<Vec<u8>, TunnelError> {
    let json = serde_json::json!({
        "type": "clientHandshakeRequest",
        "mtu": mtu,
    });
    let json_bytes = serde_json::to_vec(&json)
        .map_err(|e| TunnelError::Protocol(format!("failed to serialize handshake: {e}")))?;
    if json_bytes.len() > u16::MAX as usize {
        return Err(TunnelError::Protocol(
            "handshake JSON exceeds 65535 bytes".into(),
        ));
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&(json_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(&json_bytes);
    Ok(buf)
}

/// Perform the CDTunnel handshake over the given stream.
pub async fn exchange_tunnel_parameters<S>(stream: &mut S) -> Result<TunnelInfo, TunnelError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    exchange_tunnel_parameters_with_timeout(stream, DEFAULT_HANDSHAKE_TIMEOUT).await
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
        .map_err(|_| {
            TunnelError::Protocol(format!(
                "CDTunnel handshake timed out after {} ms",
                timeout.as_millis()
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

    // Response: "CDTunnel" (8 bytes) + body_len (u16, big-endian) = 10 bytes header
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

    let raw: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| TunnelError::Protocol(format!("invalid CDTunnel JSON: {e}")))?;

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
        client_mtu: parse_nonzero_u32(&raw["clientParameters"]["mtu"], "clientParameters.mtu")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

        exchange_tunnel_parameters(&mut client).await
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

        match err {
            TunnelError::Protocol(message) => {
                assert!(
                    message.contains("invalid clientParameters.mtu"),
                    "unexpected error: {message}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_rejects_out_of_range_client_mtu() {
        let err = exchange_with_response_json(serde_json::json!({
            "serverAddress": "fd59:2381:6956::1",
            "serverRSDPort": 58783,
            "clientParameters": {
                "address": "fd59:2381:6956::2",
                "mtu": 4294967296u64,
            }
        }))
        .await
        .unwrap_err();

        match err {
            TunnelError::Protocol(message) => {
                assert!(
                    message.contains("invalid clientParameters.mtu"),
                    "unexpected error: {message}"
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_exchange_tunnel_parameters_timeout() {
        let (mut client, _server) = tokio::io::duplex(4096);
        let err = exchange_tunnel_parameters_with_timeout(&mut client, Duration::from_millis(20))
            .await
            .unwrap_err();
        match err {
            TunnelError::Protocol(message) => {
                assert!(message.contains("timed out"), "unexpected error: {message}");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}

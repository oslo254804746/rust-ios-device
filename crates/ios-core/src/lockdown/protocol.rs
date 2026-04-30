use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::lockdown::LockdownError;

pub const LOCKDOWN_PORT: u16 = 62078;
const MAX_LOCKDOWN_FRAME_SIZE: usize = 4 * 1024 * 1024;

pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    debug_assert!(
        payload.len() <= u32::MAX as usize,
        "lockdown frame payload exceeds u32::MAX"
    );
    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

pub async fn recv_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, LockdownError> {
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let length = u32::from_be_bytes(len_buf) as usize;
    if length > MAX_LOCKDOWN_FRAME_SIZE {
        return Err(LockdownError::Protocol(format!(
            "frame too large: {length} bytes exceeds {MAX_LOCKDOWN_FRAME_SIZE}"
        )));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await?;
    Ok(payload)
}

pub async fn send_lockdown<W, T>(writer: &mut W, value: &T) -> Result<(), LockdownError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, value).map_err(|e| LockdownError::Protocol(e.to_string()))?;
    writer.write_all(&encode_frame(&bytes)).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn recv_lockdown<R, T>(reader: &mut R) -> Result<T, LockdownError>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let payload = recv_frame(reader).await?;
    plist::from_bytes(&payload).map_err(|e| LockdownError::Protocol(e.to_string()))
}

// ── Request / Response structs ────────────────────────────

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryTypeRequest {
    pub label: &'static str,
    pub request: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct QueryTypeResponse {
    #[serde(rename = "Type")]
    pub type_: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetValueRequest<'a> {
    pub label: &'static str,
    pub request: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct GetValueResponse {
    pub value: plist::Value,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SetValueRequest<'a, T>
where
    T: Serialize,
{
    pub label: &'static str,
    pub request: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<&'a str>,
    pub value: T,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct RemoveValueRequest<'a> {
    pub label: &'static str,
    pub request: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ValueOperationResponse {
    #[serde(rename = "Error")]
    pub error: Option<String>,
    #[serde(rename = "Value")]
    pub value: Option<plist::Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartSessionRequest {
    pub label: &'static str,
    pub protocol_version: &'static str,
    pub request: &'static str,
    #[serde(rename = "HostID")]
    pub host_id: String,
    #[serde(rename = "SystemBUID")]
    pub system_buid: String,
}

#[derive(Debug, Deserialize)]
pub struct StartSessionResponse {
    #[serde(rename = "SessionID")]
    pub session_id: String,
    #[serde(rename = "EnableSessionSSL")]
    pub enable_session_ssl: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartServiceRequest {
    pub label: &'static str,
    pub request: &'static str,
    pub service: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StartServiceResponse {
    #[serde(rename = "Port")]
    pub port: Option<u16>,
    #[serde(rename = "EnableServiceSSL")]
    pub enable_service_ssl: Option<bool>,
    /// Device may return an Error field instead of Port when service is unavailable.
    #[serde(rename = "Error")]
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct StopSessionRequest {
    pub label: &'static str,
    pub request: &'static str,
    #[serde(rename = "SessionID")]
    pub session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_lockdown_frame() {
        let payload = b"hello";
        let frame = encode_frame(payload);
        assert_eq!(&frame[..4], &5u32.to_be_bytes());
        assert_eq!(&frame[4..], payload);
    }

    #[tokio::test]
    async fn test_roundtrip_frame() {
        let payload = b"<plist/>";
        let frame = encode_frame(payload);
        let mut cursor = std::io::Cursor::new(frame);
        let decoded = recv_frame(&mut cursor).await.unwrap();
        assert_eq!(decoded, payload);
    }

    #[tokio::test]
    async fn test_recv_frame_empty_payload() {
        let frame = encode_frame(b"");
        let mut cursor = std::io::Cursor::new(frame);
        let decoded = recv_frame(&mut cursor).await.unwrap();
        assert!(decoded.is_empty());
    }

    #[tokio::test]
    async fn test_recv_frame_rejects_oversized_payload() {
        let mut frame = ((MAX_LOCKDOWN_FRAME_SIZE as u32) + 1)
            .to_be_bytes()
            .to_vec();
        frame.extend_from_slice(b"ignored");
        let mut cursor = std::io::Cursor::new(frame);

        let err = recv_frame(&mut cursor).await.unwrap_err();
        assert!(
            matches!(err, LockdownError::Protocol(message) if message.contains("frame too large"))
        );
    }

    #[test]
    fn test_set_value_request_serializes_domain_key_and_value() {
        let request = SetValueRequest {
            label: "ios-rs",
            request: "SetValue",
            domain: Some("com.apple.international"),
            key: Some("Language"),
            value: "en",
        };

        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &request).unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        assert!(xml.contains("<key>Request</key>"));
        assert!(xml.contains("<string>SetValue</string>"));
        assert!(xml.contains("<key>Domain</key>"));
        assert!(xml.contains("<string>com.apple.international</string>"));
        assert!(xml.contains("<key>Key</key>"));
        assert!(xml.contains("<string>Language</string>"));
        assert!(xml.contains("<key>Value</key>"));
        assert!(xml.contains("<string>en</string>"));
    }

    #[test]
    fn test_remove_value_request_serializes_domain_and_key() {
        let request = RemoveValueRequest {
            label: "ios-rs",
            request: "RemoveValue",
            domain: Some("com.apple.mobile.wireless_lockdown"),
            key: Some("EnableWifiConnections"),
        };

        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &request).unwrap();
        let xml = String::from_utf8(bytes).unwrap();

        assert!(xml.contains("<key>Request</key>"));
        assert!(xml.contains("<string>RemoveValue</string>"));
        assert!(xml.contains("<key>Domain</key>"));
        assert!(xml.contains("<string>com.apple.mobile.wireless_lockdown</string>"));
        assert!(xml.contains("<key>Key</key>"));
        assert!(xml.contains("<string>EnableWifiConnections</string>"));
    }
}

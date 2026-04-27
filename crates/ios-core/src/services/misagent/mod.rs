//! MiSAgent – provisioning profile management.
//!
//! Service: `com.apple.misagent`
//! Protocol: plist-framed (same 4-byte BE length prefix as lockdown).

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.misagent";

#[derive(Debug, thiserror::Error)]
pub enum MisagentError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("status {0}")]
    Status(u32),
}

/// Provisioning profile entry.
#[derive(Debug, Clone)]
pub struct Profile {
    pub uuid: String,
    pub name: String,
    pub app_id: String,
    pub expiry_date: Option<String>,
    pub raw_data: Vec<u8>,
}

/// MiSAgent client.
pub struct MisagentClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> MisagentClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Copy (retrieve) all installed provisioning profiles.
    pub async fn copy_all(&mut self) -> Result<Vec<Vec<u8>>, MisagentError> {
        self.send_value(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "MessageType".to_string(),
                plist::Value::String("CopyAll".into()),
            ),
            (
                "ProfileType".to_string(),
                plist::Value::String("Provisioning".into()),
            ),
        ])))
        .await?;

        let data = self.recv_raw().await?;
        let val: plist::Value =
            plist::from_bytes(&data).map_err(|e| MisagentError::Plist(e.to_string()))?;

        let status = val
            .as_dictionary()
            .and_then(|d| d.get("Status"))
            .and_then(|v| v.as_unsigned_integer())
            .unwrap_or(0) as u32;

        if status != 0 {
            return Err(MisagentError::Status(status));
        }

        let profiles = val
            .as_dictionary()
            .and_then(|d| d.get("Payload"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_data().map(|d| d.to_vec()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(profiles)
    }

    /// Copy all installed provisioning profiles and decode basic metadata.
    pub async fn list_profiles(&mut self) -> Result<Vec<Profile>, MisagentError> {
        let raw_profiles = self.copy_all().await?;
        raw_profiles
            .into_iter()
            .map(|raw_data| decode_profile(&raw_data))
            .collect()
    }

    /// Install a provisioning profile (raw DER/XML data).
    pub async fn install(&mut self, profile_data: &[u8]) -> Result<(), MisagentError> {
        self.send_value(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "MessageType".to_string(),
                plist::Value::String("Install".into()),
            ),
            (
                "ProfileType".to_string(),
                plist::Value::String("Provisioning".into()),
            ),
            (
                "Profile".to_string(),
                plist::Value::Data(profile_data.to_vec()),
            ),
        ])))
        .await?;
        let data = self.recv_raw().await?;
        let val: plist::Value =
            plist::from_bytes(&data).map_err(|e| MisagentError::Plist(e.to_string()))?;
        let status = val
            .as_dictionary()
            .and_then(|d| d.get("Status"))
            .and_then(|v| v.as_unsigned_integer())
            .unwrap_or(0) as u32;
        if status != 0 {
            return Err(MisagentError::Status(status));
        }
        Ok(())
    }

    /// Remove a provisioning profile by UUID.
    pub async fn remove(&mut self, uuid: &str) -> Result<(), MisagentError> {
        self.send_value(plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "MessageType".to_string(),
                plist::Value::String("Remove".into()),
            ),
            (
                "ProfileType".to_string(),
                plist::Value::String("Provisioning".into()),
            ),
            (
                "ProfileID".to_string(),
                plist::Value::String(uuid.to_string()),
            ),
        ])))
        .await?;
        let data = self.recv_raw().await?;
        let val: plist::Value =
            plist::from_bytes(&data).map_err(|e| MisagentError::Plist(e.to_string()))?;
        let status = val
            .as_dictionary()
            .and_then(|d| d.get("Status"))
            .and_then(|v| v.as_unsigned_integer())
            .unwrap_or(0) as u32;
        if status != 0 {
            return Err(MisagentError::Status(status));
        }
        Ok(())
    }

    async fn send_value(&mut self, plist_val: plist::Value) -> Result<(), MisagentError> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &plist_val)
            .map_err(|e| MisagentError::Plist(e.to_string()))?;
        self.stream
            .write_all(&(buf.len() as u32).to_be_bytes())
            .await?;
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv_raw(&mut self) -> Result<Vec<u8>, MisagentError> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
        if len > MAX_PLIST_SIZE {
            return Err(MisagentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"),
            )));
        }
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        Ok(buf)
    }
}

fn decode_profile(raw_data: &[u8]) -> Result<Profile, MisagentError> {
    let plist_bytes = embedded_plist_bytes(raw_data)?;
    let value: plist::Value =
        plist::from_bytes(plist_bytes).map_err(|e| MisagentError::Plist(e.to_string()))?;
    let dict = value.into_dictionary().ok_or_else(|| {
        MisagentError::Protocol("provisioning profile payload was not a dictionary".into())
    })?;

    let uuid = required_string(&dict, "UUID")?;
    let name = dict
        .get("Name")
        .and_then(plist::Value::as_string)
        .unwrap_or(&uuid)
        .to_string();
    let app_id = dict
        .get("AppIDName")
        .and_then(plist::Value::as_string)
        .or_else(|| {
            dict.get("ApplicationIdentifierPrefix")
                .and_then(plist::Value::as_array)
                .and_then(|arr| arr.first())
                .and_then(plist::Value::as_string)
        })
        .unwrap_or("")
        .to_string();
    let expiry_date = dict.get("ExpirationDate").map(plist_value_to_string);

    Ok(Profile {
        uuid,
        name,
        app_id,
        expiry_date,
        raw_data: raw_data.to_vec(),
    })
}

fn embedded_plist_bytes(raw_data: &[u8]) -> Result<&[u8], MisagentError> {
    let start = find_bytes(raw_data, b"<?xml").or_else(|| find_bytes(raw_data, b"<plist"));
    let end = find_bytes(raw_data, b"</plist>");
    match (start, end) {
        (Some(start), Some(end)) if end >= start => Ok(&raw_data[start..end + b"</plist>".len()]),
        _ => Err(MisagentError::Protocol(
            "could not locate embedded plist in provisioning profile".into(),
        )),
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn required_string(dict: &plist::Dictionary, key: &str) -> Result<String, MisagentError> {
    dict.get(key)
        .and_then(plist::Value::as_string)
        .map(ToOwned::to_owned)
        .ok_or_else(|| MisagentError::Protocol(format!("missing provisioning profile key {key}")))
}

fn plist_value_to_string(value: &plist::Value) -> String {
    match value {
        plist::Value::String(s) => s.clone(),
        plist::Value::Date(d) => d.to_xml_format(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    #[derive(Default)]
    struct MockStream {
        read_buf: Vec<u8>,
        written: Vec<u8>,
        read_pos: usize,
    }

    impl MockStream {
        fn with_response(value: plist::Value) -> Self {
            let mut payload = Vec::new();
            plist::to_writer_xml(&mut payload, &value).unwrap();
            let mut read_buf = Vec::new();
            read_buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            read_buf.extend_from_slice(&payload);
            Self {
                read_buf,
                written: Vec::new(),
                read_pos: 0,
            }
        }
    }

    impl AsyncRead for MockStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let remaining = self.read_buf.len().saturating_sub(self.read_pos);
            if remaining == 0 {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "no more test data",
                )));
            }
            let to_copy = remaining.min(buf.remaining());
            let start = self.read_pos;
            let end = start + to_copy;
            buf.put_slice(&self.read_buf[start..end]);
            self.read_pos = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for MockStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn decode_profile_extracts_basic_metadata_from_embedded_plist() {
        let xml = br#"garbage<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>UUID</key><string>ABC-123</string>
<key>Name</key><string>Example Dev Profile</string>
<key>AppIDName</key><string>Example App</string>
<key>ExpirationDate</key><date>2026-04-08T00:00:00Z</date>
</dict></plist>trailer"#;

        let profile = decode_profile(xml).unwrap();
        assert_eq!(profile.uuid, "ABC-123");
        assert_eq!(profile.name, "Example Dev Profile");
        assert_eq!(profile.app_id, "Example App");
        assert_eq!(profile.expiry_date.as_deref(), Some("2026-04-08T00:00:00Z"));
    }

    #[test]
    fn decode_profile_errors_without_embedded_plist() {
        let err = decode_profile(b"not-a-profile").unwrap_err();
        assert!(
            matches!(err, MisagentError::Protocol(message) if message.contains("embedded plist"))
        );
    }

    #[tokio::test]
    async fn copy_all_uses_copy_all_message_type() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Status".to_string(), plist::Value::Integer(0.into())),
            ("Payload".to_string(), plist::Value::Array(Vec::new())),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = MisagentClient::new(&mut stream);

        let _ = client.copy_all().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("MessageType").and_then(plist::Value::as_string),
            Some("CopyAll")
        );
    }

    #[tokio::test]
    async fn install_uses_profile_field() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::Integer(0.into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = MisagentClient::new(&mut stream);

        client.install(b"PROFILE-DATA").await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("MessageType").and_then(plist::Value::as_string),
            Some("Install")
        );
        assert_eq!(
            dict.get("Profile").and_then(plist::Value::as_data),
            Some(&b"PROFILE-DATA"[..])
        );
    }
}

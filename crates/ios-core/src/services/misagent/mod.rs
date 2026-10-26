//! MiSAgent – provisioning profile management.
//!
//! Service: `com.apple.misagent`
//! Protocol: plist-framed (same 4-byte BE length prefix as lockdown).

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    time::Instant,
};

pub const SERVICE_NAME: &str = "com.apple.misagent";

service_error!(
    MisagentError,
    #[error("status {0}")]
    Status(u64),
);

/// Provisioning profile entry.
#[derive(Debug, Clone)]
pub struct Profile {
    pub uuid: String,
    pub name: String,
    pub app_id: String,
    pub expiry_date: Option<String>,
    pub raw_data: Vec<u8>,
}

/// MiSAgent client with explicit operation deadline and connection state.
pub struct MisagentClient<S> {
    pub(crate) stream: Option<S>,
    usable: bool,
    operation_timeout: Option<Duration>,
}

impl<S> MisagentClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            usable: true,
            operation_timeout: None,
        }
    }

    pub fn is_usable(&self) -> bool {
        self.usable && self.stream.is_some()
    }

    pub fn set_operation_timeout(&mut self, timeout: Option<Duration>) {
        self.operation_timeout = timeout;
    }

    pub fn with_operation_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.set_operation_timeout(timeout);
        self
    }

    fn ensure_usable(&self) -> Result<(), MisagentError> {
        if self.is_usable() {
            Ok(())
        } else {
            Err(MisagentError::Protocol(
                "misagent connection is unusable; reconnect before another request".into(),
            ))
        }
    }

    fn deadline(&self) -> Result<Option<Instant>, MisagentError> {
        self.ensure_usable()?;
        match self.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "misagent operation deadline is too large",
                )
                .into()
            }),
            None => Ok(None),
        }
    }

    fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<MisagentRequest<'_, S>, MisagentError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(MisagentRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> MisagentClient<S> {
    /// Copy (retrieve) all installed provisioning profiles.
    pub async fn copy_all(&mut self) -> Result<Vec<Vec<u8>>, MisagentError> {
        let val = self
            .execute_request(plist::Value::Dictionary(plist::Dictionary::from_iter([
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
        self.execute_request(plist::Value::Dictionary(plist::Dictionary::from_iter([
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
        Ok(())
    }

    /// Remove a provisioning profile by UUID.
    pub async fn remove(&mut self, uuid: &str) -> Result<(), MisagentError> {
        self.execute_request(plist::Value::Dictionary(plist::Dictionary::from_iter([
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
        Ok(())
    }

    async fn execute_request(
        &mut self,
        plist_val: plist::Value,
    ) -> Result<plist::Value, MisagentError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let result = await_operation(deadline, async {
            let mut io = request.io();
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &plist_val)?;
            io.write_all(&(buf.len() as u32).to_be_bytes()).await?;
            io.write_all(&buf).await?;
            io.flush().await?;

            let mut len_buf = [0u8; 4];
            io.read_exact(&mut len_buf).await?;
            let len = u32::from_be_bytes(len_buf) as usize;
            const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
            if len > MAX_PLIST_SIZE {
                return Err(MisagentError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"),
                )));
            }
            let mut data = vec![0u8; len];
            io.read_exact(&mut data).await?;
            let val: plist::Value = plist::from_bytes(&data)?;
            let status = val
                .as_dictionary()
                .and_then(|d| d.get("Status"))
                .and_then(|v| v.as_unsigned_integer())
                .unwrap_or(0);
            if status != 0 {
                return Err(MisagentError::Status(status));
            }
            Ok(val)
        })
        .await;
        request.finish(check_deadline(deadline, result))
    }
}

struct MisagentRequest<'a, S> {
    client: &'a mut MisagentClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> MisagentRequest<'_, S> {
    fn io(&mut self) -> MisagentIo<'_, S> {
        MisagentIo {
            inner: self.stream.as_mut().expect("active misagent stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    fn finish<T>(mut self, result: Result<T, MisagentError>) -> Result<T, MisagentError> {
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.usable = false;
        }
        result
    }
}

impl<S> Drop for MisagentRequest<'_, S> {
    fn drop(&mut self) {
        if self.stream.is_none() {
            return;
        }
        if self.progressed || self.failed {
            self.client.usable = false;
        } else {
            self.client.stream = self.stream.take();
        }
    }
}

struct MisagentIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> MisagentIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for MisagentIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        *self.progressed |= buf.filled().len() > before;
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for MisagentIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        *self.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

fn elapsed_budget() -> MisagentError {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "misagent operation budget elapsed",
    )
    .into()
}

fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, MisagentError>,
) -> Result<T, MisagentError> {
    if result.is_ok() && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

async fn await_operation<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = Result<T, MisagentError>>,
) -> Result<T, MisagentError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| {
                MisagentError::Io(crate::error::context::retained_io(
                    std::io::ErrorKind::TimedOut,
                    "misagent operation deadline",
                    error,
                ))
            })?,
        None => operation.await,
    }
}

fn decode_profile(raw_data: &[u8]) -> Result<Profile, MisagentError> {
    let plist_bytes = embedded_plist_bytes(raw_data)?;
    let value: plist::Value = plist::from_bytes(plist_bytes)?;
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
    use crate::test_util::MockStream;

    use super::*;

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

    #[tokio::test]
    async fn misagent_client_zero_budget_fails_without_io_and_retains_stream() {
        let stream = MockStream::eof();
        let mut client = MisagentClient::new(stream);
        client.set_operation_timeout(Some(Duration::ZERO));
        assert!(client.is_usable());

        let err = client.copy_all().await.unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        assert!(client.is_usable());
        assert!(client.stream.is_some());
    }

    #[tokio::test]
    async fn misagent_client_progress_timeout_retires_stream_rejecting_subsequent_request() {
        let (client_side, mut server_side) = tokio::io::duplex(1024);
        let mut client = MisagentClient::new(client_side);
        client.set_operation_timeout(Some(Duration::from_millis(50)));

        tokio::spawn(async move {
            let mut buf = [0u8; 4];
            let _ = server_side.read_exact(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let err = client.copy_all().await.unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        assert!(!client.is_usable());
        assert!(client.stream.is_none());

        // Subsequent mutation is rejected immediately without reusing stale connection
        let err2 = client.remove("ANY-UUID").await.unwrap_err();
        assert!(matches!(err2, MisagentError::Protocol(msg) if msg.contains("unusable")));
    }

    #[tokio::test]
    async fn misagent_client_status_error_retires_stream() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::Integer(0xe8008015u64.into()),
        )]));
        let stream = MockStream::with_response(response);
        let mut client = MisagentClient::new(stream);
        client.set_operation_timeout(Some(Duration::from_secs(5)));

        let err = client.remove("TEST-UUID").await.unwrap_err();
        assert!(matches!(err, MisagentError::Status(0xe8008015)));
        // On failure, connection is invalidated
        assert!(!client.is_usable());
        assert!(client.stream.is_none());
    }

    #[tokio::test]
    async fn misagent_client_successful_copy_all_keeps_stream_usable() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Status".to_string(), plist::Value::Integer(0.into())),
            ("Payload".to_string(), plist::Value::Array(Vec::new())),
        ]));
        let stream = MockStream::with_response(response);
        let mut client = MisagentClient::new(stream);
        client.set_operation_timeout(Some(Duration::from_secs(5)));

        let profiles = client.copy_all().await.unwrap();
        assert!(profiles.is_empty());
        assert!(client.is_usable());
        assert!(client.stream.is_some());
    }
}

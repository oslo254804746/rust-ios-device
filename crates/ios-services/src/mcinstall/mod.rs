//! Minimal MCInstall client for read-only profile inspection.
//!
//! Service: `com.apple.mobile.MCInstall`

use openssl::pkcs12::Pkcs12;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::stack::Stack;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.MCInstall";

#[derive(Debug, thiserror::Error)]
pub enum McInstallError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("crypto error: {0}")]
    Crypto(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProfileInfo {
    pub identifier: String,
    pub display_name: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub removal_disallowed: Option<bool>,
    pub status: Option<String>,
    pub uuid: Option<String>,
    pub version: Option<u64>,
}

#[derive(Debug)]
pub struct McInstallClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> McInstallClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn list_profiles(&mut self) -> Result<Vec<ProfileInfo>, McInstallError> {
        let response = self.get_profile_list_raw().await?;
        parse_profile_list(response)
    }

    pub async fn get_profile_list_raw(&mut self) -> Result<plist::Value, McInstallError> {
        self.send_plist(&Request {
            request_type: "GetProfileList",
        })
        .await?;

        self.recv_plist().await
    }

    pub async fn get_cloud_configuration(&mut self) -> Result<plist::Dictionary, McInstallError> {
        self.send_plist(&Request {
            request_type: "GetCloudConfiguration",
        })
        .await?;

        let response: plist::Value = self.recv_plist().await?;
        parse_cloud_configuration(response)
    }

    pub async fn get_stored_profile_raw(
        &mut self,
        purpose: &str,
    ) -> Result<plist::Value, McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("GetStoredProfile".into()),
            ),
            (
                "Purpose".to_string(),
                plist::Value::String(purpose.to_string()),
            ),
        ]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        self.recv_plist().await
    }

    pub async fn flush(&mut self) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([(
            "RequestType".to_string(),
            plist::Value::String("Flush".into()),
        )]);
        send_request(&mut self.stream, request).await
    }

    pub async fn hello_host_identifier(&mut self) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([(
            "RequestType".to_string(),
            plist::Value::String("HelloHostIdentifier".into()),
        )]);
        send_request(&mut self.stream, request).await
    }

    pub async fn set_cloud_configuration(
        &mut self,
        cloud_configuration: plist::Dictionary,
    ) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("SetCloudConfiguration".into()),
            ),
            (
                "CloudConfiguration".to_string(),
                plist::Value::Dictionary(cloud_configuration),
            ),
        ]);
        send_request(&mut self.stream, request).await
    }

    pub async fn install_profile(&mut self, payload: &[u8]) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("InstallProfile".into()),
            ),
            ("Payload".to_string(), plist::Value::Data(payload.to_vec())),
        ]);
        send_request(&mut self.stream, request).await
    }

    pub async fn install_profile_silent(
        &mut self,
        payload: &[u8],
        p12_bytes: &[u8],
        password: &str,
    ) -> Result<(), McInstallError> {
        self.escalate(p12_bytes, password).await?;
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("InstallProfileSilent".into()),
            ),
            ("Payload".to_string(), plist::Value::Data(payload.to_vec())),
        ]);
        send_request(&mut self.stream, request).await
    }

    pub async fn remove_profile(&mut self, identifier: &str) -> Result<(), McInstallError> {
        let profile_identifier = match self.get_profile_list_raw().await {
            Ok(value) => build_remove_profile_identifier(&value, identifier)
                .map_err(|err| McInstallError::Protocol(err.to_string()))?
                .unwrap_or_else(|| plist::Value::String(identifier.to_string())),
            Err(_) => plist::Value::String(identifier.to_string()),
        };
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("RemoveProfile".into()),
            ),
            ("ProfileIdentifier".to_string(), profile_identifier),
        ]);
        send_request(&mut self.stream, request).await
    }

    pub async fn erase_device(
        &mut self,
        preserve_data_plan: bool,
        disallow_proximity_setup: bool,
    ) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("EraseDevice".into()),
            ),
            (
                "PreserveDataPlan".to_string(),
                plist::Value::Boolean(preserve_data_plan),
            ),
            (
                "DisallowProximitySetup".to_string(),
                plist::Value::Boolean(disallow_proximity_setup),
            ),
        ]);
        send_request_allow_eof(&mut self.stream, request).await
    }

    pub async fn escalate_unsupervised(&mut self) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("Escalate".into()),
            ),
            (
                "SupervisorCertificate".to_string(),
                plist::Value::Data(vec![0]),
            ),
        ]);
        send_request(&mut self.stream, request).await
    }

    async fn escalate(&mut self, p12_bytes: &[u8], password: &str) -> Result<(), McInstallError> {
        let pkcs12 =
            Pkcs12::from_der(p12_bytes).map_err(|err| McInstallError::Crypto(err.to_string()))?;
        let parsed = pkcs12
            .parse2(password)
            .map_err(|err| McInstallError::Crypto(err.to_string()))?;
        let cert = parsed
            .cert
            .ok_or_else(|| McInstallError::Crypto("P12 missing certificate".into()))?;
        let pkey = parsed
            .pkey
            .ok_or_else(|| McInstallError::Crypto("P12 missing private key".into()))?;

        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("Escalate".into()),
            ),
            (
                "SupervisorCertificate".to_string(),
                plist::Value::Data(
                    cert.to_der()
                        .map_err(|err| McInstallError::Crypto(err.to_string()))?,
                ),
            ),
        ]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        let response = recv_plist(&mut self.stream).await?;
        ensure_acknowledged(&response)?;
        let challenge = response
            .get("Challenge")
            .and_then(plist::Value::as_data)
            .ok_or_else(|| {
                McInstallError::Protocol("MCInstall escalate response missing Challenge".into())
            })?;
        let certs = Stack::new().map_err(|err| McInstallError::Crypto(err.to_string()))?;
        let signed_request = Pkcs7::sign(&cert, &pkey, &certs, challenge, Pkcs7Flags::BINARY)
            .and_then(|pkcs7| pkcs7.to_der())
            .map_err(|err| McInstallError::Crypto(err.to_string()))?;

        let response_request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("EscalateResponse".into()),
            ),
            (
                "SignedRequest".to_string(),
                plist::Value::Data(signed_request),
            ),
        ]);
        send_request(&mut self.stream, response_request).await?;

        let proceed_request = plist::Dictionary::from_iter([(
            "RequestType".to_string(),
            plist::Value::String("ProceedWithKeybagMigration".into()),
        )]);
        send_request(&mut self.stream, proceed_request).await
    }

    async fn send_plist<T: Serialize>(&mut self, value: &T) -> Result<(), McInstallError> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, value).map_err(|e| McInstallError::Plist(e.to_string()))?;
        self.stream
            .write_all(&(buf.len() as u32).to_be_bytes())
            .await?;
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv_plist<T>(&mut self) -> Result<T, McInstallError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
        if len > MAX_PLIST_SIZE {
            return Err(McInstallError::Protocol(format!(
                "plist length {len} exceeds max {MAX_PLIST_SIZE}"
            )));
        }
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        plist::from_bytes(&buf).map_err(|e| McInstallError::Plist(e.to_string()))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Request {
    request_type: &'static str,
}

fn parse_profile_list(value: plist::Value) -> Result<Vec<ProfileInfo>, McInstallError> {
    let dict = value.into_dictionary().ok_or_else(|| {
        McInstallError::Protocol("MCInstall response was not a dictionary".into())
    })?;

    let ordered = dict
        .get("OrderedIdentifiers")
        .and_then(plist::Value::as_array)
        .ok_or_else(|| {
            McInstallError::Protocol("MCInstall response missing OrderedIdentifiers".into())
        })?;
    let manifest_root = dict
        .get("ProfileManifest")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| {
            McInstallError::Protocol("MCInstall response missing ProfileManifest".into())
        })?;
    let metadata_root = dict
        .get("ProfileMetadata")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| {
            McInstallError::Protocol("MCInstall response missing ProfileMetadata".into())
        })?;
    let status = dict
        .get("Status")
        .and_then(plist::Value::as_string)
        .map(ToOwned::to_owned);

    let mut profiles = Vec::with_capacity(ordered.len());
    for identifier in ordered {
        let identifier = identifier.as_string().ok_or_else(|| {
            McInstallError::Protocol("OrderedIdentifiers entry was not a string".into())
        })?;
        let manifest = manifest_root
            .get(identifier)
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| {
                McInstallError::Protocol(format!("ProfileManifest missing entry for {identifier}"))
            })?;
        let metadata = metadata_root
            .get(identifier)
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| {
                McInstallError::Protocol(format!("ProfileMetadata missing entry for {identifier}"))
            })?;

        profiles.push(ProfileInfo {
            identifier: identifier.to_string(),
            display_name: metadata
                .get("PayloadDisplayName")
                .and_then(plist::Value::as_string)
                .unwrap_or(identifier)
                .to_string(),
            description: metadata
                .get("PayloadDescription")
                .and_then(plist::Value::as_string)
                .map(ToOwned::to_owned),
            is_active: manifest
                .get("IsActive")
                .and_then(plist::Value::as_boolean)
                .unwrap_or(false),
            removal_disallowed: metadata
                .get("PayloadRemovalDisallowed")
                .and_then(plist::Value::as_boolean),
            status: status.clone(),
            uuid: metadata
                .get("PayloadUUID")
                .and_then(plist::Value::as_string)
                .map(ToOwned::to_owned),
            version: metadata
                .get("PayloadVersion")
                .and_then(plist::Value::as_unsigned_integer),
        });
    }
    Ok(profiles)
}

fn parse_cloud_configuration(value: plist::Value) -> Result<plist::Dictionary, McInstallError> {
    value.into_dictionary().ok_or_else(|| {
        McInstallError::Protocol("MCInstall cloud configuration was not a dictionary".into())
    })
}

fn build_remove_profile_identifier(
    value: &plist::Value,
    identifier: &str,
) -> Result<Option<plist::Value>, plist::Error> {
    let metadata = match value
        .as_dictionary()
        .and_then(|dict| dict.get("ProfileMetadata"))
        .and_then(plist::Value::as_dictionary)
        .and_then(|metadata| metadata.get(identifier))
        .and_then(plist::Value::as_dictionary)
    {
        Some(metadata) => metadata,
        None => return Ok(None),
    };
    let payload_uuid = match metadata
        .get("PayloadUUID")
        .and_then(plist::Value::as_string)
    {
        Some(uuid) => uuid,
        None => return Ok(None),
    };
    let payload_version = match metadata
        .get("PayloadVersion")
        .and_then(plist::Value::as_unsigned_integer)
    {
        Some(version) => version,
        None => return Ok(None),
    };

    let profile_identifier = plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "PayloadType".to_string(),
            plist::Value::String("Configuration".into()),
        ),
        (
            "PayloadIdentifier".to_string(),
            plist::Value::String(identifier.to_string()),
        ),
        (
            "PayloadUUID".to_string(),
            plist::Value::String(payload_uuid.to_string()),
        ),
        (
            "PayloadVersion".to_string(),
            plist::Value::Integer((payload_version as i64).into()),
        ),
    ]));
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &profile_identifier)?;
    Ok(Some(plist::Value::Data(buf)))
}

async fn send_request<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    request: plist::Dictionary,
) -> Result<(), McInstallError> {
    send_plist(stream, &plist::Value::Dictionary(request)).await?;
    let response = recv_plist(stream).await?;
    ensure_acknowledged(&response)
}

async fn send_request_allow_eof<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    request: plist::Dictionary,
) -> Result<(), McInstallError> {
    send_plist(stream, &plist::Value::Dictionary(request)).await?;
    match recv_plist(stream).await {
        Ok(response) => ensure_acknowledged(&response),
        Err(McInstallError::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()),
        Err(err) => Err(err),
    }
}

fn ensure_acknowledged(response: &plist::Dictionary) -> Result<(), McInstallError> {
    let status = response
        .get("Status")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| McInstallError::Protocol("MCInstall response missing Status".into()))?;
    if status != "Acknowledged" {
        let detail = response
            .get("Error")
            .and_then(plist::Value::as_string)
            .map(ToOwned::to_owned)
            .or_else(|| response.get("ErrorChain").map(|value| format!("{value:?}")))
            .unwrap_or_else(|| status.to_string());
        return Err(McInstallError::Protocol(format!(
            "MCInstall request not acknowledged: {detail}"
        )));
    }
    Ok(())
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), McInstallError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| McInstallError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, McInstallError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(McInstallError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|e| McInstallError::Plist(e.to_string()))
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
            Self::with_responses(vec![value])
        }

        fn with_responses(values: Vec<plist::Value>) -> Self {
            let mut payload = Vec::new();
            let mut read_buf = Vec::new();
            for value in values {
                payload.clear();
                plist::to_writer_xml(&mut payload, &value).unwrap();
                read_buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                read_buf.extend_from_slice(&payload);
            }
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
    fn parses_ordered_profile_list() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(vec![plist::Value::String("com.example.profile".into())]),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "Description".to_string(),
                            plist::Value::String("Example".into()),
                        ),
                        ("IsActive".to_string(), plist::Value::Boolean(true)),
                    ])),
                )])),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "PayloadDisplayName".to_string(),
                            plist::Value::String("Example Profile".into()),
                        ),
                        (
                            "PayloadDescription".to_string(),
                            plist::Value::String("Example description".into()),
                        ),
                        (
                            "PayloadRemovalDisallowed".to_string(),
                            plist::Value::Boolean(false),
                        ),
                        (
                            "PayloadUUID".to_string(),
                            plist::Value::String("1234".into()),
                        ),
                        (
                            "PayloadVersion".to_string(),
                            plist::Value::Integer(1i64.into()),
                        ),
                    ])),
                )])),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));

        let profiles = parse_profile_list(response).unwrap();
        assert_eq!(profiles.len(), 1);
        let profile = &profiles[0];
        assert_eq!(profile.identifier, "com.example.profile");
        assert_eq!(profile.display_name, "Example Profile");
        assert_eq!(profile.description.as_deref(), Some("Example description"));
        assert!(profile.is_active);
        assert_eq!(profile.removal_disallowed, Some(false));
        assert_eq!(profile.status.as_deref(), Some("Acknowledged"));
        assert_eq!(profile.uuid.as_deref(), Some("1234"));
        assert_eq!(profile.version, Some(1));
    }

    #[test]
    fn cloud_configuration_requires_dictionary_response() {
        let err = parse_cloud_configuration(plist::Value::Array(Vec::new()));
        assert!(matches!(
            err,
            Err(McInstallError::Protocol(message)) if message.contains("cloud configuration")
        ));
    }

    #[test]
    fn parses_cloud_configuration_dictionary() {
        let dict = plist::Dictionary::from_iter([(
            "IsSupervised".to_string(),
            plist::Value::Boolean(true),
        )]);
        let parsed = parse_cloud_configuration(plist::Value::Dictionary(dict.clone())).unwrap();
        assert_eq!(
            parsed
                .get("IsSupervised")
                .and_then(plist::Value::as_boolean),
            Some(true)
        );
    }

    #[tokio::test]
    async fn install_profile_sends_payload_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.install_profile(b"<plist/>").await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("InstallProfile")
        );
        assert_eq!(
            dict.get("Payload").and_then(plist::Value::as_data),
            Some(&b"<plist/>"[..])
        );
    }

    #[tokio::test]
    async fn remove_profile_sends_identifier_request() {
        let profile_list = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(Vec::new()),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));
        let remove_response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_responses(vec![profile_list, remove_response]);
        let mut client = McInstallClient::new(&mut stream);

        client.remove_profile("com.example.profile").await.unwrap();

        let first_len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let offset = 4 + first_len;
        let len =
            u32::from_be_bytes(stream.written[offset..offset + 4].try_into().unwrap()) as usize;
        let payload = &stream.written[offset + 4..offset + 4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("RemoveProfile")
        );
        assert_eq!(
            dict.get("ProfileIdentifier")
                .and_then(plist::Value::as_string),
            Some("com.example.profile")
        );
    }

    #[tokio::test]
    async fn remove_profile_uses_metadata_backed_identifier_when_available() {
        let profile_list = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(vec![plist::Value::String("com.example.profile".into())]),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "IsActive".to_string(),
                        plist::Value::Boolean(true),
                    )])),
                )])),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "PayloadUUID".to_string(),
                            plist::Value::String("1234-5678".into()),
                        ),
                        (
                            "PayloadVersion".to_string(),
                            plist::Value::Integer(7.into()),
                        ),
                    ])),
                )])),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));
        let remove_response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_responses(vec![profile_list, remove_response]);
        let mut client = McInstallClient::new(&mut stream);

        client.remove_profile("com.example.profile").await.unwrap();

        let first_len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let second_offset = 4 + first_len;
        let second_len = u32::from_be_bytes(
            stream.written[second_offset..second_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let second_payload = &stream.written[second_offset + 4..second_offset + 4 + second_len];
        let second_request: plist::Dictionary = plist::from_bytes(second_payload).unwrap();
        let profile_identifier = second_request
            .get("ProfileIdentifier")
            .and_then(plist::Value::as_data)
            .expect("metadata-backed profile identifier should be plist data");
        let identifier_plist = plist::Value::from_reader(std::io::Cursor::new(profile_identifier))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(
            identifier_plist
                .get("PayloadIdentifier")
                .and_then(plist::Value::as_string),
            Some("com.example.profile")
        );
        assert_eq!(
            identifier_plist
                .get("PayloadUUID")
                .and_then(plist::Value::as_string),
            Some("1234-5678")
        );
        assert_eq!(
            identifier_plist
                .get("PayloadVersion")
                .and_then(plist::Value::as_unsigned_integer),
            Some(7)
        );
        assert_eq!(
            identifier_plist
                .get("PayloadType")
                .and_then(plist::Value::as_string),
            Some("Configuration")
        );
    }

    #[tokio::test]
    async fn get_profile_list_raw_preserves_unparsed_fields() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(vec![plist::Value::String("com.example.profile".into())]),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "IsActive".to_string(),
                        plist::Value::Boolean(true),
                    )])),
                )])),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "PayloadDisplayName".to_string(),
                        plist::Value::String("Example".into()),
                    )])),
                )])),
            ),
            (
                "Unhandled".to_string(),
                plist::Value::String("preserved".into()),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        let raw = client.get_profile_list_raw().await.unwrap();
        let dict = raw.as_dictionary().unwrap();
        assert_eq!(dict["Unhandled"].as_string(), Some("preserved"));
    }

    #[tokio::test]
    async fn get_stored_profile_raw_includes_requested_purpose() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
            (
                "ProfileData".to_string(),
                plist::Value::Data(b"<plist/>".to_vec()),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        let raw = client
            .get_stored_profile_raw("PostSetupInstallation")
            .await
            .unwrap();
        let dict = raw.as_dictionary().unwrap();
        assert_eq!(dict["Status"].as_string(), Some("Acknowledged"));
        assert_eq!(dict["ProfileData"].as_data(), Some(&b"<plist/>"[..]));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let sent: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            sent.get("RequestType").and_then(plist::Value::as_string),
            Some("GetStoredProfile")
        );
        assert_eq!(
            sent.get("Purpose").and_then(plist::Value::as_string),
            Some("PostSetupInstallation")
        );
    }

    #[tokio::test]
    async fn flush_sends_flush_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.flush().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("Flush")
        );
    }

    #[tokio::test]
    async fn hello_host_identifier_sends_request_type() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.hello_host_identifier().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("HelloHostIdentifier")
        );
    }

    #[tokio::test]
    async fn set_cloud_configuration_sends_payload() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);
        let cloud_configuration = plist::Dictionary::from_iter([
            ("AllowPairing".to_string(), plist::Value::Boolean(true)),
            (
                "SkipSetup".to_string(),
                plist::Value::Array(vec![plist::Value::String("WiFi".into())]),
            ),
        ]);

        client
            .set_cloud_configuration(cloud_configuration.clone())
            .await
            .unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("SetCloudConfiguration")
        );
        assert_eq!(
            dict.get("CloudConfiguration")
                .and_then(plist::Value::as_dictionary),
            Some(&cloud_configuration)
        );
    }

    #[tokio::test]
    async fn escalate_unsupervised_uses_zero_byte_certificate() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.escalate_unsupervised().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("Escalate")
        );
        assert_eq!(
            dict.get("SupervisorCertificate")
                .and_then(plist::Value::as_data),
            Some(&b"\x00"[..])
        );
    }

    #[tokio::test]
    async fn erase_device_sends_expected_flags() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.erase_device(true, false).await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("EraseDevice")
        );
        assert_eq!(
            dict.get("PreserveDataPlan")
                .and_then(plist::Value::as_boolean),
            Some(true)
        );
        assert_eq!(
            dict.get("DisallowProximitySetup")
                .and_then(plist::Value::as_boolean),
            Some(false)
        );
    }
}

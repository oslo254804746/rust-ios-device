//! Diagnostics relay service.
//!
//! Provides access to device diagnostic info and MobileGestalt queries.
//! Service: `com.apple.mobile.diagnostics_relay`
//!
//! Protocol: lockdown plist framing (4-byte BE length prefix).

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.diagnostics_relay";

#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("mobilegestalt deprecated: {0}")]
    Deprecated(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BatteryDiagnostics {
    #[serde(default)]
    pub instant_amperage: Option<i64>,
    #[serde(default)]
    pub temperature: Option<i64>,
    #[serde(default)]
    pub voltage: Option<i64>,
    #[serde(default)]
    pub is_charging: Option<bool>,
    #[serde(default)]
    pub current_capacity: Option<i64>,
    #[serde(default)]
    pub design_capacity: Option<u64>,
    #[serde(default)]
    pub nominal_charge_capacity: Option<u64>,
    #[serde(default)]
    pub absolute_capacity: Option<u64>,
    #[serde(default)]
    pub apple_raw_current_capacity: Option<u64>,
    #[serde(default)]
    pub apple_raw_max_capacity: Option<u64>,
    #[serde(default)]
    pub cycle_count: Option<u64>,
    #[serde(default)]
    pub at_critical_level: Option<bool>,
    #[serde(default)]
    pub at_warn_level: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct MobileGestaltRequest<'a> {
    request: &'static str,
    #[serde(rename = "MobileGestaltKeys")]
    keys: Vec<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct IoRegistryRequest<'a> {
    request: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_class: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entry_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_plane: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoRegistryQuery<'a> {
    pub entry_class: Option<&'a str>,
    pub entry_name: Option<&'a str>,
    pub current_plane: Option<&'a str>,
}

impl<'a> IoRegistryQuery<'a> {
    pub fn by_class(entry_class: &'a str) -> Self {
        Self {
            entry_class: Some(entry_class),
            entry_name: None,
            current_plane: None,
        }
    }
}

/// Query MobileGestalt keys from the device.
pub async fn query_mobile_gestalt<S>(
    stream: &mut S,
    keys: &[&str],
) -> Result<plist::Value, DiagnosticsError>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    send_plist(
        stream,
        &MobileGestaltRequest {
            request: "MobileGestalt",
            keys: keys.to_vec(),
        },
    )
    .await?;

    let response = recv_response_dict(stream).await?;
    let diagnostics = extract_diagnostics_payload(&response)?;

    extract_mobile_gestalt_payload(diagnostics)
}

/// Query the complete diagnostics payload exposed by diagnostics_relay.
pub async fn query_all_values<S>(stream: &mut S) -> Result<plist::Value, DiagnosticsError>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct AllRequest {
        request: &'static str,
    }

    send_plist(stream, &AllRequest { request: "All" }).await?;
    let response = recv_response_dict(stream).await?;
    extract_diagnostics_payload(&response)
}

/// Query the battery IORegistry block exposed by diagnostics_relay.
pub async fn query_battery<S>(stream: &mut S) -> Result<BatteryDiagnostics, DiagnosticsError>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    let io_registry = query_ioregistry(stream, "IOPMPowerSource").await?;
    plist::from_value(&io_registry).map_err(|e| DiagnosticsError::Plist(e.to_string()))
}

/// Query an arbitrary IORegistry entry class exposed by diagnostics_relay.
pub async fn query_ioregistry<S>(
    stream: &mut S,
    entry_class: &str,
) -> Result<plist::Value, DiagnosticsError>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    query_ioregistry_with(stream, IoRegistryQuery::by_class(entry_class)).await
}

pub async fn query_ioregistry_with<S>(
    stream: &mut S,
    query: IoRegistryQuery<'_>,
) -> Result<plist::Value, DiagnosticsError>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    if query.entry_class.is_none() && query.entry_name.is_none() {
        return Err(DiagnosticsError::Protocol(
            "IORegistry query requires EntryClass or EntryName".into(),
        ));
    }

    send_plist(
        stream,
        &IoRegistryRequest {
            request: "IORegistry",
            entry_class: query.entry_class,
            entry_name: query.entry_name,
            current_plane: query.current_plane,
        },
    )
    .await?;

    let response = recv_response_dict(stream).await?;
    let diagnostics = extract_diagnostics_payload(&response)?;
    let dict = diagnostics.into_dictionary().ok_or_else(|| {
        DiagnosticsError::Protocol("diagnostics payload was not a dictionary".into())
    })?;
    let io_registry = dict
        .get("IORegistry")
        .cloned()
        .ok_or_else(|| DiagnosticsError::Protocol("diagnostics missing IORegistry".into()))?;
    Ok(io_registry)
}

/// Reboot the device.
pub async fn reboot<S>(stream: &mut S) -> Result<(), DiagnosticsError>
where
    S: AsyncRead + AsyncWrite + Unpin + ?Sized,
{
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Request {
        request: &'static str,
    }
    send_plist(stream, &Request { request: "Restart" }).await?;
    recv_plist_raw(stream).await?;
    Ok(())
}

// ── plist framing ──────────────────────────────────────────────────────────────

async fn send_plist<S, T>(stream: &mut S, value: &T) -> Result<(), DiagnosticsError>
where
    S: AsyncWrite + Unpin + ?Sized,
    T: Serialize,
{
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| DiagnosticsError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist_raw<S>(stream: &mut S) -> Result<Vec<u8>, DiagnosticsError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    // Guard against DoS via enormous length field (max 4 MiB)
    const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(DiagnosticsError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn recv_response_dict<S>(stream: &mut S) -> Result<plist::Dictionary, DiagnosticsError>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let data = recv_plist_raw(stream).await?;
    let value: plist::Value =
        plist::from_bytes(&data).map_err(|e| DiagnosticsError::Plist(e.to_string()))?;
    value.into_dictionary().ok_or_else(|| {
        DiagnosticsError::Protocol("diagnostics response payload was not a dictionary".into())
    })
}

fn extract_diagnostics_payload(
    response: &plist::Dictionary,
) -> Result<plist::Value, DiagnosticsError> {
    response
        .get("Diagnostics")
        .cloned()
        .ok_or_else(|| missing_diagnostics_error(response))
}

fn missing_diagnostics_error(response: &plist::Dictionary) -> DiagnosticsError {
    let status = response
        .get("Status")
        .and_then(plist::Value::as_string)
        .map(|value| format!(" (Status={value})"))
        .unwrap_or_default();
    let rendered = render_plist_value(&plist::Value::Dictionary(response.clone()));
    DiagnosticsError::Protocol(format!(
        "diagnostics response missing Diagnostics{status}: {rendered}"
    ))
}

fn render_plist_value(value: &plist::Value) -> String {
    serde_json::to_string(&plist_to_json(value))
        .unwrap_or_else(|_| "<failed to render plist value>".to_string())
}

fn plist_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(plist_to_json).collect())
        }
        plist::Value::Boolean(value) => serde_json::Value::Bool(*value),
        plist::Value::Data(bytes) => {
            serde_json::Value::Array(bytes.iter().copied().map(serde_json::Value::from).collect())
        }
        plist::Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        plist::Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        plist::Value::Real(value) => serde_json::Value::from(*value),
        plist::Value::String(value) => serde_json::Value::String(value.clone()),
        plist::Value::Uid(value) => serde_json::Value::from(value.get()),
        _ => serde_json::Value::Null,
    }
}

fn extract_mobile_gestalt_payload(
    diagnostics: plist::Value,
) -> Result<plist::Value, DiagnosticsError> {
    let Some(dict) = diagnostics.as_dictionary() else {
        return Ok(diagnostics);
    };

    let Some(mobile_gestalt) = dict.get("MobileGestalt") else {
        return Ok(diagnostics);
    };

    let mut inner = mobile_gestalt.as_dictionary().cloned().ok_or_else(|| {
        DiagnosticsError::Protocol("MobileGestalt payload was not a dictionary".into())
    })?;

    if let Some(status) = inner.get("Status").and_then(|value| value.as_string()) {
        match status {
            "Success" => {
                inner.remove("Status");
            }
            "MobileGestaltDeprecated" => {
                return Err(DiagnosticsError::Deprecated(
                    "diagnostics relay reports MobileGestaltDeprecated on this OS".into(),
                ));
            }
            other => {
                return Err(DiagnosticsError::Protocol(format!(
                    "unexpected MobileGestalt status: {other}"
                )));
            }
        }
    }

    Ok(plist::Value::Dictionary(inner))
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

    #[tokio::test]
    async fn query_mobile_gestalt_uses_mobile_gestalt_keys_field() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            )])));

        let _ = query_mobile_gestalt(&mut stream, &["ProductVersion"])
            .await
            .unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Request"].as_string(), Some("MobileGestalt"));
        let keys = dict["MobileGestaltKeys"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].as_string(), Some("ProductVersion"));
        assert!(!dict.contains_key("Keys"));
    }

    #[tokio::test]
    async fn query_mobile_gestalt_returns_diagnostics_payload() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ProductVersion".to_string(),
                    plist::Value::String("26.0".into()),
                )])),
            )])));

        let value = query_mobile_gestalt(&mut stream, &["ProductVersion"])
            .await
            .unwrap();
        let dict = value.into_dictionary().unwrap();
        assert_eq!(dict["ProductVersion"].as_string(), Some("26.0"));
    }

    #[tokio::test]
    async fn query_mobile_gestalt_strips_nested_success_status() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "MobileGestalt".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        ("Status".to_string(), plist::Value::String("Success".into())),
                        (
                            "ProductVersion".to_string(),
                            plist::Value::String("26.0".into()),
                        ),
                    ])),
                )])),
            )])));

        let value = query_mobile_gestalt(&mut stream, &["ProductVersion"])
            .await
            .unwrap();
        let dict = value.into_dictionary().unwrap();
        assert_eq!(dict["ProductVersion"].as_string(), Some("26.0"));
        assert!(!dict.contains_key("Status"));
    }

    #[tokio::test]
    async fn query_mobile_gestalt_returns_deprecated_error() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "MobileGestalt".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "Status".to_string(),
                        plist::Value::String("MobileGestaltDeprecated".into()),
                    )])),
                )])),
            )])));

        let err = query_mobile_gestalt(&mut stream, &["ProductVersion"])
            .await
            .unwrap_err();
        assert!(matches!(err, DiagnosticsError::Deprecated(_)));
        assert!(err.to_string().contains("deprecated"));
    }

    #[tokio::test]
    async fn query_all_values_sends_all_request_and_returns_diagnostics_payload() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "GasGauge".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "CycleCount".to_string(),
                        plist::Value::Integer(315.into()),
                    )])),
                )])),
            )])));

        let value = query_all_values(&mut stream).await.unwrap();
        let dict = value.into_dictionary().unwrap();
        assert!(dict.contains_key("GasGauge"));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Request"].as_string(), Some("All"));
    }

    #[tokio::test]
    async fn query_battery_sends_ioregistry_request_for_power_source() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "IORegistry".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::new()),
                )])),
            )])));

        let _ = query_battery(&mut stream).await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Request"].as_string(), Some("IORegistry"));
        assert_eq!(dict["EntryClass"].as_string(), Some("IOPMPowerSource"));
    }

    #[tokio::test]
    async fn query_battery_extracts_ioregistry_payload() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "IORegistry".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "CurrentCapacity".to_string(),
                            plist::Value::Integer(82.into()),
                        ),
                        ("IsCharging".to_string(), plist::Value::Boolean(true)),
                        ("CycleCount".to_string(), plist::Value::Integer(315.into())),
                    ])),
                )])),
            )])));

        let battery = query_battery(&mut stream).await.unwrap();
        assert_eq!(battery.current_capacity, Some(82));
        assert_eq!(battery.is_charging, Some(true));
        assert_eq!(battery.cycle_count, Some(315));
    }

    #[tokio::test]
    async fn query_ioregistry_returns_raw_ioregistry_payload() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "IORegistry".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "ProductName".to_string(),
                        plist::Value::String("iPhone".into()),
                    )])),
                )])),
            )])));

        let value = query_ioregistry(&mut stream, "IOPlatformExpertDevice")
            .await
            .unwrap();
        let dict = value.into_dictionary().unwrap();
        assert_eq!(dict["ProductName"].as_string(), Some("iPhone"));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Request"].as_string(), Some("IORegistry"));
        assert_eq!(
            dict["EntryClass"].as_string(),
            Some("IOPlatformExpertDevice")
        );
        assert!(!dict.contains_key("EntryName"));
        assert!(!dict.contains_key("CurrentPlane"));
    }

    #[tokio::test]
    async fn query_ioregistry_with_name_and_plane_encodes_optional_fields() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Diagnostics".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "IORegistry".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::new()),
                )])),
            )])));

        let _ = query_ioregistry_with(
            &mut stream,
            IoRegistryQuery {
                entry_class: None,
                entry_name: Some("device-tree"),
                current_plane: Some("IODeviceTree"),
            },
        )
        .await
        .unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Request"].as_string(), Some("IORegistry"));
        assert_eq!(dict["EntryName"].as_string(), Some("device-tree"));
        assert_eq!(dict["CurrentPlane"].as_string(), Some("IODeviceTree"));
        assert!(!dict.contains_key("EntryClass"));
    }

    #[tokio::test]
    async fn query_ioregistry_reports_status_when_diagnostics_are_missing() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Status".to_string(),
                    plist::Value::String("LookupFailed".into()),
                ),
                (
                    "Error".to_string(),
                    plist::Value::String("Entry not found".into()),
                ),
            ])));

        let err = query_ioregistry_with(
            &mut stream,
            IoRegistryQuery {
                entry_class: Some("IO80211Interface"),
                entry_name: None,
                current_plane: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, DiagnosticsError::Protocol(_)));
        let message = err.to_string();
        assert!(message.contains("LookupFailed"));
        assert!(message.contains("Entry not found"));
        assert!(message.contains("Diagnostics"));
    }
}

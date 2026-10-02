use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::mux::MuxError;

const MAX_MUX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

pub fn encode_message(payload: &[u8], tag: u32) -> Result<Vec<u8>, MuxError> {
    let total = checked_mux_message_len(payload.len())?;
    let mut buf = Vec::with_capacity(total);
    buf.extend_from_slice(&(total as u32).to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes()); // version
    buf.extend_from_slice(&8u32.to_le_bytes()); // type = plist
    buf.extend_from_slice(&tag.to_le_bytes());
    buf.extend_from_slice(payload);
    Ok(buf)
}

fn checked_mux_message_len(payload_len: usize) -> Result<usize, MuxError> {
    let total = payload_len
        .checked_add(16)
        .ok_or_else(|| MuxError::Protocol("mux message length overflow".to_string()))?;
    if total > u32::MAX as usize {
        return Err(MuxError::Protocol(format!(
            "mux message too large: {total} bytes exceeds u32::MAX"
        )));
    }
    Ok(total)
}

pub async fn send_plist<W, T>(writer: &mut W, value: &T, tag: u32) -> Result<(), MuxError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut plist_bytes = Vec::new();
    plist::to_writer_xml(&mut plist_bytes, value).map_err(|error| {
        MuxError::Io(crate::error::context::retained_io(
            std::io::ErrorKind::InvalidInput,
            "encode usbmuxd request plist",
            error,
        ))
    })?;
    let msg = encode_message(&plist_bytes, tag)?;
    writer.write_all(&msg).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn recv_plist<R, T>(reader: &mut R) -> Result<T, MuxError>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut header = [0u8; 16];
    reader.read_exact(&mut header).await?;
    let length = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    if length < 16 {
        return Err(MuxError::Protocol(format!(
            "invalid message length: {length}"
        )));
    }
    if length > MAX_MUX_MESSAGE_SIZE {
        return Err(MuxError::Protocol(format!(
            "message too large: {length} bytes exceeds {MAX_MUX_MESSAGE_SIZE}"
        )));
    }
    let mut payload = vec![0u8; length - 16];
    reader.read_exact(&mut payload).await?;
    let value = plist::from_bytes(&payload).map_err(|error| {
        MuxError::Io(crate::error::context::retained_io(
            std::io::ErrorKind::InvalidData,
            "decode usbmuxd response plist",
            error,
        ))
    })?;
    Ok(value)
}

// ── Device discovery messages ────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ListDevicesRequest {
    pub message_type: &'static str,
    pub prog_name: &'static str,
    pub client_version_string: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct DeviceList {
    pub device_list: Vec<DeviceEntryRaw>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct DeviceEntryRaw {
    #[serde(rename = "DeviceID")]
    pub device_id: u32,
    pub properties: DevicePropertiesRaw,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct DevicePropertiesRaw {
    pub serial_number: String,
    pub connection_type: String,
    #[serde(rename = "ProductID", alias = "ProductId")]
    pub product_id: Option<u16>,
}

// ── ReadPairRecord ───────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ReadPairRecordRequest {
    pub message_type: &'static str,
    pub prog_name: &'static str,
    pub client_version_string: &'static str,
    pub bundle_id: &'static str,
    #[serde(rename = "kLibUSBMuxVersion")]
    pub lib_usbmux_version: u32,
    #[serde(rename = "PairRecordID")]
    pub pair_record_id: String,
}

// ── ReadBUID ─────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ReadBuidRequest {
    pub message_type: &'static str,
    pub prog_name: &'static str,
    pub client_version_string: &'static str,
    pub bundle_id: &'static str,
    #[serde(rename = "kLibUSBMuxVersion")]
    pub lib_usbmux_version: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ReadBuidResponse {
    #[serde(rename = "BUID")]
    pub buid: String,
}

// ── Connect messages ─────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ConnectRequest {
    pub message_type: &'static str,
    pub prog_name: &'static str,
    pub client_version_string: &'static str,
    pub bundle_id: &'static str,
    #[serde(rename = "kLibUSBMuxVersion")]
    pub lib_usbmux_version: u32,
    #[serde(rename = "DeviceID")]
    pub device_id: u32,
    pub port_number: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ConnectResponse {
    #[allow(dead_code)]
    pub message_type: String,
    pub number: u32,
}

// ── Listen message ───────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct ListenRequest {
    pub message_type: &'static str,
    pub prog_name: &'static str,
    pub client_version_string: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub(crate) struct DeviceEvent {
    pub message_type: String,
    #[serde(rename = "DeviceID")]
    pub device_id: u32,
    pub properties: Option<DevicePropertiesRaw>,
}

#[cfg(test)]
mod source_contract_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_listing_reads_canonical_product_id_and_retains_legacy_alias() {
        for key in ["ProductID", "ProductId"] {
            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "DeviceList".to_string(),
                plist::Value::Array(vec![plist::Value::Dictionary(
                    plist::Dictionary::from_iter([
                        ("DeviceID".to_string(), plist::Value::Integer(42.into())),
                        (
                            "Properties".to_string(),
                            plist::Value::Dictionary(plist::Dictionary::from_iter([
                                (
                                    "SerialNumber".to_string(),
                                    plist::Value::String("synthetic".to_string()),
                                ),
                                (
                                    "ConnectionType".to_string(),
                                    plist::Value::String("USB".to_string()),
                                ),
                                (key.to_string(), plist::Value::Integer(1234.into())),
                            ])),
                        ),
                    ]),
                )]),
            )]));
            let mut bytes = Vec::new();
            response.to_writer_xml(&mut bytes).unwrap();
            let decoded: DeviceList = plist::from_bytes(&bytes).unwrap();
            assert_eq!(decoded.device_list[0].properties.product_id, Some(1234));
        }
    }

    #[test]
    fn attach_notification_preserves_advertised_and_missing_product_id() {
        for product in [None, Some(1234)] {
            let mut properties = plist::Dictionary::from_iter([
                (
                    "SerialNumber".to_string(),
                    plist::Value::String("synthetic".to_string()),
                ),
                (
                    "ConnectionType".to_string(),
                    plist::Value::String("USB".to_string()),
                ),
            ]);
            if let Some(product) = product {
                properties.insert(
                    "ProductID".to_string(),
                    plist::Value::Integer(product.into()),
                );
            }
            let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "MessageType".to_string(),
                    plist::Value::String("Attached".to_string()),
                ),
                ("DeviceID".to_string(), plist::Value::Integer(42.into())),
                (
                    "Properties".to_string(),
                    plist::Value::Dictionary(properties),
                ),
            ]));
            let mut bytes = Vec::new();
            value.to_writer_xml(&mut bytes).unwrap();
            let decoded: DeviceEvent = plist::from_bytes(&bytes).unwrap();
            assert_eq!(
                decoded.properties.unwrap().product_id,
                product.map(|v| v as u16)
            );
        }
    }

    #[tokio::test]
    async fn recv_plist_rejects_oversized_message() {
        let mut header = [0u8; 16];
        header[..4].copy_from_slice(&((MAX_MUX_MESSAGE_SIZE as u32) + 1).to_le_bytes());
        let mut cursor = std::io::Cursor::new(header);

        let err = recv_plist::<_, plist::Value>(&mut cursor)
            .await
            .unwrap_err();
        assert!(
            matches!(err, MuxError::Protocol(message) if message.contains("message too large"))
        );
    }

    #[test]
    fn checked_mux_message_len_rejects_overflow() {
        let err = checked_mux_message_len(usize::MAX).unwrap_err();

        assert!(matches!(
            err,
            MuxError::Protocol(message) if message.contains("mux message length overflow")
        ));
    }
}

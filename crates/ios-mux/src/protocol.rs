use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::MuxError;

pub fn encode_message(payload: &[u8], tag: u32) -> Vec<u8> {
    let total = 16 + payload.len();
    let mut buf = Vec::with_capacity(total);
    buf.extend_from_slice(&(total as u32).to_le_bytes());
    buf.extend_from_slice(&1u32.to_le_bytes()); // version
    buf.extend_from_slice(&8u32.to_le_bytes()); // type = plist
    buf.extend_from_slice(&tag.to_le_bytes());
    buf.extend_from_slice(payload);
    buf
}

pub async fn send_plist<W, T>(writer: &mut W, value: &T, tag: u32) -> Result<(), MuxError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut plist_bytes = Vec::new();
    plist::to_writer_xml(&mut plist_bytes, value).map_err(|e| MuxError::Protocol(e.to_string()))?;
    let msg = encode_message(&plist_bytes, tag);
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
    let length = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
    if length < 16 {
        return Err(MuxError::Protocol(format!(
            "invalid message length: {length}"
        )));
    }
    let mut payload = vec![0u8; length - 16];
    reader.read_exact(&mut payload).await?;
    let value = plist::from_bytes(&payload).map_err(|e| MuxError::Protocol(e.to_string()))?;
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

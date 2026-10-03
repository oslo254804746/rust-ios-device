//! MobileBackup2 request shapes, bounded transfer frames and status decoding.
//!
//! Functions operate on supplied streams/values; host paths, file ownership,
//! transfer budgets and the DeviceLink session remain with their owners.

use super::Mobilebackup2Error;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) const FILE_TRANSFER_CODE_SUCCESS: u8 = 0x00; // Transfer completed successfully
pub(super) const FILE_TRANSFER_CODE_LOCAL_ERROR: u8 = 0x06; // Local (host) file I/O error
pub(super) const FILE_TRANSFER_CODE_FILE_DATA: u8 = 0x0c; // Payload contains file data chunk
pub(super) const FILE_TRANSFER_CODE_REMOTE_ERROR: u8 = 0x0b; // Remote (device) reported an error
pub(super) const BULK_OPERATION_ERROR: i64 = -13;
pub(super) const PURGE_DISK_SPACE_ERROR: i64 = -1;
pub(super) const PURGE_DISK_SPACE_ERROR_STRING: &str = "DLPurgeDiskSpace failed to purge";
// BackupAgent2 adds this amount to every purge request beyond the actual free-space threshold.
const PURGE_REQUEST_OVERSHOOT: u64 = 0x8000_0000;
pub(super) const MB_ERROR_INSUFFICIENT_DISK_SPACE: u64 = 105;
pub(super) const EMPTY_PARAMETER_STRING: &str = "___EmptyParameterString___";
// BackupAgent2 buffers one complete transfer frame before writing it to disk. Keep frames small
// enough for large incremental Manifest.db files to avoid exhausting the device process.
pub(super) const DOWNLOAD_CHUNK_SIZE: usize = 32 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct HelloRequest {
    pub(super) message_name: &'static str,
    pub(super) supported_protocol_versions: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct BackupRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct RestoreRequestOptions {
    pub(super) restore_should_reboot: bool,
    pub(super) restore_dont_copy_backup: bool,
    pub(super) restore_preserve_settings: bool,
    pub(super) restore_system_files: bool,
    pub(super) remove_items_not_restored: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct RestoreRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
    pub(super) source_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) password: Option<&'a str>,
    pub(super) options: RestoreRequestOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ChangePasswordRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) old_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) new_password: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct InfoRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_identifier: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ListRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
    pub(super) source_identifier: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct UnbackRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_identifier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) password: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
pub(super) struct ExtractRequest<'a> {
    pub(super) message_name: &'static str,
    pub(super) target_identifier: &'a str,
    pub(super) domain_name: &'a str,
    pub(super) relative_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) source_identifier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) password: Option<&'a str>,
}

/// Maximum size for a length-prefixed string (64 KiB). Device names and file
/// paths are never anywhere near this limit; the guard protects against
/// corrupted or malicious size fields causing unbounded allocation.
const MAX_PREFIXED_STRING_SIZE: usize = 64 * 1024;
pub(super) async fn read_prefixed_string<S>(stream: &mut S) -> Result<String, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let size = read_u32_be(stream).await? as usize;
    if size == 0 {
        return Ok(String::new());
    }
    if size > MAX_PREFIXED_STRING_SIZE {
        return Err(Mobilebackup2Error::Protocol(format!(
            "prefixed string too large: {size} bytes (max {MAX_PREFIXED_STRING_SIZE})"
        )));
    }

    let mut buf = vec![0u8; size];
    stream.read_exact(&mut buf).await?;
    String::from_utf8(buf).map_err(|error| {
        Mobilebackup2Error::Io(crate::error::context::retained_io(
            std::io::ErrorKind::InvalidData,
            "decode backup path UTF-8",
            error,
        ))
    })
}

pub(super) async fn read_u32_be<S>(stream: &mut S) -> Result<u32, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}

pub(super) async fn copy_transfer_payload<S, W>(
    stream: &mut S,
    writer: &mut W,
    mut remaining: usize,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0u8; DOWNLOAD_CHUNK_SIZE];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len());
        stream.read_exact(&mut buffer[..chunk_len]).await?;
        writer.write_all(&buffer[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

pub(super) async fn discard_transfer_payload<S>(
    stream: &mut S,
    mut remaining: usize,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let mut buffer = [0u8; DOWNLOAD_CHUNK_SIZE];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len());
        stream.read_exact(&mut buffer[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

pub(super) async fn write_prefixed_string<S>(
    stream: &mut S,
    value: &str,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&(value.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(value.as_bytes()).await?;
    Ok(())
}

pub(super) async fn write_transfer_frame<S>(
    stream: &mut S,
    code: u8,
    payload: &[u8],
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&((payload.len() as u32) + 1).to_be_bytes())
        .await?;
    stream.write_all(&[code]).await?;
    if !payload.is_empty() {
        stream.write_all(payload).await?;
    }
    Ok(())
}

pub(super) fn plist_number_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned(),
        plist::Value::Real(value) if value.is_finite() && *value >= 0.0 => {
            // Rust's float-to-integer cast saturates, so an unchecked cast
            // would silently turn negative, fractional, NaN, or oversized
            // protocol values into a different error/space value.
            let upper_bound = (u64::MAX as f64) + 1.0;
            if *value < upper_bound && value.fract() == 0.0 {
                Some(*value as u64)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn plist_number_to_f64(value: &plist::Value) -> Option<f64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned().map(|value| value as f64),
        plist::Value::Real(value) => Some(*value),
        _ => None,
    }
}

pub(super) fn redacted_protocol_dictionary(dict: &plist::Dictionary) -> plist::Dictionary {
    dict.iter()
        .map(|(key, value)| {
            let value = if is_sensitive_protocol_key(key) {
                plist::Value::String("<redacted>".into())
            } else {
                redacted_protocol_value(value)
            };
            (key.clone(), value)
        })
        .collect()
}

pub(super) fn redacted_protocol_value(value: &plist::Value) -> plist::Value {
    match value {
        plist::Value::Array(values) => {
            plist::Value::Array(values.iter().map(redacted_protocol_value).collect())
        }
        plist::Value::Dictionary(dict) => {
            plist::Value::Dictionary(redacted_protocol_dictionary(dict))
        }
        _ => value.clone(),
    }
}

fn is_sensitive_protocol_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "password"
            | "oldpassword"
            | "newpassword"
            | "passcode"
            | "unlocktoken"
            | "p12"
            | "privatekey"
    )
}

pub(super) fn derive_required_free_space(
    reported: Option<u64>,
    requested: Option<u64>,
) -> Option<u64> {
    let reported = reported?;
    let requested = requested?;
    if requested < PURGE_REQUEST_OVERSHOOT {
        return None;
    }

    requested
        .checked_add(reported)
        .and_then(|value| value.checked_sub(PURGE_REQUEST_OVERSHOOT))
}

pub(super) fn insufficient_disk_space_message(
    response: &plist::Dictionary,
    reported: Option<u64>,
    required: Option<u64>,
) -> String {
    let detail = "the device counts hardlink and clone references at full size because each uploaded path is stored separately";
    match (reported, required) {
        (Some(reported), Some(required)) => {
            let additional = required.saturating_add(1).saturating_sub(reported);
            format!(
                "backup process returned ErrorCode={MB_ERROR_INSUFFICIENT_DISK_SPACE}: device needs more than {required} bytes free, host reported {reported} bytes ({additional} more needed) (MBErrorDomain/{MB_ERROR_INSUFFICIENT_DISK_SPACE}); {detail}; response: {response:?}"
            )
        }
        _ => format!(
            "backup process returned ErrorCode={MB_ERROR_INSUFFICIENT_DISK_SPACE}: {response:?}; {detail}"
        ),
    }
}

pub(super) fn plist_value_to_bool(value: &plist::Value) -> Option<bool> {
    match value {
        plist::Value::Boolean(value) => Some(*value),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(|value| value != 0)
            .or_else(|| value.as_unsigned().map(|value| value != 0)),
        _ => None,
    }
}

pub(super) fn insert_file_failure(
    failures: &mut plist::Dictionary,
    rel: &str,
    err: &std::io::Error,
) {
    let mut failure = plist::Dictionary::from_iter([(
        "DLFileErrorString".to_string(),
        plist::Value::String(err.to_string()),
    )]);
    if let Some(code) = file_error_code_from_os_error(err) {
        failure.insert(
            "DLFileErrorCode".to_string(),
            plist::Value::Integer(code.into()),
        );
    }
    failures.insert(rel.to_string(), plist::Value::Dictionary(failure));
}

fn file_error_code_from_os_error(error: &std::io::Error) -> Option<i64> {
    match error.raw_os_error()? {
        2 => Some(-6),
        17 => Some(-7),
        20 => Some(-8),
        21 => Some(-9),
        62 => Some(-10),
        5 => Some(-11),
        28 => Some(-15),
        _ => None,
    }
}

pub(super) fn non_empty_protocol_string(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_required_free_space_from_purge_request() {
        let required = derive_required_free_space(Some(1_000_000), Some(2_335_105_975));
        assert_eq!(required, Some(188_622_327));
        assert_eq!(
            derive_required_free_space(Some(1_000_000), Some(PURGE_REQUEST_OVERSHOOT - 1)),
            None
        );
        assert_eq!(
            derive_required_free_space(None, Some(PURGE_REQUEST_OVERSHOOT)),
            None
        );
    }

    #[test]
    fn plist_u64_parser_rejects_non_integral_and_out_of_range_reals() {
        assert_eq!(plist_number_to_u64(&plist::Value::Real(42.0)), Some(42));
        for value in [-1.0, 42.5, f64::NAN, f64::INFINITY, u64::MAX as f64] {
            assert_eq!(
                plist_number_to_u64(&plist::Value::Real(value)),
                None,
                "real protocol value must be a finite non-negative integer: {value:?}"
            );
        }
    }

    #[test]
    fn insufficient_space_message_includes_requirement_and_host_report() {
        let response = plist::Dictionary::from_iter([(
            "ErrorDescription".to_string(),
            plist::Value::String("Insufficient free disk space".into()),
        )]);
        let message =
            insufficient_disk_space_message(&response, Some(1_000_000), Some(188_622_327));
        assert!(message.contains("needs more than 188622327 bytes free"));
        assert!(message.contains("host reported 1000000 bytes"));
        assert!(message.contains("187622328 more needed"));
        assert!(message.contains("MBErrorDomain/105"));
    }

    #[tokio::test]
    async fn read_prefixed_string_rejects_oversized_allocation() {
        // Craft a size field that exceeds MAX_PREFIXED_STRING_SIZE
        let size = (MAX_PREFIXED_STRING_SIZE as u32) + 1;
        let data = size.to_be_bytes();
        let mut cursor = std::io::Cursor::new(data.to_vec());
        let err = read_prefixed_string(&mut cursor).await.unwrap_err();
        assert!(
            err.to_string().contains("too large"),
            "expected size guard error, got: {err}"
        );
    }

    #[tokio::test]
    async fn read_prefixed_string_accepts_normal_size() {
        let payload = b"hello";
        let size = (payload.len() as u32).to_be_bytes();
        let mut data = size.to_vec();
        data.extend_from_slice(payload);
        let mut cursor = std::io::Cursor::new(data);
        let result = read_prefixed_string(&mut cursor).await.unwrap();
        assert_eq!(result, "hello");
    }

    #[tokio::test]
    async fn native_source_prefixed_path_retains_utf8_error() {
        let mut data = 2u32.to_be_bytes().to_vec();
        data.extend_from_slice(&[b'/', 0xff]);
        let error = read_prefixed_string(&mut std::io::Cursor::new(data))
            .await
            .unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(crate::test_native::has_source::<std::string::FromUtf8Error>(&error));
        assert!(error.device_operation_timeout().is_none());
    }
}

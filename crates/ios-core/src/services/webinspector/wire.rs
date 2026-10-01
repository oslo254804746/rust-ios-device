//! Bounded WebInspector plist frames and typed value decoding.
//! Supplied streams/values only; protocol state belongs to the client/sessions.

use super::WebInspectorError;
use serde_json::Value as JsonValue;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_PLIST_SIZE: usize = 16 * 1024 * 1024;

pub(super) async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), WebInspectorError> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value)?;
    if payload.len() > MAX_PLIST_SIZE {
        return Err(WebInspectorError::Protocol(format!(
            "plist length {} exceeds max {MAX_PLIST_SIZE}",
            payload.len()
        )));
    }
    let length = u32::try_from(payload.len()).map_err(|_| {
        WebInspectorError::Protocol(format!(
            "plist length {} does not fit in a 32-bit frame",
            payload.len()
        ))
    })?;
    stream.write_all(&length.to_be_bytes()).await?;
    stream.write_all(&payload).await?;
    stream.flush().await?;
    Ok(())
}

pub(super) async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, WebInspectorError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PLIST_SIZE {
        return Err(WebInspectorError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok(plist::from_bytes(&payload)?)
}

pub(super) fn required_string<'a>(
    dict: &'a plist::Dictionary,
    key: &str,
) -> Result<&'a str, WebInspectorError> {
    dict.get(key)
        .and_then(plist::Value::as_string)
        .ok_or_else(|| WebInspectorError::Protocol(format!("missing string field '{key}'")))
}

pub(super) fn optional_string<'a>(dict: &'a plist::Dictionary, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(plist::Value::as_string)
}

pub(super) fn required_bool(
    dict: &plist::Dictionary,
    key: &str,
) -> Result<bool, WebInspectorError> {
    optional_bool(dict, key)
        .ok_or_else(|| WebInspectorError::Protocol(format!("missing bool field '{key}'")))
}

pub(super) fn optional_bool(dict: &plist::Dictionary, key: &str) -> Option<bool> {
    match dict.get(key) {
        Some(plist::Value::Boolean(value)) => Some(*value),
        Some(plist::Value::Integer(value)) => value
            .as_unsigned()
            .map(|value| value != 0)
            .or_else(|| value.as_signed().map(|value| value != 0)),
        _ => None,
    }
}

pub(super) fn plist_integer_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value
            .as_unsigned()
            .or_else(|| value.as_signed().map(|value| value as u64)),
        _ => None,
    }
}

pub(super) fn pid_from_identifier(identifier: &str) -> Result<u64, WebInspectorError> {
    identifier
        .rsplit(':')
        .next()
        .ok_or_else(|| {
            WebInspectorError::Protocol(format!(
                "application identifier '{identifier}' does not contain ':'"
            ))
        })?
        .parse::<u64>()
        .map_err(|error| {
            WebInspectorError::Protocol(format!(
                "failed to parse PID from identifier '{identifier}': {error}"
            ))
        })
}

pub(super) fn extract_json_payload(
    dict: &plist::Dictionary,
    key: &str,
) -> Result<JsonValue, WebInspectorError> {
    match dict.get(key) {
        Some(plist::Value::Data(payload)) => Ok(serde_json::from_slice(payload)?),
        Some(plist::Value::String(payload)) => Ok(serde_json::from_str(payload)?),
        Some(other) => Err(WebInspectorError::Protocol(format!(
            "{key} expected data/string payload, got {other:?}"
        ))),
        None => Err(WebInspectorError::Protocol(format!(
            "missing JSON payload field '{key}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::duplex;

    #[test]
    fn extract_json_payload_accepts_string_payload() {
        let dict = plist::Dictionary::from_iter([(
            "WIRMessageDataKey".to_string(),
            plist::Value::String("{\"id\":1}".into()),
        )]);

        assert_eq!(
            extract_json_payload(&dict, "WIRMessageDataKey").unwrap(),
            json!({ "id": 1 })
        );
    }

    #[test]
    fn pid_from_identifier_rejects_non_numeric_suffix() {
        let err = pid_from_identifier("PID:not-a-number")
            .expect_err("invalid pid suffix must return an error");
        assert!(err
            .to_string()
            .contains("failed to parse PID from identifier 'PID:not-a-number'"));
    }

    #[tokio::test]
    async fn recv_plist_rejects_oversized_frames() {
        let (client, mut server) = duplex(64);
        let task = tokio::spawn(async move {
            let mut stream = client;
            recv_plist(&mut stream).await
        });

        server
            .write_all(&((MAX_PLIST_SIZE as u32) + 1).to_be_bytes())
            .await
            .unwrap();

        let err = task.await.unwrap().expect_err("oversized plist must fail");
        assert!(err.to_string().contains(&format!(
            "plist length {} exceeds max {}",
            MAX_PLIST_SIZE + 1,
            MAX_PLIST_SIZE
        )));
    }
}

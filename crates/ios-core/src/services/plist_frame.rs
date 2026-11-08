crate::redaction::error_debug!(PlistFrameError);

use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(thiserror::Error)]
pub(crate) enum PlistFrameError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub(crate) async fn write_xml_plist_frame<W, T>(
    writer: &mut W,
    value: &T,
    max_len: usize,
) -> Result<(), PlistFrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value)?;
    write_raw_plist_frame(writer, &payload, max_len).await
}

#[allow(dead_code)]
pub(crate) async fn write_binary_plist_frame<W, T>(
    writer: &mut W,
    value: &T,
    max_len: usize,
) -> Result<(), PlistFrameError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut payload = Vec::new();
    plist::to_writer_binary(&mut payload, value)?;
    write_raw_plist_frame(writer, &payload, max_len).await
}

pub(crate) async fn write_raw_plist_frame<W>(
    writer: &mut W,
    payload: &[u8],
    max_len: usize,
) -> Result<(), PlistFrameError>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > max_len {
        return Err(PlistFrameError::Protocol(format!(
            "plist frame length {} exceeds max {max_len}",
            payload.len()
        )));
    }
    let len = checked_payload_len(payload.len())?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

fn checked_payload_len(len: usize) -> Result<u32, PlistFrameError> {
    u32::try_from(len).map_err(|error| {
        PlistFrameError::Io(crate::error::context::retained_io(
            std::io::ErrorKind::InvalidInput,
            "encode plist frame length",
            error,
        ))
    })
}

pub(crate) async fn read_plist_frame<R, T>(
    reader: &mut R,
    max_len: usize,
) -> Result<T, PlistFrameError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > max_len {
        return Err(PlistFrameError::Protocol(format!(
            "plist frame length {len} exceeds max {max_len}"
        )));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await?;
    plist::from_bytes(&payload).map_err(PlistFrameError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Value;
    use tokio::io::AsyncReadExt;

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn native_plist_length_conversion_retains_input_source() {
        let error = checked_payload_len(u32::MAX as usize + 1).unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::InvalidInput
        );
        assert!(crate::test_native::has_source::<std::num::TryFromIntError>(
            &error
        ));
        assert_eq!(checked_payload_len(u32::MAX as usize).unwrap(), u32::MAX);
    }

    #[tokio::test]
    async fn write_xml_plist_frame_writes_big_endian_len_and_payload() {
        let value = Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            Value::String("Acknowledged".to_string()),
        )]));
        let mut output = Vec::new();

        write_xml_plist_frame(&mut output, &value, 1024)
            .await
            .unwrap();

        let len = u32::from_be_bytes(output[..4].try_into().unwrap()) as usize;
        let decoded: Value = plist::from_bytes(&output[4..4 + len]).unwrap();
        assert_eq!(decoded, value);
    }

    #[tokio::test]
    async fn write_xml_plist_frame_rejects_oversized_payload() {
        let value = Value::String("payload".repeat(64));
        let mut output = Vec::new();

        let err = write_xml_plist_frame(&mut output, &value, 8)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("plist frame"));
        assert!(output.is_empty());
    }

    #[tokio::test]
    async fn read_plist_frame_rejects_oversized_len_before_allocating() {
        let mut input = std::io::Cursor::new((1025u32).to_be_bytes());

        let err = read_plist_frame::<_, Value>(&mut input, 1024)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("exceeds max"));
    }

    #[tokio::test]
    async fn read_plist_frame_reads_exact_payload() {
        let value = Value::String("ok".to_string());
        let mut framed = Vec::new();
        write_xml_plist_frame(&mut framed, &value, 1024)
            .await
            .unwrap();
        framed.extend_from_slice(b"trailing");
        let mut input = std::io::Cursor::new(framed);

        let decoded: Value = read_plist_frame(&mut input, 1024).await.unwrap();

        assert_eq!(decoded, value);
        let mut trailing = Vec::new();
        input.read_to_end(&mut trailing).await.unwrap();
        assert_eq!(trailing, b"trailing");
    }
}

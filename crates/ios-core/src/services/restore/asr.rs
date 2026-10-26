//! ASR filesystem transfer on its own restored-selected data port.
//! Wire: pymobiledevice3@03ba2270 restore/asr.py and restore.py::send_filesystem.
//! ASR uses unframed XML plists, then raw image bytes, not restored framing.
use super::{error_mapping::native_error, RestoreError};
use crate::session::OperationCancel;
use plist::{Dictionary, Value};
use sha1::{Digest, Sha1};
use std::{io::SeekFrom, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

pub const DEFAULT_PORT: u16 = 12345;
const CHUNK: usize = 0x20000;
const MAX_COMMAND: usize = 1024 * 1024;

/// A local transfer receipt. It does not acknowledge installation or restore
/// completion; the owner must continue consuming restored's terminal status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrTransferReceipt {
    pub image_bytes_sent: u64,
    pub validation_bytes_sent: u64,
    pub checksum_chunks: bool,
}

/// Consumes the owned data-port stream. Cancellation/timeout drops that stream;
/// it cannot be reused to consume a late ASR command. The image remains caller-owned.
pub async fn send_image<S, R>(
    stream: S,
    image: &mut R,
    total: Duration,
    cancel: OperationCancel,
) -> Result<AsrTransferReceipt, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + AsyncSeek + Unpin,
{
    let deadline = Instant::now()
        .checked_add(total)
        .filter(|_| !total.is_zero())
        .ok_or_else(|| RestoreError::Protocol("invalid ASR total deadline".into()))?;
    send_image_until(stream, image, deadline, cancel).await
}

pub(crate) async fn send_image_until<S, R>(
    mut stream: S,
    image: &mut R,
    deadline: Instant,
    cancel: OperationCancel,
) -> Result<AsrTransferReceipt, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    R: AsyncRead + AsyncSeek + Unpin,
{
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(RestoreError::Io(std::io::Error::other(crate::OperationCancelled::new()))),
        result = tokio::time::timeout_at(deadline, async {
            let initiate = read_command(&mut stream).await?;
            if initiate.get("Command").and_then(Value::as_string) != Some("Initiate") {
                return Err(RestoreError::Protocol("ASR expected Initiate".into()));
            }
            let checksum = match initiate.get("Checksum Chunks") {
                None => false,
                Some(Value::Boolean(value)) => *value,
                _ => return Err(RestoreError::Protocol("invalid ASR Checksum Chunks".into())),
            };
            let size = image.seek(SeekFrom::End(0)).await?;
            if size == 0 { return Err(RestoreError::Protocol("ASR image is empty".into())); }
            image.seek(SeekFrom::Start(0)).await?;
            let mut response = Dictionary::from_iter([
                ("FEC Slice Stride", Value::Integer(40u64.into())),
                ("Packet Payload Size", Value::Integer(1450u64.into())),
                ("Packets Per FEC", Value::Integer(25u64.into())),
                ("Stream ID", Value::Integer(1u64.into())),
                ("Version", Value::Integer(1u64.into())),
                ("Payload", Value::Dictionary(Dictionary::from_iter([
                    ("Port", Value::Integer(1u64.into())), ("Size", Value::Integer(size.into()))
                ]))),
            ]);
            if checksum { response.insert("Checksum Chunk Size".into(), Value::Integer((CHUNK as u64).into())); }
            let mut bytes = Vec::new();
            plist::to_writer_xml(&mut bytes, &response)?;
            if !bytes.ends_with(b"\n") { bytes.push(b'\n'); }
            stream.write_all(&bytes).await?;
            stream.flush().await?;
            let mut buffer = vec![0; CHUNK];
            let mut validation_bytes = 0u64;
            loop {
                let command = read_command(&mut stream).await?;
                match command.get("Command").and_then(Value::as_string) {
                    Some("Payload") => break,
                    Some("OOBData") => {
                        let offset = integer(&command, "OOB Offset")?;
                        let count = integer(&command, "OOB Length")?;
                        if offset.checked_add(count).map_or(true, |end| end > size) {
                            return Err(RestoreError::Protocol("ASR OOB request exceeds image".into()));
                        }
                        image.seek(SeekFrom::Start(offset)).await?;
                        copy_exact(image, &mut stream, count, &mut buffer, false).await?;
                        stream.flush().await?;
                        validation_bytes = validation_bytes.checked_add(count)
                            .ok_or_else(|| RestoreError::Protocol("ASR validation counter overflow".into()))?;
                    }
                    _ => return Err(RestoreError::Protocol("unknown ASR validation command".into())),
                }
                tokio::task::yield_now().await;
            }
            image.seek(SeekFrom::Start(0)).await?;
            copy_exact(image, &mut stream, size, &mut buffer, checksum).await?;
            stream.flush().await?;
            Ok(AsrTransferReceipt { image_bytes_sent: size, validation_bytes_sent: validation_bytes, checksum_chunks: checksum })
        }) => result.map_err(|elapsed| native_error(std::io::ErrorKind::TimedOut, "ASR image transfer", elapsed))?,
    }
}

fn integer(command: &Dictionary, key: &str) -> Result<u64, RestoreError> {
    command
        .get(key)
        .and_then(Value::as_unsigned_integer)
        .ok_or_else(|| RestoreError::Protocol(format!("ASR missing or invalid {key}")))
}

async fn read_command<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Dictionary, RestoreError> {
    let mut bytes = Vec::new();
    loop {
        if bytes.len() == MAX_COMMAND {
            return Err(RestoreError::Protocol(
                "ASR command exceeds byte limit".into(),
            ));
        }
        bytes.push(stream.read_u8().await?);
        if bytes.ends_with(b"</plist>\n") {
            break;
        }
    }
    plist::from_bytes(&bytes).map_err(Into::into)
}

async fn copy_exact<R, S>(
    image: &mut R,
    stream: &mut S,
    mut remaining: u64,
    buffer: &mut [u8],
    checksum: bool,
) -> Result<(), RestoreError>
where
    R: AsyncRead + Unpin,
    S: AsyncWrite + Unpin,
{
    while remaining > 0 {
        let count = remaining.min(buffer.len() as u64) as usize;
        image.read_exact(&mut buffer[..count]).await?;
        stream.write_all(&buffer[..count]).await?;
        if checksum {
            stream.write_all(&Sha1::digest(&buffer[..count])).await?;
        }
        remaining -= count as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

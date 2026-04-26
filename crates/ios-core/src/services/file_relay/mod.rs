//! File relay service client.
//!
//! Service: `com.apple.mobile.file_relay`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.file_relay";

#[derive(Debug, thiserror::Error)]
pub enum FileRelayError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct FileRelayClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> FileRelayClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn request_sources(&mut self, sources: &[&str]) -> Result<Vec<u8>, FileRelayError> {
        let request = plist::Dictionary::from_iter([(
            "Sources".to_string(),
            plist::Value::Array(
                sources
                    .iter()
                    .map(|source| plist::Value::String((*source).to_string()))
                    .collect(),
            ),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        let response = recv_plist(&mut self.stream).await?;
        match response.get("Status").and_then(plist::Value::as_string) {
            Some("Acknowledged") => {}
            Some(other) => {
                let error = response
                    .get("Error")
                    .and_then(plist::Value::as_string)
                    .unwrap_or(other);
                return Err(FileRelayError::Protocol(error.to_string()));
            }
            None => {
                return Err(FileRelayError::Protocol(
                    "file relay response missing Status".into(),
                ));
            }
        }

        let mut data = Vec::new();
        self.stream.read_to_end(&mut data).await?;
        Ok(data)
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), FileRelayError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| FileRelayError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, FileRelayError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(FileRelayError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|e| FileRelayError::Plist(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    struct MockStream {
        read_buf: Vec<u8>,
        written: Vec<u8>,
        read_pos: usize,
    }

    impl MockStream {
        fn with_response(plist_value: plist::Value, raw: &[u8]) -> Self {
            let mut payload = Vec::new();
            plist::to_writer_xml(&mut payload, &plist_value).unwrap();
            let mut read_buf = Vec::new();
            read_buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            read_buf.extend_from_slice(&payload);
            read_buf.extend_from_slice(raw);
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
                return Poll::Ready(Ok(()));
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
    async fn request_sources_reads_acknowledged_archive() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response, b"archive-bytes");
        let mut client = FileRelayClient::new(&mut stream);

        let archive = client.request_sources(&["Network"]).await.unwrap();
        assert_eq!(archive, b"archive-bytes");

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let sources = dict["Sources"].as_array().unwrap();
        assert_eq!(sources[0].as_string(), Some("Network"));
    }
}

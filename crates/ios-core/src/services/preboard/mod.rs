//! Preboard service client.
//!
//! Service: `com.apple.preboardservice_v2`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.preboardservice_v2";

#[derive(Debug, thiserror::Error)]
pub enum PreboardError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
}

pub struct PreboardClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> PreboardClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn create_stashbag(
        &mut self,
        manifest: plist::Dictionary,
    ) -> Result<plist::Dictionary, PreboardError> {
        self.send_command("CreateStashbag", manifest).await
    }

    pub async fn commit_stashbag(
        &mut self,
        manifest: plist::Dictionary,
    ) -> Result<plist::Dictionary, PreboardError> {
        self.send_command("CommitStashbag", manifest).await
    }

    async fn send_command(
        &mut self,
        command: &str,
        manifest: plist::Dictionary,
    ) -> Result<plist::Dictionary, PreboardError> {
        let request = plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String(command.to_string()),
            ),
            ("Manifest".to_string(), plist::Value::Dictionary(manifest)),
        ]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        recv_plist(&mut self.stream).await
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), PreboardError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| PreboardError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, PreboardError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|e| PreboardError::Plist(e.to_string()))
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
    async fn create_stashbag_sends_manifest() {
        let response = plist::Value::Dictionary(plist::Dictionary::new());
        let mut stream = MockStream::with_response(response);
        let mut client = PreboardClient::new(&mut stream);

        client
            .create_stashbag(plist::Dictionary::from_iter([(
                "Example".to_string(),
                plist::Value::Boolean(true),
            )]))
            .await
            .unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("CreateStashbag"));
        assert_eq!(
            dict["Manifest"]
                .as_dictionary()
                .and_then(|m| m.get("Example"))
                .and_then(plist::Value::as_boolean),
            Some(true)
        );
    }
}

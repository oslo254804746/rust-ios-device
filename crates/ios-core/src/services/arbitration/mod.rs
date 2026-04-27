//! Device arbitration service client.
//!
//! Service: `com.apple.dt.devicearbitration`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.dt.devicearbitration";
const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ArbitrationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct ArbitrationClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> ArbitrationClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn version(&mut self) -> Result<plist::Dictionary, ArbitrationError> {
        self.send_command(plist::Dictionary::from_iter([(
            "command".to_string(),
            plist::Value::String("version".into()),
        )]))
        .await
    }

    pub async fn check_in(&mut self, hostname: &str, force: bool) -> Result<(), ArbitrationError> {
        let response = self
            .send_command(plist::Dictionary::from_iter([
                (
                    "command".to_string(),
                    plist::Value::String(if force { "force-check-in" } else { "check-in" }.into()),
                ),
                (
                    "hostname".to_string(),
                    plist::Value::String(hostname.to_string()),
                ),
            ]))
            .await?;
        ensure_success(&response)
    }

    pub async fn check_out(&mut self) -> Result<(), ArbitrationError> {
        let response = self
            .send_command(plist::Dictionary::from_iter([(
                "command".to_string(),
                plist::Value::String("check-out".into()),
            )]))
            .await?;
        ensure_success(&response)
    }

    async fn send_command(
        &mut self,
        request: plist::Dictionary,
    ) -> Result<plist::Dictionary, ArbitrationError> {
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        recv_plist(&mut self.stream).await
    }
}

fn ensure_success(response: &plist::Dictionary) -> Result<(), ArbitrationError> {
    match response.get("result").and_then(plist::Value::as_string) {
        Some("success") => Ok(()),
        Some(other) => Err(ArbitrationError::Protocol(other.to_string())),
        None => Err(ArbitrationError::Protocol(
            "device arbitration response missing result".into(),
        )),
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), ArbitrationError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| ArbitrationError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, ArbitrationError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PLIST_SIZE {
        return Err(ArbitrationError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|e| ArbitrationError::Plist(e.to_string()))
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
    async fn check_in_sends_hostname() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "result".to_string(),
            plist::Value::String("success".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = ArbitrationClient::new(&mut stream);

        client.check_in("host", false).await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["command"].as_string(), Some("check-in"));
        assert_eq!(dict["hostname"].as_string(), Some("host"));
    }

    #[tokio::test]
    async fn recv_plist_rejects_oversized_frame() {
        let mut read_buf = ((MAX_PLIST_SIZE as u32) + 1).to_be_bytes().to_vec();
        read_buf.extend_from_slice(b"ignored");
        let mut stream = MockStream {
            read_buf,
            written: Vec::new(),
            read_pos: 0,
        };

        let err = recv_plist(&mut stream).await.unwrap_err();
        assert!(
            matches!(err, ArbitrationError::Protocol(message) if message.contains("exceeds max"))
        );
    }
}

//! IDAM (Inter-Device Audio and MIDI) service client.
//!
//! Service: `com.apple.idamd`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.idamd";
pub const RSD_SERVICE_NAME: &str = "com.apple.idamd.shim.remote";

#[derive(Debug, thiserror::Error)]
pub enum IdamError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct IdamClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> IdamClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn configuration_inquiry(&mut self) -> Result<plist::Value, IdamError> {
        let request = plist::Dictionary::from_iter([(
            "Configuration Inquiry".to_string(),
            plist::Value::Boolean(true),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        Ok(plist::Value::Dictionary(
            recv_plist(&mut self.stream).await?,
        ))
    }

    pub async fn set_configuration(&mut self, enabled: bool) -> Result<plist::Value, IdamError> {
        let request = plist::Dictionary::from_iter([(
            "Set IDAM Configuration".to_string(),
            plist::Value::Boolean(enabled),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        match recv_plist(&mut self.stream).await {
            Ok(response) => Ok(plist::Value::Dictionary(response)),
            Err(IdamError::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                Ok(plist::Value::Dictionary(plist::Dictionary::new()))
            }
            Err(err) => Err(err),
        }
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), IdamError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value).map_err(|e| IdamError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(stream: &mut S) -> Result<plist::Dictionary, IdamError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(IdamError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|e| IdamError::Plist(e.to_string()))
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

        fn eof() -> Self {
            Self {
                read_buf: Vec::new(),
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
    async fn configuration_inquiry_sends_expected_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "SupportsIDAM".to_string(),
            plist::Value::Boolean(true),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = IdamClient::new(&mut stream);

        let value = client.configuration_inquiry().await.unwrap();
        let dict = value
            .as_dictionary()
            .expect("configuration inquiry should return a plist dictionary");
        assert_eq!(
            dict.get("SupportsIDAM").and_then(plist::Value::as_boolean),
            Some(true),
            "configuration inquiry should return plist payload"
        );

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Configuration Inquiry"].as_boolean(), Some(true));
    }

    #[tokio::test]
    async fn set_configuration_sends_expected_boolean() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = IdamClient::new(&mut stream);

        let value = client.set_configuration(false).await.unwrap();
        let dict = value
            .as_dictionary()
            .expect("set_configuration should return a plist dictionary");
        assert_eq!(
            dict.get("Status").and_then(plist::Value::as_string),
            Some("Acknowledged")
        );

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Set IDAM Configuration"].as_boolean(), Some(false));
    }

    #[tokio::test]
    async fn set_configuration_treats_eof_as_success() {
        let mut stream = MockStream::eof();
        let mut client = IdamClient::new(&mut stream);

        let value = client.set_configuration(true).await.unwrap();
        assert_eq!(
            value.as_dictionary().map(plist::Dictionary::len),
            Some(0),
            "EOF after set should be treated as a successful empty response"
        );

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Set IDAM Configuration"].as_boolean(), Some(true));
    }
}

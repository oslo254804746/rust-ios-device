//! Minimal mobileactivationd client.
//!
//! Current scope: read-only session-info request used by the activation flow.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobileactivationd";

#[derive(Debug, thiserror::Error)]
pub enum MobileActivationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug)]
pub struct MobileActivationClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> MobileActivationClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn request_session_info(
        &mut self,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let request = plist::Dictionary::from_iter([(
            "Command".to_string(),
            plist::Value::String("CreateTunnel1SessionInfoRequest".into()),
        )]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        recv_plist(&mut self.stream).await
    }

    pub async fn request_activation_info(
        &mut self,
        handshake_response: &[u8],
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let request = plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("CreateActivationInfoRequest".into()),
            ),
            (
                "Value".to_string(),
                plist::Value::Data(handshake_response.to_vec()),
            ),
            (
                "Options".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "BasebandWaitCount".to_string(),
                    plist::Value::Integer(90i64.into()),
                )])),
            ),
        ]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        recv_plist(&mut self.stream).await
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), MobileActivationError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value)
        .map_err(|e| MobileActivationError::Plist(e.to_string()))?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, MobileActivationError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(MobileActivationError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    plist::from_bytes(&buf).map_err(|e| MobileActivationError::Plist(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    #[derive(Default)]
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
    async fn request_session_info_sends_tunnel1_command_and_returns_response_dict() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Command".to_string(),
                    plist::Value::String("CreateTunnel1SessionInfoRequest".into()),
                ),
                (
                    "Value".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "HandshakeRequestMessage".to_string(),
                        plist::Value::Data(vec![1, 2, 3]),
                    )])),
                ),
            ])));
        let mut client = MobileActivationClient::new(&mut stream);

        let response = client.request_session_info().await.unwrap();
        assert_eq!(
            response.get("Command").and_then(plist::Value::as_string),
            Some("CreateTunnel1SessionInfoRequest")
        );
        assert!(response.contains_key("Value"));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(plist::Value::as_string),
            Some("CreateTunnel1SessionInfoRequest")
        );
    }

    #[tokio::test]
    async fn request_activation_info_sends_handshake_value_and_options() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Command".to_string(),
                    plist::Value::String("CreateActivationInfoRequest".into()),
                ),
                (
                    "Value".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "ActivationInfoXML".to_string(),
                        plist::Value::String("<plist/>".into()),
                    )])),
                ),
            ])));
        let mut client = MobileActivationClient::new(&mut stream);

        let response = client.request_activation_info(&[9, 8, 7]).await.unwrap();
        assert_eq!(
            response.get("Command").and_then(plist::Value::as_string),
            Some("CreateActivationInfoRequest")
        );

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("Command").and_then(plist::Value::as_string),
            Some("CreateActivationInfoRequest")
        );
        assert_eq!(
            dict.get("Value").and_then(plist::Value::as_data),
            Some(&b"\x09\x08\x07"[..])
        );
        let options = dict
            .get("Options")
            .and_then(plist::Value::as_dictionary)
            .expect("Options dictionary");
        assert_eq!(
            options
                .get("BasebandWaitCount")
                .and_then(plist::Value::as_unsigned_integer),
            Some(90)
        );
    }
}

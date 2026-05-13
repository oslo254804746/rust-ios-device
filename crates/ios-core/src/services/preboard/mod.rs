//! Preboard service client.
//!
//! Service: `com.apple.preboardservice_v2`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.preboardservice_v2";
const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PreboardError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
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
    plist::to_writer_xml(&mut buf, value)?;
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
    if len > MAX_PLIST_SIZE {
        return Err(PreboardError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(plist::from_bytes(&buf)?)
}

#[cfg(test)]
mod tests {
    use crate::test_util::MockStream;

    use super::*;

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

    #[tokio::test]
    async fn recv_plist_rejects_oversized_frame() {
        let mut read_buf = ((MAX_PLIST_SIZE as u32) + 1).to_be_bytes().to_vec();
        read_buf.extend_from_slice(b"ignored");
        let mut stream = MockStream::new(read_buf);

        let err = recv_plist(&mut stream).await.unwrap_err();
        assert!(matches!(err, PreboardError::Protocol(message) if message.contains("exceeds max")));
    }
}

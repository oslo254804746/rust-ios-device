//! Device arbitration service client.
//!
//! Service: `com.apple.dt.devicearbitration`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.dt.devicearbitration";
const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

service_error!(ArbitrationError);

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
    use crate::test_util::MockStream;

    use super::*;

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
        let mut stream = MockStream::new(read_buf);

        let err = recv_plist(&mut stream).await.unwrap_err();
        assert!(
            matches!(err, ArbitrationError::Protocol(message) if message.contains("exceeds max"))
        );
    }
}

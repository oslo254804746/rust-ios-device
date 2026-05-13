//! Minimal heartbeat service client.
//!
//! Service: `com.apple.mobile.heartbeat`

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SERVICE_NAME: &str = "com.apple.mobile.heartbeat";

service_error!(HeartbeatError);

pub struct HeartbeatClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> HeartbeatClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn recv_message(&mut self) -> Result<plist::Value, HeartbeatError> {
        recv_plist(&mut self.stream).await
    }

    pub async fn send_polo(&mut self) -> Result<(), HeartbeatError> {
        send_plist(
            &mut self.stream,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Command".to_string(),
                plist::Value::String("Polo".into()),
            )])),
        )
        .await
    }

    pub async fn ping(&mut self) -> Result<plist::Value, HeartbeatError> {
        let message = self.recv_message().await?;
        self.send_polo().await?;
        Ok(message)
    }
}

async fn send_plist<S>(stream: &mut S, value: &plist::Value) -> Result<(), HeartbeatError>
where
    S: AsyncWrite + Unpin,
{
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value)?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S>(stream: &mut S) -> Result<plist::Value, HeartbeatError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(HeartbeatError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(plist::from_bytes(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ping_reads_message_and_sends_polo() {
        let (client_side, mut server_side) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let incoming = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Command".to_string(),
                plist::Value::String("Marco".into()),
            )]));
            let mut buf = Vec::new();
            plist::to_writer_xml(&mut buf, &incoming).unwrap();
            server_side
                .write_all(&(buf.len() as u32).to_be_bytes())
                .await
                .unwrap();
            server_side.write_all(&buf).await.unwrap();

            let mut len_buf = [0u8; 4];
            server_side.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut payload = vec![0u8; len];
            server_side.read_exact(&mut payload).await.unwrap();
            let response: plist::Value = plist::from_bytes(&payload).unwrap();
            let dict = response.into_dictionary().unwrap();
            assert_eq!(dict["Command"].as_string(), Some("Polo"));
        });

        let mut client = HeartbeatClient::new(client_side);
        let message = client.ping().await.unwrap();
        let dict = message.into_dictionary().unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Marco"));
    }
}

//! Minimal heartbeat service client.
//!
//! Service: `com.apple.mobile.heartbeat`

use std::{future::Future, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::Instant,
};

pub const SERVICE_NAME: &str = "com.apple.mobile.heartbeat";

service_error!(HeartbeatError);

pub struct HeartbeatClient<S> {
    pub(crate) stream: Option<S>,
    read_buf: Vec<u8>,
    usable: bool,
    operation_timeout: Option<Duration>,
    idle_timeout: Option<Duration>,
}

impl<S> HeartbeatClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            read_buf: Vec::new(),
            usable: true,
            operation_timeout: None,
            idle_timeout: None,
        }
    }

    pub fn is_usable(&self) -> bool {
        self.usable && self.stream.is_some()
    }

    pub fn set_operation_timeout(&mut self, timeout: Option<Duration>) {
        self.operation_timeout = timeout;
    }

    pub fn with_operation_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.set_operation_timeout(timeout);
        self
    }

    pub fn set_idle_timeout(&mut self, timeout: Option<Duration>) {
        self.idle_timeout = timeout;
    }

    pub fn with_idle_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.set_idle_timeout(timeout);
        self
    }

    fn ensure_usable(&self) -> Result<(), HeartbeatError> {
        if self.is_usable() {
            Ok(())
        } else {
            Err(HeartbeatError::Protocol(
                "heartbeat connection is unusable; reconnect before another request".into(),
            ))
        }
    }

    fn deadline_for(&self, timeout: Option<Duration>) -> Result<Option<Instant>, HeartbeatError> {
        self.ensure_usable()?;
        match timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "heartbeat operation deadline is too large",
                )
                .into()
            }),
            None => Ok(None),
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> HeartbeatClient<S> {
    /// Receive an incoming heartbeat frame from the device.
    ///
    /// Respects `idle_timeout` without using short request budgets for
    /// long idle monitoring. Partially received frames are retained across
    /// pauses or cancellations in `read_buf` without framing loss.
    pub async fn recv_message(&mut self) -> Result<plist::Value, HeartbeatError> {
        self.ensure_usable()?;
        let deadline = self.deadline_for(self.idle_timeout)?;
        let stream = self.stream.as_mut().expect("active heartbeat stream");

        let result = await_operation(deadline, async {
            let mut temp_buf = [0u8; 4096];
            loop {
                if self.read_buf.len() >= 4 {
                    let len = u32::from_be_bytes(self.read_buf[..4].try_into().unwrap()) as usize;
                    const MAX_PLIST_SIZE: usize = 1024 * 1024;
                    if len > MAX_PLIST_SIZE {
                        return Err(HeartbeatError::Protocol(format!(
                            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
                        )));
                    }
                    if self.read_buf.len() >= 4 + len {
                        let frame_bytes = &self.read_buf[4..4 + len];
                        let val: plist::Value = plist::from_bytes(frame_bytes)?;
                        self.read_buf.drain(..4 + len);
                        return Ok(val);
                    }
                }
                let n = stream.read(&mut temp_buf).await?;
                if n == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF while reading heartbeat frame",
                    )
                    .into());
                }
                self.read_buf.extend_from_slice(&temp_buf[..n]);
            }
        })
        .await;

        match result {
            Ok(val) => Ok(val),
            Err(err) => {
                // If it is a protocol error or non-timeout IO error (like EOF), fail closed
                if matches!(&err, HeartbeatError::Protocol(_))
                    || matches!(&err, HeartbeatError::Io(e) if e.kind() != std::io::ErrorKind::TimedOut)
                {
                    self.usable = false;
                    self.stream = None;
                }
                Err(err)
            }
        }
    }

    /// Send a Polo reply frame to acknowledge device heartbeat.
    pub async fn send_polo(&mut self) -> Result<(), HeartbeatError> {
        self.ensure_usable()?;
        let deadline = self.deadline_for(self.operation_timeout)?;
        let stream = self.stream.as_mut().expect("active heartbeat stream");

        let result = await_operation(deadline, async {
            send_plist(
                stream,
                &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "Command".to_string(),
                    plist::Value::String("Polo".into()),
                )])),
            )
            .await
        })
        .await;

        if result.is_err() {
            self.usable = false;
            self.stream = None;
        }
        result
    }

    /// Perform a full heartbeat exchange (recv message then send Polo reply).
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

fn elapsed_budget() -> HeartbeatError {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "heartbeat operation budget elapsed",
    )
    .into()
}

async fn await_operation<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = Result<T, HeartbeatError>>,
) -> Result<T, HeartbeatError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| {
                HeartbeatError::Io(crate::error::context::retained_io(
                    std::io::ErrorKind::TimedOut,
                    "heartbeat operation deadline",
                    error,
                ))
            })?,
        None => operation.await,
    }
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
        assert!(client.is_usable());
    }

    #[tokio::test]
    async fn heartbeat_zero_budget_fails_without_io_and_retains_stream() {
        let (client_side, _) = tokio::io::duplex(1024);
        let mut client = HeartbeatClient::new(client_side).with_idle_timeout(Some(Duration::ZERO));
        assert!(client.is_usable());

        let err = client.recv_message().await.unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        assert!(client.is_usable());
        assert!(client.stream.is_some());
    }

    #[tokio::test]
    async fn heartbeat_partial_frame_retained_across_cancellation() {
        let (client_side, mut server_side) = tokio::io::duplex(1024);
        let mut client = HeartbeatClient::new(client_side);

        let incoming = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Command".to_string(),
            plist::Value::String("Marco".into()),
        )]));
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &incoming).unwrap();
        let mut full_frame = (buf.len() as u32).to_be_bytes().to_vec();
        full_frame.extend(buf);

        // Send first 2 bytes of the 4-byte header
        server_side.write_all(&full_frame[..2]).await.unwrap();

        // First recv with very short timeout cancels while only partial header was read
        client.set_idle_timeout(Some(Duration::from_millis(20)));
        let err = client.recv_message().await.unwrap_err();
        assert_eq!(crate::classify_error(&err), crate::ErrorClass::Timeout);
        // Partial bytes are retained in client buffer without breaking framing
        assert_eq!(client.read_buf.len(), 2);
        assert!(client.is_usable());

        // Send the rest of the frame
        server_side.write_all(&full_frame[2..]).await.unwrap();

        // Second recv with adequate timeout succeeds, resuming reading the frame!
        client.set_idle_timeout(Some(Duration::from_secs(5)));
        let msg = client.recv_message().await.unwrap();
        let dict = msg.into_dictionary().unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Marco"));
        assert_eq!(client.read_buf.len(), 0);
        assert!(client.is_usable());
    }

    #[tokio::test]
    async fn heartbeat_idle_listening_not_killed_by_short_request_timeout() {
        let (client_side, mut server_side) = tokio::io::duplex(1024);
        let mut client = HeartbeatClient::new(client_side);
        // Short request timeout for replies (Polo)
        client.set_operation_timeout(Some(Duration::from_millis(50)));
        // Idle timeout is None (unbounded wait for Marco from device)
        client.set_idle_timeout(None);

        tokio::spawn(async move {
            // Sleep 150ms before device sends Marco (longer than operation_timeout!)
            tokio::time::sleep(Duration::from_millis(150)).await;
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
        });

        // recv_message should NOT time out because idle_timeout is None
        let msg = client.recv_message().await.unwrap();
        let dict = msg.into_dictionary().unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Marco"));
    }
}

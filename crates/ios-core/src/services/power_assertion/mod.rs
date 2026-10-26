//! Power assertion service client.
//!
//! Service: `com.apple.mobile.assertion_agent`

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::Instant;

pub const SERVICE_NAME: &str = "com.apple.mobile.assertion_agent";

service_error!(PowerAssertionError);

fn elapsed_budget() -> PowerAssertionError {
    PowerAssertionError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "power assertion operation budget elapsed",
    ))
}

fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, PowerAssertionError>,
) -> Result<T, PowerAssertionError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

async fn await_operation<T, F: Future<Output = Result<T, PowerAssertionError>>>(
    deadline: Option<Instant>,
    future: F,
) -> Result<T, PowerAssertionError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| elapsed_budget())?,
        None => future.await,
    }
}

pub struct PowerAssertionClient<S> {
    stream: Option<S>,
    operation_timeout: Option<Duration>,
    usable: bool,
}

impl<S> PowerAssertionClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream: Some(stream),
            operation_timeout: None,
            usable: true,
        }
    }

    pub fn set_operation_timeout(&mut self, timeout: Option<Duration>) {
        self.operation_timeout = timeout;
    }

    pub fn is_usable(&self) -> bool {
        self.usable && self.stream.is_some()
    }

    fn ensure_usable(&self) -> Result<(), PowerAssertionError> {
        if !self.is_usable() {
            Err(PowerAssertionError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "power assertion connection is not usable",
            )))
        } else {
            Ok(())
        }
    }

    fn deadline(&self) -> Result<Option<Instant>, PowerAssertionError> {
        self.ensure_usable()?;
        match self.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                PowerAssertionError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "power assertion operation deadline is too large",
                ))
            }),
            None => Ok(None),
        }
    }

    fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<PowerAssertionRequest<'_, S>, PowerAssertionError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(PowerAssertionRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> PowerAssertionClient<S> {
    pub async fn create_assertion(
        &mut self,
        assertion_type: &str,
        name: &str,
        timeout_seconds: f64,
        details: Option<&str>,
    ) -> Result<plist::Dictionary, PowerAssertionError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;

        let mut req_dict = plist::Dictionary::from_iter([
            (
                "CommandKey".to_string(),
                plist::Value::String("CommandCreateAssertion".into()),
            ),
            (
                "AssertionTypeKey".to_string(),
                plist::Value::String(assertion_type.to_string()),
            ),
            (
                "AssertionNameKey".to_string(),
                plist::Value::String(name.to_string()),
            ),
            (
                "AssertionTimeoutKey".to_string(),
                plist::Value::Real(timeout_seconds),
            ),
        ]);
        if let Some(details) = details {
            req_dict.insert(
                "AssertionDetailKey".to_string(),
                plist::Value::String(details.to_string()),
            );
        }

        let result = await_operation(deadline, async {
            let mut io = request.io();
            send_plist(&mut io, &plist::Value::Dictionary(req_dict)).await?;
            let response = recv_plist(&mut io).await?;
            if let Some(error) = response.get("Error").and_then(plist::Value::as_string) {
                return Err(PowerAssertionError::Protocol(error.to_string()));
            }
            Ok(response)
        })
        .await;

        request.finish(check_deadline(deadline, result))
    }
}

struct PowerAssertionRequest<'a, S> {
    client: &'a mut PowerAssertionClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> PowerAssertionRequest<'_, S> {
    fn io(&mut self) -> PowerAssertionIo<'_, S> {
        PowerAssertionIo {
            inner: self.stream.as_mut().expect("active power assertion stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    fn finish<T>(
        mut self,
        result: Result<T, PowerAssertionError>,
    ) -> Result<T, PowerAssertionError> {
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.usable = false;
        }
        result
    }
}

impl<S> Drop for PowerAssertionRequest<'_, S> {
    fn drop(&mut self) {
        if self.stream.is_none() {
            return;
        }
        if self.progressed || self.failed {
            self.client.usable = false;
        } else {
            self.client.stream = self.stream.take();
        }
    }
}

struct PowerAssertionIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> PowerAssertionIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PowerAssertionIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        *self.progressed |= buf.filled().len() > before;
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PowerAssertionIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        *self.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        *self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), PowerAssertionError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, value)?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, PowerAssertionError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(PowerAssertionError::Protocol(format!(
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
    use crate::test_util::MockStream;
    use tokio::io::duplex;

    #[tokio::test]
    async fn create_assertion_sends_expected_payload() {
        let response = plist::Value::Dictionary(plist::Dictionary::new());
        let mut stream = MockStream::with_response(response);
        let mut client = PowerAssertionClient::new(&mut stream);

        client
            .create_assertion("PreventUserIdleSystemSleep", "ios-cli", 30.0, Some("test"))
            .await
            .unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("CommandKey").and_then(plist::Value::as_string),
            Some("CommandCreateAssertion")
        );
        assert_eq!(
            dict.get("AssertionTypeKey")
                .and_then(plist::Value::as_string),
            Some("PreventUserIdleSystemSleep")
        );
        assert_eq!(
            dict.get("AssertionNameKey")
                .and_then(plist::Value::as_string),
            Some("ios-cli")
        );
        assert_eq!(
            dict.get("AssertionDetailKey")
                .and_then(plist::Value::as_string),
            Some("test")
        );
    }

    #[tokio::test]
    async fn power_assertion_zero_budget_fails_without_io() {
        let (client_io, _server_io) = duplex(64);
        let mut client = PowerAssertionClient::new(client_io);
        client.set_operation_timeout(Some(Duration::ZERO));

        let err = client
            .create_assertion("PreventUserIdleSystemSleep", "ios-cli", 30.0, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PowerAssertionError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut)
        );
        assert!(client.is_usable());
    }

    #[tokio::test]
    async fn power_assertion_shared_deadline_and_invalidation() {
        let (client_io, mut server_io) = duplex(1024);
        let mut client = PowerAssertionClient::new(client_io);
        client.set_operation_timeout(Some(Duration::from_millis(50)));

        let server_task = tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            server_io.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            server_io.read_exact(&mut buf).await.unwrap();
            tokio::time::sleep(Duration::from_millis(150)).await;
        });

        let err = client
            .create_assertion("PreventUserIdleSystemSleep", "ios-cli", 30.0, None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, PowerAssertionError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut)
        );
        assert!(
            !client.is_usable(),
            "partial write/stalled reply must invalidate client"
        );

        let _ = server_task.await;
    }
}

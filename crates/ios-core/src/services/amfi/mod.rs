//! AMFI (Apple Mobile File Integrity) – developer mode control.
//!
//! Service: `com.apple.amfi.lockdown`

crate::redaction::error_debug!(AmfiError);

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::time::Instant;

pub const SERVICE_NAME: &str = "com.apple.amfi.lockdown";

const DEVELOPER_MODE_REVEAL: u64 = 0;
const DEVELOPER_MODE_ENABLE: u64 = 1;

#[derive(thiserror::Error)]
pub enum AmfiError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
    #[error("error: {0}")]
    Device(String),
}

fn elapsed_budget() -> AmfiError {
    AmfiError::Io(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "amfi operation budget elapsed",
    ))
}

fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, AmfiError>,
) -> Result<T, AmfiError> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

async fn await_operation<T, F: Future<Output = Result<T, AmfiError>>>(
    deadline: Option<Instant>,
    future: F,
) -> Result<T, AmfiError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, future)
            .await
            .map_err(|_| elapsed_budget())?,
        None => future.await,
    }
}

/// AMFI client with shared request deadline, cancellation guard, and connection invalidation.
pub struct AmfiClient<S> {
    stream: Option<S>,
    operation_timeout: Option<Duration>,
    usable: bool,
}

impl<S> AmfiClient<S> {
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

    fn ensure_usable(&self) -> Result<(), AmfiError> {
        if !self.is_usable() {
            Err(AmfiError::Io(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "amfi connection is not usable",
            )))
        } else {
            Ok(())
        }
    }

    fn deadline(&self) -> Result<Option<Instant>, AmfiError> {
        self.ensure_usable()?;
        match self.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                AmfiError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "amfi operation deadline is too large",
                ))
            }),
            None => Ok(None),
        }
    }

    fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<AmfiRequest<'_, S>, AmfiError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(AmfiRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AmfiClient<S> {
    /// Enable developer mode on the device.
    ///
    /// After calling this, the device needs to be rebooted.
    pub async fn enable_developer_mode(&mut self) -> Result<(), AmfiError> {
        self.send_action(DEVELOPER_MODE_ENABLE).await
    }

    /// Reveal the Developer Mode option in the device Settings UI.
    pub async fn reveal_developer_mode(&mut self) -> Result<(), AmfiError> {
        self.send_action(DEVELOPER_MODE_REVEAL).await
    }

    async fn send_action(&mut self, action: u64) -> Result<(), AmfiError> {
        let deadline = self.deadline()?;
        check_deadline(deadline, Ok(()))?;
        let mut request = self.begin_request(deadline)?;
        let result = await_operation(deadline, async {
            let mut io = request.io();
            send_developer_mode_action_inner(&mut io, action).await
        })
        .await;
        request.finish(check_deadline(deadline, result))
    }
}

struct AmfiRequest<'a, S> {
    client: &'a mut AmfiClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> AmfiRequest<'_, S> {
    fn io(&mut self) -> AmfiIo<'_, S> {
        AmfiIo {
            inner: self.stream.as_mut().expect("active amfi stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    fn finish<T>(mut self, result: Result<T, AmfiError>) -> Result<T, AmfiError> {
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.usable = false;
        }
        result
    }
}

impl<S> Drop for AmfiRequest<'_, S> {
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

struct AmfiIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> AmfiIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for AmfiIo<'_, S> {
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

impl<S: AsyncWrite + Unpin> AsyncWrite for AmfiIo<'_, S> {
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

/// Enable developer mode on the device (legacy compatibility wrapper).
pub async fn enable_developer_mode<S>(stream: &mut S) -> Result<(), AmfiError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_developer_mode_action_inner(stream, DEVELOPER_MODE_ENABLE).await
}

/// Reveal the Developer Mode option in the device Settings UI (legacy compatibility wrapper).
pub async fn reveal_developer_mode<S>(stream: &mut S) -> Result<(), AmfiError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_developer_mode_action_inner(stream, DEVELOPER_MODE_REVEAL).await
}

async fn send_developer_mode_action_inner<S>(stream: &mut S, action: u64) -> Result<(), AmfiError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let req = plist::Value::Dictionary({
        let mut d = plist::Dictionary::new();
        d.insert("action".to_string(), plist::Value::Integer(action.into()));
        d
    });

    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &req)?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(AmfiError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"),
        )));
    }
    let mut resp_buf = vec![0u8; len];
    stream.read_exact(&mut resp_buf).await?;

    let val: plist::Value = plist::from_bytes(&resp_buf)?;

    if let Some(dict) = val.as_dictionary() {
        if let Some(err) = dict.get("Error").and_then(|v| v.as_string()) {
            return Err(AmfiError::Device(err.to_string()));
        }
        if dict
            .get("success")
            .and_then(|v| v.as_boolean())
            .is_some_and(|success| !success)
        {
            return Err(AmfiError::Device(format!("{val:?}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn make_amfi_success_response() -> Vec<u8> {
        let mut dict = plist::Dictionary::new();
        dict.insert("success".into(), plist::Value::Boolean(true));
        let val = plist::Value::Dictionary(dict);
        let mut xml = Vec::new();
        plist::to_writer_xml(&mut xml, &val).unwrap();
        let mut wire = Vec::new();
        wire.extend_from_slice(&(xml.len() as u32).to_be_bytes());
        wire.extend_from_slice(&xml);
        wire
    }

    #[tokio::test]
    async fn amfi_zero_budget_fails_without_io_and_retains_stream() {
        let (client_io, _server_io) = duplex(64);
        let mut client = AmfiClient::new(client_io);
        client.set_operation_timeout(Some(Duration::ZERO));

        let err = client.reveal_developer_mode().await.unwrap_err();
        assert!(matches!(err, AmfiError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut));
        assert!(
            client.is_usable(),
            "stream should be retained on zero budget"
        );
    }

    #[tokio::test]
    async fn amfi_send_and_recv_share_single_deadline() {
        let (client_io, mut server_io) = duplex(1024);
        let mut client = AmfiClient::new(client_io);
        client.set_operation_timeout(Some(Duration::from_millis(50)));

        let server_task = tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            server_io.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            server_io.read_exact(&mut buf).await.unwrap();
            // Stalled response causes client timeout
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = server_io.write_all(&make_amfi_success_response()).await;
        });

        let err = client.reveal_developer_mode().await.unwrap_err();
        assert!(matches!(err, AmfiError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut));
        assert!(
            !client.is_usable(),
            "client must be invalidated after timeout crossing send/recv"
        );

        let _ = server_task.await;
    }

    #[tokio::test]
    async fn amfi_partial_send_cancellation_invalidates_client() {
        let (client_io, mut server_io) = duplex(2); // small buffer to cause write pause
        let mut client = AmfiClient::new(client_io);
        client.set_operation_timeout(Some(Duration::from_millis(40)));

        // Server only reads 2 bytes then stops
        let server_task = tokio::spawn(async move {
            let mut buf = [0u8; 2];
            let _ = server_io.read_exact(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        });

        let err = client.enable_developer_mode().await.unwrap_err();
        assert!(matches!(err, AmfiError::Io(ref e) if e.kind() == std::io::ErrorKind::TimedOut));
        assert!(
            !client.is_usable(),
            "partial write progress must invalidate client"
        );

        let _ = server_task.await;
    }

    #[tokio::test]
    async fn amfi_successful_action_restores_stream() {
        let (client_io, mut server_io) = duplex(1024);
        let mut client = AmfiClient::new(client_io);
        client.set_operation_timeout(Some(Duration::from_secs(5)));

        let server_task = tokio::spawn(async move {
            let mut len_buf = [0u8; 4];
            server_io.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            server_io.read_exact(&mut buf).await.unwrap();
            server_io
                .write_all(&make_amfi_success_response())
                .await
                .unwrap();
        });

        client.reveal_developer_mode().await.unwrap();
        assert!(
            client.is_usable(),
            "successful request restores stream usability"
        );

        let _ = server_task.await;
    }
}

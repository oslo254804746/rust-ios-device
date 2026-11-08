use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{error::Elapsed, Instant};

service_error!(DeviceLinkError);

const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);

impl From<super::plist_frame::PlistFrameError> for DeviceLinkError {
    fn from(error: super::plist_frame::PlistFrameError) -> Self {
        match error {
            super::plist_frame::PlistFrameError::Io(error) => Self::Io(error),
            super::plist_frame::PlistFrameError::Plist(error) => Self::Plist(error),
            super::plist_frame::PlistFrameError::Protocol(message) => Self::Protocol(message),
        }
    }
}

#[cfg(test)]
mod lifecycle_tests;

pub struct DeviceLinkClient<S> {
    stream: S,
    state: DeviceLinkState,
}

struct DeviceLinkState {
    usable: bool,
    operation_timeout: Option<Duration>,
    progressed: bool,
    failed: bool,
}

impl Default for DeviceLinkState {
    fn default() -> Self {
        Self {
            usable: true,
            operation_timeout: None,
            progressed: false,
            failed: false,
        }
    }
}

// Count logical bytes, including reads from or writes into a caller's buffered
// transport. A failed operation cannot leave its remainder for a new caller.
struct DeviceLinkIo<'a, S> {
    inner: &'a mut S,
    state: &'a mut DeviceLinkState,
}

impl<S: AsyncRead + Unpin> AsyncRead for DeviceLinkIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        self.state.progressed |= buf.filled().len() > before;
        self.state.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for DeviceLinkIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        self.state.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

struct DeviceLinkOperation<'a, S> {
    client: &'a mut DeviceLinkClient<S>,
    deadline: Option<Instant>,
    completed: bool,
}

impl<S> DeviceLinkOperation<'_, S> {
    fn finish<T>(
        mut self,
        result: Result<Result<T, DeviceLinkError>, Elapsed>,
    ) -> Result<T, DeviceLinkError> {
        match result {
            Ok(result) => {
                self.completed = true;
                if self.client.state.failed || (result.is_err() && self.client.state.progressed) {
                    self.client.state.usable = false;
                }
                result
            }
            Err(elapsed) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into()),
        }
    }
}

impl<S> Drop for DeviceLinkOperation<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.client.state.progressed || self.client.state.failed) {
            self.client.state.usable = false;
        }
    }
}

fn await_device_link_operation<T>(
    deadline: Option<Instant>,
    operation: impl std::future::Future<Output = Result<T, DeviceLinkError>>,
) -> impl std::future::Future<Output = Result<Result<T, DeviceLinkError>, Elapsed>> {
    // Box before creating the awaiting state, keeping callers' future stacks small.
    let operation = Box::pin(operation);
    async move {
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, operation).await,
            None => Ok(operation.await),
        }
    }
}

impl<S> DeviceLinkClient<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            state: DeviceLinkState::default(),
        }
    }

    /// Whether tracked DeviceLink operations permit local reuse. This does not
    /// establish peer liveness, remote cleanup or safe retry of a side effect.
    pub fn is_usable(&self) -> bool {
        self.state.usable
    }

    /// Share an absolute budget across one message, complete version exchange
    /// or process call. None preserves unbounded timing, including idle reads.
    /// Serialization and decoding are cooperative and cannot be preempted.
    pub fn set_operation_timeout(&mut self, budget: Option<Duration>) {
        self.state.operation_timeout = budget;
    }

    /// Transfer the raw stream and responsibility for its current protocol
    /// state to the caller. This does not restore an unusable connection.
    pub fn into_inner(self) -> S {
        self.stream
    }

    /// Access the raw transport. Raw I/O bypasses DeviceLink progress tracking;
    /// the caller must own cancellation, framing and total-operation policy.
    pub fn stream_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    pub(crate) fn stream_ref(&self) -> &S {
        &self.stream
    }

    fn begin_operation(
        &mut self,
        budget: Option<Duration>,
    ) -> Result<DeviceLinkOperation<'_, S>, DeviceLinkError> {
        if !self.is_usable() {
            return Err(DeviceLinkError::Protocol(
                "device link connection is unusable; reconnect before another operation".into(),
            ));
        }
        let deadline = match budget {
            Some(budget) if budget.is_zero() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "device link operation budget is zero",
                )
                .into());
            }
            Some(budget) => Some(Instant::now().checked_add(budget).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "device link operation deadline is too large",
                )
            })?),
            None => None,
        };
        self.state.progressed = false;
        self.state.failed = false;
        Ok(DeviceLinkOperation {
            client: self,
            deadline,
            completed: false,
        })
    }
}

impl<S> DeviceLinkClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn version_exchange(&mut self) -> Result<u64, DeviceLinkError> {
        let operation = self.begin_operation(self.state.operation_timeout)?;
        let result = await_device_link_operation(
            operation.deadline,
            operation.client.version_exchange_inner(),
        )
        .await;
        operation.finish(result)
    }

    async fn version_exchange_inner(&mut self) -> Result<u64, DeviceLinkError> {
        let response = self.recv_message_inner().await?;
        let message = response.as_array().ok_or_else(|| {
            DeviceLinkError::Protocol(format!(
                "device link version exchange expected array, got {response:?}"
            ))
        })?;

        let message_type = message
            .first()
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "device link version exchange missing message type: {response:?}"
                ))
            })?;
        if message_type != "DLMessageVersionExchange" {
            return Err(DeviceLinkError::Protocol(format!(
                "expected DLMessageVersionExchange, got {message_type}"
            )));
        }

        let version = message
            .get(1)
            .and_then(|value| match value {
                plist::Value::Integer(value) => value.as_unsigned(),
                _ => None,
            })
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "device link version exchange missing major version: {response:?}"
                ))
            })?;

        self.send_message_inner(&vec![
            plist::Value::String("DLMessageVersionExchange".into()),
            plist::Value::String("DLVersionsOk".into()),
            plist::Value::Integer(version.into()),
        ])
        .await?;

        let ready = self.recv_message_inner().await?;
        let ready_message = ready.as_array().ok_or_else(|| {
            DeviceLinkError::Protocol(format!("device ready expected array, got {ready:?}"))
        })?;
        let ready_type = ready_message
            .first()
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!("device ready missing message type: {ready:?}"))
            })?;
        if ready_type != "DLMessageDeviceReady" {
            return Err(DeviceLinkError::Protocol(format!(
                "expected DLMessageDeviceReady, got {ready_type}"
            )));
        }

        Ok(version)
    }

    pub async fn send_process_message<T>(&mut self, message: &T) -> Result<(), DeviceLinkError>
    where
        T: Serialize,
    {
        self.send_message(&("DLMessageProcessMessage", message))
            .await
    }

    pub async fn recv_process_message(&mut self) -> Result<plist::Dictionary, DeviceLinkError> {
        let response = self.recv_message().await?;
        Self::parse_process_message(response)
    }

    /// Send a process request and receive its reply under one budget and
    /// cancellation owner. Separate send/receive calls have separate ownership.
    pub async fn call_process_message<T>(
        &mut self,
        message: &T,
    ) -> Result<plist::Dictionary, DeviceLinkError>
    where
        T: Serialize,
    {
        let operation = self.begin_operation(self.state.operation_timeout)?;
        let result = await_device_link_operation(operation.deadline, async {
            operation
                .client
                .send_message_inner(&("DLMessageProcessMessage", message))
                .await?;
            operation.client.recv_message_inner().await
        })
        .await;
        // A complete decoded reply keeps legacy service-envelope validation
        // separate from transport framing failure.
        Self::parse_process_message(operation.finish(result)?)
    }

    fn parse_process_message(response: plist::Value) -> Result<plist::Dictionary, DeviceLinkError> {
        let message = response.as_array().ok_or_else(|| {
            DeviceLinkError::Protocol(format!("process message expected array, got {response:?}"))
        })?;

        let message_type = message
            .first()
            .and_then(plist::Value::as_string)
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "process message missing message type: {response:?}"
                ))
            })?;
        if message_type != "DLMessageProcessMessage" {
            return Err(DeviceLinkError::Protocol(format!(
                "expected DLMessageProcessMessage, got {message_type}"
            )));
        }

        message
            .get(1)
            .and_then(plist::Value::as_dictionary)
            .cloned()
            .ok_or_else(|| {
                DeviceLinkError::Protocol(format!(
                    "process message missing dictionary payload: {response:?}"
                ))
            })
    }

    pub async fn send_message<T>(&mut self, message: &T) -> Result<(), DeviceLinkError>
    where
        T: Serialize,
    {
        let operation = self.begin_operation(self.state.operation_timeout)?;
        let result = await_device_link_operation(
            operation.deadline,
            operation.client.send_message_inner(message),
        )
        .await;
        operation.finish(result)
    }

    async fn send_message_inner<T: Serialize>(
        &mut self,
        message: &T,
    ) -> Result<(), DeviceLinkError> {
        super::plist_frame::write_xml_plist_frame(
            &mut DeviceLinkIo {
                inner: &mut self.stream,
                state: &mut self.state,
            },
            message,
            MAX_PLIST_SIZE,
        )
        .await
        .map_err(DeviceLinkError::from)
    }

    pub async fn recv_message(&mut self) -> Result<plist::Value, DeviceLinkError> {
        let operation = self.begin_operation(self.state.operation_timeout)?;
        let result =
            await_device_link_operation(operation.deadline, operation.client.recv_message_inner())
                .await;
        operation.finish(result)
    }

    async fn recv_message_inner(&mut self) -> Result<plist::Value, DeviceLinkError> {
        super::plist_frame::read_plist_frame(
            &mut DeviceLinkIo {
                inner: &mut self.stream,
                state: &mut self.state,
            },
            MAX_PLIST_SIZE,
        )
        .await
        .map_err(DeviceLinkError::from)
    }

    /// Send the existing disconnect message with a five-second cleanup cap,
    /// clamped by a shorter configured budget. Success closes local reuse; no
    /// acknowledgement or remote cleanup is inferred. Drop performs no I/O.
    pub async fn disconnect(&mut self) -> Result<(), DeviceLinkError> {
        let budget = self
            .state
            .operation_timeout
            .map_or(DISCONNECT_TIMEOUT, |budget| budget.min(DISCONNECT_TIMEOUT));
        let operation = self.begin_operation(Some(budget))?;
        let result = await_device_link_operation(operation.deadline, async {
            operation
                .client
                .send_message_inner(&vec![
                    plist::Value::String("DLMessageDisconnect".into()),
                    plist::Value::String("___EmptyParameterString___".into()),
                ])
                .await
        })
        .await;
        let result = operation.finish(result);
        if result.is_ok() {
            self.state.usable = false;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn encode_frame(value: &plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, value).expect("plist serialization");
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    async fn read_frame(stream: &mut tokio::io::DuplexStream) -> plist::Value {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await.expect("frame length");
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stream
            .read_exact(&mut payload)
            .await
            .expect("frame payload");
        plist::from_bytes(&payload).expect("plist decode")
    }

    #[tokio::test]
    async fn version_exchange_sends_versions_ok_and_returns_major_version() {
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = DeviceLinkClient::new(client_stream);
            client.version_exchange().await.unwrap()
        });

        server_stream
            .write_all(&encode_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300u64.into()),
            ])))
            .await
            .unwrap();

        let versions_ok = read_frame(&mut server_stream).await;
        assert_eq!(
            versions_ok.as_array(),
            Some(&vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::String("DLVersionsOk".into()),
                plist::Value::Integer(300u64.into()),
            ])
        );

        server_stream
            .write_all(&encode_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .unwrap();

        assert_eq!(task.await.unwrap(), 300);
    }

    #[tokio::test]
    async fn recv_process_message_requires_dictionary_payload() {
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = DeviceLinkClient::new(client_stream);
            client.recv_process_message().await
        });

        server_stream
            .write_all(&encode_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::String("not-a-dict".into()),
            ])))
            .await
            .unwrap();

        let err = task
            .await
            .unwrap()
            .expect_err("non-dictionary payload must fail");
        assert!(err
            .to_string()
            .contains("process message missing dictionary payload"));
    }

    #[tokio::test]
    async fn disconnect_sends_expected_message() {
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = DeviceLinkClient::new(client_stream);
            client.disconnect().await.unwrap();
        });

        let disconnect = read_frame(&mut server_stream).await;
        assert_eq!(
            disconnect.as_array(),
            Some(&vec![
                plist::Value::String("DLMessageDisconnect".into()),
                plist::Value::String("___EmptyParameterString___".into()),
            ])
        );

        task.await.unwrap();
    }
}

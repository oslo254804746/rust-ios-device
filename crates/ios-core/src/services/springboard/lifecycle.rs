use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Instant,
};

use super::{SpringboardClient, SpringboardError};

#[derive(Debug)]
pub(super) struct SpringboardState {
    usable: bool,
    operation_timeout: Option<Duration>,
}

impl Default for SpringboardState {
    fn default() -> Self {
        Self {
            usable: true,
            operation_timeout: None,
        }
    }
}

impl<S> SpringboardClient<S> {
    /// Whether the original stream permits another local request. This does
    /// not check peer liveness or establish that a failed mutation is retryable.
    pub fn is_usable(&self) -> bool {
        self.state.usable && self.stream.is_some()
    }

    /// Set the total budget for each request, including encoding, sending,
    /// receiving and response parsing. None preserves unbounded legacy waits.
    /// Synchronous encoding/parsing cannot be preempted, but elapsed work cannot
    /// return success or begin sending after encoding exhausts the budget.
    ///
    /// After wire progress, cancellation or failure drops the original stream
    /// and rejects reuse. A zero/overflowing budget performs no IO. Before any
    /// progress, cancellation or expiry keeps the stream available.
    pub fn set_operation_timeout(&mut self, budget: Option<Duration>) {
        self.state.operation_timeout = budget;
    }

    fn ensure_usable(&self) -> Result<(), SpringboardError> {
        if self.is_usable() {
            Ok(())
        } else {
            Err(SpringboardError::Protocol(
                "springboard connection is unusable; reconnect before another request".into(),
            ))
        }
    }

    pub(super) fn deadline(&self) -> Result<Option<Instant>, SpringboardError> {
        self.ensure_usable()?;
        match self.state.operation_timeout {
            Some(budget) if budget.is_zero() => Err(elapsed_budget()),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "springboard operation deadline is too large",
                )
                .into()
            }),
            None => Ok(None),
        }
    }

    pub(super) fn begin_request(
        &mut self,
        deadline: Option<Instant>,
    ) -> Result<SpringboardRequest<'_, S>, SpringboardError> {
        self.ensure_usable()?;
        let stream = self.stream.take();
        Ok(SpringboardRequest {
            client: self,
            stream,
            deadline,
            progressed: false,
            failed: false,
        })
    }
}

/// Owns the exact stream while a request is active. Drop restores an untouched
/// stream, or retires a stream after progress/failure even on caller cancellation.
pub(super) struct SpringboardRequest<'a, S> {
    client: &'a mut SpringboardClient<S>,
    stream: Option<S>,
    deadline: Option<Instant>,
    progressed: bool,
    failed: bool,
}

impl<S> SpringboardRequest<'_, S> {
    pub(super) fn io(&mut self) -> SpringboardIo<'_, S> {
        SpringboardIo {
            inner: self.stream.as_mut().expect("active springboard stream"),
            deadline: self.deadline,
            progressed: &mut self.progressed,
            failed: &mut self.failed,
        }
    }

    pub(super) fn finish<T>(
        mut self,
        result: Result<T, SpringboardError>,
    ) -> Result<T, SpringboardError> {
        // An EOF tolerated by setIconState can return Ok while the transport
        // is terminal; it must still be dropped rather than restored.
        if !self.failed && (result.is_ok() || !self.progressed) {
            self.client.stream = self.stream.take();
        } else {
            self.client.state.usable = false;
        }
        result
    }
}

impl<S> Drop for SpringboardRequest<'_, S> {
    fn drop(&mut self) {
        if self.stream.is_none() {
            return;
        }
        if self.progressed || self.failed {
            self.client.state.usable = false;
        } else {
            self.client.stream = self.stream.take();
        }
    }
}

pub(super) struct SpringboardIo<'a, S> {
    inner: &'a mut S,
    deadline: Option<Instant>,
    progressed: &'a mut bool,
    failed: &'a mut bool,
}

impl<S> SpringboardIo<'_, S> {
    fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for SpringboardIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // The enclosing timeout polls its body before its timer. Fence each IO
        // stage even when an earlier Ready poll consumed the remaining clock.
        // Pending lets that actual expired timer retain its original Elapsed;
        // it never manufactures a native error or polls the original stream.
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

impl<S: AsyncWrite + Unpin> AsyncWrite for SpringboardIo<'_, S> {
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
        if self.expired() {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        *self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

fn elapsed_budget() -> SpringboardError {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "springboard operation budget elapsed",
    )
    .into()
}

pub(super) fn check_deadline<T>(
    deadline: Option<Instant>,
    result: Result<T, SpringboardError>,
) -> Result<T, SpringboardError> {
    // Preserve a primary IO/plist/protocol failure even when the operation
    // completes in the same poll that exhausts the clock.
    if result.is_ok() && deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        Err(elapsed_budget())
    } else {
        result
    }
}

pub(super) async fn await_operation<T>(
    deadline: Option<Instant>,
    operation: impl Future<Output = Result<T, SpringboardError>>,
) -> Result<T, SpringboardError> {
    match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, operation)
            .await
            .map_err(|error| {
                SpringboardError::Io(crate::error::context::retained_io(
                    std::io::ErrorKind::TimedOut,
                    "springboard operation deadline",
                    error,
                ))
            })?,
        None => operation.await,
    }
}

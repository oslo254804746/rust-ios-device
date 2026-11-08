use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{error::Elapsed, Instant},
};

use super::{NotificationProxyClient, NotificationProxyError};

pub(super) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(super) struct NotificationState {
    usable: bool,
    operation_timeout: Option<Duration>,
    progressed: bool,
    failed: bool,
}

impl Default for NotificationState {
    fn default() -> Self {
        Self {
            usable: true,
            operation_timeout: None,
            progressed: false,
            failed: false,
        }
    }
}

impl<S> NotificationProxyClient<S> {
    /// Whether tracked operations permit local reuse. This is not a peer
    /// liveness check, cleanup acknowledgement or safe-retry guarantee.
    pub fn is_usable(&self) -> bool {
        self.state.usable
    }

    /// Set a total budget for one public operation. Explicit waits use the
    /// shorter budget; None preserves unbounded sends and idle receives.
    /// Synchronous encoding/decoding cannot be preempted by the timer.
    pub fn set_operation_timeout(&mut self, budget: Option<Duration>) {
        self.state.operation_timeout = budget;
    }

    pub(super) fn invalidate(&mut self) {
        self.state.usable = false;
        self.observing.clear();
    }

    pub(super) fn deadline(
        &self,
        explicit: Option<Duration>,
    ) -> Result<Option<Instant>, NotificationProxyError> {
        self.ensure_usable()?;
        let budget = match (explicit, self.state.operation_timeout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match budget {
            Some(budget) if budget.is_zero() => Err(NotificationProxyError::Timeout),
            Some(budget) => Instant::now().checked_add(budget).map(Some).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "notification operation deadline is too large",
                )
                .into()
            }),
            None => Ok(None),
        }
    }

    fn ensure_usable(&self) -> Result<(), NotificationProxyError> {
        if !self.is_usable() {
            return Err(NotificationProxyError::Protocol(
                "notification connection is unusable; reconnect before another operation".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn begin_frame(
        &mut self,
    ) -> Result<NotificationFrame<'_, S>, NotificationProxyError> {
        // Every internal frame also checks state, including cached observe and
        // the first read after a completed subscription.
        self.ensure_usable()?;
        self.state.progressed = false;
        self.state.failed = false;
        Ok(NotificationFrame {
            client: self,
            completed: false,
        })
    }
}

pub(super) struct NotificationFrame<'a, S> {
    pub(super) client: &'a mut NotificationProxyClient<S>,
    completed: bool,
}

impl<S> NotificationFrame<'_, S> {
    pub(super) fn io(&mut self) -> NotificationIo<'_, S> {
        NotificationIo {
            inner: &mut self.client.stream,
            state: &mut self.client.state,
        }
    }

    pub(super) fn finish<T>(
        mut self,
        result: Result<T, NotificationProxyError>,
    ) -> Result<T, NotificationProxyError> {
        self.completed = true;
        if self.client.state.failed || (result.is_err() && self.client.state.progressed) {
            self.client.invalidate();
        }
        result
    }
}

impl<S> Drop for NotificationFrame<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.client.state.progressed || self.client.state.failed) {
            self.client.invalidate();
        }
    }
}

pub(super) struct NotificationIo<'a, S> {
    inner: &'a mut S,
    state: &'a mut NotificationState,
}

impl<S: AsyncRead + Unpin> AsyncRead for NotificationIo<'_, S> {
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

impl<S: AsyncWrite + Unpin> AsyncWrite for NotificationIo<'_, S> {
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

pub(super) fn await_notification_operation<T>(
    deadline: Option<Instant>,
    context: &'static str,
    operation: impl std::future::Future<Output = Result<T, NotificationProxyError>>,
) -> impl std::future::Future<Output = Result<T, NotificationProxyError>> {
    let operation = Box::pin(operation);
    async move {
        match deadline {
            Some(deadline) => {
                tokio::time::timeout_at(deadline, operation)
                    .await
                    .map_err(|error: Elapsed| {
                        NotificationProxyError::Io(crate::error::context::retained_io(
                            std::io::ErrorKind::TimedOut,
                            context,
                            error,
                        ))
                    })?
            }
            None => operation.await,
        }
    }
}

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

use super::{WebInspectorClient, WebInspectorError};

#[derive(Debug)]
pub(super) struct WebInspectorState {
    usable: bool,
    operation_timeout: Option<Duration>,
    progressed: bool,
    failed: bool,
}

impl Default for WebInspectorState {
    fn default() -> Self {
        Self {
            usable: true,
            operation_timeout: None,
            progressed: false,
            failed: false,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct Deadline {
    pub(super) instant: Instant,
    pub(super) budget: Duration,
}

impl<S> WebInspectorClient<S> {
    /// Whether tracked operations permit local reuse. This does not test peer
    /// liveness or authorize retrying an interrupted device operation.
    pub fn is_usable(&self) -> bool {
        self.state.usable
    }

    /// Set a total budget for one client operation, including automatic listing
    /// writes while routing an event. None keeps the existing unbounded default.
    /// Explicit startup/event budgets use the shorter configured limit. Timers
    /// cannot preempt synchronous codecs or an already running worker.
    pub fn set_operation_timeout(&mut self, budget: Option<Duration>) {
        self.state.operation_timeout = budget;
    }

    pub(super) fn ensure_usable(&self) -> Result<(), WebInspectorError> {
        if !self.is_usable() {
            return Err(WebInspectorError::Protocol(
                "webinspector connection is unusable; reconnect before another operation".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        self.state.usable = false;
        self.automation_availability = None;
        self.applications = Default::default();
        self.application_pages = Default::default();
        self.pending_events = Default::default();
    }

    pub(super) fn deadline(
        &self,
        explicit: Option<Duration>,
    ) -> Result<Option<Deadline>, WebInspectorError> {
        self.ensure_usable()?;
        let budget = match (explicit, self.state.operation_timeout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        budget.map(checked_deadline).transpose()
    }

    pub(super) fn begin_frame(&mut self) -> Result<WebInspectorFrame<'_, S>, WebInspectorError> {
        self.ensure_usable()?;
        self.state.progressed = false;
        self.state.failed = false;
        Ok(WebInspectorFrame {
            client: self,
            completed: false,
        })
    }
}

pub(super) fn checked_deadline(budget: Duration) -> Result<Deadline, WebInspectorError> {
    if budget.is_zero() {
        return Err(WebInspectorError::Timeout(budget));
    }
    let instant = Instant::now().checked_add(budget).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "webinspector operation deadline is too large",
        )
    })?;
    Ok(Deadline { instant, budget })
}

pub(super) struct WebInspectorFrame<'a, S> {
    client: &'a mut WebInspectorClient<S>,
    completed: bool,
}

impl<S> WebInspectorFrame<'_, S> {
    pub(super) fn io(&mut self) -> WebInspectorIo<'_, S> {
        WebInspectorIo {
            inner: &mut self.client.stream,
            state: &mut self.client.state,
        }
    }

    pub(super) fn finish<T>(
        mut self,
        result: Result<T, WebInspectorError>,
    ) -> Result<T, WebInspectorError> {
        self.completed = true;
        if self.client.state.failed || (result.is_err() && self.client.state.progressed) {
            self.client.invalidate();
        }
        result
    }
}

impl<S> Drop for WebInspectorFrame<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.client.state.progressed || self.client.state.failed) {
            self.client.invalidate();
        }
    }
}

// A completed frame may still belong to an unfinished logical operation, such
// as startup or an incoming application list that triggers outgoing listings.
pub(super) struct WebInspectorOperation<'a, S> {
    pub(super) client: &'a mut WebInspectorClient<S>,
    committed: bool,
    completed: bool,
}

impl<'a, S> WebInspectorOperation<'a, S> {
    pub(super) fn new(client: &'a mut WebInspectorClient<S>) -> Result<Self, WebInspectorError> {
        client.ensure_usable()?;
        Ok(Self {
            client,
            committed: false,
            completed: false,
        })
    }

    pub(super) fn commit(&mut self) {
        self.committed = true;
    }

    pub(super) fn finish<T>(
        mut self,
        result: Result<T, WebInspectorError>,
    ) -> Result<T, WebInspectorError> {
        self.completed = true;
        if result.is_err() && self.committed {
            self.client.invalidate();
        }
        result
    }
}

impl<S> Drop for WebInspectorOperation<'_, S> {
    fn drop(&mut self) {
        if !self.completed && self.committed {
            self.client.invalidate();
        }
    }
}

pub(super) struct WebInspectorIo<'a, S> {
    inner: &'a mut S,
    state: &'a mut WebInspectorState,
}

impl<S: AsyncRead + Unpin> AsyncRead for WebInspectorIo<'_, S> {
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

impl<S: AsyncWrite + Unpin> AsyncWrite for WebInspectorIo<'_, S> {
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

pub(super) async fn await_operation<T>(
    deadline: Option<Deadline>,
    phase: &'static str,
    operation: impl Future<Output = Result<T, WebInspectorError>>,
) -> Result<T, WebInspectorError> {
    let mut operation = Box::pin(operation);
    match deadline {
        None => operation.await,
        Some(deadline) => {
            let guarded_operation = std::future::poll_fn(|cx| {
                if Instant::now() >= deadline.instant {
                    Poll::Pending
                } else {
                    operation.as_mut().poll(cx)
                }
            });
            tokio::select! {
                biased;
                error = tokio::time::timeout_at(deadline.instant, std::future::pending::<()>()) => {
                    Err(super::super::native_timeout(deadline.budget, phase,
                        error.expect_err("pending deadline cannot complete")))
                }
                result = guarded_operation => result,
            }
        }
    }
}

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

use super::super::{
    retained::{event_size, MAX_EVENTS, MAX_EVENT_BYTES},
    WebInspectorEvent,
};
use super::{WebInspectorClient, WebInspectorError};
use std::collections::VecDeque;

#[derive(Debug)]
pub(super) struct WebInspectorState {
    usable: bool,
    operation_timeout: Option<Duration>,
    progressed: bool,
    failed: bool,
    inherited_deadline: Option<Deadline>,
}

impl Default for WebInspectorState {
    fn default() -> Self {
        Self {
            usable: true,
            operation_timeout: None,
            progressed: false,
            failed: false,
            inherited_deadline: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(in super::super) struct Deadline {
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

    pub(in super::super) fn ensure_usable(&self) -> Result<(), WebInspectorError> {
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
        self.connected_application_snapshot_seen = false;
        self.pending_events = Default::default();
        self.pending_event_bytes = 0;
    }

    pub(in super::super) fn deadline(
        &self,
        explicit: Option<Duration>,
    ) -> Result<Option<Deadline>, WebInspectorError> {
        self.ensure_usable()?;
        let budget = match (explicit, self.state.operation_timeout) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if budget.is_some_and(|budget| budget.is_zero()) {
            return Err(WebInspectorError::Timeout(Duration::ZERO));
        }
        if let Some(inherited) = self.state.inherited_deadline {
            if budget.map_or(true, |budget| {
                budget >= inherited.instant.saturating_duration_since(Instant::now())
            }) {
                return Ok(Some(inherited));
            }
        }
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
pub(in super::super) struct WebInspectorOperation<'a, S> {
    pub(in super::super) client: &'a mut WebInspectorClient<S>,
    committed: bool,
    completed: bool,
    skipped: VecDeque<WebInspectorEvent>,
    skipped_bytes: usize,
    previous_deadline: Option<Deadline>,
}

impl<'a, S> WebInspectorOperation<'a, S> {
    pub(in super::super) fn new(
        client: &'a mut WebInspectorClient<S>,
    ) -> Result<Self, WebInspectorError> {
        client.ensure_usable()?;
        let previous_deadline = client.state.inherited_deadline;
        Ok(Self {
            client,
            committed: false,
            completed: false,
            skipped: VecDeque::new(),
            skipped_bytes: 0,
            previous_deadline,
        })
    }

    pub(in super::super) fn with_deadline(
        client: &'a mut WebInspectorClient<S>,
        deadline: Option<Deadline>,
    ) -> Result<Self, WebInspectorError> {
        let operation = Self::new(client)?;
        operation.client.state.inherited_deadline = deadline.or(operation.previous_deadline);
        Ok(operation)
    }

    pub(in super::super) fn commit(&mut self) {
        self.committed = true;
    }

    /// A fully routed unrelated event is retained, with no partial frame or
    /// automatic write outstanding. Cancellation can restore these same events.
    pub(in super::super) fn retained_event_boundary(&mut self) {
        self.committed = false;
    }

    pub(in super::super) fn preserve(
        &mut self,
        event: WebInspectorEvent,
    ) -> Result<(), WebInspectorError> {
        if self.skipped.len() + self.client.pending_events.len() >= MAX_EVENTS {
            return Err(super::super::local_limit(
                "retained event count",
                MAX_EVENTS as u64,
            ));
        }
        let used = self.skipped_bytes + self.client.pending_event_bytes;
        let bytes = event_size(&event, MAX_EVENT_BYTES.saturating_sub(used))?;
        self.skipped.push_back(event);
        self.skipped_bytes += bytes;
        Ok(())
    }

    // The matching response completed the wire obligation, including an
    // explicit business error. Parsing that response does not await more IO.
    pub(in super::super) fn finish_response<T>(
        mut self,
        result: Result<T, WebInspectorError>,
    ) -> Result<T, WebInspectorError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.committed = false;
        self.finish(result)
    }

    pub(in super::super) fn finish<T>(
        mut self,
        result: Result<T, WebInspectorError>,
    ) -> Result<T, WebInspectorError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.completed = true;
        if result.is_err() && self.committed {
            self.client.invalidate();
            return result;
        }
        let restored = self
            .client
            .restore_events(std::mem::take(&mut self.skipped));
        match result {
            Err(error) => Err(error),
            Ok(value) => restored.map(|()| value),
        }
    }
}

impl<S> Drop for WebInspectorOperation<'_, S> {
    fn drop(&mut self) {
        if !self.completed && self.committed {
            self.client.invalidate();
        } else if !self.completed && self.client.is_usable() && !self.skipped.is_empty() {
            // preserve() already admitted these immutable events together with
            // the pending queue. No IO occurs between that admission and here.
            self.client.pending_event_bytes += self.skipped_bytes;
            while let Some(event) = self.skipped.pop_back() {
                self.client.pending_events.push_front(event);
            }
        }
        self.client.state.inherited_deadline = self.previous_deadline;
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

pub(in super::super) async fn await_operation<T>(
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

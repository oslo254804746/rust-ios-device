use std::{
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Instant,
};

use super::CompanionError;

pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) fn usable(state: &AtomicBool) -> bool {
    state.load(Ordering::Acquire)
}

pub(super) fn invalidate(state: &AtomicBool) {
    state.store(false, Ordering::Release);
}

pub(super) fn ensure_usable(state: &AtomicBool) -> Result<(), CompanionError> {
    if usable(state) {
        Ok(())
    } else {
        Err(CompanionError::Protocol(
            "companion connection is unusable; reconnect before another operation".into(),
        ))
    }
}

pub(super) fn deadline(budget: Duration) -> Result<Instant, CompanionError> {
    if budget.is_zero() {
        return Err(CompanionError::Timeout(budget));
    }
    Instant::now().checked_add(budget).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "companion operation deadline is too large",
        )
        .into()
    })
}

pub(super) async fn await_operation<T>(
    budget: Duration,
    phase: &'static str,
    operation: impl std::future::Future<Output = Result<T, CompanionError>>,
) -> Result<T, CompanionError> {
    let deadline = deadline(budget)?;
    let operation = Box::pin(operation);
    tokio::time::timeout_at(deadline, operation)
        .await
        .map_err(|error| super::native_timeout(budget, phase, error))?
}

pub(super) struct CompanionOperation<'a, S> {
    stream: &'a mut S,
    state: &'a AtomicBool,
    progressed: bool,
    failed: bool,
    completed: bool,
}

impl<'a, S> CompanionOperation<'a, S> {
    pub(super) fn new(stream: &'a mut S, state: &'a AtomicBool) -> Result<Self, CompanionError> {
        // This check happens after the shared mutex is acquired, too. Another
        // owner may have invalidated the connection while this one waited.
        ensure_usable(state)?;
        Ok(Self {
            stream,
            state,
            progressed: false,
            failed: false,
            completed: false,
        })
    }

    pub(super) fn finish<T>(
        mut self,
        result: Result<T, CompanionError>,
    ) -> Result<T, CompanionError> {
        self.completed = true;
        if self.failed || (result.is_err() && self.progressed) {
            invalidate(self.state);
        }
        result
    }
}

impl<S> Drop for CompanionOperation<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.progressed || self.failed) {
            invalidate(self.state);
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CompanionOperation<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut *self.stream).poll_read(cx, buf);
        self.progressed |= buf.filled().len() > before;
        self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CompanionOperation<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut *self.stream).poll_write(cx, bytes);
        self.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        self.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.stream).poll_flush(cx);
        self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut *self.stream).poll_shutdown(cx);
        self.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

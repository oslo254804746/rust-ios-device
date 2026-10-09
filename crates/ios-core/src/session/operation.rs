//! One cooperative operation clock and cancellation fence, with owned IO.

use std::future::{poll_fn, Future};
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::watch;
use tokio::time::Instant;

/// Local cancellation, independent of native IO Interrupted.
#[derive(Clone)]
pub struct OperationCancel(Arc<watch::Sender<bool>>);

impl Default for OperationCancel {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationCancel {
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    /// Idempotent. Does not acknowledge remote rollback or cleanup.
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    pub(crate) async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        loop {
            if *receiver.borrow_and_update() {
                return;
            }
            // self retains its sender throughout this wait.
            if receiver.changed().await.is_err() {
                return;
            }
        }
    }
}

impl std::fmt::Debug for OperationCancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationCancel")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OperationOptions {
    pub(crate) total: Duration,
}

impl OperationOptions {
    pub fn new(total: Duration) -> Self {
        Self { total }
    }
}

/// Private construction, no Clone and no raw transport escape.
pub(crate) struct OperationContext {
    pub(crate) deadline: Instant,
    pub(crate) caller: OperationCancel,
    pub(crate) owner: OperationCancel,
    // Only actual Ready(Err) observations enter this slot. In particular, a
    // Pending operation or a zero-byte successful native read is not invented
    // as a failure. Keep the original object across an upper future's Pending.
    pub(super) native: Mutex<Option<Arc<io::Error>>>,
}

struct NativeIoCause(Arc<io::Error>);

impl std::fmt::Display for NativeIoCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("retained operation native IO failure")
    }
}

impl std::fmt::Debug for NativeIoCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeIoCause")
            .field("kind", &self.0.kind())
            .finish()
    }
}

impl std::error::Error for NativeIoCause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

fn native_error(source: Arc<io::Error>) -> io::Error {
    io::Error::new(source.kind(), NativeIoCause(source))
}

fn cancelled() -> io::Error {
    io::Error::other(crate::OperationCancelled::new())
}

impl OperationContext {
    fn take_native_failure(&self) -> Option<io::Error> {
        self.native
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .map(native_error)
    }

    /// A core operation which handles an observed native error and then waits
    /// for non-IO work must acknowledge that handling. An arbitrary Future's
    /// error-handling intent cannot be inferred from Poll::Pending. Successful
    /// completion, a terminal result, and a subsequent allowed native poll
    /// also discard the preceding candidate. This is not automatic retry.
    pub(crate) fn discard_handled_native_error(&self) {
        drop(
            self.native
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take(),
        );
    }

    fn retain_native(&self, source: io::Error) -> io::Error {
        let mut native = self
            .native
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let source = native.get_or_insert_with(|| Arc::new(source));
        native_error(Arc::clone(source))
    }

    fn cancellation(&self) -> io::Result<()> {
        if self.caller.is_cancelled() || self.owner.is_cancelled() {
            Err(cancelled())
        } else {
            Ok(())
        }
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        self.cancellation()?;
        if Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "SDK operation total exhausted",
            ));
        }
        Ok(())
    }

    pub(crate) fn guard<S>(&self, stream: S) -> OperationIo<'_, S> {
        OperationIo {
            context: self,
            inner: stream,
        }
    }

    /// Keep acquired errors ahead of cancellation/expiry in the same poll.
    /// The future and any late success are dropped before this returns.
    pub(crate) async fn run<T, E>(&self, future: impl Future<Output = Result<T, E>>) -> Result<T, E>
    where
        E: From<io::Error>,
    {
        if self.cancellation().is_err() {
            return Err(E::from(
                self.take_native_failure().unwrap_or_else(cancelled),
            ));
        }
        let mut future = Some(Box::pin(future));
        let result = {
            let fenced = poll_fn(|cx| {
                if self.check().is_err() || future.is_none() {
                    return Poll::Pending;
                }
                let result = future
                    .as_mut()
                    .expect("checked operation future")
                    .as_mut()
                    .poll(cx);
                match result {
                    // A completed native or protocol error acquired in this
                    // poll wins over a cancellation or deadline in that poll.
                    Poll::Ready(Err(error)) => {
                        self.discard_handled_native_error();
                        Poll::Ready(Err(error))
                    }
                    Poll::Ready(Ok(value)) if self.check().is_err() => {
                        self.discard_handled_native_error();
                        drop(value);
                        drop(future.take());
                        Poll::Pending
                    }
                    Poll::Ready(Ok(value)) => {
                        self.discard_handled_native_error();
                        Poll::Ready(Ok(value))
                    }
                    result => result,
                }
            });
            let work = async {
                tokio::select! {
                    biased;
                    result = fenced => result,
                    _ = self.caller.cancelled() => Err(E::from(
                        self.take_native_failure().unwrap_or_else(cancelled))),
                    _ = self.owner.cancelled() => Err(E::from(
                        self.take_native_failure().unwrap_or_else(cancelled))),
                }
            };
            match tokio::time::timeout_at(self.deadline, work).await {
                Ok(result) => result,
                Err(error) => {
                    Err(E::from(self.take_native_failure().unwrap_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, error)
                    })))
                }
            }
        };
        drop(future);
        result
    }
}

/// Owns the original stream while borrowing its operation scope.
/// No into_inner, get_mut or Deref permits bypassing the fences.
pub(crate) struct OperationIo<'ctx, S> {
    context: &'ctx OperationContext,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for OperationIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.context.check().is_err() {
            return Poll::Pending;
        }
        self.context.discard_handled_native_error();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(self.context.retain_native(error))),
            result => result,
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for OperationIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.context.check().is_err() {
            return Poll::Pending;
        }
        self.context.discard_handled_native_error();
        match Pin::new(&mut self.inner).poll_write(cx, bytes) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(self.context.retain_native(error))),
            result => result,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.context.check().is_err() {
            return Poll::Pending;
        }
        self.context.discard_handled_native_error();
        match Pin::new(&mut self.inner).poll_flush(cx) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(self.context.retain_native(error))),
            result => result,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.context.check().is_err() {
            return Poll::Pending;
        }
        self.context.discard_handled_native_error();
        match Pin::new(&mut self.inner).poll_shutdown(cx) {
            Poll::Ready(Err(error)) => Poll::Ready(Err(self.context.retain_native(error))),
            result => result,
        }
    }
}

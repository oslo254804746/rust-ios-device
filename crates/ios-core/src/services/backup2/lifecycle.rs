//! Ownership across framed messages, raw transfers and host work.

use std::{
    error::Error,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{error::Elapsed, Instant},
};

use super::{Mobilebackup2Client, Mobilebackup2Error};

pub(super) struct BackupTransport<S> {
    inner: S,
    pub(super) state: SessionState,
}

pub(super) struct SessionState {
    pub(super) usable: bool,
    pub(super) operation_timeout: Option<Duration>,
    pub(super) progressed: bool,
    pub(super) failed: bool,
    pub(super) terminal_reply: bool,
    cleanup_primary: Option<Mobilebackup2Error>,
}

impl<S> BackupTransport<S> {
    pub(super) fn new(inner: S) -> Self {
        Self {
            inner,
            state: SessionState {
                usable: true,
                operation_timeout: None,
                progressed: false,
                failed: false,
                terminal_reply: false,
                cleanup_primary: None,
            },
        }
    }
}

fn invalid_transport() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotConnected,
        "MobileBackup2 session is unusable; reconnect before another operation",
    )
}

impl<S: AsyncRead + Unpin> AsyncRead for BackupTransport<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.state.usable {
            return Poll::Ready(Err(invalid_transport()));
        }
        let before = buf.filled().len();
        let requested = buf.remaining() > 0;
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.state.progressed |= buf.filled().len() > before;
        self.state.failed |= matches!(result, Poll::Ready(Err(_)))
            || (requested && matches!(result, Poll::Ready(Ok(()))) && buf.filled().len() == before);
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BackupTransport<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if !self.state.usable {
            return Poll::Ready(Err(invalid_transport()));
        }
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        self.state.progressed |= matches!(result, Poll::Ready(Ok(n)) if n > 0);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)))
            || (!bytes.is_empty() && matches!(result, Poll::Ready(Ok(0))));
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if !self.state.usable {
            return Poll::Ready(Err(invalid_transport()));
        }
        let result = Pin::new(&mut self.inner).poll_flush(cx);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let result = Pin::new(&mut self.inner).poll_shutdown(cx);
        self.state.failed |= matches!(result, Poll::Ready(Err(_)));
        result
    }
}

pub(super) struct SessionOperation<'a, S> {
    pub(super) client: &'a mut Mobilebackup2Client<S>,
    pub(super) deadline: Option<Instant>,
    completed: bool,
}

impl<S> Mobilebackup2Client<S> {
    pub(super) fn operation_timeout(&self) -> Option<Duration> {
        self.device_link.stream_ref().state.operation_timeout
    }

    pub(super) fn invalidate(&mut self) {
        self.device_link.stream_mut().state.usable = false;
        self.reported_free_space = None;
        self.required_free_space = None;
        self.backup_filter = None;
        self.discarded_files.clear();
        self.device_link.stream_mut().state.cleanup_primary = None;
    }

    pub(super) fn reject_incomplete_failure(&mut self) {
        let state = &self.device_link.stream_ref().state;
        if state.failed || (state.progressed && !state.terminal_reply) {
            // The enclosing body may still need the discarded-file list for
            // local cleanup. Release that state at finish/caller cancellation.
            self.device_link.stream_mut().state.usable = false;
        }
    }

    pub(super) fn begin_operation(
        &mut self,
        budget: Option<Duration>,
    ) -> Result<SessionOperation<'_, S>, Mobilebackup2Error> {
        if !self.is_usable() {
            return Err(Mobilebackup2Error::Protocol(
                "MobileBackup2 session is unusable; reconnect before another operation".into(),
            ));
        }
        let deadline = match budget {
            Some(budget) if budget.is_zero() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "MobileBackup2 operation budget is zero",
                )
                .into());
            }
            Some(budget) => Some(Instant::now().checked_add(budget).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "MobileBackup2 operation deadline is too large",
                )
            })?),
            None => None,
        };
        let state = &mut self.device_link.stream_mut().state;
        state.progressed = false;
        state.failed = false;
        state.terminal_reply = false;
        state.cleanup_primary = None;
        Ok(SessionOperation {
            client: self,
            deadline,
            completed: false,
        })
    }

    pub(super) fn retain_cleanup_result<T>(
        &mut self,
        result: Result<T, Mobilebackup2Error>,
    ) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(primary) => {
                self.device_link.stream_mut().state.cleanup_primary = Some(primary);
                None
            }
        }
    }

    pub(super) fn recover_cleanup_result<T>(
        &mut self,
        value: Option<T>,
    ) -> Result<T, Mobilebackup2Error> {
        match value {
            Some(value) => Ok(value),
            None => Err(self
                .device_link
                .stream_mut()
                .state
                .cleanup_primary
                .take()
                .expect("cleanup retains its operation failure")),
        }
    }

    pub(super) async fn finish_host_cleanup<T>(
        &mut self,
        result: Result<T, Mobilebackup2Error>,
        cleanup: impl std::future::Future<Output = Result<(), Mobilebackup2Error>>,
    ) -> Result<T, Mobilebackup2Error> {
        let value = self.retain_cleanup_result(result);
        let cleanup = Box::pin(cleanup).await;
        finish_results(self.recover_cleanup_result(value), cleanup)
    }
}

impl<S> SessionOperation<'_, S> {
    pub(super) fn finish<T>(
        mut self,
        result: Result<T, Mobilebackup2Error>,
    ) -> Result<T, Mobilebackup2Error> {
        self.completed = true;
        if result.is_err() {
            self.client.reject_incomplete_failure();
        }
        if !self.client.is_usable() {
            self.client.invalidate();
        }
        result
    }

    pub(super) fn finish_timed<T>(
        self,
        result: Result<Result<T, Mobilebackup2Error>, Elapsed>,
    ) -> Result<T, Mobilebackup2Error> {
        match result {
            Ok(result) => self.finish(result),
            Err(elapsed) => {
                // Leave the guard incomplete even if a terminal reply arrived:
                // cancellation during later cleanup/host work still owns it.
                let timeout = std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into();
                match self
                    .client
                    .device_link
                    .stream_mut()
                    .state
                    .cleanup_primary
                    .take()
                {
                    Some(primary) => finish_results(Err(primary), Err(timeout)),
                    None => Err(timeout),
                }
            }
        }
    }
}

impl<S> Drop for SessionOperation<'_, S> {
    fn drop(&mut self) {
        let state = &self.client.device_link.stream_ref().state;
        if !self.completed
            && (state.progressed || state.failed || !self.client.device_link.is_usable())
        {
            self.client.invalidate();
        }
    }
}

pub(super) fn await_operation<T>(
    deadline: Option<Instant>,
    operation: impl std::future::Future<Output = Result<T, Mobilebackup2Error>>,
) -> impl std::future::Future<Output = Result<Result<T, Mobilebackup2Error>, Elapsed>> {
    let operation = Box::pin(operation);
    async move {
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, operation).await,
            None => Ok(operation.await),
        }
    }
}

/// Original operation and cleanup failures, retained without flattening.
/// Classification and the standard source chain follow the operation failure;
/// the separate cleanup branch is available through [`Self::cleanup_error`].
pub struct SessionCleanupFailure {
    operation: Mobilebackup2Error,
    cleanup: Mobilebackup2Error,
}

impl SessionCleanupFailure {
    pub fn operation_error(&self) -> &Mobilebackup2Error {
        &self.operation
    }

    pub fn cleanup_error(&self) -> &Mobilebackup2Error {
        &self.cleanup
    }
}

impl std::fmt::Display for SessionCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MobileBackup2 operation failed; cleanup also failed")
    }
}

impl std::fmt::Debug for SessionCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCleanupFailure")
            .field("operation_class", &crate::classify_error(&self.operation))
            .field("cleanup_class", &crate::classify_error(&self.cleanup))
            .finish_non_exhaustive()
    }
}

impl Error for SessionCleanupFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.operation)
    }
}

impl Mobilebackup2Error {
    /// Inspect at most 32 typed sources, including direct custom IO payloads.
    pub fn cleanup_failure(&self) -> Option<&SessionCleanupFailure> {
        let mut next: Option<&(dyn Error + 'static)> = Some(self);
        for _ in 0..32 {
            let Some(error) = next else { break };
            if let Some(failure) = error.downcast_ref::<SessionCleanupFailure>() {
                return Some(failure);
            }
            next = error
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::get_ref)
                .map(|source| source as &(dyn Error + 'static))
                .or_else(|| error.source());
        }
        None
    }
}

pub(super) fn finish_results<T>(
    operation: Result<T, Mobilebackup2Error>,
    cleanup: Result<(), Mobilebackup2Error>,
) -> Result<T, Mobilebackup2Error> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(operation), Ok(())) => Err(operation),
        (Err(operation), Err(cleanup)) => Err(Mobilebackup2Error::Io(std::io::Error::other(
            SessionCleanupFailure { operation, cleanup },
        ))),
    }
}

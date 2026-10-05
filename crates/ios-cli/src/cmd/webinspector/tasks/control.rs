//! Checked frontend totals and directly owned signal/cleanup futures.
use std::{future::Future, time::Duration};

use anyhow::{Context, Result};
use ios_core::webinspector::WebInspectorError;
use tokio::time::Instant;

#[derive(Clone, Copy)]
pub(in crate::cmd::webinspector) struct Budget {
    pub(super) deadline: Instant,
    pub(super) duration: Duration,
    phase: &'static str,
}

impl Budget {
    pub(in crate::cmd::webinspector) fn new(
        duration: Duration,
        phase: &'static str,
    ) -> Result<Self> {
        if duration.is_zero() {
            return Err(anyhow::Error::new(WebInspectorError::Timeout(duration))
                .context(format!("webinspector {phase} timed out after {duration:?}")));
        }
        Ok(Self {
            deadline: super::webinspector_deadline(duration)?,
            duration,
            phase,
        })
    }

    pub(in crate::cmd::webinspector) fn remaining(self) -> Result<Duration> {
        super::remaining_webinspector_time(self.deadline, self.duration)
    }

    pub(in crate::cmd::webinspector) fn cap(
        self,
        client: &mut ios_core::webinspector::WebInspectorClient<ios_core::device::ServiceStream>,
    ) -> Result<Duration> {
        let remaining = self.remaining()?;
        client.set_operation_timeout(Some(remaining));
        Ok(remaining)
    }

    pub(in crate::cmd::webinspector) async fn drive<T>(
        self,
        operation: impl Future<Output = Result<T>>,
        signal: impl Future<Output = std::io::Result<()>>,
    ) -> Result<T> {
        let mut operation = Box::pin(operation);
        tokio::pin!(signal);
        let expiry = tokio::time::timeout_at(self.deadline, std::future::pending::<()>());
        tokio::pin!(expiry);
        let guarded = std::future::poll_fn(|cx| {
            // A native timer may need another poll after its nominal deadline.
            // Suppress ready work in that gap; retain its actual Elapsed.
            if Instant::now() >= self.deadline {
                std::task::Poll::Pending
            } else {
                let result = operation.as_mut().poll(cx);
                if matches!(&result, std::task::Poll::Ready(Ok(_)))
                    && Instant::now() >= self.deadline
                {
                    // A single cooperative poll may cross the total. Do not
                    // accept its late success or poll a completed future again.
                    // Existing native failures remain primary.
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                } else {
                    result
                }
            }
        });
        tokio::select! {
            biased;
            signal = &mut signal => {
                signal.context("failed waiting for Ctrl+C")?;
                Err(anyhow::Error::new(ios_core::OperationCancelled::new())
                    .context(format!("webinspector {} cancelled", self.phase)))
            }
            result = &mut expiry => {
                let error = result.expect_err("pending operation cannot complete");
                Err(anyhow::Error::new(error).context(format!(
                    "webinspector {} timed out after {:?}", self.phase, self.duration)))
            }
            result = guarded => result,
        }
    }
}

/// Preserve the existing positive minimum; finite command zero is admitted as
/// an actual zero budget so it rejects before default-device discovery.
pub(super) fn finite_duration(seconds: f64) -> Result<Duration> {
    let duration = super::super::workflows::duration_from_secs(seconds)?;
    Ok(if seconds == 0.0 {
        Duration::ZERO
    } else {
        duration
    })
}

pub(super) fn finish<T>(primary: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (primary, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(CleanupFailure { primary, cleanup }.into()),
    }
}

pub(super) struct CleanupFailure {
    pub(super) primary: anyhow::Error,
    pub(super) cleanup: anyhow::Error,
}

impl std::fmt::Display for CleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("webinspector operation failed; cleanup also failed")
    }
}
impl std::fmt::Debug for CleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebInspectorCleanupFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field(
                "cleanup_class",
                &ios_core::classify_error(self.cleanup.as_ref()),
            )
            .finish()
    }
}
impl std::error::Error for CleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}

#[cfg(test)]
mod tests;

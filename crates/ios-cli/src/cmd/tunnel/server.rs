//! The parent owns the scanner; returned shutdown gets an independent budget.
use std::future::{Future, IntoFuture};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use tokio::net::TcpListener;

use super::TunnelAgentState;

pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

struct Owner(TunnelAgentState);

impl Drop for Owner {
    fn drop(&mut self) {
        // Caller Drop cannot await. Signal HTTP shutdown and cancel pending
        // publication; release unlocked tables without spawning a cleanup task.
        self.0.notify_shutdown();
        self.0.inner.lifecycle.clear();
        if let Ok(mut tunnels) = self.0.inner.tunnels.try_write() {
            tunnels.clear();
        }
        if let Ok(mut failed) = self.0.inner.failed_attempts.try_write() {
            failed.clear();
        }
    }
}

pub(super) async fn serve(
    listener: TcpListener,
    app: Router,
    state: TunnelAgentState,
    scan_interval: Duration,
    cleanup_timeout: Duration,
    signal: impl Future<Output = std::io::Result<()>>,
) -> Result<()> {
    let _owner = Owner(state.clone());
    let shutdown_state = state.clone();
    let mut server = Box::pin(
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown_state.wait_shutdown().await })
            .into_future(),
    );
    let monitor_state = state.clone();
    let mut monitor = Box::pin(async move {
        let mut interval = tokio::time::interval(scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = monitor_state.refresh_once().await {
                tracing::warn!(error = %crate::output::error_message(&error), "tunnel agent refresh failed");
            }
        }
    });
    tokio::pin!(signal);
    let mut server_finished = false;
    let primary = tokio::select! {
        biased;
        _ = state.wait_shutdown() => Ok(()),
        result = &mut signal => result.context("waiting for tunnel server Ctrl-C"),
        result = &mut server => {
            server_finished = true;
            result.context("serving tunnel manager")
        }
        _ = &mut monitor => Ok(()),
    };
    state.notify_shutdown();
    // Box owns the actual scanner future, including every active candidate.
    // Dropping a Pin<&mut _> alone would not release the underlying future.
    drop(monitor);
    let cleanup = tokio::time::timeout(cleanup_timeout, async {
        state.clear().await;
        if !server_finished {
            server.await.context("draining tunnel manager HTTP")?;
        }
        Ok(())
    })
    .await
    .context("tunnel manager cleanup deadline expired")
    .and_then(|result| result);
    match (primary, cleanup) {
        (Ok(()), cleanup) => cleanup,
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
        f.write_str("tunnel manager failed; cleanup also failed")
    }
}

impl std::fmt::Debug for CleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelServerCleanupFailure")
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

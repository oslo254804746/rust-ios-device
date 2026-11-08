//! The parent owns the scanner; returned shutdown gets an independent budget.
use std::error::Error;
use std::future::{Future, IntoFuture};
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use tokio::net::TcpListener;

use super::marker_identity::Cutover;
use super::retirement::{CleanupClock, Registry, Snapshot};
use super::TunnelAgentState;

pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

struct Owner {
    state: TunnelAgentState,
    shutdown: Cutover<1>,
    clear: Cutover<1>,
}

impl Drop for Owner {
    fn drop(&mut self) {
        // Caller Drop cannot await. Signal HTTP shutdown and cancel pending
        // publication; release unlocked tables without spawning a cleanup task.
        self.state.notify_shutdown(&mut self.shutdown);
        self.state.inner.lifecycle.clear(&mut self.clear);
        if let Ok(mut tunnels) = self.state.inner.tunnels.try_write() {
            let removed = tunnels.take_all();
            drop(tunnels);
            drop(removed);
        }
        if let Ok(mut failed) = self.state.inner.failed_attempts.try_write() {
            let removed = failed.take_all();
            drop(failed);
            drop(removed);
        }
    }
}

/// Prepare the complete original server future before its Source work. The
/// returned future starts lifecycle ownership only when it is actually polled.
pub(super) fn prepare<S: Future<Output = std::io::Result<()>>>(
    listener: TcpListener,
    app: Router,
    state: TunnelAgentState,
    scan_interval: Duration,
    cleanup_timeout: Duration,
    signal: S,
) -> Result<impl Future<Output = Result<()>>> {
    let bank = state.inner.global_metadata_bytes.clone();
    let backing = super::metadata_vec::exact_backing(&bank, 1)?;
    let cutovers = state.prepare_cutover::<4>()?.into_individual();
    Ok(super::future_storage::PaidFuture::from_backing(
        serve_admitted(
            listener,
            app,
            state,
            scan_interval,
            cleanup_timeout,
            signal,
            cutovers,
        ),
        backing,
    ))
}

#[cfg(test)]
pub(super) fn frame_bytes<S: Future<Output = std::io::Result<()>>>(_signal: &S) -> usize {
    fn size<S, F: Future>(
        _create: impl FnOnce(
            TcpListener,
            Router,
            TunnelAgentState,
            Duration,
            Duration,
            S,
            [Cutover<1>; 4],
        ) -> F,
    ) -> usize {
        std::mem::size_of::<F>()
    }
    size(serve_admitted::<S>)
}

async fn serve_admitted<S: Future<Output = std::io::Result<()>>>(
    listener: TcpListener,
    app: Router,
    state: TunnelAgentState,
    scan_interval: Duration,
    cleanup_timeout: Duration,
    signal: S,
    cutovers: [Cutover<1>; 4],
) -> Result<()> {
    let [mut shutdown, mut clear, owner_shutdown, owner_clear] = cutovers;
    let _owner = Owner {
        state: state.clone(),
        shutdown: owner_shutdown,
        clear: owner_clear,
    };
    let shutdown_state = state.clone();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown_state.wait_shutdown().await })
        .into_future();
    tokio::pin!(server);
    let monitor_state = state.clone();
    let monitor = Some(async move {
        let mut interval = tokio::time::interval(scan_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = monitor_state.drain_native(CLEANUP_TIMEOUT).await {
                tracing::warn!(error = %crate::output::error_message(&error), "native tunnel retirement failed");
            }
            if let Err(error) = monitor_state.refresh_once().await {
                tracing::warn!(error = %crate::output::error_message(&error), "tunnel agent refresh failed");
            }
            if tracing::enabled!(tracing::Level::DEBUG) {
                let payload = monitor_state.retained_error_payloads();
                tracing::debug!(
                    known_payload_capacity_bytes = payload.known_payload_capacity_bytes,
                    opaque_nodes = payload.opaque_nodes,
                    visited_nodes = payload.visited_nodes,
                    active_scans = payload.active_scans,
                    unjoined_native_owners = payload.unjoined_native_owners,
                    truncated = payload.truncated,
                    fully_measured_payloads = payload.fully_measured_payloads(),
                    "sampled manager retained error payloads; not total heap or admission budget"
                );
            }
        }
    });
    tokio::pin!(monitor);
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
        _ = async { monitor.as_mut().as_pin_mut().expect("live scanner").await } => Ok(()),
    };
    let clock = CleanupClock::new(cleanup_timeout);
    state.notify_shutdown(&mut shutdown);
    // Drop the actual inline scanner, including all active candidates, in
    // place before reading history. Dropping only its Pin reference is not enough.
    monitor.set(None);
    let scan_history = state.inner.scan_outcomes.history();
    let cleanup = match clock {
        Err(error) => Err(error),
        Ok(clock) => {
            let phases = StdMutex::new(CleanupPhases::default());
            let native_phases = &phases;
            let metadata_phases = &phases;
            let work = async {
                tokio::join!(
                    async {
                        let result = state.inner.native.drain_retired(clock).await;
                        let mut acquired = native_phases.lock().unwrap_or_else(|p| p.into_inner());
                        acquired.native_complete = true;
                        match result {
                            Ok(report) => acquired.native_selected = report.selected,
                            Err(error) => acquired.native = Some(error),
                        }
                    },
                    async {
                        state.clear(&mut clear).await;
                        let result = if server_finished {
                            Ok(())
                        } else {
                            server.await.context("draining tunnel manager HTTP")
                        };
                        let mut acquired =
                            metadata_phases.lock().unwrap_or_else(|p| p.into_inner());
                        acquired.metadata_http_complete = true;
                        if let Err(error) = result {
                            acquired.metadata_http = Some(error);
                        }
                    },
                );
            };
            let elapsed = clock.run(work).await.err();
            state.inner.native.capture_retired_outcomes();
            let acquired = phases.into_inner().unwrap_or_else(|p| p.into_inner());
            let snapshot = state.inner.native.snapshot();
            let history = state.inner.native.historical_failure();
            if elapsed.is_none()
                && acquired.native.is_none()
                && acquired.metadata_http.is_none()
                && history.is_none()
                && scan_history.is_none()
                && acquired.native_complete
                && acquired.metadata_http_complete
                && snapshot.closed_and_empty()
            {
                Ok(())
            } else if acquired.native_complete
                && acquired.native_selected == 0
                && acquired.native.is_none()
                && acquired.metadata_http.is_none()
                && history.is_none()
                && scan_history.is_none()
                && snapshot.closed_and_empty()
                && elapsed.is_some()
            {
                // Preserve the existing metadata-only server timeout contract.
                // An attempt that selected actual native owners instead carries
                // its ownership and phase evidence in TerminalCleanupFailure.
                Err(elapsed.expect("actual cleanup timer").into())
            } else {
                Err(TerminalCleanupFailure {
                    native: acquired.native,
                    history,
                    scan_history,
                    metadata_http: acquired.metadata_http,
                    elapsed,
                    snapshot,
                    native_complete: acquired.native_complete,
                    metadata_http_complete: acquired.metadata_http_complete,
                }
                .into())
            }
        }
    };
    match (primary, cleanup) {
        (Ok(()), cleanup) => cleanup,
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(CleanupFailure { primary, cleanup }.into()),
    }
}

#[derive(Default)]
struct CleanupPhases {
    native: Option<super::retirement::DrainFailure>,
    metadata_http: Option<anyhow::Error>,
    native_complete: bool,
    native_selected: usize,
    metadata_http_complete: bool,
}

pub(super) struct TerminalCleanupFailure {
    native: Option<super::retirement::DrainFailure>,
    history: Option<super::retirement::HistoryFailure>,
    // The prepaid root owns the array. The separate terminal wrapper/anyhow
    // private body remains outside exact snapshot-body accounting.
    scan_history: Option<super::scan::HistoryOwner>,
    metadata_http: Option<anyhow::Error>,
    elapsed: Option<tokio::time::error::Elapsed>,
    snapshot: Snapshot,
    native_complete: bool,
    metadata_http_complete: bool,
}

impl TerminalCleanupFailure {
    pub(super) fn primary(&self) -> &(dyn Error + 'static) {
        // A non-expiry native history survives a previous monitor drain. Fresh
        // phase IO takes precedence over a purely historical cleanup expiry.
        if let Some(history) = &self.history {
            if !super::retirement::shutdown_primary_is_elapsed(history.primary()) {
                return history;
            }
        }
        if let Some(native) = &self.native {
            if !native.primary_is_elapsed() {
                return native;
            }
        }
        if let Some(metadata) = &self.metadata_http {
            return metadata.as_ref();
        }
        if let Some(scan) = self
            .scan_history
            .as_ref()
            .and_then(|history| history.first())
        {
            return scan.as_ref();
        }
        if let Some(elapsed) = &self.elapsed {
            return elapsed;
        }
        if let Some(native) = &self.native {
            return native;
        }
        if let Some(history) = &self.history {
            return history;
        }
        self
    }

    pub(super) fn native_snapshot(&self) -> Snapshot {
        self.snapshot
    }

    pub(super) fn abandoned_scans(&self) -> Option<&super::scan::HistoryOwner> {
        self.scan_history.as_ref()
    }
}

impl std::fmt::Display for TerminalCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel manager cleanup failed")
    }
}

impl std::fmt::Debug for TerminalCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelTerminalCleanupFailure")
            .field(
                "native_class",
                &self
                    .native
                    .as_ref()
                    .map(|cause| ios_core::classify_error(cause)),
            )
            .field(
                "history_class",
                &self
                    .history
                    .as_ref()
                    .map(|cause| ios_core::classify_error(cause)),
            )
            .field(
                "metadata_http_class",
                &self
                    .metadata_http
                    .as_ref()
                    .map(|cause| ios_core::classify_error(cause.as_ref())),
            )
            .field("elapsed", &self.elapsed.is_some())
            .field(
                "abandoned_scans",
                &self.abandoned_scans().map_or(0, |root| root.len()),
            )
            .field("native_complete", &self.native_complete)
            .field("metadata_http_complete", &self.metadata_http_complete)
            .field("native_ownership", &self.native_snapshot())
            .finish()
    }
}

impl Error for TerminalCleanupFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if self.native.is_none()
            && self.history.is_none()
            && self.scan_history.is_none()
            && self.metadata_http.is_none()
            && self.elapsed.is_none()
        {
            None
        } else {
            Some(self.primary())
        }
    }
}

pub(super) async fn drain_native_terminal(native: &Registry, clock: CleanupClock) -> Result<()> {
    let result = native.drain_retired(clock).await;
    native.capture_retired_outcomes();
    let failure = result.err();
    let history = native.historical_failure();
    let snapshot = native.snapshot();
    if failure.is_none() && history.is_none() && snapshot.closed_and_empty() {
        Ok(())
    } else {
        Err(TerminalCleanupFailure {
            native: failure,
            history,
            scan_history: None,
            metadata_http: None,
            elapsed: None,
            snapshot,
            native_complete: true,
            metadata_http_complete: true,
        }
        .into())
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

#[cfg(test)]
mod tests;

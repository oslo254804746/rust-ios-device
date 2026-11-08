//! Store-owned watcher handles survive removal and interrupted explicit joins.
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use tokio::{
    sync::Mutex,
    task::{AbortHandle, JoinHandle},
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum Completion {
    Normal,
    RequestedAbort,
    Unexpected,
}

struct JoinState {
    handle: JoinHandle<Result<()>>,
    completed: Option<Completion>,
    error: Option<anyhow::Error>,
}

pub(super) struct TaskOwner {
    abort: AbortHandle,
    requested_abort: AtomicBool,
    join: Mutex<JoinState>,
}

impl TaskOwner {
    pub(super) fn new(handle: JoinHandle<Result<()>>) -> Self {
        Self {
            abort: handle.abort_handle(),
            requested_abort: AtomicBool::new(false),
            join: Mutex::new(JoinState {
                handle,
                completed: None,
                error: None,
            }),
        }
    }

    pub(super) fn abort(&self) {
        // Abort does not wait for the join mutex. A removal cancelled while
        // waiting for that mutex must still have issued its watcher abort.
        self.requested_abort.store(true, Ordering::SeqCst);
        self.abort.abort();
    }

    pub(super) async fn join(&self) -> Result<()> {
        let mut state = self.join.lock().await;
        self.wait(&mut state).await;
        state.error.take().map_or(Ok(()), Err)
    }

    pub(super) async fn join_if_finished(&self) -> Option<Result<()>> {
        let mut state = self.join.try_lock().ok()?;
        if state.completed.is_none() && !state.handle.is_finished() {
            return None;
        }
        self.wait(&mut state).await;
        Some(state.error.take().map_or(Ok(()), Err))
    }

    async fn wait(&self, state: &mut JoinState) {
        if state.completed.is_some() {
            return;
        }
        let result = (&mut state.handle).await;
        let (completed, error) = match result {
            Ok(result) => (Completion::Normal, result.err()),
            Err(error) if self.requested_abort.load(Ordering::SeqCst) && error.is_cancelled() => {
                (Completion::RequestedAbort, None)
            }
            Err(error) => (
                Completion::Unexpected,
                Some(anyhow::Error::new(error).context("automation watcher join failed")),
            ),
        };
        state.completed = Some(completed);
        state.error = error;
        // No await after recording completion or transferring its native
        // outcome. Repeated joins never poll a completed JoinHandle again.
    }

    #[cfg(test)]
    pub(super) async fn completion(&self) -> Option<Completion> {
        self.join.lock().await.completed
    }

    #[cfg(test)]
    pub(super) async fn prevent_join(&self) -> impl Drop + '_ {
        self.join.lock().await
    }

    #[cfg(test)]
    pub(super) fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }
}

impl Drop for TaskOwner {
    fn drop(&mut self) {
        // Store/caller Drop cannot perform an async join; explicit owners do.
        self.abort.abort();
    }
}

pub(super) fn report(result: Result<()>) {
    if let Err(error) = result {
        tracing::warn!(error = %crate::output::error_message(&error), "automation session closed with error");
    }
}

#[cfg(test)]
mod tests;

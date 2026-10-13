//! Child results survive bounded supervisor abort; every explicit stop joins.
use super::super::tasks::control::{self, Budget};
#[cfg(test)]
use super::CDP_PAGE_CLEANUP_TIMEOUT;
use anyhow::Result;
use futures_util::{stream::FuturesUnordered, StreamExt};
use serde_json::Value as JsonValue;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch};

#[derive(Default)]
pub(super) struct PageOutcome {
    primary: Option<anyhow::Error>,
    cleanup: Option<Result<()>>,
    join: Option<std::result::Result<(), tokio::task::JoinError>>,
    requested_abort: bool,
    discovery_closed: bool,
    final_message: Option<JsonValue>,
    context_candidate: Option<JsonValue>,
    terminal: bool,
}

impl std::fmt::Debug for PageOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpPageOutcome")
            .field("joined", &self.join.is_some())
            .field("requested_abort", &self.requested_abort)
            .field(
                "primary_class",
                &self
                    .primary
                    .as_ref()
                    .map(|error| ios_core::classify_error(error.as_ref())),
            )
            .field(
                "cleanup_class",
                &self
                    .cleanup
                    .as_ref()
                    .and_then(|result| result.as_ref().err())
                    .map(|error| ios_core::classify_error(error.as_ref())),
            )
            .finish()
    }
}

impl PageOutcome {
    pub(super) fn terminal(&mut self) {
        self.terminal = true;
    }
    pub(super) fn is_terminal(&self) -> bool {
        self.terminal
    }
    pub(super) fn observe_discovery_closed(&mut self) {
        self.discovery_closed = true;
        if self.final_message.is_none() {
            self.final_message = self.context_candidate.clone();
        }
    }
    pub(super) fn context_candidate(&mut self, message: Option<JsonValue>) {
        self.context_candidate = message;
        if self.discovery_closed && self.final_message.is_none() {
            self.final_message = self.context_candidate.clone();
        }
    }
    pub(super) fn final_message(&mut self, message: Option<JsonValue>) {
        if self.final_message.is_none() {
            self.final_message = message;
        }
    }
    pub(super) fn take_final_message(&mut self) -> Option<JsonValue> {
        self.final_message.take()
    }
    pub(super) fn primary(&mut self, result: Result<()>) {
        self.primary = result.err();
    }
    pub(super) fn cleanup(&mut self, result: Result<()>) {
        self.cleanup = Some(control::finish(
            self.cleanup.take().unwrap_or(Ok(())),
            result,
        ));
    }
    pub(super) fn is_joined(&self) -> bool {
        self.join.is_some()
    }
    #[cfg(test)]
    pub(super) fn is_aborted(&self) -> bool {
        self.requested_abort
    }
    fn take(&mut self) -> Result<()> {
        let join = self.join.take();
        let completed = matches!(&join, Some(Ok(())));
        let cleanup = self.cleanup.take().unwrap_or_else(|| {
            if completed {
                Err(anyhow::Error::new(
                    ios_core::webinspector::WebInspectorError::Protocol(
                        "CDP child exited without its cleanup outcome".into(),
                    ),
                ))
            } else {
                Ok(())
            }
        });
        let result = control::finish(self.primary.take().map_or(Ok(()), Err), cleanup);
        let joined = match join {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) if self.requested_abort && error.is_cancelled() => Ok(()),
            Some(Err(error)) => {
                Err(anyhow::Error::new(error).context("CDP child task join failed"))
            }
            None => Err(anyhow::Error::new(
                ios_core::webinspector::WebInspectorError::Protocol(
                    "CDP child is missing its join outcome".into(),
                ),
            )),
        };
        control::finish(result, joined)
    }
}

pub(super) type Outcome = Arc<Mutex<PageOutcome>>;

pub(super) fn outcome() -> Outcome {
    Arc::new(Mutex::new(PageOutcome::default()))
}

#[derive(Debug)]
pub(super) struct BrowserPageSession {
    pub(super) sender: mpsc::Sender<JsonValue>,
    pub(super) cancel: watch::Sender<bool>,
    pub(super) task: tokio::task::JoinHandle<()>,
    pub(super) outcome: Outcome,
}

impl Drop for BrowserPageSession {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        // Async joins belong to explicit owners; Drop is a fallback abort.
        self.task.abort();
    }
}

#[cfg(test)]
pub(super) async fn stop_browser_page(page: BrowserPageSession) -> Result<()> {
    stop_browser_pages(vec![page], CDP_PAGE_CLEANUP_TIMEOUT).await
}

pub(super) async fn stop_browser_pages(
    pages: Vec<BrowserPageSession>,
    duration: Duration,
) -> Result<()> {
    BrowserPageCleanup::new(pages, duration).stop().await
}

/// Borrowing stop can be cancelled without losing owners, completed joins or
/// an already issued deadline error. Explicit parent cleanup can then resume.
pub(super) struct BrowserPageCleanup {
    pages: Vec<BrowserPageSession>,
    duration: Duration,
    budget: Option<Budget>,
    waited: Option<Result<()>>,
}

impl BrowserPageCleanup {
    pub(super) fn new(pages: Vec<BrowserPageSession>, duration: Duration) -> Self {
        Self {
            pages,
            duration,
            budget: None,
            waited: None,
        }
    }

    pub(super) fn push_with_budget(&mut self, page: BrowserPageSession, budget: Budget) {
        let _ = page.cancel.send(true);
        if self.budget.is_none() && self.waited.is_none() {
            self.budget = Some(budget);
        }
        self.pages.push(page);
    }

    #[cfg(test)]
    pub(super) fn push(&mut self, page: BrowserPageSession) {
        self.pages.push(page);
    }

    pub(super) fn into_remaining(self) -> (Vec<BrowserPageSession>, Result<()>) {
        (self.pages, self.waited.unwrap_or(Ok(())))
    }

    pub(super) async fn stop(&mut self) -> Result<()> {
        if self.pages.is_empty() {
            return Ok(());
        }
        if self.budget.is_none() && self.waited.is_none() {
            match Budget::new(self.duration, "CDP child cleanup") {
                Ok(budget) => self.budget = Some(budget),
                Err(error) => self.waited = Some(Err(error)),
            }
            for page in &self.pages {
                let _ = page.cancel.send(true);
            }
        }
        if self.waited.is_none() {
            let budget = self.budget.expect("cleanup budget was admitted");
            self.waited = Some(
                budget
                    .drive(
                        Box::pin(async {
                            let mut pending = FuturesUnordered::new();
                            for page in &mut self.pages {
                                if !page
                                    .outcome
                                    .lock()
                                    .unwrap_or_else(|poison| poison.into_inner())
                                    .is_joined()
                                {
                                    pending.push(async move {
                                        let result = (&mut page.task).await;
                                        page.outcome
                                            .lock()
                                            .unwrap_or_else(|poison| poison.into_inner())
                                            .join = Some(result);
                                    });
                                }
                            }
                            while pending.next().await.is_some() {}
                            Ok(())
                        }),
                        std::future::pending::<std::io::Result<()>>(),
                    )
                    .await,
            );
        }
        // All borrowed join futures have been dropped. Record every abort
        // before joining any task, so cancellation here also remains resumable.
        for page in &self.pages {
            let mut outcome = page
                .outcome
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if !outcome.is_joined() {
                outcome.requested_abort = true;
                page.task.abort();
            }
        }
        for page in &mut self.pages {
            let joined = page
                .outcome
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_joined();
            if !joined {
                let result = (&mut page.task).await;
                page.outcome
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .join = Some(result);
            }
        }
        // No await after taking outcomes: all causes transfer to the caller
        // together, rather than being lost during cancellation partway through.
        let mut result = Ok(());
        for page in &self.pages {
            let owned = page
                .outcome
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take();
            result = control::finish(result, owned);
        }
        self.pages.clear();
        self.budget = None;
        control::finish(result, self.waited.take().unwrap_or(Ok(())))
    }
}

pub(super) fn report_cleanup(result: Result<()>) {
    if let Err(error) = result {
        let _ = crate::output::write_error_line(format_args!(
            "cdp page task closed with error: {}",
            crate::output::error_message(&error)
        ));
    }
}

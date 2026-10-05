//! Child results survive bounded supervisor abort; every explicit stop joins.
use super::super::tasks::control::{self, Budget};
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
}

impl std::fmt::Debug for PageOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpPageOutcome")
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
    pub(super) fn primary(&mut self, result: Result<()>) {
        self.primary = result.err();
    }
    pub(super) fn cleanup(&mut self, result: Result<()>) {
        self.cleanup = Some(result);
    }
    fn take(&mut self, completed: bool) -> Result<()> {
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
        control::finish(self.primary.take().map_or(Ok(()), Err), cleanup)
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

pub(super) async fn stop_browser_page(page: BrowserPageSession) -> Result<()> {
    stop_browser_pages(vec![page], CDP_PAGE_CLEANUP_TIMEOUT).await
}

pub(super) async fn stop_browser_pages(
    mut pages: Vec<BrowserPageSession>,
    duration: Duration,
) -> Result<()> {
    if pages.is_empty() {
        return Ok(());
    }
    let budget = Budget::new(duration, "CDP child cleanup");
    for page in &pages {
        let _ = page.cancel.send(true);
    }
    let mut joined = Vec::with_capacity(pages.len());
    joined.resize_with(pages.len(), || None);
    let waited = match budget {
        Ok(budget) => {
            budget
                .drive(
                    Box::pin(async {
                        let mut pending = FuturesUnordered::new();
                        for (index, page) in pages.iter_mut().enumerate() {
                            pending.push(async move { (index, (&mut page.task).await) });
                        }
                        while let Some((index, result)) = pending.next().await {
                            joined[index] = Some(result);
                        }
                        Ok(())
                    }),
                    std::future::pending::<std::io::Result<()>>(),
                )
                .await
        }
        Err(error) => Err(error),
    };
    // The controller has dropped all borrowed join futures before this pass.
    // Abort every remaining task before joining any one of them.
    for (index, page) in pages.iter().enumerate() {
        if joined[index].is_none() {
            page.task.abort();
        }
    }
    let mut result = Ok(());
    for (index, page) in pages.iter_mut().enumerate() {
        let aborted = joined[index].is_none();
        let join = match joined[index].take() {
            Some(result) => result,
            None => (&mut page.task).await,
        };
        let owned = page
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take(join.is_ok());
        result = control::finish(result, owned);
        if let Err(error) = join {
            if !(aborted && error.is_cancelled()) {
                result = control::finish(
                    result,
                    Err(anyhow::Error::new(error).context("CDP child task join failed")),
                );
            }
        }
    }
    control::finish(result, waited)
}

pub(super) fn report_cleanup(result: Result<()>) {
    if let Err(error) = result {
        let _ = crate::output::write_error_line(format_args!(
            "cdp page task closed with error: {}",
            crate::output::error_message(&error)
        ));
    }
}

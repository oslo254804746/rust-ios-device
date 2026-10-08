//! Bounded failure writes after the scanner's connection window has finished.
use super::{AttemptIdentity, FailedTunnelAttempt, Ticket, TunnelAgentState};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

pub(super) struct DeferredFailure {
    pub(super) identity: AttemptIdentity,
    pub(super) slot: Arc<OwnedSemaphorePermit>,
}

pub(super) struct FailureIntent {
    pub(super) udid: String,
    pub(super) ticket: Ticket,
    pub(super) failed: DeferredFailure,
}

// Context leaves the original primary anyhow object in its downcast/source
// chain. The independent secondary is inspectable without logging device data.
pub(super) struct FailureContext {
    secondary: anyhow::Error,
}

impl FailureContext {
    pub(super) fn new(secondary: anyhow::Error) -> Self {
        Self { secondary }
    }

    #[cfg(test)]
    pub(super) fn secondary(&self) -> &anyhow::Error {
        &self.secondary
    }
}

impl std::fmt::Display for FailureContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel scan failure bookkeeping also failed")
    }
}

impl std::fmt::Debug for FailureContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelScanBookkeepingFailure")
            .field(
                "secondary_class",
                &ios_core::classify_error(self.secondary.as_ref()),
            )
            .finish()
    }
}

impl TunnelAgentState {
    pub(super) async fn flush_failure_intents(&self, intents: Vec<FailureIntent>) -> Result<()> {
        if intents.is_empty() {
            return Ok(());
        }
        let deadline = super::lifecycle::ConnectionDeadline::for_scan_bookkeeping(
            self.inner.failure_write_timeout,
        )?;
        deadline
            .run(async {
                // Keep displaced rows outside the guards until their original
                // permit/metadata owners can be released without either fence.
                let mut displaced = Vec::new();
                let mut failed = self.inner.failed_attempts.write().await;
                let result = (|| {
                    for intent in intents {
                        if !intent.ticket.is_current() {
                            continue;
                        }
                        #[cfg(test)]
                        self.inner.lifecycle.before_failure_record();
                        let outcome = intent.ticket.commit(|| {
                            deadline.check()?;
                            let previous = failed.get(&intent.udid);
                            let next = FailedTunnelAttempt {
                                last_attempt: Instant::now(),
                                fail_count: previous
                                    .filter(|old| old.identity == intent.failed.identity)
                                    .map_or(1, |old| old.fail_count.saturating_add(1)),
                                identity: intent.failed.identity,
                                slot: previous.map_or_else(
                                    || Some(intent.failed.slot.clone()),
                                    |old| old.slot.clone(),
                                ),
                            };
                            let old = failed.insert(intent.udid.clone(), next);
                            #[cfg(test)]
                            {
                                let callback =
                                    self.inner.failure_after_insert.lock().unwrap().take();
                                if let Some(callback) = callback {
                                    callback();
                                }
                            }
                            if let Err(error) = deadline.check() {
                                // This row is unobserved under both table and
                                // ticket guards. Restore the exact old value.
                                if let Some(late) = failed.remove(&intent.udid) {
                                    displaced.push(late);
                                }
                                if let Some(old) = old {
                                    failed.insert(intent.udid, old);
                                }
                                return Err(error);
                            }
                            if let Some(old) = old {
                                displaced.push(old);
                            }
                            Ok(())
                        });
                        if let Err(error) = outcome {
                            if !error.is::<ios_core::OperationCancelled>() {
                                return Err(error);
                            }
                        }
                    }
                    Ok(())
                })();
                drop(failed);
                drop(displaced);
                result
            })
            .await
    }
}

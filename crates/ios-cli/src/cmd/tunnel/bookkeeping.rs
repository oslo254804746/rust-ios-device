//! Bounded failure writes and contested success cleanup after the scan window.
use super::{AttemptIdentity, FailedTunnelAttempt, Ticket, TunnelAgentState};
use std::collections::HashMap;
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

pub(super) struct DeferredSuccess {
    // This identifies the exact failure row observed before the successful
    // connection, independently of its timestamp, count and shared permit.
    pub(super) version: Arc<()>,
}

pub(super) enum ScanAttemptOutcome {
    NoChange,
    Failure(DeferredFailure),
    Success(DeferredSuccess),
}

pub(super) struct SuccessIntent {
    pub(super) udid: String,
    pub(super) ticket: Ticket,
    pub(super) success: DeferredSuccess,
}

pub(super) enum ScanIntent {
    Failure(FailureIntent),
    Success(SuccessIntent),
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
        formatter.write_str("tunnel scan completion also failed")
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
    #[cfg(test)]
    pub(super) async fn flush_failure_intents(&self, intents: Vec<FailureIntent>) -> Result<()> {
        self.flush_failure_intents_with_total(intents, None).await
    }

    #[cfg(test)]
    pub(super) async fn flush_failure_intents_with_total(
        &self,
        intents: Vec<FailureIntent>,
        whole: Option<&super::scan::Deadline>,
    ) -> Result<()> {
        self.flush_scan_intents_with_total(
            intents.into_iter().map(ScanIntent::Failure).collect(),
            whole,
        )
        .await
    }

    // Call with the failure-table writer held. The old row remains outside the
    // commit's watch guards after success, and a clock crossing restores that
    // exact row before another observer can acquire the table.
    pub(super) fn remove_success_row(
        &self,
        failed: &mut HashMap<String, FailedTunnelAttempt>,
        udid: &str,
        success: &DeferredSuccess,
        ticket: &Ticket,
        deadline: &super::lifecycle::ConnectionDeadline,
    ) -> Result<Option<FailedTunnelAttempt>> {
        if !failed
            .get(udid)
            .is_some_and(|old| Arc::ptr_eq(&old.version, &success.version))
        {
            return Ok(None);
        }
        #[cfg(test)]
        {
            let callback = self.inner.success_before_remove.lock().unwrap().take();
            if let Some(callback) = callback {
                callback();
            }
        }
        ticket.commit(|| {
            deadline.check()?;
            let old = failed.remove_entry(udid);
            #[cfg(test)]
            {
                let callback = self.inner.success_after_remove.lock().unwrap().take();
                if let Some(callback) = callback {
                    callback();
                }
            }
            if let Err(error) = deadline.check() {
                if let Some((key, old)) = old {
                    failed.insert(key, old);
                }
                return Err(error);
            }
            Ok(old.map(|(_, old)| old))
        })
    }

    pub(super) async fn flush_scan_intents_with_total(
        &self,
        intents: Vec<ScanIntent>,
        whole: Option<&super::scan::Deadline>,
    ) -> Result<()> {
        if intents.is_empty() {
            return Ok(());
        }
        let deadline = super::lifecycle::ConnectionDeadline::for_scan_bookkeeping(
            self.inner.failure_write_timeout,
        )?
        .within_scan(whole);
        deadline
            .run(async {
                // Keep displaced rows outside the guards until their original
                // permit/metadata owners can be released without either fence.
                let mut displaced = Vec::new();
                let mut failed = self.inner.failed_attempts.write().await;
                let result = (|| {
                    for intent in intents {
                        let outcome = match intent {
                            ScanIntent::Success(intent) => {
                                if !intent.ticket.is_current() {
                                    continue;
                                }
                                self.remove_success_row(
                                    &mut failed,
                                    &intent.udid,
                                    &intent.success,
                                    &intent.ticket,
                                    &deadline,
                                )
                                .map(|old| {
                                    if let Some(old) = old {
                                        displaced.push(old);
                                    }
                                })
                            }
                            ScanIntent::Failure(intent) => {
                                if !intent.ticket.is_current() {
                                    continue;
                                }
                                #[cfg(test)]
                                self.inner.lifecycle.before_failure_record();
                                intent.ticket.commit(|| {
                                    deadline.check()?;
                                    let previous = failed.get(&intent.udid);
                                    let next = FailedTunnelAttempt {
                                        version: Arc::new(()),
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
                                })
                            }
                        };
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

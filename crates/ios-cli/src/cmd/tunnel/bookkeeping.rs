//! Bounded failure writes and contested success cleanup after the scan window.
use super::{AttemptIdentity, FailedTunnelAttempt, Ticket, TunnelAgentState};
use std::collections::HashMap;
use std::sync::Arc;

use super::metadata_vec::KnownVec;
use anyhow::Result;
use std::sync::Mutex;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::time::Instant;

pub(super) struct DeferredFailure {
    pub(super) identity: AttemptIdentity,
    pub(super) slot: Arc<OwnedSemaphorePermit>,
    pub(super) receipt: Option<Arc<super::reconnect::FailureReceipt>>,
}

pub(super) struct FailureIntent {
    pub(super) udid: String,
    pub(super) ticket: Ticket,
    pub(super) failed: DeferredFailure,
    pub(super) _input: Option<Arc<super::candidate_input::Input>>,
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
    pub(super) _input: Option<Arc<super::candidate_input::Input>>,
    pub(super) _key_storage: Option<OwnedSemaphorePermit>,
}

pub(super) enum ScanIntent {
    Failure(FailureIntent),
    Success(SuccessIntent),
}

// The primary bookkeeping error stays the original anyhow object. The context
// exposes independent Source originals without allocating diagnostic strings.
pub(super) struct SourceFailuresContext {
    receipts: KnownVec<Arc<super::reconnect::FailureReceipt>>,
}
impl SourceFailuresContext {
    pub(super) fn receipts(&self) -> &[Arc<super::reconnect::FailureReceipt>] {
        self.receipts.as_slice()
    }
}
impl std::fmt::Display for SourceFailuresContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("automatic connection originals retained; failure bookkeeping also failed")
    }
}
impl std::fmt::Debug for SourceFailuresContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelSourceFailureCompletion")
            .field("receipts", &self.receipts().len())
            .finish()
    }
}

struct WriteState {
    receipts: KnownVec<Arc<super::reconnect::FailureReceipt>>,
    finished: bool,
    in_scan: bool,
}

pub(super) struct FailureWriteOwner(Mutex<WriteState>);

impl FailureWriteOwner {
    pub(super) fn in_scan(&self) {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).in_scan = true;
    }

    fn finish(&self, result: Result<()>) -> Result<()> {
        let state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.in_scan {
            let marker = result.as_ref().err().is_some_and(|error| {
                error.is::<super::scan::DeadlineReached>()
                    || error.is::<ios_core::OperationCancelled>()
            });
            if !marker {
                let completion = match &result {
                    Ok(()) => super::reconnect::Completion::Abandoned,
                    Err(error) => super::reconnect::Completion::Failed(
                        crate::output::operation_error_class(error.as_ref()),
                    ),
                };
                for receipt in state.receipts.as_slice() {
                    receipt.complete(completion);
                }
            }
            // Keep every root and its admitted backing until Session sees the
            // actual overall result, including discovery/ready failures after
            // an otherwise successful writer and converted whole timers.
            return result;
        }
        drop(state);
        self.finish_scan(result)
    }

    pub(super) fn finish_scan(&self, result: Result<()>) -> Result<()> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.finished {
            return result;
        }
        state.finished = true;
        let completion = match &result {
            Ok(()) => super::reconnect::Completion::Abandoned,
            Err(error) => super::reconnect::Completion::Failed(
                crate::output::operation_error_class(error.as_ref()),
            ),
        };
        for receipt in state.receipts.as_slice() {
            receipt.complete(completion);
        }
        match result {
            Err(error) if state.receipts.len() != 0 => Err(error.context(SourceFailuresContext {
                receipts: state.receipts.take(),
            })),
            result => result,
        }
    }

    pub(super) fn abandon(&self, primary: Option<anyhow::Error>) -> Option<anyhow::Error> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.finished {
            return primary;
        }
        state.finished = true;
        for receipt in state.receipts.as_slice() {
            receipt.complete(super::reconnect::Completion::Abandoned);
        }
        match primary {
            Some(error) if state.receipts.len() != 0 => {
                Some(error.context(SourceFailuresContext {
                    receipts: state.receipts.take(),
                }))
            }
            other => other,
        }
    }
}
impl Drop for FailureWriteOwner {
    fn drop(&mut self) {
        let state = self.0.get_mut().unwrap_or_else(|p| p.into_inner());
        if !state.finished {
            for receipt in state.receipts.as_slice() {
                receipt.complete(super::reconnect::Completion::Abandoned);
            }
        }
    }
}

// All three destination allocations are admitted before any Source request.
// There is at most one outcome/receipt/displaced row per candidate. No later
// byte admission can discard an acquired original after its Ready(Err) poll.
pub(super) struct WriteBatch {
    owner: Arc<FailureWriteOwner>,
    intents: KnownVec<ScanIntent>,
    displaced: KnownVec<FailedTunnelAttempt>,
}

impl WriteBatch {
    pub(super) fn new(bank: &Arc<Semaphore>, capacity: usize) -> Result<Self> {
        anyhow::ensure!(
            capacity <= super::MAX_FAILURE_RECORDS,
            crate::output::resource_decision("tunnel scan write batch limit")
        );
        let mut intents = KnownVec::new(Arc::clone(bank), capacity);
        intents.reserve(capacity)?;
        let mut receipts = KnownVec::new(Arc::clone(bank), capacity);
        receipts.reserve(capacity)?;
        let mut displaced = KnownVec::new(Arc::clone(bank), capacity);
        displaced.reserve(capacity)?;
        Ok(Self {
            owner: Arc::new(FailureWriteOwner(Mutex::new(WriteState {
                receipts,
                finished: false,
                in_scan: false,
            }))),
            intents,
            displaced,
        })
    }

    pub(super) fn record(&mut self, intent: ScanIntent) -> Result<()> {
        if let ScanIntent::Failure(failure) = &intent {
            if let Some(receipt) = &failure.failed.receipt {
                self.owner
                    .0
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .receipts
                    .push(Arc::clone(receipt))?;
            }
        }
        self.intents.push(intent)
    }

    pub(super) fn finish(self, result: Result<()>) -> Result<()> {
        self.owner.finish(result)
    }

    pub(super) fn owner(&self) -> Arc<FailureWriteOwner> {
        Arc::clone(&self.owner)
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(capacity: usize) -> usize {
        capacity
            * (std::mem::size_of::<ScanIntent>()
                + std::mem::size_of::<Arc<super::reconnect::FailureReceipt>>()
                + std::mem::size_of::<FailedTunnelAttempt>())
    }
}

// Test adapters receive already-owned external argument vectors, rather than
// a production Source window. If admission refuses, return those exact roots
// in a safe context without allocating a replacement diagnostic/error list.
#[cfg(test)]
struct ExternalIntentsContext(Vec<ScanIntent>);

#[cfg(test)]
impl std::fmt::Display for ExternalIntentsContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("test-supplied original scan intents returned after admission refusal")
    }
}

#[cfg(test)]
impl std::fmt::Debug for ExternalIntentsContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalScanIntents")
            .field("intents", &self.0.len())
            .finish()
    }
}

// Context leaves the original primary anyhow object in its downcast/source
// chain. The independent secondary is inspectable without logging device data.
pub(super) struct FailureContext {
    secondary: anyhow::Error,
    _storage: Option<OwnedSemaphorePermit>,
}

impl FailureContext {
    #[cfg(test)]
    pub(super) fn new(secondary: anyhow::Error) -> Self {
        Self {
            secondary,
            _storage: None,
        }
    }

    pub(super) fn admitted(secondary: anyhow::Error, storage: OwnedSemaphorePermit) -> Self {
        Self {
            secondary,
            _storage: Some(storage),
        }
    }

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
        let failure_stamp = ticket.failure_stamp();
        ticket.commit(|| {
            deadline.check()?;
            let old = failed.remove_entry(udid);
            ticket.clear_failure_version(&success.version);
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
                ticket.restore_failure_stamp(failure_stamp);
                return Err(error);
            }
            Ok(old.map(|(_, old)| old))
        })
    }

    #[cfg(test)]
    pub(super) async fn flush_scan_intents_with_total(
        &self,
        intents: Vec<ScanIntent>,
        whole: Option<&super::scan::Deadline>,
    ) -> Result<()> {
        let mut batch = match WriteBatch::new(&self.inner.global_metadata_bytes, intents.len()) {
            Ok(batch) => batch,
            Err(error) => return Err(error.context(ExternalIntentsContext(intents))),
        };
        for intent in intents {
            batch.record(intent)?;
        }
        self.flush_write_batch(batch, whole).await
    }

    pub(super) async fn flush_write_batch(
        &self,
        batch: WriteBatch,
        whole: Option<&super::scan::Deadline>,
    ) -> Result<()> {
        let WriteBatch {
            owner,
            intents,
            mut displaced,
        } = batch;
        if intents.len() == 0 {
            return owner.finish(Ok(()));
        }
        let deadline = match super::lifecycle::ConnectionDeadline::for_scan_bookkeeping(
            self.inner.failure_write_timeout,
        ) {
            Ok(deadline) => deadline.within_scan(whole),
            Err(error) => return owner.finish(Err(error)),
        };
        let result = deadline
            .run(async {
                // Keep displaced rows outside the guards until their original
                // permit/metadata owners can be released without either fence.
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
                                        displaced
                                            .push(old)
                                            .expect("admitted displaced slot per intent");
                                    }
                                })
                            }
                            ScanIntent::Failure(intent) => {
                                if !intent.ticket.is_current() {
                                    continue;
                                }
                                #[cfg(test)]
                                self.inner.lifecycle.before_failure_record();
                                let previous_current =
                                    failed.get(&intent.udid).is_some_and(|old| {
                                        intent.ticket.failure_is_current(&old.version)
                                    });
                                intent.ticket.commit(|| {
                                    deadline.check()?;
                                    let failure_stamp = intent.ticket.failure_stamp();
                                    let previous = failed.get(&intent.udid);
                                    let next = FailedTunnelAttempt {
                                        version: Arc::new(()),
                                        last_attempt: Instant::now(),
                                        fail_count: previous
                                            .filter(|old| {
                                                previous_current
                                                    && old.identity == intent.failed.identity
                                            })
                                            .map_or(1, |old| old.fail_count.saturating_add(1)),
                                        identity: intent.failed.identity,
                                        slot: previous.map_or_else(
                                            || Some(intent.failed.slot.clone()),
                                            |old| old.slot.clone(),
                                        ),
                                    };
                                    let version = next.version.clone();
                                    let old = failed.insert(intent.udid.clone(), next);
                                    intent.ticket.remember_failure(Some(version));
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
                                            displaced
                                                .push(late)
                                                .expect("admitted displaced slot per intent");
                                        }
                                        if let Some(old) = old {
                                            failed.insert(intent.udid, old);
                                        }
                                        intent.ticket.restore_failure_stamp(failure_stamp);
                                        return Err(error);
                                    }
                                    if let Some(old) = old {
                                        displaced
                                            .push(old)
                                            .expect("admitted displaced slot per intent");
                                    }
                                    if let Some(receipt) = &intent.failed.receipt {
                                        receipt.complete(super::reconnect::Completion::Recorded);
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
            .await;
        owner.finish(result)
    }
}

#[cfg(test)]
mod tests;

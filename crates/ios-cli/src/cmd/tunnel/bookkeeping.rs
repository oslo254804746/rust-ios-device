//! Bounded failure writes and contested success cleanup after the scan window.
use super::failure_table::{Destination, OwnedNode, RemovedBatch, Table};
use super::{Ticket, TunnelAgentState};
use std::sync::Arc;

use super::metadata_vec::KnownVec;
use anyhow::Result;
use std::sync::Mutex;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::time::Instant;

pub(super) struct DeferredFailure {
    pub(super) node: OwnedNode,
    pub(super) receipt: Option<Arc<super::reconnect::FailureReceipt>>,
    // Direct-attempt fixtures also reserve their destination before Source.
    #[cfg(test)]
    pub(super) destination: Option<Destination>,
}

pub(super) struct FailureIntent {
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

pub(super) enum SuccessKey {
    Owned(String),
    Coordinator {
        desired: Arc<()>,
        authority: crate::access::DeviceAuthority,
    },
}
impl From<String> for SuccessKey {
    fn from(key: String) -> Self {
        Self::Owned(key)
    }
}
impl From<&str> for SuccessKey {
    fn from(key: &str) -> Self {
        Self::Owned(key.to_owned())
    }
}

pub(super) struct SuccessIntent {
    pub(super) udid: SuccessKey,
    pub(super) ticket: Ticket,
    pub(super) success: DeferredSuccess,
    pub(super) _input: Option<Arc<super::candidate_input::Input>>,
    pub(super) _key_storage: Option<OwnedSemaphorePermit>,
}
impl SuccessIntent {
    fn udid(&self) -> &str {
        match &self.udid {
            SuccessKey::Owned(key) => key,
            SuccessKey::Coordinator { .. } => self.ticket.udid(),
        }
    }
}

pub(super) enum ScanIntent {
    Failure(FailureIntent),
    Success(SuccessIntent),
}

// The primary bookkeeping error stays the original anyhow object. The context
// exposes independent Source originals without allocating diagnostic strings.
pub(super) struct SourceFailuresContext {
    receipts: KnownVec<Arc<super::reconnect::FailureReceipt>>,
    _storage: OwnedSemaphorePermit,
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
    context_storage: Option<OwnedSemaphorePermit>,
}

pub(super) struct FailureWriteOwner {
    state: Mutex<WriteState>,
    _storage: OwnedSemaphorePermit,
}

impl FailureWriteOwner {
    pub(super) fn in_scan(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).in_scan = true;
    }

    fn finish(&self, result: Result<()>) -> Result<()> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
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
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
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
        let context_storage = state
            .context_storage
            .take()
            .expect("one admitted source failure context");
        match result {
            Err(error) if state.receipts.len() != 0 => Err(error.context(SourceFailuresContext {
                receipts: state.receipts.take(),
                _storage: context_storage,
            })),
            result => result,
        }
    }

    pub(super) fn abandon(&self, primary: Option<anyhow::Error>) -> Option<anyhow::Error> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.finished {
            return primary;
        }
        state.finished = true;
        for receipt in state.receipts.as_slice() {
            receipt.complete(super::reconnect::Completion::Abandoned);
        }
        let context_storage = state
            .context_storage
            .take()
            .expect("one admitted source failure context");
        match primary {
            Some(error) if state.receipts.len() != 0 => {
                Some(error.context(SourceFailuresContext {
                    receipts: state.receipts.take(),
                    _storage: context_storage,
                }))
            }
            other => other,
        }
    }
}
impl Drop for FailureWriteOwner {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(|p| p.into_inner());
        if !state.finished {
            for receipt in state.receipts.as_slice() {
                receipt.complete(super::reconnect::Completion::Abandoned);
            }
        }
    }
}

// All outcome/receipt destinations and the table backing are admitted before Source.
// There is at most one outcome/receipt/displaced row per candidate. No later
// byte admission can discard an acquired original after its Ready(Err) poll.
pub(super) struct WriteBatch {
    owner: Arc<FailureWriteOwner>,
    intents: KnownVec<ScanIntent>,
    displaced: RemovedBatch,
    destination: Option<Destination>,
}

impl WriteBatch {
    #[cfg(test)]
    pub(super) fn new(bank: &Arc<Semaphore>, capacity: usize) -> Result<Self> {
        anyhow::ensure!(
            capacity <= super::MAX_FAILURE_RECORDS,
            crate::output::resource_decision("tunnel scan write batch limit")
        );
        Self::with_destination(
            bank,
            capacity,
            if capacity == 0 {
                None
            } else {
                Some(Destination::new(bank)?)
            },
        )
    }

    pub(super) fn success_only(bank: &Arc<Semaphore>, capacity: usize) -> Result<Self> {
        Self::with_capacities(bank, capacity, 0, None)
    }

    pub(super) fn with_destination(
        bank: &Arc<Semaphore>,
        capacity: usize,
        destination: Option<Destination>,
    ) -> Result<Self> {
        Self::with_capacities(bank, capacity, capacity, destination)
    }

    fn with_capacities(
        bank: &Arc<Semaphore>,
        capacity: usize,
        receipt_capacity: usize,
        destination: Option<Destination>,
    ) -> Result<Self> {
        anyhow::ensure!(
            capacity <= super::MAX_FAILURE_RECORDS,
            crate::output::resource_decision("tunnel scan write batch limit")
        );
        // Reserve both application-owned payloads before the Source window.
        // Error Ready(Err) never has to acquire more bytes to keep its original.
        let mut payloads = super::admission::known_metadata_charge(
            bank,
            std::mem::size_of::<FailureWriteOwner>() + std::mem::size_of::<SourceFailuresContext>(),
        )?;
        let owner_storage = payloads
            .split(std::mem::size_of::<FailureWriteOwner>())
            .expect("reserved failure write owner payload");
        let mut intents = KnownVec::new(Arc::clone(bank), capacity);
        intents.reserve(capacity)?;
        let mut receipts = KnownVec::new(Arc::clone(bank), receipt_capacity);
        receipts.reserve(receipt_capacity)?;
        Ok(Self {
            owner: Arc::new(FailureWriteOwner {
                state: Mutex::new(WriteState {
                    receipts,
                    finished: false,
                    in_scan: false,
                    context_storage: Some(payloads),
                }),
                _storage: owner_storage,
            }),
            intents,
            displaced: RemovedBatch::default(),
            destination,
        })
    }

    pub(super) fn record(&mut self, intent: ScanIntent) -> Result<()> {
        if let ScanIntent::Failure(failure) = &intent {
            if let Some(receipt) = &failure.failed.receipt {
                self.owner
                    .state
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .receipts
                    .push(Arc::clone(receipt))?;
            }
        }
        self.intents.push(intent)
    }

    pub(super) fn record_success_reserved(&mut self, intent: SuccessIntent) {
        self.intents.push_reserved(ScanIntent::Success(intent));
    }

    pub(super) fn finish(self, result: Result<()>) -> Result<()> {
        self.owner.finish(result)
    }

    pub(super) fn owner(&self) -> Arc<FailureWriteOwner> {
        Arc::clone(&self.owner)
    }

    #[cfg(test)]
    pub(super) fn success_only_bytes(capacity: usize) -> usize {
        std::mem::size_of::<FailureWriteOwner>()
            + std::mem::size_of::<SourceFailuresContext>()
            + capacity * std::mem::size_of::<ScanIntent>()
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(capacity: usize) -> usize {
        std::mem::size_of::<FailureWriteOwner>()
            + std::mem::size_of::<SourceFailuresContext>()
            + capacity
                * (std::mem::size_of::<ScanIntent>()
                    + std::mem::size_of::<Arc<super::reconnect::FailureReceipt>>())
            + if capacity == 0 {
                0
            } else {
                Destination::minimum_bytes()
            }
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
        failed: &mut Table,
        udid: &str,
        success: &DeferredSuccess,
        ticket: &Ticket,
        deadline: &super::lifecycle::ConnectionDeadline,
    ) -> Result<Option<OwnedNode>> {
        self.remove_success_row_authorized(failed, udid, success, ticket, deadline, None)
    }

    fn remove_success_row_authorized(
        &self,
        failed: &mut Table,
        udid: &str,
        success: &DeferredSuccess,
        ticket: &Ticket,
        deadline: &super::lifecycle::ConnectionDeadline,
        authority: Option<&crate::access::DeviceAuthority>,
    ) -> Result<Option<OwnedNode>> {
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
            if let Some(authority) = authority {
                authority.check_clock()?;
            }
            let old = failed.remove(udid);
            ticket.clear_failure_version(&success.version);
            #[cfg(test)]
            {
                let callback = self.inner.success_after_remove.lock().unwrap().take();
                if let Some(callback) = callback {
                    callback();
                }
            }
            let final_check = deadline.check().and_then(|()| {
                authority.map_or(Ok(()), crate::access::DeviceAuthority::check_clock)
            });
            if let Err(error) = final_check {
                if let Some(old) = old {
                    failed.restore(old);
                }
                ticket.restore_failure_stamp(failure_stamp);
                return Err(error);
            }
            Ok(old)
        })
    }

    #[cfg(test)]
    pub(super) async fn flush_scan_intents_with_total(
        &self,
        mut intents: Vec<ScanIntent>,
        whole: Option<&super::scan::Deadline>,
    ) -> Result<()> {
        let mut destination = None;
        for intent in &mut intents {
            if let ScanIntent::Failure(failure) = intent {
                if destination.is_none() {
                    destination = failure.failed.destination.take();
                }
            }
        }
        if destination.is_none()
            && intents
                .iter()
                .any(|intent| matches!(intent, ScanIntent::Failure(_)))
        {
            destination = match Destination::new(&self.inner.global_metadata_bytes) {
                Ok(destination) => Some(destination),
                Err(error) => return Err(error.context(ExternalIntentsContext(intents))),
            };
        }
        let mut batch = match WriteBatch::with_destination(
            &self.inner.global_metadata_bytes,
            intents.len(),
            destination,
        ) {
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
            destination,
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
                let mut destination = destination;
                let mut remaining = Some(intents);
                let result = (|| {
                    deadline.check()?;
                    if let Some(destination) = destination.take() {
                        failed.adopt(destination);
                    }
                    deadline.check()?;
                    for intent in remaining.take().expect("one original intent batch") {
                        let outcome = match intent {
                            ScanIntent::Success(intent) => {
                                if !intent.ticket.is_current() {
                                    continue;
                                }
                                let outcome = match &intent.udid {
                                    SuccessKey::Owned(_) => self.remove_success_row(
                                        &mut failed,
                                        intent.udid(),
                                        &intent.success,
                                        &intent.ticket,
                                        &deadline,
                                    ),
                                    SuccessKey::Coordinator { desired, authority } => {
                                        if !authority.current()
                                            || !Arc::ptr_eq(
                                                desired,
                                                &intent.ticket.desired().version,
                                            )
                                        {
                                            continue;
                                        }
                                        // Authority is acquired outside Ticket's state fence.
                                        authority.commit(intent.udid(), || {
                                            self.remove_success_row_authorized(
                                                &mut failed,
                                                intent.udid(),
                                                &intent.success,
                                                &intent.ticket,
                                                &deadline,
                                                Some(authority),
                                            )
                                        })
                                    }
                                };
                                outcome.map(|old| {
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
                                let previous_current =
                                    failed.get(&intent.failed.node.udid).is_some_and(|old| {
                                        old.matches_global(&intent.ticket.global_epoch())
                                            && intent.ticket.failure_is_current(&old.version)
                                    });
                                let mut prepared = Some(intent.failed.node);
                                let outcome = intent.ticket.commit(|| {
                                    deadline.check()?;
                                    let failure_stamp = intent.ticket.failure_stamp();
                                    let mut next =
                                        prepared.take().expect("one pre-Source failure node");
                                    next.row.global = Some(intent.ticket.global_epoch());
                                    let previous = failed.get(&next.udid);
                                    next.row.last_attempt = Instant::now();
                                    next.row.fail_count = previous
                                        .filter(|old| {
                                            previous_current && old.identity == next.row.identity
                                        })
                                        .map_or(1, |old| old.fail_count.saturating_add(1));
                                    if let Some(previous) = previous {
                                        next.row.slot = previous.slot.clone();
                                    }
                                    let version = next.row.version.clone();
                                    let change = failed.insert_reserved(next);
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
                                        displaced.push(failed.rollback(change));
                                        intent.ticket.restore_failure_stamp(failure_stamp);
                                        return Err(error);
                                    }
                                    if let Some(old) = Table::commit(change) {
                                        displaced.push(old);
                                    }
                                    if let Some(receipt) = &intent.failed.receipt {
                                        receipt.complete(super::reconnect::Completion::Recorded);
                                    }
                                    Ok(())
                                });
                                drop(prepared);
                                outcome
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
                failed.release_empty_backing();
                drop(failed);
                drop(displaced);
                drop(remaining);
                drop(destination);
                result
            })
            .await;
        owner.finish(result)
    }
}

#[cfg(test)]
mod tests;

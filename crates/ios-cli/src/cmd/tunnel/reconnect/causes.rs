//! Bounded originals for automatic Source failures. Receipt slots share the
//! original failure admission permit; no display string replaces an error.
use super::*;
use std::error::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum Completion {
    Pending,
    Recorded,
    Failed(ErrorClass),
    Abandoned,
}

pub(in super::super) struct FailureReceipt {
    identity: AttemptIdentity,
    ticket: Ticket,
    slot: Arc<OwnedSemaphorePermit>,
    version: Arc<()>,
    first: Arc<anyhow::Error>,
    latest: Arc<anyhow::Error>,
    failures: u64,
    completion: Mutex<Completion>,
}

impl FailureReceipt {
    pub(in super::super) fn primary(&self) -> &anyhow::Error {
        &self.first
    }
    pub(in super::super) fn latest(&self) -> &anyhow::Error {
        &self.latest
    }
    pub(in super::super) fn slot(&self) -> Arc<OwnedSemaphorePermit> {
        Arc::clone(&self.slot)
    }
    pub(in super::super) fn version(&self) -> Arc<()> {
        Arc::clone(&self.version)
    }
    pub(in super::super) fn same_ticket(&self, ticket: &Ticket) -> bool {
        self.ticket.same_incarnation(ticket)
    }
    pub(in super::super) fn same_identity(
        &self,
        identity: &AttemptIdentity,
        ticket: &Ticket,
    ) -> bool {
        self.identity == *identity && self.same_ticket(ticket)
    }
    pub(in super::super) fn selected(
        &self,
        udid: &str,
        predicate: &impl Fn(&str, &AttemptIdentity, &Ticket) -> bool,
    ) -> bool {
        predicate(udid, &self.identity, &self.ticket)
    }
    pub(in super::super) fn complete(&self, completion: Completion) {
        let mut current = self.completion.lock().unwrap_or_else(|p| p.into_inner());
        if *current == Completion::Pending {
            *current = completion;
        }
    }
    #[cfg(test)]
    pub(in super::super) fn completion(&self) -> Completion {
        *self.completion.lock().unwrap_or_else(|p| p.into_inner())
    }
    #[cfg(test)]
    pub(in super::super) fn failures(&self) -> u64 {
        self.failures
    }
}

impl std::fmt::Display for FailureReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("automatic tunnel Source failure receipt")
    }
}
impl std::fmt::Debug for FailureReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelSourceFailureReceipt")
            .field(
                "primary_class",
                &crate::output::operation_error_class(self.primary().as_ref()),
            )
            .field(
                "latest_class",
                &crate::output::operation_error_class(self.latest().as_ref()),
            )
            .field("failures", &self.failures)
            .field(
                "completion",
                &*self.completion.lock().unwrap_or_else(|p| p.into_inner()),
            )
            .finish()
    }
}
impl Error for FailureReceipt {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.primary().as_ref())
    }
}

pub(in super::super) struct SourceFailure {
    receipt: Arc<FailureReceipt>,
}
impl SourceFailure {
    pub(in super::super) fn receipt(&self) -> Arc<FailureReceipt> {
        Arc::clone(&self.receipt)
    }
}
impl std::fmt::Display for SourceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("automatic tunnel Source connection failed")
    }
}
impl std::fmt::Debug for SourceFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelSourceFailure")
            .field("receipt", &self.receipt)
            .finish()
    }
}
impl Error for SourceFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.receipt.latest().as_ref())
    }
}

impl Ledger {
    pub(in super::super) fn payload_receipts(&self) -> Vec<Arc<FailureReceipt>> {
        let state = self.lock();
        state
            .causes
            .values()
            .chain(state.retired_causes.iter().map(|(_, receipt)| receipt))
            .cloned()
            .collect()
    }

    pub(in super::super) fn clear_retired_causes(&self) {
        let retired = std::mem::take(&mut self.lock().retired_causes);
        drop(retired);
    }

    pub(in super::super) fn capture_source(
        &self,
        attempt: &Attempt,
        original: anyhow::Error,
    ) -> anyhow::Error {
        let original = Arc::new(original);
        let mut retained = None;
        let mut displaced = None;
        let commit = attempt.ticket.commit(|| {
            let mut state = self.lock();
            if state.sealed
                || !state
                    .rows
                    .get(&attempt.udid)
                    .is_some_and(|row| Arc::ptr_eq(&row.version, &attempt.version))
            {
                return Err(super::super::lifecycle::cancelled());
            }
            let old = state
                .causes
                .get(&attempt.udid)
                .filter(|old| old.same_identity(&attempt.identity, &attempt.ticket));
            let receipt = Arc::new(FailureReceipt {
                identity: attempt.identity.clone(),
                ticket: attempt.ticket.clone(),
                slot: Arc::clone(&attempt.slot),
                version: Arc::clone(&attempt.version),
                first: old.map_or_else(|| Arc::clone(&original), |old| Arc::clone(&old.first)),
                latest: Arc::clone(&original),
                failures: old.map_or(1, |old| old.failures.saturating_add(1)),
                completion: Mutex::new(Completion::Pending),
            });
            displaced = state
                .causes
                .insert(attempt.udid.clone(), Arc::clone(&receipt));
            retained = Some(receipt);
            Ok(())
        });
        if commit.is_err() {
            // A Source error acquired in a clear/close poll remains historical;
            // it cannot resurrect a current UDID row, schedule or readiness.
            let receipt = Arc::new(FailureReceipt {
                identity: attempt.identity.clone(),
                ticket: attempt.ticket.clone(),
                slot: Arc::clone(&attempt.slot),
                version: Arc::clone(&attempt.version),
                first: Arc::clone(&original),
                latest: original,
                failures: 1,
                completion: Mutex::new(Completion::Abandoned),
            });
            let mut state = self.lock();
            state
                .retired_causes
                .push((attempt.udid.clone(), Arc::clone(&receipt)));
            debug_assert!(state.causes.len() + state.retired_causes.len() <= MAX_FAILURE_RECORDS);
            retained = Some(receipt);
        }
        drop(displaced);
        anyhow::Error::new(SourceFailure {
            receipt: retained.expect("acquired original has receipt"),
        })
    }

    #[cfg(test)]
    pub(in super::super) fn receipt(&self, udid: &str) -> Option<Arc<FailureReceipt>> {
        self.lock().causes.get(udid).cloned()
    }

    #[cfg(test)]
    pub(in super::super) fn receipt_count(&self) -> usize {
        let state = self.lock();
        state.causes.len() + state.retired_causes.len()
    }
    #[cfg(test)]
    pub(in super::super) fn retired_receipts(&self) -> Vec<Arc<FailureReceipt>> {
        self.lock()
            .retired_causes
            .iter()
            .map(|(_, receipt)| Arc::clone(receipt))
            .collect()
    }
}

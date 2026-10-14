//! Original Source objects enter preallocated, byte-owned receipt capsules.
//! Opaque error payloads and private Arc/anyhow allocations stay incomplete.
use super::*;
use std::error::Error;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) enum Completion {
    Pending,
    Recorded,
    Failed(ErrorClass),
    Abandoned,
}

struct Original {
    error: OnceLock<anyhow::Error>,
    _storage: OwnedSemaphorePermit,
}
impl Original {
    fn error(&self) -> &anyhow::Error {
        self.error.get().expect("captured Source original")
    }
}
struct History {
    first: Arc<Original>,
    failures: u64,
}

pub(in super::super) struct FailureReceipt {
    pub(super) udid: String,
    pub(super) identity: AttemptIdentity,
    pub(super) ticket: Ticket,
    pub(super) slot: Arc<OwnedSemaphorePermit>,
    pub(super) version: Arc<()>,
    original: Arc<Original>,
    history: OnceLock<History>,
    completion: Mutex<Completion>,
    wrapper_storage: Mutex<Option<OwnedSemaphorePermit>>,
    _storage: OwnedSemaphorePermit,
}

impl FailureReceipt {
    /// Construct every owned capsule and copied selector before Source poll.
    pub(super) fn reserve(
        udid: &str,
        identity: &AttemptIdentity,
        ticket: &Ticket,
        slot: Arc<OwnedSemaphorePermit>,
        version: Arc<()>,
        bank: &Arc<Semaphore>,
    ) -> Result<Arc<Self>> {
        let bytes = std::mem::size_of::<Self>()
            .checked_add(udid.len())
            .and_then(|bytes| {
                bytes.checked_add(super::super::candidate_input::route_length(&identity.route))
            })
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel Source receipt size overflow")
            })?;
        let mut storage = super::super::admission::known_metadata_charge(bank, bytes)?;
        let original_storage =
            super::super::admission::known_metadata_charge(bank, std::mem::size_of::<Original>())?;
        let wrapper_storage = super::super::admission::known_metadata_charge(
            bank,
            std::mem::size_of::<SourceFailure>(),
        )?;
        // No copied selector exists while any required capsule is unadmitted.
        let udid = udid.to_owned();
        let identity = identity.clone();
        let actual = std::mem::size_of::<Self>()
            + udid.capacity()
            + super::super::candidate_input::route_capacity(&identity.route);
        anyhow::ensure!(
            actual <= bytes,
            crate::output::resource_decision("tunnel Source receipt clone capacity exceeded")
        );
        let exact = storage
            .split(actual)
            .expect("admitted Source receipt selectors");
        drop(storage);
        Ok(Arc::new(Self {
            udid,
            identity,
            ticket: ticket.clone(),
            slot,
            version,
            original: Arc::new(Original {
                error: OnceLock::new(),
                _storage: original_storage,
            }),
            history: OnceLock::new(),
            completion: Mutex::new(Completion::Pending),
            wrapper_storage: Mutex::new(Some(wrapper_storage)),
            _storage: exact,
        }))
    }
    pub(in super::super) fn primary(&self) -> &anyhow::Error {
        self.history
            .get()
            .expect("captured Source receipt")
            .first
            .error()
    }
    pub(in super::super) fn latest(&self) -> &anyhow::Error {
        self.original.error()
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
        self.history
            .get()
            .expect("captured Source receipt")
            .failures
    }
    #[cfg(test)]
    pub(super) fn initial_bytes(udid: &str, identity: &AttemptIdentity) -> usize {
        std::mem::size_of::<Self>()
            + udid.len()
            + super::super::candidate_input::route_length(&identity.route)
            + std::mem::size_of::<Original>()
            + std::mem::size_of::<SourceFailure>()
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
            .field(
                "failures",
                &self
                    .history
                    .get()
                    .expect("captured Source receipt")
                    .failures,
            )
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
    _storage: OwnedSemaphorePermit,
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
    pub(in super::super) fn payload_receipts(
        &self,
    ) -> Result<super::super::metadata_vec::KnownVec<Arc<FailureReceipt>>> {
        let state = self.lock();
        let mut roots =
            super::super::metadata_vec::KnownVec::new(Arc::clone(&self.bank), state.causes.len());
        let count = state
            .causes
            .as_slice()
            .iter()
            .filter(|node| node.cause_state != CauseState::Pending)
            .count();
        roots.reserve(count)?;
        for node in state.causes.as_slice() {
            if node.cause_state != CauseState::Pending {
                roots.push_reserved(Arc::clone(node.cause.as_ref().expect("cause node")));
            }
        }
        Ok(roots)
    }
    pub(in super::super) fn clear_retired_causes(&self) {
        let removed = {
            let mut state = self.lock();
            for node in state.rows.as_mut_slice() {
                node.selected = false;
            }
            for node in state.causes.as_mut_slice() {
                node.selected = node.cause_state == CauseState::Retired;
            }
            let removed = state.remove_selected();
            state.release_empty_backing();
            removed
        };
        drop(removed);
    }
    pub(in super::super) fn capture_source(
        &self,
        attempt: &Attempt,
        original: anyhow::Error,
    ) -> anyhow::Error {
        let receipt = &attempt.receipt;
        assert!(
            receipt.original.error.set(original).is_ok(),
            "Source original captured once"
        );
        let mut displaced = None;
        let commit = receipt.ticket.commit(|| {
            let mut state = self.lock();
            if state.sealed
                || !state
                    .row(&receipt.udid)
                    .is_some_and(|row| Arc::ptr_eq(&row.version, &receipt.version))
            {
                return Err(super::super::lifecycle::cancelled());
            }
            let old = state
                .cause(&receipt.udid)
                .filter(|old| old.same_identity(&receipt.identity, &receipt.ticket));
            let history = History {
                first: old.map_or_else(
                    || Arc::clone(&receipt.original),
                    |old| Arc::clone(&old.history.get().expect("captured prior receipt").first),
                ),
                failures: old.map_or(1, |old| {
                    old.history
                        .get()
                        .expect("captured prior receipt")
                        .failures
                        .saturating_add(1)
                }),
            };
            assert!(receipt.history.set(history).is_ok());
            displaced = state.remove_cause(&receipt.udid);
            let index = state.pending_index(receipt).expect("pre-Source cause node");
            state.causes.as_mut_slice()[index].cause_state = CauseState::Active;
            Ok(())
        });
        if commit.is_err() {
            // Clear/close never removes an in-flight reservation. This exact
            // original enters its existing durable historical node without a
            // copied key, heap growth, quota wait or replacement error.
            assert!(receipt
                .history
                .set(History {
                    first: Arc::clone(&receipt.original),
                    failures: 1
                })
                .is_ok());
            receipt.complete(Completion::Abandoned);
            let mut state = self.lock();
            let index = state
                .pending_index(receipt)
                .expect("pre-Source retired cause node");
            state.causes.as_mut_slice()[index].cause_state = CauseState::Retired;
        }
        drop(displaced);
        let storage = receipt
            .wrapper_storage
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .expect("pre-Source wrapper payload reservation");
        anyhow::Error::new(SourceFailure {
            receipt: Arc::clone(receipt),
            _storage: storage,
        })
    }
    #[cfg(test)]
    pub(in super::super) fn receipt(&self, udid: &str) -> Option<Arc<FailureReceipt>> {
        self.lock().cause(udid).cloned()
    }
    #[cfg(test)]
    pub(in super::super) fn receipt_count(&self) -> usize {
        self.lock()
            .causes
            .as_slice()
            .iter()
            .filter(|node| node.cause_state != CauseState::Pending)
            .count()
    }
    #[cfg(test)]
    pub(in super::super) fn retired_receipts(&self) -> Vec<Arc<FailureReceipt>> {
        self.lock()
            .causes
            .as_slice()
            .iter()
            .filter(|node| node.cause_state == CauseState::Retired)
            .map(|node| Arc::clone(node.cause.as_ref().expect("cause node")))
            .collect()
    }
}

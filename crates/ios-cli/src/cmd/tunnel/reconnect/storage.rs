//! Keyed actual Vec backing and preallocated, allocation-free removal owners.
use super::super::metadata_vec::KnownVec;
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CauseState {
    Pending,
    Active,
    Retired,
}

pub(super) struct State {
    pub(super) sealed: bool,
    pub(super) rows: KnownVec<Box<Removed>>,
    pub(super) causes: KnownVec<Box<Removed>>,
}

impl State {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            sealed: false,
            rows: KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS),
            causes: KnownVec::new(
                Arc::clone(bank),
                MAX_FAILURE_RECORDS + super::super::lifecycle::MAX_CONNECTS,
            ),
        }
    }
    pub(super) fn row_index(&self, udid: &str) -> Option<usize> {
        self.rows.as_slice().iter().position(|row| row.udid == udid)
    }
    pub(super) fn row(&self, udid: &str) -> Option<&Episode> {
        self.row_index(udid)
            .and_then(|index| self.rows.as_slice()[index].row.as_ref())
    }
    pub(super) fn row_mut(&mut self, udid: &str) -> Option<&mut Episode> {
        let index = self.row_index(udid)?;
        self.rows.as_mut_slice()[index].row.as_mut()
    }
    pub(super) fn remove_row(&mut self, udid: &str) -> Option<Box<Removed>> {
        let index = self.row_index(udid)?;
        Some(self.rows.remove(index))
    }
    pub(super) fn cause_index(&self, udid: &str) -> Option<usize> {
        self.causes.as_slice().iter().position(|node| {
            node.cause_state == CauseState::Active
                && node
                    .cause
                    .as_ref()
                    .is_some_and(|receipt| receipt.udid == udid)
        })
    }
    pub(super) fn cause(&self, udid: &str) -> Option<&Arc<FailureReceipt>> {
        self.cause_index(udid)
            .and_then(|index| self.causes.as_slice()[index].cause.as_ref())
    }
    pub(super) fn pending_index(&self, receipt: &Arc<FailureReceipt>) -> Option<usize> {
        self.causes.as_slice().iter().position(|node| {
            node.cause_state == CauseState::Pending
                && node
                    .cause
                    .as_ref()
                    .is_some_and(|old| Arc::ptr_eq(old, receipt))
        })
    }
    pub(super) fn remove_cause(&mut self, udid: &str) -> Option<Box<Removed>> {
        let index = self.cause_index(udid)?;
        Some(self.causes.remove(index))
    }
    pub(super) fn remove_pending(&mut self, receipt: &Arc<FailureReceipt>) -> Option<Box<Removed>> {
        let index = self.pending_index(receipt)?;
        Some(self.causes.remove(index))
    }

    /// Predicate and deadline checks can run without a copied key/index vector.
    /// Each owner has its removal node before Source first poll.
    pub(super) fn select(&mut self, predicate: &impl Fn(&str, &AttemptIdentity, &Ticket) -> bool) {
        for node in self.rows.as_mut_slice() {
            let row = node.row.as_ref().expect("episode node");
            node.selected = predicate(&node.udid, &row.identity, &row.ticket);
        }
        for node in self.causes.as_mut_slice() {
            let receipt = node.cause.as_ref().expect("cause node");
            // A still-polled Source owns its reserved node across clear/close.
            node.selected = node.cause_state != CauseState::Pending
                && receipt.selected(&receipt.udid, predicate);
        }
    }
    pub(super) fn remove_selected(&mut self) -> RemovedBatch {
        let mut removed = RemovedBatch::default();
        // Prepending in reverse preserves the prior row/cause insertion order.
        for index in (0..self.causes.len()).rev() {
            if self.causes.as_slice()[index].selected {
                let mut node = self.causes.remove(index);
                node.position = index;
                removed.prepend(node);
            }
        }
        for index in (0..self.rows.len()).rev() {
            if self.rows.as_slice()[index].selected {
                let mut node = self.rows.remove(index);
                node.position = index;
                removed.prepend(node);
            }
        }
        removed
    }
    pub(super) fn restore(&mut self, removed: RemovedBatch) {
        for mut node in removed {
            node.selected = false;
            let position = node.position;
            if node.row.is_some() {
                self.rows.insert_reserved(position, node);
            } else {
                self.causes.insert_reserved(position, node);
            }
        }
    }
    pub(super) fn release_empty_backing(&mut self) {
        if self.rows.len() == 0 {
            drop(self.rows.take());
        }
        if self.causes.len() == 0 {
            drop(self.causes.take());
        }
    }
}

pub(in super::super) struct Removed {
    pub(super) udid: String,
    pub(super) row: Option<Episode>,
    pub(super) cause: Option<Arc<FailureReceipt>>,
    pub(super) cause_state: CauseState,
    pub(super) selected: bool,
    position: usize,
    next: Option<Box<Removed>>,
    // Box payload and actual owned String/route capacities; Arc private headers
    // and allocator overhead remain outside this known-storage contract.
    _storage: OwnedSemaphorePermit,
}

impl Removed {
    pub(super) fn episode(
        udid: &str,
        identity: &super::AttemptIdentity,
        make_row: impl FnOnce(super::AttemptIdentity) -> Episode,
        bank: &Arc<Semaphore>,
    ) -> Result<Box<Self>> {
        let bytes = std::mem::size_of::<Self>()
            .checked_add(udid.len())
            .and_then(|bytes| {
                bytes.checked_add(super::super::candidate_input::route_length(&identity.route))
            })
            .ok_or_else(|| crate::output::resource_decision("tunnel retry row size overflow"))?;
        let mut storage = super::super::admission::known_metadata_charge(bank, bytes)?;
        // Admission precedes both independent selector buffers and the Box.
        let udid = udid.to_owned();
        let identity = identity.clone();
        let actual = std::mem::size_of::<Self>()
            + udid.capacity()
            + super::super::candidate_input::route_capacity(&identity.route);
        anyhow::ensure!(
            actual <= bytes,
            crate::output::resource_decision("tunnel retry row clone capacity exceeded")
        );
        let exact = storage.split(actual).expect("admitted retry row selectors");
        drop(storage);
        Ok(Box::new(Self {
            udid,
            row: Some(make_row(identity)),
            cause: None,
            cause_state: CauseState::Pending,
            selected: false,
            position: 0,
            next: None,
            _storage: exact,
        }))
    }
    pub(super) fn receipt(
        receipt: Arc<FailureReceipt>,
        bank: &Arc<Semaphore>,
    ) -> Result<Box<Self>> {
        let storage =
            super::super::admission::known_metadata_charge(bank, std::mem::size_of::<Self>())?;
        Ok(Box::new(Self {
            udid: String::new(),
            row: None,
            cause: Some(receipt),
            cause_state: CauseState::Pending,
            selected: false,
            position: 0,
            next: None,
            _storage: storage,
        }))
    }
}

#[derive(Default)]
pub(in super::super) struct RemovedBatch {
    head: Option<Box<Removed>>,
}
impl RemovedBatch {
    fn prepend(&mut self, mut node: Box<Removed>) {
        node.next = self.head.take();
        self.head = Some(node);
    }
}
impl Iterator for RemovedBatch {
    type Item = Box<Removed>;
    fn next(&mut self) -> Option<Self::Item> {
        let mut node = self.head.take()?;
        self.head = node.next.take();
        Some(node)
    }
}
impl Drop for RemovedBatch {
    fn drop(&mut self) {
        // Avoid recursive linked-list destruction for the bounded large table.
        while self.next().is_some() {}
    }
}

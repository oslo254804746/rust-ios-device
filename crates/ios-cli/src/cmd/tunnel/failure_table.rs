//! Admitted failure nodes and allocation-free original-owner rollback.
//! Arc/anyhow private allocations and arbitrary dyn Error remain separate.
use std::ops::Deref;
#[cfg(test)]
use std::ops::Index;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

#[cfg(test)]
use super::candidate_input::route_capacity;
use super::candidate_input::Input;
use super::metadata_vec::KnownVec;
use super::shared_value::Shared;
use super::{AttemptIdentity, FailedTunnelAttempt, Route, MAX_FAILURE_RECORDS};

struct IdentityOwner {
    value: AttemptIdentity,
    _storage: OwnedSemaphorePermit,
}

/// Clones share the actual route buffer and its fee; they never clone String.
#[derive(Clone)]
pub(super) struct Identity(Shared<IdentityOwner>);

impl Identity {
    fn copied(route: &Route, native_id: Option<u32>, bank: &Arc<Semaphore>) -> Result<Self> {
        let route_bytes = match route {
            Route::Usbmux => 0,
            Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::len),
            Route::Mobdev2(host) => host.len(),
        };
        let bytes = std::mem::size_of::<IdentityOwner>()
            .checked_add(route_bytes)
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel failure identity size overflow")
            })?;
        let mut storage = super::admission::known_metadata_charge(bank, bytes)?;
        let mut copier =
            super::selector_copy::Copier::new(storage, std::mem::size_of::<IdentityOwner>())?;
        let route = copier.route(route)?;
        storage = copier.finish();
        Ok(Self(Shared::new(IdentityOwner {
            value: AttemptIdentity { route, native_id },
            _storage: storage,
        })))
    }

    /// External adapters move their already-owned identity instead of cloning it.
    #[cfg(test)]
    pub(super) fn owned(value: AttemptIdentity, bank: &Arc<Semaphore>) -> Result<Self> {
        let bytes = std::mem::size_of::<IdentityOwner>()
            .checked_add(route_capacity(&value.route))
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel failure identity size overflow")
            })?;
        let storage = super::admission::known_metadata_charge(bank, bytes)?;
        Ok(Self(Shared::new(IdentityOwner {
            value,
            _storage: storage,
        })))
    }
}

impl Deref for Identity {
    type Target = AttemptIdentity;
    fn deref(&self) -> &Self::Target {
        &self.0.value
    }
}
impl std::borrow::Borrow<AttemptIdentity> for Identity {
    fn borrow(&self) -> &AttemptIdentity {
        &self.0.value
    }
}
impl PartialEq for Identity {
    fn eq(&self, other: &Self) -> bool {
        self.0.value == other.0.value
    }
}
impl Eq for Identity {}
impl PartialEq<AttemptIdentity> for Identity {
    fn eq(&self, other: &AttemptIdentity) -> bool {
        self.0.value == *other
    }
}
impl PartialEq<Identity> for AttemptIdentity {
    fn eq(&self, other: &Identity) -> bool {
        *self == other.0.value
    }
}
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TunnelFailureIdentity")
    }
}

/// Prepared before Source first poll; this exact Box becomes the table row.
/// uid uses Input's single actual destination. Its independent original fee
/// follows the key through failure nodes and deferred success cleanup.
pub(super) type OwnedNode = super::owned_box::Owner<Node>;

pub(super) struct Node {
    pub(super) udid: String,
    pub(super) row: FailedTunnelAttempt,
    input: Option<Arc<Input>>,
    _key_storage: Option<OwnedSemaphorePermit>,
    selected: bool,
    position: usize,
    next: Option<OwnedNode>,
}

impl Node {
    pub(super) fn input(
        input: Arc<Input>,
        native_id: Option<u32>,
        bank: &Arc<Semaphore>,
    ) -> Result<OwnedNode> {
        anyhow::ensure!(
            Arc::ptr_eq(input.storage_bank(), bank),
            crate::output::resource_decision("tunnel failure input metadata bank mismatch")
        );
        // Prepare node/identity before consuming Input's single key destination.
        let destination = super::owned_box::Destination::<Self>::new(bank)?;
        let identity = Identity::copied(&input.route, native_id, bank)?;
        let (udid, key_storage) = input.outcome_key()?;
        Ok(destination.place(Self {
            udid,
            row: FailedTunnelAttempt {
                version: Arc::new(()),
                global: None,
                last_attempt: Instant::now(),
                fail_count: 0,
                identity,
                slot: None,
            },
            input: Some(input),
            _key_storage: Some(key_storage),
            selected: false,
            position: 0,
            next: None,
        }))
    }

    /// Only for test-supplied rows/intents, not an unchecked production constructor.
    #[cfg(test)]
    pub(super) fn owned(
        incoming_udid: String,
        identity: AttemptIdentity,
        bank: &Arc<Semaphore>,
    ) -> Result<OwnedNode> {
        let destination = super::owned_box::Destination::<Self>::new(bank)?;
        let key_storage = super::admission::known_metadata_charge(bank, incoming_udid.capacity())?;
        let udid = incoming_udid;
        let identity = Identity::owned(identity, bank)?;
        Ok(destination.place(Self {
            udid,
            row: FailedTunnelAttempt {
                version: Arc::new(()),
                global: None,
                last_attempt: Instant::now(),
                fail_count: 0,
                identity,
                slot: None,
            },
            input: None,
            _key_storage: Some(key_storage),
            selected: false,
            position: 0,
            next: None,
        }))
    }

    /// Success reuses the single prepared UID copy. Never clone another outcome key.
    pub(super) fn into_success_key(
        self,
    ) -> (String, Option<Arc<Input>>, Option<OwnedSemaphorePermit>) {
        assert!(self.next.is_none());
        let Self {
            udid,
            input,
            _key_storage,
            ..
        } = self;
        (udid, input, _key_storage)
    }

    #[cfg(test)]
    pub(super) fn input_owner(&self) -> Option<&Arc<Input>> {
        self.input.as_ref()
    }

    #[cfg(test)]
    pub(super) fn input_bytes(route: &Route) -> usize {
        let route_bytes = match route {
            Route::Usbmux => 0,
            Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::len),
            Route::Mobdev2(host) => host.len(),
        };
        std::mem::size_of::<Self>() + std::mem::size_of::<IdentityOwner>() + route_bytes
    }
}

/// Separate backing prepared by the Source window. It is not tied to a live
/// empty table that clear/stop may destroy while Source is still pending.
pub(super) struct Destination {
    rows: KnownVec<OwnedNode>,
}
impl Destination {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Result<Self> {
        let mut rows = KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS);
        rows.reserve(MAX_FAILURE_RECORDS)?;
        Ok(Self { rows })
    }
    #[cfg(test)]
    pub(super) fn minimum_bytes() -> usize {
        MAX_FAILURE_RECORDS * std::mem::size_of::<OwnedNode>()
    }
}

pub(super) struct Table {
    rows: KnownVec<OwnedNode>,
    #[cfg(test)]
    after_remove: Option<Box<dyn FnOnce() + Send + Sync>>,
    #[cfg(test)]
    after_adopt: Option<Box<dyn FnOnce() + Send + Sync>>,
}
impl Table {
    pub(super) fn new(bank: &Arc<Semaphore>) -> Self {
        Self {
            rows: KnownVec::new(Arc::clone(bank), MAX_FAILURE_RECORDS),
            #[cfg(test)]
            after_remove: None,
            #[cfg(test)]
            after_adopt: None,
        }
    }

    /// Called with the writer held, before ticket/watch commit. No byte request,
    /// allocation, selector clone or original-owner destruction occurs here.
    pub(super) fn adopt(&mut self, destination: Destination) {
        let old = std::mem::replace(&mut self.rows, destination.rows);
        for node in old {
            self.rows.push_reserved(node);
        }
        #[cfg(test)]
        if let Some(callback) = self.after_adopt.take() {
            callback();
        }
        // old IntoIter retains its actual backing fee through the final move.
    }

    fn position(&self, udid: &str) -> Option<usize> {
        self.rows
            .as_slice()
            .iter()
            .position(|node| node.udid == udid)
    }
    pub(super) fn get(&self, udid: &str) -> Option<&FailedTunnelAttempt> {
        self.position(udid)
            .map(|index| &self.rows.as_slice()[index].row)
    }
    #[cfg(test)]
    pub(super) fn get_mut(&mut self, udid: &str) -> Option<&mut FailedTunnelAttempt> {
        let index = self.position(udid)?;
        Some(&mut self.rows.as_mut_slice()[index].row)
    }
    #[cfg(test)]
    pub(super) fn contains_key(&self, udid: &str) -> bool {
        self.position(udid).is_some()
    }
    pub(super) fn len(&self) -> usize {
        self.rows.len()
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = (&String, &FailedTunnelAttempt)> {
        self.rows
            .as_slice()
            .iter()
            .map(|node| (&node.udid, &node.row))
    }
    #[cfg(test)]
    pub(super) fn values(&self) -> impl Iterator<Item = &FailedTunnelAttempt> {
        self.rows.as_slice().iter().map(|node| &node.row)
    }
    #[cfg(test)]
    pub(super) fn keys(&self) -> impl Iterator<Item = &String> {
        self.rows.as_slice().iter().map(|node| &node.udid)
    }

    /// The destination was allocated before Source; replacement never charges.
    pub(super) fn insert_reserved(&mut self, node: OwnedNode) -> Change {
        let index = self.position(&node.udid).unwrap_or(self.len());
        let old = if index < self.len() {
            let mut old = std::mem::replace(&mut self.rows.as_mut_slice()[index], node);
            old.position = index;
            Some(old)
        } else {
            self.rows.push_reserved(node);
            None
        };
        Change { index, old }
    }

    /// Roll back under the same table and ticket guards; caller retains the
    /// unobserved late node until both guards have been released.
    pub(super) fn rollback(&mut self, change: Change) -> OwnedNode {
        let late = self.rows.remove(change.index);
        if let Some(old) = change.old {
            self.rows.insert_reserved(change.index, old);
        }
        late
    }
    pub(super) fn commit(change: Change) -> Option<OwnedNode> {
        change.old
    }

    pub(super) fn remove(&mut self, udid: &str) -> Option<OwnedNode> {
        let index = self.position(udid)?;
        let mut old = self.rows.remove(index);
        old.position = index;
        Some(old)
    }
    pub(super) fn restore(&mut self, mut node: OwnedNode) {
        node.selected = false;
        let index = node.position;
        self.rows.insert_reserved(index, node);
    }

    /// Predicate evaluation happens before a watch guard, without key copies.
    fn select(&mut self, keep: &impl Fn(&str, &FailedTunnelAttempt) -> bool) {
        for node in self.rows.as_mut_slice() {
            node.selected = !keep(&node.udid, &node.row);
        }
    }
    fn remove_selected(&mut self) -> RemovedBatch {
        let mut removed = RemovedBatch::default();
        for index in (0..self.len()).rev() {
            if self.rows.as_slice()[index].selected {
                let mut node = self.rows.remove(index);
                node.position = index;
                removed.push(node);
            }
        }
        removed
    }
    fn restore_all(&mut self, removed: RemovedBatch) {
        for node in removed {
            self.restore(node);
        }
    }
    pub(super) fn release_empty_backing(&mut self) {
        if self.is_empty() {
            drop(self.rows.take());
        }
    }
    pub(super) fn take_all(&mut self) -> RemovedBatch {
        self.select(&|_, _| false);
        let removed = self.remove_selected();
        self.release_empty_backing();
        removed
    }

    pub(super) fn take_other_globals(&mut self, current: Option<&Arc<()>>) -> RemovedBatch {
        self.select(&|_, row| {
            row.global
                .as_ref()
                .zip(current)
                .is_some_and(|(row, current)| Arc::ptr_eq(row, current))
        });
        let removed = self.remove_selected();
        self.release_empty_backing();
        removed
    }

    pub(super) fn retain(
        &mut self,
        clock: &super::scan::Deadline,
        epoch: &super::scan::Epoch,
        keep: impl Fn(&str, &FailedTunnelAttempt) -> bool,
    ) -> Result<RemovedBatch> {
        self.select(&keep);
        let removed = epoch.commit(clock, || {
            let removed = self.remove_selected();
            #[cfg(test)]
            if let Some(callback) = self.after_remove.take() {
                callback();
            }
            if let Err(error) = clock.check() {
                self.restore_all(removed);
                return Err(error);
            }
            Ok(removed)
        })?;
        // Only successful removal may relinquish empty backing; rollback kept it.
        self.release_empty_backing();
        Ok(removed)
    }

    #[cfg(test)]
    pub(super) fn on_after_remove(&mut self, callback: impl FnOnce() + Send + Sync + 'static) {
        assert!(self.after_remove.is_none());
        self.after_remove = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn on_after_adopt(&mut self, callback: impl FnOnce() + Send + Sync + 'static) {
        assert!(self.after_adopt.is_none());
        self.after_adopt = Some(Box::new(callback));
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<OwnedNode>()
    }
}

#[cfg(test)]
impl Index<&str> for Table {
    type Output = FailedTunnelAttempt;
    fn index(&self, key: &str) -> &Self::Output {
        self.get(key).expect("failure key exists")
    }
}

/// Must be committed or rolled back while its original table guard is held.
pub(super) struct Change {
    index: usize,
    old: Option<OwnedNode>,
}

#[derive(Default)]
pub(super) struct RemovedBatch {
    head: Option<OwnedNode>,
}
impl RemovedBatch {
    pub(super) fn push(&mut self, mut node: OwnedNode) {
        assert!(node.next.is_none());
        node.next = self.head.take();
        self.head = Some(node);
    }
}
impl Iterator for RemovedBatch {
    type Item = OwnedNode;
    fn next(&mut self) -> Option<Self::Item> {
        let mut node = self.head.take()?;
        self.head = node.next.take();
        Some(node)
    }
}
impl Drop for RemovedBatch {
    fn drop(&mut self) {
        while self.next().is_some() {}
    }
}

#[cfg(test)]
mod tests;

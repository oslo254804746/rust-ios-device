//! Admitted stream membership and original arrival-ordered tombstones.
use super::windows::{Iter as MapIter, WindowMap};
use crate::xpc::raw_storage::{exact_vec, limit};
use std::collections::{HashSet, VecDeque};
use std::{io, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) enum Membership {
    Legacy(HashSet<u32>),
    Banked(WindowMap<()>),
}
impl Membership {
    pub(super) fn new(bank: Option<&Arc<Semaphore>>) -> Self {
        match bank {
            None => Self::Legacy(HashSet::new()),
            Some(bank) => Self::Banked(WindowMap::new(Some(bank))),
        }
    }
    pub(super) fn prepare(&mut self, key: u32) -> io::Result<()> {
        self.prepare_replacing(key, None)
    }
    pub(super) fn prepare_replacing(&mut self, key: u32, removed: Option<u32>) -> io::Result<()> {
        match self {
            Self::Legacy(_) => Ok(()),
            Self::Banked(map) => map.prepare_replacing(key, removed),
        }
    }
    pub(super) fn insert(&mut self, key: u32) -> bool {
        match self {
            Self::Legacy(set) => set.insert(key),
            Self::Banked(map) => map.insert_reserved(key, ()).is_none(),
        }
    }
    pub(super) fn contains(&self, key: &u32) -> bool {
        match self {
            Self::Legacy(set) => set.contains(key),
            Self::Banked(map) => map.contains_key(key),
        }
    }
    pub(super) fn remove(&mut self, key: &u32) -> bool {
        match self {
            Self::Legacy(set) => set.remove(key),
            Self::Banked(map) => map.remove(key).is_some(),
        }
    }
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Legacy(set) => set.len(),
            Self::Banked(map) => map.len(),
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub(super) fn clear(&mut self) {
        match self {
            Self::Legacy(set) => set.clear(),
            Self::Banked(map) => map.clear(),
        }
    }
    pub(super) fn iter(&self) -> SetIter<'_> {
        match self {
            Self::Legacy(set) => SetIter::Legacy(set.iter()),
            Self::Banked(map) => SetIter::Banked(map.iter()),
        }
    }
    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        match self {
            Self::Legacy(_) => 0,
            Self::Banked(map) => map.backing_bytes(),
        }
    }
}
pub(super) enum SetIter<'a> {
    Legacy(std::collections::hash_set::Iter<'a, u32>),
    Banked(MapIter<'a, ()>),
}
impl<'a> Iterator for SetIter<'a> {
    type Item = &'a u32;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Legacy(iter) => iter.next(),
            Self::Banked(iter) => iter.next().map(|(key, _)| key),
        }
    }
}

pub(super) enum Arrival {
    Legacy(VecDeque<u32>),
    Banked(Order),
}
pub(super) struct Order {
    values: Vec<u32>,
    bank: Arc<Semaphore>,
    fee: Option<OwnedSemaphorePermit>,
}
#[cfg(test)]
type OrderHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! {
    pub(super) static ORDER_ADMISSION:OrderHook=const{std::cell::RefCell::new(None)};
}
impl Arrival {
    pub(super) fn new(bank: Option<&Arc<Semaphore>>) -> Self {
        match bank {
            None => Self::Legacy(VecDeque::new()),
            Some(bank) => Self::Banked(Order {
                values: Vec::new(),
                bank: Arc::clone(bank),
                fee: None,
            }),
        }
    }
    pub(super) fn prepare_push(&mut self, evict: bool) -> io::Result<()> {
        let Self::Banked(owner) = self else {
            return Ok(());
        };
        let needed = owner
            .values
            .len()
            .checked_add(usize::from(!evict))
            .ok_or_else(limit)?;
        if needed <= owner.values.capacity() {
            return Ok(());
        }
        let capacity = owner
            .values
            .capacity()
            .max(2)
            .checked_mul(2)
            .ok_or_else(limit)?
            .max(needed);
        #[cfg(test)]
        ORDER_ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(capacity * std::mem::size_of::<u32>());
            }
        });
        let (fee, mut replacement) = exact_vec(&owner.bank, capacity)?;
        replacement.extend_from_slice(&owner.values);
        drop(std::mem::replace(&mut owner.values, replacement));
        owner.fee = Some(fee);
        Ok(())
    }
    pub(super) fn push_back(&mut self, key: u32) {
        match self {
            Self::Legacy(queue) => queue.push_back(key),
            Self::Banked(owner) => {
                assert!(
                    owner.values.len() < owner.values.capacity(),
                    "tombstone arrival slot not admitted"
                );
                owner.values.push(key);
            }
        }
    }
    pub(super) fn pop_front(&mut self) -> Option<u32> {
        match self {
            Self::Legacy(queue) => queue.pop_front(),
            Self::Banked(owner) if owner.values.is_empty() => None,
            Self::Banked(owner) => Some(owner.values.remove(0)),
        }
    }
    pub(super) fn front(&self) -> Option<&u32> {
        match self {
            Self::Legacy(queue) => queue.front(),
            Self::Banked(owner) => owner.values.first(),
        }
    }
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Legacy(queue) => queue.len(),
            Self::Banked(owner) => owner.values.len(),
        }
    }
    pub(super) fn retain(&mut self, predicate: impl FnMut(&u32) -> bool) {
        match self {
            Self::Legacy(queue) => queue.retain(predicate),
            Self::Banked(owner) => owner.values.retain(predicate),
        }
    }
    pub(super) fn clear(&mut self) {
        match self {
            Self::Legacy(queue) => queue.clear(),
            Self::Banked(owner) => owner.values.clear(),
        }
    }
    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        match self {
            Self::Legacy(_) => 0,
            Self::Banked(owner) => owner.values.capacity() * std::mem::size_of::<u32>(),
        }
    }
}

/// A newly observed reset is not silently discarded if its record cannot be
/// admitted. The original quota source and exact peer facts survive framer drop.
pub(crate) struct PeerStateAdmission {
    pub(crate) stream_id: u32,
    pub(crate) reset_code: Option<u32>,
    source: io::Error,
}
impl PeerStateAdmission {
    pub(super) fn reset(stream_id: u32, error_code: u32, source: io::Error) -> io::Error {
        io::Error::new(
            source.kind(),
            Self {
                stream_id,
                reset_code: Some(error_code),
                source,
            },
        )
    }
    pub(super) fn end(stream_id: u32, source: io::Error) -> io::Error {
        io::Error::new(
            source.kind(),
            Self {
                stream_id,
                reset_code: None,
                source,
            },
        )
    }
}
impl std::fmt::Debug for PeerStateAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerStateAdmission")
            .field("stream_id", &self.stream_id)
            .field("reset_code", &self.reset_code)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Display for PeerStateAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("retain observed peer stream state")
    }
}
impl std::error::Error for PeerStateAdmission {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests;

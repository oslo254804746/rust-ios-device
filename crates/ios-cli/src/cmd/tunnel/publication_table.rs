//! The only publication destination: exact Vec slots and original boxed nodes.
//! Every destination is admitted before Source; removal uses paid intrusive links.
use std::alloc::{alloc, Layout};
use std::ops::Index;
use std::ptr::NonNull;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_ROWS: usize = super::lifecycle::MAX_DEVICE_GATES;

/// The caller has admitted both banks for this exact application layout.
/// Allocator rounding/private headers are outside reported Vec capacity.
fn exact_vec<U>(capacity: usize) -> Result<Vec<U>> {
    let layout = Layout::array::<U>(capacity).map_err(|_| {
        crate::output::resource_decision("tunnel publication storage layout overflow")
    })?;
    if layout.size() == 0 {
        return Ok(Vec::with_capacity(capacity));
    }
    // SAFETY: the nonzero allocation uses U's exact array layout. Vec receives
    // the same pointer/capacity, assumes no initialized U, and frees that layout.
    let pointer = unsafe { alloc(layout) };
    let pointer = NonNull::new(pointer.cast::<U>()).ok_or_else(|| {
        crate::output::resource_decision("tunnel publication storage allocation failed")
    })?;
    Ok(unsafe { Vec::from_raw_parts(pointer.as_ptr(), 0, capacity) })
}

pub(super) struct Node<T> {
    // Value/link precede permits so their actual allocations end before refund.
    value: Option<T>,
    next: Option<Box<Node<T>>>,
    key: String,
    original_slot: usize,
    incarnation: u64,
    _global: OwnedSemaphorePermit,
    _published: OwnedSemaphorePermit,
}

pub(super) struct Table<T> {
    slots: Vec<Option<Box<Node<T>>>>,
    global_backing: Option<OwnedSemaphorePermit>,
    published_backing: Option<OwnedSemaphorePermit>,
    global: Arc<Semaphore>,
    published: Arc<Semaphore>,
    len: usize,
    next_incarnation: u64,
}

fn published_charge(bank: &Arc<Semaphore>, bytes: usize) -> Result<OwnedSemaphorePermit> {
    bank.clone()
        .try_acquire_many_owned(u32::try_from(bytes).map_err(|_| {
            crate::output::resource_decision("tunnel publication storage size overflow")
        })?)
        .map_err(|_| {
            crate::output::resource_decision("tunnel published metadata byte limit exceeded")
        })
}

impl<T> Table<T> {
    pub(super) fn new(global: &Arc<Semaphore>, published: &Arc<Semaphore>) -> Self {
        Self {
            slots: Vec::new(),
            global_backing: None,
            published_backing: None,
            global: Arc::clone(global),
            published: Arc::clone(published),
            len: 0,
            next_incarnation: 0,
        }
    }

    fn ensure_backing(&mut self) -> Result<()> {
        if !self.slots.is_empty() {
            return Ok(());
        }
        let bytes = MAX_ROWS
            .checked_mul(std::mem::size_of::<Option<Box<Node<T>>>>())
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel publication backing size overflow")
            })?;
        let global = super::admission::known_metadata_charge(&self.global, bytes)?;
        let published = published_charge(&self.published, bytes)?;
        let mut slots = exact_vec(MAX_ROWS)?;
        slots.resize_with(MAX_ROWS, || None);
        self.slots = slots;
        self.global_backing = Some(global);
        self.published_backing = Some(published);
        Ok(())
    }

    /// Own the actual node, key buffer and shared/sub-limit fees before Source.
    /// The allocated Vec is this table's sole production storage, not a receipt.
    pub(super) fn prepare(&mut self, udid: &str) -> Result<Box<Node<T>>> {
        super::admission::selectors(udid, None)?;
        anyhow::ensure!(
            self.len < MAX_ROWS || self.contains_key(udid),
            crate::output::resource_decision("tunnel publication entry limit")
        );
        self.ensure_backing()?;
        let incarnation = self.next_incarnation.checked_add(1).ok_or_else(|| {
            crate::output::resource_decision("tunnel publication incarnation exhausted")
        })?;
        let bytes = std::mem::size_of::<Node<T>>()
            .checked_add(udid.len())
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel publication node size overflow")
            })?;
        let global = super::admission::known_metadata_charge(&self.global, bytes)?;
        let published = published_charge(&self.published, bytes)?;
        let mut key = exact_vec::<u8>(udid.len())?;
        key.extend_from_slice(udid.as_bytes());
        // SAFETY: the copied bytes came from a str; exact capacity is unchanged.
        let key = unsafe { String::from_utf8_unchecked(key) };
        self.next_incarnation = incarnation;
        Ok(Box::new(Node {
            value: None,
            next: None,
            key,
            original_slot: usize::MAX,
            incarnation,
            _global: global,
            _published: published,
        }))
    }

    fn slot(&self, key: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|node| node.key == key))
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.len
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub(super) fn contains_key(&self, key: &str) -> bool {
        self.slot(key).is_some()
    }
    pub(super) fn get(&self, key: &str) -> Option<&T> {
        self.get_key_value(key).map(|(_, value)| value)
    }
    pub(super) fn get_mut(&mut self, key: &str) -> Option<&mut T> {
        let slot = self.slot(key)?;
        self.slots[slot].as_mut()?.value.as_mut()
    }
    pub(super) fn get_key_value(&self, key: &str) -> Option<(&String, &T)> {
        let node = self.slots[self.slot(key)?].as_ref()?;
        Some((&node.key, node.value.as_ref()?))
    }
    pub(super) fn iter(&self) -> Iter<'_, T> {
        Iter {
            slots: self.slots.iter(),
            remaining: self.len,
        }
    }
    pub(super) fn values(&self) -> impl ExactSizeIterator<Item = &T> {
        self.iter().map(|(_, value)| value)
    }
    #[cfg(test)]
    pub(super) fn proof(&self, key: &str) -> Option<(usize, u64)> {
        let node = self.slots[self.slot(key)?].as_ref()?;
        Some((&**node as *const Node<T> as usize, node.incarnation))
    }

    /// Caller holds the original ticket/authority fences. No allocation occurs.
    /// Replacement preserves the original node and original key String buffer.
    pub(super) fn destination(&self, prepared: &Node<T>) -> Result<usize> {
        self.slot(&prepared.key)
            .or_else(|| self.slots.iter().position(Option::is_none))
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel publication destination absent")
            })
    }

    pub(super) fn publish(
        &mut self,
        slot: usize,
        mut prepared: Box<Node<T>>,
        value: T,
    ) -> Change<T> {
        assert!(prepared.value.is_none() && prepared.next.is_none());
        if self.slots[slot].is_some() {
            let previous = std::mem::replace(
                self.get_mut(&prepared.key).expect("original publication"),
                value,
            );
            let original = self.slots[slot].as_mut().expect("original publication");
            let previous_incarnation =
                std::mem::replace(&mut original.incarnation, prepared.incarnation);
            Change {
                slot,
                displaced: Some((previous, previous_incarnation)),
                unused: Some(prepared),
            }
        } else {
            prepared.value = Some(value);
            prepared.original_slot = slot;
            self.slots[slot] = Some(prepared);
            self.len += 1;
            Change {
                slot,
                displaced: None,
                unused: None,
            }
        }
    }

    pub(super) fn rollback(&mut self, mut change: Change<T>) -> Retired<T> {
        match change.displaced.take() {
            Some((original, incarnation)) => {
                let node = self.slots[change.slot]
                    .as_mut()
                    .expect("unobserved publication");
                let late = node.value.replace(original).expect("late value");
                node.incarnation = incarnation;
                Retired {
                    _value: Some(late),
                    _unused: change.unused.take(),
                }
            }
            None => {
                let node = self.slots[change.slot]
                    .take()
                    .expect("unobserved publication");
                self.len -= 1;
                Retired {
                    _value: None,
                    _unused: Some(node),
                }
            }
        }
    }

    pub(super) fn remove(&mut self, key: &str) -> Option<Removed<T>> {
        let slot = self.slot(key)?;
        let node = self.slots[slot].take().expect("located publication");
        self.len -= 1;
        let mut removed = Removed::new();
        removed.push(node, slot);
        Some(removed)
    }

    /// Restore the original paid nodes to their retained vacant slots.
    pub(super) fn restore(&mut self, mut removed: Removed<T>) {
        while let Some(mut node) = removed.head.take() {
            removed.head = node.next.take();
            let slot = node.original_slot;
            assert!(self.slots[slot].is_none());
            self.slots[slot] = Some(node);
            self.len += 1;
        }
    }

    /// Move the entire real backing and all nodes. The empty successor retains
    /// only banks and the monotonic counter; old tickets are fenced by clear.
    pub(super) fn take_all(&mut self) -> Whole<T> {
        self.len = 0;
        Whole {
            _slots: std::mem::take(&mut self.slots),
            _global: self.global_backing.take(),
            _published: self.published_backing.take(),
        }
    }

    pub(super) fn selection(&mut self) -> Selection<'_, T> {
        Selection {
            table: self,
            removed: Removed::new(),
            committed: false,
        }
    }

    pub(super) fn retain(&mut self, keep: impl Fn(&str, &T) -> bool) -> Removed<T> {
        let mut selected = self.selection();
        selected.select(keep);
        selected.commit()
    }

    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.slots.capacity()
    }
    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        self.slots.capacity() * std::mem::size_of::<Option<Box<Node<T>>>>()
    }
    #[cfg(test)]
    pub(super) fn owned_publication_bytes(&self) -> usize {
        self.published_backing
            .as_ref()
            .map_or(0, OwnedSemaphorePermit::num_permits)
            + self
                .slots
                .iter()
                .flatten()
                .map(|node| node._published.num_permits())
                .sum::<usize>()
    }
}

pub(super) struct Iter<'a, T> {
    slots: std::slice::Iter<'a, Option<Box<Node<T>>>>,
    remaining: usize,
}
impl<'a, T> Iterator for Iter<'a, T> {
    type Item = (&'a String, &'a T);
    fn next(&mut self) -> Option<Self::Item> {
        let node = self.slots.find_map(Option::as_ref)?;
        self.remaining -= 1;
        Some((&node.key, node.value.as_ref().expect("published value")))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl<T> ExactSizeIterator for Iter<'_, T> {}

impl<T> Index<&str> for Table<T> {
    type Output = T;
    fn index(&self, key: &str) -> &Self::Output {
        self.get(key).expect("publication key")
    }
}

pub(super) struct Change<T> {
    slot: usize,
    displaced: Option<(T, u64)>,
    unused: Option<Box<Node<T>>>,
}
impl<T> Change<T> {
    pub(super) fn finish(mut self) -> Retired<T> {
        Retired {
            _value: self.displaced.take().map(|(value, _)| value),
            _unused: self.unused.take(),
        }
    }
}

pub(super) struct Retired<T> {
    _value: Option<T>,
    _unused: Option<Box<Node<T>>>,
}

pub(super) struct Whole<T> {
    _slots: Vec<Option<Box<Node<T>>>>,
    _global: Option<OwnedSemaphorePermit>,
    _published: Option<OwnedSemaphorePermit>,
}

pub(super) struct Removed<T> {
    head: Option<Box<Node<T>>>,
    len: usize,
}
impl<T> Removed<T> {
    fn new() -> Self {
        Self { head: None, len: 0 }
    }
    fn push(&mut self, mut node: Box<Node<T>>, slot: usize) {
        assert!(node.next.is_none());
        node.original_slot = slot;
        node.next = self.head.take();
        self.head = Some(node);
        self.len += 1;
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.len
    }
}
impl<T> Drop for Removed<T> {
    fn drop(&mut self) {
        // Iterative destruction avoids a 4096-node recursive destructor chain.
        while let Some(mut node) = self.head.take() {
            self.head = node.next.take();
            drop(node);
        }
    }
}

pub(super) struct Selection<'a, T> {
    table: &'a mut Table<T>,
    removed: Removed<T>,
    committed: bool,
}
impl<T> Selection<'_, T> {
    pub(super) fn select(&mut self, keep: impl Fn(&str, &T) -> bool) {
        for slot in 0..self.table.slots.len() {
            let remove = self.table.slots[slot].as_ref().is_some_and(|node| {
                !keep(&node.key, node.value.as_ref().expect("published value"))
            });
            if remove {
                let node = self.table.slots[slot].take().expect("selected publication");
                self.table.len -= 1;
                self.removed.push(node, slot);
            }
        }
    }
    pub(super) fn commit(mut self) -> Removed<T> {
        self.committed = true;
        std::mem::replace(&mut self.removed, Removed::new())
    }
}
impl<T> Drop for Selection<'_, T> {
    fn drop(&mut self) {
        if !self.committed {
            self.table
                .restore(std::mem::replace(&mut self.removed, Removed::new()));
        }
    }
}

#[cfg(test)]
mod tests;

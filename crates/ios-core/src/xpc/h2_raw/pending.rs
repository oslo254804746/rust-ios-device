//! Original DATA frames in an exactly admitted ring; no private deque layout.
use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::DataFrame;
use crate::xpc::raw_storage::{exact_vec, limit};

pub(super) enum PendingFrames {
    Legacy(VecDeque<DataFrame>),
    Banked(Ring),
}

pub(super) struct Ring {
    slots: Vec<Option<DataFrame>>,
    head: usize,
    len: usize,
    bank: Arc<Semaphore>,
    // Original frames and actual allocation drop before this grant.
    fee: Option<OwnedSemaphorePermit>,
}

#[cfg(test)]
type AdmissionHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! {
    pub(super) static ADMISSION: AdmissionHook = const { std::cell::RefCell::new(None) };
}

impl PendingFrames {
    pub(super) fn new(bank: Option<&Arc<Semaphore>>) -> Self {
        match bank {
            None => Self::Legacy(VecDeque::new()),
            Some(bank) => Self::Banked(Ring {
                slots: Vec::new(),
                head: 0,
                len: 0,
                bank: Arc::clone(bank),
                fee: None,
            }),
        }
    }

    pub(super) fn prepare_push(&mut self) -> io::Result<()> {
        let Self::Banked(ring) = self else {
            return Ok(());
        };
        if ring.len < ring.slots.len() {
            return Ok(());
        }
        let capacity = ring.slots.len().max(2).checked_mul(2).ok_or_else(limit)?;
        let _bytes = capacity
            .checked_mul(std::mem::size_of::<Option<DataFrame>>())
            .ok_or_else(limit)?;
        #[cfg(test)]
        ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(_bytes);
            }
        });
        // Both old and replacement allocations remain paid during the move.
        // Fee comes first in local bindings: replacement dies first on unwind.
        let (fee, mut replacement) = exact_vec(&ring.bank, capacity)?;
        replacement.resize_with(capacity, || None);
        for (offset, slot) in replacement.iter_mut().take(ring.len).enumerate() {
            let index = (ring.head + offset) % ring.slots.len();
            *slot = ring.slots[index].take();
        }
        drop(std::mem::replace(&mut ring.slots, replacement));
        ring.head = 0;
        ring.fee = Some(fee);
        Ok(())
    }

    pub(super) fn push_back(&mut self, frame: DataFrame) {
        match self {
            Self::Legacy(queue) => queue.push_back(frame),
            Self::Banked(ring) => {
                assert!(
                    ring.len < ring.slots.len(),
                    "pending DATA slot not admitted"
                );
                let index = (ring.head + ring.len) % ring.slots.len();
                ring.slots[index] = Some(frame);
                ring.len += 1;
            }
        }
    }

    pub(super) fn push_front(&mut self, frame: DataFrame) {
        match self {
            Self::Legacy(queue) => queue.push_front(frame),
            Self::Banked(ring) => {
                // Returning a frame just popped must never grow or re-admit.
                assert!(
                    ring.len < ring.slots.len(),
                    "pending DATA rollback slot lost"
                );
                ring.head = (ring.head + ring.slots.len() - 1) % ring.slots.len();
                ring.slots[ring.head] = Some(frame);
                ring.len += 1;
            }
        }
    }

    pub(super) fn pop_front(&mut self) -> Option<DataFrame> {
        match self {
            Self::Legacy(queue) => queue.pop_front(),
            Self::Banked(ring) if ring.len == 0 => None,
            Self::Banked(ring) => {
                let frame = ring.slots[ring.head].take();
                ring.head = (ring.head + 1) % ring.slots.len();
                ring.len -= 1;
                frame
            }
        }
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Legacy(queue) => queue.len(),
            Self::Banked(ring) => ring.len,
        }
    }
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &DataFrame> {
        (0..self.len()).map(move |offset| match self {
            Self::Legacy(queue) => &queue[offset],
            Self::Banked(ring) => ring.slots[(ring.head + offset) % ring.slots.len()]
                .as_ref()
                .expect("occupied pending DATA slot"),
        })
    }

    pub(super) fn clear(&mut self) {
        match self {
            Self::Legacy(queue) => queue.clear(),
            Self::Banked(_) => {
                while let Some(frame) = self.pop_front() {
                    drop(frame);
                }
            }
        }
    }

    pub(super) fn clear_stream(&mut self, stream_id: u32) -> usize {
        let mut removed = 0usize;
        match self {
            Self::Legacy(queue) => queue.retain(|frame| {
                if frame.stream_id == stream_id {
                    removed = removed.saturating_add(frame.payload.len());
                    false
                } else {
                    true
                }
            }),
            Self::Banked(_) => {
                for _ in 0..self.len() {
                    let frame = self.pop_front().expect("original pending DATA frame");
                    if frame.stream_id == stream_id {
                        removed = removed.saturating_add(frame.payload.len());
                    } else {
                        self.push_back(frame);
                    }
                }
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests;

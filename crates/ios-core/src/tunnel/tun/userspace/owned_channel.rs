//! Metered message storage has an actual fixed backing; legacy uses Tokio.
//! Complete private roots are prepaid; opaque waiter/task internals remain separate.
use std::collections::VecDeque;
use std::mem;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc as legacy, Notify, OwnedSemaphorePermit, Semaphore};

use super::owned_buffer::{rejected, BufferAdmission};
use super::runtime_storage::charge;
use super::TunnelError;

pub(super) mod roots;
use roots::{Destination, Root};
pub(super) use roots::{SlotPermit, Slots};

pub(super) use legacy::error;

struct Entry<T> {
    value: T,
    _slot: SlotPermit,
}
struct State<T> {
    items: VecDeque<Entry<T>>,
    senders: usize,
    receiver: bool,
    backing: Option<OwnedSemaphorePermit>,
}
pub(super) struct Shared<T> {
    state: Mutex<State<T>>,
    slots: Slots,
    arrived: Notify,
    closed: Notify,
}
struct Detached<T> {
    _items: VecDeque<Entry<T>>,
    _backing: Option<OwnedSemaphorePermit>,
}

pub(super) enum Sender<T> {
    Legacy(legacy::Sender<T>),
    Metered(Root<Shared<T>>),
}
pub(super) enum Receiver<T> {
    Legacy(legacy::Receiver<T>),
    Metered(Root<Shared<T>>),
}

pub(super) enum SendPermit<'a, T> {
    Legacy(legacy::Permit<'a, T>),
    Metered(&'a Root<Shared<T>>, SlotPermit),
}
impl<T> SendPermit<'_, T> {
    pub(super) fn send(self, value: T) -> Result<(), error::SendError<T>> {
        match self {
            Self::Legacy(permit) => {
                permit.send(value);
                Ok(())
            }
            Self::Metered(shared, slot) => shared.enqueue(value, slot),
        }
    }
}

#[cfg(test)]
pub(super) fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (sender, receiver) = legacy::channel(capacity);
    (Sender::Legacy(sender), Receiver::Legacy(receiver))
}

pub(super) fn with_bank<T>(
    capacity: usize,
    bank: Option<&Arc<Semaphore>>,
) -> Result<(Sender<T>, Receiver<T>), TunnelError> {
    let Some(bank) = bank else {
        let (sender, receiver) = legacy::channel(capacity);
        return Ok((Sender::Legacy(sender), Receiver::Legacy(receiver)));
    };
    assert!(capacity > 0, "bounded channel capacity is zero");
    let bytes = capacity
        .checked_mul(mem::size_of::<Entry<T>>())
        .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
    let body_bytes = Root::<Shared<T>>::body_bytes()
        .checked_add(Slots::body_bytes())
        .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
    let whole_bytes = body_bytes
        .checked_add(bytes)
        .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
    let mut whole = charge(Some(bank), whole_bytes)?.expect("whole channel roots and ring");
    let shared_fee = whole
        .split(Root::<Shared<T>>::body_bytes())
        .expect("admitted channel root");
    let slot_fee = whole
        .split(Slots::body_bytes())
        .expect("admitted slot root");
    let destination = Destination::from_grant(bank, shared_fee)?;
    let slots = Destination::from_grant(bank, slot_fee)?;
    let (backing, items) =
        crate::device::prepared_rsd::exact_vec_from_grant::<Entry<T>>(bank, capacity, whole)
            .map_err(|source| TunnelError::Io(std::io::Error::other(source)))?;
    // Vec -> VecDeque is publicly guaranteed O(1), with no reallocation or
    // additional allocation. The exact original empty ring remains unchanged.
    let items = VecDeque::from(items);
    let shared = destination.finish(Shared {
        state: Mutex::new(State {
            items,
            senders: 1,
            receiver: true,
            backing: Some(backing),
        }),
        slots: Slots::place(slots, capacity),
        arrived: Notify::new(),
        closed: Notify::new(),
    });
    Ok((Sender::Metered(shared.clone()), Receiver::Metered(shared)))
}

#[cfg(test)]
pub(super) fn required_bytes<T>(capacity: usize) -> usize {
    capacity * mem::size_of::<Entry<T>>() + Root::<Shared<T>>::body_bytes() + Slots::body_bytes()
}

impl<T> Shared<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
    fn enqueue(&self, value: T, slot: SlotPermit) -> Result<(), error::SendError<T>> {
        self.enqueue_marked(value, slot, || {})
    }
    fn enqueue_marked(
        &self,
        value: T,
        slot: SlotPermit,
        mark: impl FnOnce(),
    ) -> Result<(), error::SendError<T>> {
        let mut state = self.lock();
        if !state.receiver {
            drop(state);
            drop(slot);
            return Err(error::SendError(value));
        }
        assert!(state.items.len() < state.items.capacity());
        state.items.push_back(Entry { value, _slot: slot });
        mark();
        drop(state);
        self.arrived.notify_one();
        Ok(())
    }
}
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Legacy(sender) => Self::Legacy(sender.clone()),
            Self::Metered(shared) => {
                let mut state = shared.lock();
                state.senders = state
                    .senders
                    .checked_add(1)
                    .expect("live sender count overflow");
                drop(state);
                Self::Metered(shared.clone())
            }
        }
    }
}
impl<T> Sender<T> {
    pub(super) async fn reserve(&self) -> Result<SendPermit<'_, T>, ()> {
        match self {
            Self::Legacy(sender) => sender
                .reserve()
                .await
                .map(SendPermit::Legacy)
                .map_err(|_| ()),
            Self::Metered(shared) => shared
                .slots
                .clone()
                .acquire_owned()
                .await
                .map(|slot| SendPermit::Metered(shared, slot))
                .map_err(|_| ()),
        }
    }

    /// Banked insertion marks durable state before its receiver is woken.
    /// Legacy Tokio exposes only send completion; an unwinding caller must
    /// retain an uncertain submission rather than retrying a duplicate item.
    pub(super) async fn send_marked(
        &self,
        value: T,
        mark: impl FnOnce(),
    ) -> Result<(), error::SendError<T>> {
        match self {
            Self::Legacy(sender) => {
                let permit = match sender.reserve().await {
                    Ok(permit) => permit,
                    Err(_) => return Err(error::SendError(value)),
                };
                permit.send(value);
                mark();
                Ok(())
            }
            Self::Metered(shared) => {
                let slot = match shared.slots.clone().acquire_owned().await {
                    Ok(slot) => slot,
                    Err(_) => return Err(error::SendError(value)),
                };
                shared.enqueue_marked(value, slot, mark)
            }
        }
    }
    pub(super) async fn send(&self, value: T) -> Result<(), error::SendError<T>> {
        match self {
            Self::Legacy(sender) => sender.send(value).await,
            Self::Metered(shared) => {
                let slot = match shared.slots.clone().acquire_owned().await {
                    Ok(slot) => slot,
                    Err(_) => return Err(error::SendError(value)),
                };
                // No await after admission: cancellation either owns the
                // pending value or leaves it in the original fixed FIFO.
                shared.enqueue(value, slot)
            }
        }
    }
    pub(super) fn try_send(&self, value: T) -> Result<(), error::TrySendError<T>> {
        match self {
            Self::Legacy(sender) => sender.try_send(value),
            Self::Metered(shared) => {
                let slot = match shared.slots.clone().try_acquire_owned() {
                    Ok(slot) => slot,
                    Err(_) => {
                        return Err(if shared.lock().receiver {
                            error::TrySendError::Full(value)
                        } else {
                            error::TrySendError::Closed(value)
                        })
                    }
                };
                shared
                    .enqueue(value, slot)
                    .map_err(|error| error::TrySendError::Closed(error.0))
            }
        }
    }
    pub(super) async fn closed(&self) {
        match self {
            Self::Legacy(sender) => sender.closed().await,
            Self::Metered(shared) => loop {
                let notified = shared.closed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if !shared.lock().receiver {
                    return;
                }
                notified.await;
            },
        }
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Self::Metered(shared) = self {
            let mut state = shared.lock();
            state.senders -= 1;
            let last = state.senders == 0;
            drop(state);
            if last {
                shared.arrived.notify_waiters();
            }
        }
    }
}
impl<T> Receiver<T> {
    pub(super) fn try_recv(&mut self) -> Result<T, error::TryRecvError> {
        match self {
            Self::Legacy(receiver) => receiver.try_recv(),
            Self::Metered(shared) => {
                let mut state = shared.lock();
                let entry = state.items.pop_front();
                let closed = state.senders == 0;
                drop(state);
                match entry {
                    Some(Entry { value, _slot }) => {
                        drop(_slot);
                        Ok(value)
                    }
                    None if closed => Err(error::TryRecvError::Disconnected),
                    None => Err(error::TryRecvError::Empty),
                }
            }
        }
    }
    pub(super) async fn recv(&mut self) -> Option<T> {
        match self {
            Self::Legacy(receiver) => receiver.recv().await,
            Self::Metered(shared) => loop {
                let notified = shared.arrived.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let (entry, closed) = {
                    let mut state = shared.lock();
                    (state.items.pop_front(), state.senders == 0)
                };
                if let Some(Entry { value, _slot }) = entry {
                    drop(_slot);
                    return Some(value);
                }
                if closed {
                    return None;
                }
                notified.await;
            },
        }
    }
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        match self {
            Self::Legacy(receiver) => receiver.len(),
            Self::Metered(shared) => shared.lock().items.len(),
        }
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        if let Self::Metered(shared) = self {
            let mut state = shared.lock();
            state.receiver = false;
            let detached = Detached {
                _items: mem::take(&mut state.items),
                _backing: state.backing.take(),
            };
            shared.slots.close();
            drop(state);
            shared.closed.notify_waiters();
            // Original messages and backing die outside the channel lock; the
            // separate shared-body fee follows the last sender/receiver/future.
            drop(detached);
        }
    }
}

#[cfg(test)]
mod tests;

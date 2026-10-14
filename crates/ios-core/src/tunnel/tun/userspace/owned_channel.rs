//! Metered message storage has an actual fixed backing; legacy uses Tokio.
//! Notify/semaphore waiter futures and Arc allocator headers remain separate.
use std::collections::VecDeque;
use std::mem;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc as legacy, Notify, OwnedSemaphorePermit, Semaphore};

use super::owned_buffer::{rejected, BufferAdmission};
use super::runtime_storage::charge;
use super::TunnelError;

pub(super) use legacy::error;

struct Entry<T> {
    value: T,
    _slot: OwnedSemaphorePermit,
}
struct State<T> {
    items: VecDeque<Entry<T>>,
    senders: usize,
    receiver: bool,
    backing: Option<OwnedSemaphorePermit>,
}
pub(super) struct Shared<T> {
    state: Mutex<State<T>>,
    slots: Arc<Semaphore>,
    arrived: Notify,
    closed: Notify,
    _body: OwnedSemaphorePermit,
}
struct Detached<T> {
    _items: VecDeque<Entry<T>>,
    _backing: Option<OwnedSemaphorePermit>,
}

pub(super) enum Sender<T> {
    Legacy(legacy::Sender<T>),
    Metered(Arc<Shared<T>>),
}
pub(super) enum Receiver<T> {
    Legacy(legacy::Receiver<T>),
    Metered(Arc<Shared<T>>),
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
    let body_bytes = mem::size_of::<Shared<T>>()
        .checked_add(mem::size_of::<Semaphore>())
        .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
    let body = charge(Some(bank), body_bytes)?.expect("metered channel body");
    let mut backing = charge(Some(bank), bytes)?.expect("metered channel backing");
    let mut items = VecDeque::new();
    items
        .try_reserve_exact(capacity)
        .map_err(|original| rejected(BufferAdmission::Allocation(original)))?;
    let actual = items
        .capacity()
        .checked_mul(mem::size_of::<Entry<T>>())
        .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
    if actual > bytes {
        return Err(rejected(BufferAdmission::UnexpectedCapacity));
    }
    let exact = backing.split(actual).expect("prepaid channel capacity");
    drop(backing);
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            items,
            senders: 1,
            receiver: true,
            backing: Some(exact),
        }),
        slots: Arc::new(Semaphore::new(capacity)),
        arrived: Notify::new(),
        closed: Notify::new(),
        _body: body,
    });
    Ok((
        Sender::Metered(Arc::clone(&shared)),
        Receiver::Metered(shared),
    ))
}

#[cfg(test)]
pub(super) fn required_bytes<T>(capacity: usize) -> usize {
    capacity * mem::size_of::<Entry<T>>()
        + mem::size_of::<Shared<T>>()
        + mem::size_of::<Semaphore>()
}

impl<T> Shared<T> {
    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
    fn enqueue(&self, value: T, slot: OwnedSemaphorePermit) -> Result<(), error::SendError<T>> {
        let mut state = self.lock();
        if !state.receiver {
            drop(state);
            drop(slot);
            return Err(error::SendError(value));
        }
        assert!(state.items.len() < state.items.capacity());
        state.items.push_back(Entry { value, _slot: slot });
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
                Self::Metered(Arc::clone(shared))
            }
        }
    }
}
impl<T> Sender<T> {
    pub(super) async fn send(&self, value: T) -> Result<(), error::SendError<T>> {
        match self {
            Self::Legacy(sender) => sender.send(value).await,
            Self::Metered(shared) => {
                let slot = match Arc::clone(&shared.slots).acquire_owned().await {
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
                let slot = match Arc::clone(&shared.slots).try_acquire_owned() {
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

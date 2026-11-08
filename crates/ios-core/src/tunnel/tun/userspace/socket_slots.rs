//! Real dynamically admitted SocketStorage, preserving smoltcp's original Meta.
use std::mem;
use std::sync::Arc;

use smoltcp::iface::{SocketSet, SocketStorage};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::owned_buffer::{rejected, BufferAdmission};
use super::runtime_storage::charge;
use super::TunnelError;

#[cfg(test)]
type AdmissionHook = std::cell::RefCell<Option<Box<dyn FnOnce(usize)>>>;
#[cfg(test)]
thread_local! {
    static ADMISSION: AdmissionHook = const { std::cell::RefCell::new(None) };
}

pub(super) struct SocketSlots {
    slots: Vec<SocketStorage<'static>>,
    // Actual socket storage and socket buffers drop before this backing fee.
    backing: Option<OwnedSemaphorePermit>,
    bank: Option<Arc<Semaphore>>,
}

impl SocketSlots {
    pub(super) fn new(bank: Option<&Arc<Semaphore>>) -> Self {
        Self {
            slots: Vec::new(),
            backing: None,
            bank: bank.cloned(),
        }
    }

    /// The view borrows our actual storage; dropping it destroys no sockets.
    /// Its universally quantified lifetime prevents borrowed sockets or views
    /// from being stored/returned by a callback. Only owned/static sockets can
    /// cross this boundary, and no view survives a storage growth operation.
    pub(super) fn with_set<R>(
        &mut self,
        action: impl for<'scope> FnOnce(&mut SocketSet<'scope>) -> R,
    ) -> R {
        fn view<'scope>(slots: &'scope mut [SocketStorage<'static>]) -> SocketSet<'scope> {
            // SAFETY: this lifetime-only cast shortens the socket lifetime for
            // a temporary mutable view; every stored buffer is owned/static.
            // The HRTB callback cannot add a shorter-lived borrowed socket or
            // return a reference to this storage. No other alias or mutation
            // exists until the view is dropped before with_set returns.
            let scoped = unsafe {
                std::slice::from_raw_parts_mut(
                    slots.as_mut_ptr().cast::<SocketStorage<'scope>>(),
                    slots.len(),
                )
            };
            SocketSet::new(scoped)
        }
        action(&mut view(&mut self.slots))
    }

    /// Finish all fallible admission before exposing a fixed-size view for add.
    /// Growth moves the whole original storage, including private Meta, without
    /// remove/re-add or resetting any socket/handle/endpoint/transport state.
    pub(super) fn prepare_add(&mut self) -> Result<(), TunnelError> {
        if self.with_set(|sockets| sockets.iter().count()) < self.slots.len() {
            return Ok(());
        }
        let capacity = self
            .slots
            .len()
            .max(2)
            .checked_mul(2)
            .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
        let requested = capacity
            .checked_mul(mem::size_of::<SocketStorage<'static>>())
            .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
        #[cfg(test)]
        ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().take() {
                callback(requested);
            }
        });
        let mut fee = charge(self.bank.as_ref(), requested)?;
        let mut replacement = Vec::new();
        replacement
            .try_reserve_exact(capacity)
            .map_err(|original| rejected(BufferAdmission::Allocation(original)))?;
        let actual = replacement
            .capacity()
            .checked_mul(mem::size_of::<SocketStorage<'static>>())
            .ok_or_else(|| rejected(BufferAdmission::SizeOverflow))?;
        if actual > requested {
            return Err(rejected(BufferAdmission::UnexpectedCapacity));
        }
        fee = fee.map(|mut fee| {
            let actual = fee
                .split(actual)
                .expect("pre-admitted socket storage capacity");
            drop(fee);
            actual
        });
        replacement.append(&mut self.slots);
        replacement.resize_with(capacity, || SocketStorage::EMPTY);
        drop(mem::replace(&mut self.slots, replacement));
        self.backing = fee;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn backing_bytes(&self) -> usize {
        self.slots.capacity() * mem::size_of::<SocketStorage<'static>>()
    }
}

#[cfg(test)]
mod tests;

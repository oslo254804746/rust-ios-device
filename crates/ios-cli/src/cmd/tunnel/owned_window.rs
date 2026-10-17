//! Bounded scan futures: actual slots and pinned payloads before any child poll.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Result;
use futures_util::Stream;
use tokio::sync::Semaphore;

use super::{
    metadata_vec::{exact_backing, AdmittedVec, KnownVec},
    owned_join::PaidFuture,
};

struct Slot<F> {
    future: Option<PaidFuture<F>>,
    spare: Option<AdmittedVec<F>>,
}

/// Empty but actually allocated F payloads, retained before optional discovery.
/// Vec length stays zero until an original candidate future is moved into it.
pub(super) struct Prepared<F> {
    slots: KnownVec<Slot<F>>,
    bank: Arc<Semaphore>,
}

pub(super) fn prepare<T, F: Future>(
    bank: &Arc<Semaphore>,
    count: usize,
    _factory: impl FnOnce(T) -> F,
) -> Result<Prepared<F>> {
    Prepared::new(bank, count)
}

#[cfg(test)]
pub(super) fn measured_bytes<A, B, C, D, F: Future>(
    _factory: impl FnOnce(A, B, C, D) -> F,
    count: usize,
) -> usize {
    // Measure the same typed allocations instead of duplicating capacity or
    // async-layout formulas in exact-quota fixtures. No F is initialized/polled.
    let total = super::MAX_GLOBAL_KNOWN_METADATA_BYTES;
    let bank = Arc::new(Semaphore::new(total));
    let prepared = Prepared::<F>::new(&bank, count).unwrap();
    let bytes = total - bank.available_permits();
    drop(prepared);
    assert_eq!(bank.available_permits(), total);
    bytes
}

impl<F: Future> Prepared<F> {
    fn new(bank: &Arc<Semaphore>, count: usize) -> Result<Self> {
        let mut prepared = Self {
            slots: KnownVec::new(bank.clone(), super::lifecycle::MAX_CONNECTS),
            bank: bank.clone(),
        };
        prepared.ensure(count)?;
        Ok(prepared)
    }

    fn ensure(&mut self, count: usize) -> Result<()> {
        let count = count.min(super::lifecycle::MAX_CONNECTS);
        if count > self.slots.len() {
            self.slots.reserve(count - self.slots.len())?;
            while self.slots.len() < count {
                self.slots.push_reserved(Slot {
                    future: None,
                    spare: None,
                });
            }
        }
        #[cfg(test)]
        if count != 0 {
            let callback = PREFLIGHT.with(|hook| hook.borrow_mut().take());
            if let Some(callback) = callback {
                callback(std::mem::size_of::<F>(), count);
            }
        }
        for slot in self.slots.as_mut_slice() {
            if slot.future.is_none() && slot.spare.is_none() {
                slot.spare = Some(exact_backing::<F>(&self.bank, 1)?);
            }
        }
        Ok(())
    }
}

pub(super) struct Window<I, F> {
    slots: KnownVec<Slot<F>>,
    inputs: Option<I>,
    bank: Arc<Semaphore>,
    cursor: usize,
    failed: bool,
}

// Only the original F is pinned, inside PaidFuture's Box. The iterator/slots
// are never structurally pinned or exposed through a pinned projection.
impl<I, F> Unpin for Window<I, F> {}

impl<I: ExactSizeIterator<Item = F>, F: Future> Window<I, F> {
    #[cfg(test)]
    pub(super) fn new(bank: &Arc<Semaphore>, inputs: I) -> Result<Self> {
        Self::from_prepared(Prepared::new(bank, inputs.len())?, inputs)
    }

    pub(super) fn from_prepared(mut prepared: Prepared<F>, inputs: I) -> Result<Self> {
        prepared.ensure(inputs.len())?;
        let mut window = Self {
            slots: prepared.slots,
            inputs: Some(inputs),
            bank: prepared.bank,
            cursor: 0,
            failed: false,
        };
        // Admit/create the complete initial window before any original Source
        // can be polled. A late body refusal destroys all unpolled inputs.
        window.fill()?;
        Ok(window)
    }

    fn fill(&mut self) -> Result<()> {
        let inputs = self.inputs.as_mut().expect("live scan window iterator");
        for slot in self.slots.as_mut_slice() {
            if slot.future.is_none() {
                let Some(future) = inputs.next() else { break };
                let allocation = match slot.spare.take() {
                    Some(allocation) => allocation,
                    None => exact_backing::<F>(&self.bank, 1)?,
                };
                slot.future = Some(PaidFuture::from_backing(future, allocation));
            }
        }
        Ok(())
    }

    fn refuse(&mut self) {
        // End actual pending/unpolled futures, not just a borrow of the stream.
        for slot in self.slots.as_mut_slice() {
            slot.future = None;
            slot.spare = None;
        }
        self.inputs.take();
        self.failed = true;
    }
}

#[cfg(test)]
type PreflightHook = std::cell::RefCell<Option<Box<dyn FnOnce(usize, usize)>>>;
#[cfg(test)]
thread_local! {
    static PREFLIGHT: PreflightHook = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) struct HookGuard;
#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        PREFLIGHT.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
pub(super) fn on_preflight(hook: impl FnOnce(usize, usize) + 'static) -> HookGuard {
    PREFLIGHT.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none());
        *current = Some(Box::new(hook));
    });
    HookGuard
}

impl<I: ExactSizeIterator<Item = F>, F: Future> Stream for Window<I, F> {
    type Item = Result<F::Output>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.failed {
            return Poll::Ready(None);
        }
        if let Err(error) = this.fill() {
            this.refuse();
            return Poll::Ready(Some(Err(error)));
        }
        let count = this.slots.len();
        let mut pending = false;
        for offset in 0..count {
            let index = (this.cursor + offset) % count;
            if let Some(future) = &mut this.slots.as_mut_slice()[index].future {
                match Pin::new(future).poll(cx) {
                    Poll::Ready(output) => {
                        this.slots.as_mut_slice()[index].future = None;
                        this.cursor = (index + 1) % count;
                        return Poll::Ready(Some(Ok(output)));
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        // At most four children share the parent Context, with no collector
        // child-waker/node collection or extra task. Ready child errors stay
        // ordinary outputs; only actual storage refusal stops this collector.
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}

#[cfg(test)]
mod tests;

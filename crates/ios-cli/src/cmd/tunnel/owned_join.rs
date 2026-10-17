//! Wait-all federation collection with admitted, actual future/result storage.
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::metadata_vec::{exact_backing, AdmittedVec, KnownVec};

pub(super) struct PaidFuture<F> {
    // Destroy the original future and its allocation before refunding the fee.
    future: Pin<Box<F>>,
    _storage: OwnedSemaphorePermit,
}

impl<F: Future> PaidFuture<F> {
    pub(super) fn new(future: F, bank: &Arc<Semaphore>) -> Result<Self> {
        let allocation = exact_backing::<F>(bank, 1)?;
        Ok(Self::from_backing(future, allocation))
    }

    pub(super) fn from_backing(future: F, allocation: AdmittedVec<F>) -> Self {
        let storage = allocation.storage;
        let mut values = allocation.values;
        assert!(values.is_empty());
        assert!(std::mem::size_of::<F>() == 0 || values.capacity() == 1);
        values.push(future);
        let pointer = values.as_mut_ptr();
        std::mem::forget(values);
        // SAFETY: the Vec owns exactly one initialized F. For nonzero F its
        // exact capacity is one and Layout::array::<F>(1) == Layout::new::<F>().
        // A ZST uses the same valid, aligned dangling pointer and needs no
        // deallocation. No fallible step separates forgetting Vec and Box.
        let future = Box::into_pin(unsafe { Box::from_raw(pointer) });
        Self {
            future,
            _storage: storage,
        }
    }
}

impl<F: Future> Future for PaidFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}

struct Slot<F: Future> {
    future: Option<PaidFuture<F>>,
    output: Option<F::Output>,
}

pub(super) async fn all<F: Future>(
    bank: &Arc<Semaphore>,
    futures: impl ExactSizeIterator<Item = F>,
) -> Result<KnownVec<F::Output>> {
    let count = futures.len();
    if count > super::MAX_UPSTREAMS {
        return Err(crate::output::resource_decision(
            "tunnel federation future entry limit",
        ));
    }
    let mut slots = KnownVec::new(bank.clone(), count);
    slots.reserve(count)?;
    let mut outputs = KnownVec::new(bank.clone(), count);
    outputs.reserve(count)?;
    // Every actual body and both arrays exist and are paid before any child
    // is polled. A late refusal drops all still-unpolled original futures.
    for future in futures {
        slots.push_reserved(Slot {
            future: Some(PaidFuture::new(future, bank)?),
            output: None,
        });
    }
    poll_fn(|cx| {
        let mut pending = false;
        for slot in slots.as_mut_slice() {
            if let Some(future) = &mut slot.future {
                match Pin::new(future).poll(cx) {
                    Poll::Ready(output) => {
                        slot.future = None;
                        slot.output = Some(output);
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        // At most 32 children share the parent's Context: no spawned task,
        // private child-waker collection, or fail-fast error handling.
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    for slot in slots.as_mut_slice() {
        outputs.push_reserved(slot.output.take().expect("completed federation child"));
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests;

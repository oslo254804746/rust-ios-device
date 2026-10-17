//! Wait-all federation collection with admitted, actual future/result storage.
use std::future::{poll_fn, Future};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use anyhow::Result;
use tokio::sync::Semaphore;

use super::{future_storage::PaidFuture, metadata_vec::KnownVec};

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

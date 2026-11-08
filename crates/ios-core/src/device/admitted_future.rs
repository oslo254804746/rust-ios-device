//! Exact storage for a borrowed preparation phase; no runtime task is spawned.
use crate::CoreError;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) struct Admitted<F> {
    // End the actual boxed payload, including all retained fields on unwind,
    // before refunding its exact original allocation grant.
    future: Pin<Box<F>>,
    _storage: OwnedSemaphorePermit,
}

pub(super) fn prepare<F: Future>(
    bank: &Arc<Semaphore>,
    make: impl FnOnce() -> F,
) -> Result<Admitted<F>, CoreError> {
    let storage = super::prepared_rsd::charge(bank, std::mem::size_of::<F>())?;
    Ok(Admitted {
        future: Box::pin(make()),
        _storage: storage,
    })
}
impl<F: Future> Future for Admitted<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Payload<'a> {
        constructor: &'a AtomicUsize,
        polls: &'a AtomicUsize,
    }
    impl Future for Payload<'_> {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            Poll::Pending
        }
    }
    impl Drop for Payload<'_> {
        fn drop(&mut self) {
            self.constructor.fetch_sub(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn exact_phase_grant_precedes_constructor_and_follows_borrowed_payload_drop() {
        let live = AtomicUsize::new(0);
        let polls = AtomicUsize::new(0);
        let bytes = std::mem::size_of::<Payload<'_>>();
        let make = || {
            live.fetch_add(1, Ordering::SeqCst);
            Payload {
                constructor: &live,
                polls: &polls,
            }
        };
        let refused = Arc::new(Semaphore::new(bytes - 1));
        assert_eq!(
            prepare(&refused, make).err().unwrap().class(),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(refused.available_permits(), bytes - 1);
        let bank = Arc::new(Semaphore::new(bytes));
        let future = prepare(&bank, make).unwrap();
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(live.load(Ordering::SeqCst), 1);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        bank.close();
        drop(future);
        assert_eq!(live.load(Ordering::SeqCst), 0);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert_eq!(bank.available_permits(), bytes);
    }
}

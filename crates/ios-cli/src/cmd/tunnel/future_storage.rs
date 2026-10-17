//! Exact concrete future allocation; the original Box is destroyed before its fee.
use super::metadata_vec::{exact_backing, AdmittedVec};
use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

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

/// An unsized view of the SAME admitted allocation, with its fee outside Box.
pub(super) struct Dynamic<'a, T> {
    future: Pin<Box<dyn Future<Output = T> + Send + 'a>>,
    _storage: OwnedSemaphorePermit,
    #[cfg(test)]
    _after_drop: AfterDrop,
}
impl<'a, T> Dynamic<'a, T> {
    pub(super) fn new<F: Future<Output = T> + Send + 'a>(
        future: F,
        bank: &Arc<Semaphore>,
    ) -> Result<Self> {
        #[cfg(test)]
        {
            let callback = DYNAMIC_LAYOUT.with(|hook| hook.borrow_mut().take());
            if let Some(callback) = callback {
                callback(std::mem::size_of::<F>());
            }
        }
        let paid = PaidFuture::new(future, bank)?;
        Ok(Self {
            future: paid.future,
            _storage: paid._storage,
            #[cfg(test)]
            _after_drop: AfterDrop,
        })
    }

    #[cfg(test)]
    pub(super) fn data_pointer(&self) -> usize {
        self.future.as_ref().get_ref() as *const _ as *const () as usize
    }
}

#[cfg(test)]
struct AfterDrop;
#[cfg(test)]
impl Drop for AfterDrop {
    fn drop(&mut self) {
        let callback = DYNAMIC_DROPPED.with(|hook| hook.borrow_mut().take());
        if let Some(callback) = callback {
            callback();
        }
    }
}
#[cfg(test)]
type DroppedHook = std::cell::RefCell<Option<Box<dyn FnOnce()>>>;
#[cfg(test)]
thread_local! { static DYNAMIC_DROPPED: DroppedHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) struct DropGuard;
#[cfg(test)]
impl Drop for DropGuard {
    fn drop(&mut self) {
        DYNAMIC_DROPPED.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
pub(super) fn after_drop(hook: impl FnOnce() + 'static) -> DropGuard {
    DYNAMIC_DROPPED.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none());
        *current = Some(Box::new(hook));
    });
    DropGuard
}

#[cfg(test)]
type DynamicHook = std::cell::RefCell<Option<Box<dyn FnOnce(usize)>>>;
#[cfg(test)]
thread_local! { static DYNAMIC_LAYOUT: DynamicHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) struct HookGuard;
#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        DYNAMIC_LAYOUT.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
pub(super) fn on_dynamic_layout(hook: impl FnOnce(usize) + 'static) -> HookGuard {
    DYNAMIC_LAYOUT.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none());
        *current = Some(Box::new(hook));
    });
    HookGuard
}
impl<T> Future for Dynamic<'_, T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        self.get_mut().future.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests;

//! Admit the actual boxed future payload, without estimating Tokio internals.
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::runtime_storage::charge;
use super::{Ledger, OwnedTask, ParentRole, TunnelError};

pub(super) enum PreparedWorker<F> {
    Legacy(F),
    Metered(AdmittedFuture<F>),
}

pub(super) struct AdmittedFuture<F> {
    // The actual allocation dies before its permit, including cancellation and
    // unwind. The wrapper adds no second heap allocation of its own.
    future: Pin<Box<F>>,
    _storage: OwnedSemaphorePermit,
}

#[cfg(test)]
type AdmissionHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! {
    pub(super) static ADMISSION: AdmissionHook = const { std::cell::RefCell::new(None) };
}

impl<F> PreparedWorker<F>
where
    F: Future + Send + 'static,
{
    pub(super) fn prepare(future: F, bank: Option<&Arc<Semaphore>>) -> Result<Self, TunnelError> {
        let Some(bank) = bank else {
            return Ok(Self::Legacy(future));
        };
        let bytes = std::mem::size_of_val(&future);
        #[cfg(test)]
        ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(bytes);
            }
        });
        let storage = charge(Some(bank), bytes)?.expect("banked future payload");
        Ok(Self::Metered(AdmittedFuture {
            future: Box::pin(future),
            _storage: storage,
        }))
    }

    pub(super) fn spawn(self, role: ParentRole, ledger: Ledger) -> OwnedTask
    where
        F: Future<Output = Result<(), TunnelError>>,
    {
        let handle = match self {
            Self::Legacy(future) => tokio::spawn(future),
            Self::Metered(future) => tokio::spawn(future),
        };
        OwnedTask::new(handle, role, ledger)
    }

    pub(super) fn spawn_child(self, children: &mut super::owner::ChildGuard<'_>)
    where
        F: Future<Output = ()>,
    {
        match self {
            Self::Legacy(future) => children.spawn(future),
            Self::Metered(future) => children.spawn(future),
        }
    }
}

impl<F: Future> Future for AdmittedFuture<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}

#[cfg(test)]
mod tests;

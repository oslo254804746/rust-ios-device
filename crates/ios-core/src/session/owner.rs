//! Bounded admission and closure of actual operation leases.

use std::future::{poll_fn, Future};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use super::operation::{OperationCancel, OperationContext, OperationOptions};
use super::{failure, OperationValue};
use crate::{CoreError, ErrorClass};

/// A pool shared explicitly by owners in one binding runtime.
#[derive(Clone)]
pub struct SharedAdmission(Arc<Semaphore>);

impl SharedAdmission {
    pub fn new(limit: usize) -> Result<Self, CoreError> {
        if !(1..=64).contains(&limit) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid shared admission limit",
            ));
        }
        Ok(Self(Arc::new(Semaphore::new(limit))))
    }

    pub fn available(&self) -> usize {
        self.0.available_permits()
    }
}

struct Book {
    closed: bool,
    active: usize,
}

struct Owner {
    book: Mutex<Book>,
    changed: Notify,
    cancel: OperationCancel,
    local: Arc<Semaphore>,
    shared: SharedAdmission,
    handles: AtomicUsize,
}

impl Owner {
    fn book(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn close(&self) {
        {
            let mut book = self.book();
            book.closed = true;
        }
        self.local.close();
        self.cancel.cancel();
    }
}

pub struct SessionOwner {
    inner: Arc<Owner>,
}

impl Clone for SessionOwner {
    fn clone(&self) -> Self {
        self.inner.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for SessionOwner {
    fn drop(&mut self) {
        if self.inner.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.close();
        }
    }
}

impl SessionOwner {
    /// Each operation must use its own fresh transport; there is no waiter queue.
    pub fn new(shared: SharedAdmission, concurrent: usize) -> Result<Self, CoreError> {
        if !(1..=4).contains(&concurrent) {
            return Err(failure(
                ErrorClass::InvalidInput,
                "invalid owner admission limit",
            ));
        }
        Ok(Self {
            inner: Arc::new(Owner {
                book: Mutex::new(Book {
                    closed: false,
                    active: 0,
                }),
                changed: Notify::new(),
                cancel: OperationCancel::new(),
                local: Arc::new(Semaphore::new(concurrent)),
                shared,
                handles: AtomicUsize::new(1),
            }),
        })
    }

    pub fn active(&self) -> usize {
        self.inner.book().active
    }

    pub fn is_closed(&self) -> bool {
        self.inner.book().closed
    }

    pub fn request_close(&self) {
        self.inner.close();
    }

    /// Seal admission, broadcast cancellation, then await actual lease release.
    /// Timeout leaves the owner closed and retains its outstanding counts.
    /// A repeated close can finish after callers drop those leases.
    pub async fn close(&self, cleanup: Duration) -> Result<(), CoreError> {
        self.request_close();
        if self.active() == 0 {
            return Ok(());
        }
        let deadline = Instant::now()
            .checked_add(cleanup)
            .ok_or_else(|| failure(ErrorClass::InvalidInput, "unrepresentable close budget"))?;
        let mut wait = Box::pin(async {
            loop {
                let changed = self.inner.changed.notified();
                // Register before inspecting to avoid losing the final release.
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.active() == 0 {
                    return;
                }
                changed.await;
            }
        });
        let fenced = poll_fn(|cx| {
            if Instant::now() >= deadline {
                return Poll::Pending;
            }
            match wait.as_mut().poll(cx) {
                Poll::Ready(()) if Instant::now() >= deadline => Poll::Pending,
                result => result,
            }
        });
        tokio::time::timeout_at(deadline, fenced)
            .await
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::TimedOut, error).into())
    }

    /// Admission precedes transport work and never awaits a permit.
    /// The total starts here, including time before the prepared future is polled.
    pub fn try_begin(
        &self,
        options: OperationOptions,
        cancel: OperationCancel,
    ) -> Result<OperationLease, CoreError> {
        if self.is_closed() {
            return Err(failure(ErrorClass::Precondition, "owner is closed"));
        }
        if cancel.is_cancelled() {
            return Err(std::io::Error::other(crate::OperationCancelled::new()).into());
        }
        if options.total.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "zero SDK operation total",
            )
            .into());
        }
        let deadline = Instant::now()
            .checked_add(options.total)
            .ok_or_else(|| failure(ErrorClass::InvalidInput, "unrepresentable operation total"))?;
        let global = Arc::clone(&self.inner.shared.0)
            .try_acquire_owned()
            .map_err(|_| failure(ErrorClass::ResourceLimit, "shared admission is full"))?;
        let local = Arc::clone(&self.inner.local)
            .try_acquire_owned()
            .map_err(|_| {
                if self.is_closed() {
                    failure(ErrorClass::Precondition, "owner is closed")
                } else {
                    failure(ErrorClass::Conflict, "owner admission is full")
                }
            })?;
        {
            let mut book = self.inner.book();
            if book.closed {
                return Err(failure(ErrorClass::Precondition, "owner is closed"));
            }
            if cancel.is_cancelled() {
                return Err(std::io::Error::other(crate::OperationCancelled::new()).into());
            }
            book.active += 1;
        }
        Ok(OperationLease {
            context: OperationContext {
                deadline,
                caller: cancel,
                owner: self.inner.cancel.clone(),
                native: Mutex::new(None),
            },
            registration: Registration {
                owner: Arc::clone(&self.inner),
                global: Some(global),
                local: Some(local),
            },
        })
    }
}

struct Registration {
    owner: Arc<Owner>,
    global: Option<OwnedSemaphorePermit>,
    local: Option<OwnedSemaphorePermit>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // Release admission before publishing the final count to close waiters.
        // Keep publication in the admission book's critical section: a caller
        // which acquires the newly returned local permit cannot publish its
        // registration before this registration has been removed.
        {
            let mut book = self.owner.book();
            drop(self.global.take());
            drop(self.local.take());
            book.active -= 1;
        }
        self.owner.changed.notify_waiters();
    }
}

/// Consumed by a finite prepared operation. Unpolled Drop returns admission.
/// Its internal context and run closure are not an external raw IO API.
pub struct OperationLease {
    // Context precedes registration so a normal lease Drop releases it first.
    context: OperationContext,
    registration: Registration,
}

impl OperationLease {
    #[cfg(test)]
    pub(crate) fn context(&self) -> &OperationContext {
        &self.context
    }

    pub(crate) fn run<T: OperationValue>(
        self,
        operation: impl for<'ctx> FnOnce(
                &'ctx OperationContext,
            )
                -> futures_util::future::BoxFuture<'ctx, Result<T, CoreError>>
            + Send,
    ) -> impl std::future::Future<Output = Result<T, CoreError>> + Send {
        // This is constructed before the returned future is first polled.
        // An abandoned prepared operation must destroy its captured original
        // stream before returning admission, even if its factory never ran.
        let pending = PendingOperation {
            operation: Some(operation),
            lease: Some(self),
        };
        async move {
            let mut pending = pending;
            let Self {
                registration,
                context,
            } = pending
                .lease
                .take()
                .expect("pending operation owns its lease");
            let operation = pending
                .operation
                .take()
                .expect("pending operation owns its factory");
            let result = match context.check() {
                Ok(()) => context.run(operation(&context)).await,
                Err(error) => {
                    drop(operation);
                    // An expired check must still retain the actual Elapsed
                    // acquired by the total timer, without invoking a factory.
                    context
                        .run(async { Err::<T, CoreError>(error.into()) })
                        .await
                }
            };
            drop(context);
            drop(registration);
            result
        }
    }
}

struct PendingOperation<F> {
    operation: Option<F>,
    lease: Option<OperationLease>,
}

impl<F> Drop for PendingOperation<F> {
    fn drop(&mut self) {
        drop(self.operation.take());
        drop(self.lease.take());
    }
}

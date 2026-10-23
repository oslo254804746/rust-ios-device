//! Original row identity and connection lifetime, with preadmitted payload.
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::Poll;

use anyhow::Result;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

type TicketFees = (OwnedSemaphorePermit, super::lifecycle::TicketOriginGrants);

/// Independent actual Ticket allocation, counted in both publication banks.
/// Displaced/retired values retain it without relying on an old Node's Box.
pub(super) struct TicketOwner(super::shared_weak_allocation::Strong<super::Ticket, TicketFees>);

impl TicketOwner {
    pub(super) fn body_bytes() -> usize {
        super::shared_weak_allocation::Strong::<super::Ticket, TicketFees>::body_bytes()
    }

    pub(super) fn new(
        ticket: &super::Ticket,
        global: &Arc<Semaphore>,
        published: &Arc<Semaphore>,
    ) -> Result<Self> {
        Self::admit(ticket, global, published).map_err(|error| error.context(Admission))
    }

    fn admit(
        ticket: &super::Ticket,
        global: &Arc<Semaphore>,
        published: &Arc<Semaphore>,
    ) -> Result<Self> {
        let bytes = Self::body_bytes();
        let global_storage = super::admission::known_metadata_charge(global, bytes)?;
        let published_storage = published
            .clone()
            .try_acquire_many_owned(u32::try_from(bytes).map_err(|_| {
                crate::output::resource_decision("tunnel publication Ticket layout overflow")
            })?)
            .map_err(|_| {
                crate::output::resource_decision("tunnel published metadata byte limit exceeded")
            })?;
        // Both original banks already cover the whole actual allocation. An
        // identity from P must not be charged a duplicate origin mirror in P.
        let origins = ticket.origin_grants_with_covered(global, Some(&published_storage), bytes)?;
        Ok(Self(super::shared_weak_allocation::Strong::new(
            ticket.clone(),
            global,
            global_storage,
            (published_storage, origins),
        )?))
    }

    #[cfg(test)]
    fn ptr_eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl Clone for TicketOwner {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl std::ops::Deref for TicketOwner {
    type Target = super::Ticket;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub(super) struct Life {
    closed: AtomicBool,
    changed: Notify,
}

pub(super) type LifeOwner = super::shared_dual_body::Strong<Life>;

impl Life {
    pub(super) fn new(global: &Arc<Semaphore>, published: &Arc<Semaphore>) -> Result<LifeOwner> {
        Self::admit(global, published).map_err(|error| error.context(Admission))
    }

    fn admit(global: &Arc<Semaphore>, published: &Arc<Semaphore>) -> Result<LifeOwner> {
        let bytes = LifeOwner::body_bytes();
        let global_storage = super::admission::known_metadata_charge(global, bytes)?;
        let published_storage = published
            .clone()
            .try_acquire_many_owned(u32::try_from(bytes).map_err(|_| {
                crate::output::resource_decision("tunnel publication lifetime size overflow")
            })?)
            .map_err(|_| {
                crate::output::resource_decision("tunnel published metadata byte limit exceeded")
            })?;
        // Locked Notify::new owns an inline atomic and intrusive waiters mutex;
        // no new waiter container is created here. The declared lifetime root
        // is paid above; opaque runtime waiter machinery remains separate.
        let life = LifeOwner::new(
            Self {
                closed: AtomicBool::new(false),
                changed: Notify::new(),
            },
            global,
            published,
            global_storage,
            published_storage,
        )?;
        #[cfg(test)]
        CREATED.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(&*life as *const Life as usize);
            }
        });
        Ok(life)
    }

    pub(super) fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.changed.notify_waiters();
        }
    }

    pub(super) fn check(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            Err(super::lifecycle::cancelled())
        } else {
            Ok(())
        }
    }

    async fn closed(&self) {
        let notified = self.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }

    pub(super) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::pin!(work);
        let closed = self.closed();
        tokio::pin!(closed);
        std::future::poll_fn(|cx| {
            if self.closed.load(Ordering::Acquire) || closed.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(super::lifecycle::cancelled()));
            }
            // Preserve an original ready error if this work poll crosses close.
            work.as_mut().poll(cx)
        })
        .await
    }

    #[cfg(test)]
    pub(super) fn bytes() -> usize {
        LifeOwner::body_bytes()
    }
}

impl LifeOwner {
    pub(super) fn belongs_to(&self, bank: &Arc<Semaphore>) -> bool {
        self.first_bank_matches(bank)
    }
}

#[derive(Debug)]
pub(super) struct Admission;
impl std::fmt::Display for Admission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel publication lifetime admission failed")
    }
}

#[cfg(test)]
type CreationHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! {
    static CREATED: CreationHook = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
struct HookGuard;
#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        CREATED.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
fn on_created(hook: impl FnMut(usize) + 'static) -> HookGuard {
    CREATED.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none());
        *current = Some(Box::new(hook));
    });
    HookGuard
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod ticket_tests;

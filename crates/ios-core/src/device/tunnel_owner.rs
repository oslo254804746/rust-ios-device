//! Optional caller ownership before a connection's initial RSD probe.

use std::sync::Arc;

use crate::{CoreError, TunnelHandle};

/// Reserves caller-owned cleanup capacity before each tunnel strategy attempt.
///
/// Owner-aware connection functions call this before opening the strategy's
/// transport. A connection can try several targets, so each attempt gets its
/// own reservation. Returning an error prevents that attempt's transport work.
/// The implementation must remain synchronous and avoid blocking or reentering
/// the connection. Ordinary connection functions do not invoke this interface.
pub trait TunnelOwner: Send + Sync {
    fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError>;

    /// Keep an admitted reservation's byte grant outside its original Box.
    /// Existing implementations retain their original callback and allocation.
    fn reserve_owned(&self) -> Result<TunnelReservationOwner<'_>, CoreError> {
        self.reserve().map(|reservation| TunnelReservationOwner {
            reservation,
            #[cfg(test)]
            _after_free: AfterFree,
            _storage: None,
        })
    }
}

/// The original reservation allocation, destroyed before its optional grant.
/// This owner is stored inline and introduces no second allocation.
pub struct TunnelReservationOwner<'a> {
    reservation: Box<dyn TunnelReservation + 'a>,
    #[cfg(test)]
    _after_free: AfterFree,
    _storage: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl<'a> TunnelReservationOwner<'a> {
    /// Adopt an already admitted original Box. The caller must acquire this
    /// exact grant before allocating the Box. A mismatch returns both original
    /// owners, in destruction order, without allocating or changing either.
    pub fn try_from_grant(
        reservation: Box<dyn TunnelReservation + 'a>,
        storage: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<
        Self,
        (
            Box<dyn TunnelReservation + 'a>,
            tokio::sync::OwnedSemaphorePermit,
        ),
    > {
        if storage.num_permits() != std::mem::size_of_val(&*reservation)
            || !reservation
                .byte_bank()
                .is_some_and(|bank| Arc::ptr_eq(&bank, storage.semaphore()))
        {
            return Err((reservation, storage));
        }
        Ok(Self {
            reservation,
            #[cfg(test)]
            _after_free: AfterFree,
            _storage: Some(storage),
        })
    }
}

impl<'a> std::ops::Deref for TunnelReservationOwner<'a> {
    type Target = dyn TunnelReservation + 'a;

    fn deref(&self) -> &Self::Target {
        &*self.reservation
    }
}

impl std::ops::DerefMut for TunnelReservationOwner<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.reservation
    }
}

/// Caller cleanup responsibility for one attempted tunnel runtime.
///
/// Dropping an unattached reservation releases its capacity. After attachment,
/// Drop must retain the original handle in the caller's cleanup owner and
/// request stop. It cannot await a native join. A successful `complete` transfers
/// that responsibility to the owner until the caller explicitly publishes or
/// retires the returned device. Neither method certifies native completion.
pub trait TunnelReservation: Send {
    /// Optional bank for application-owned tunnel runtime buffer capacity.
    /// Defaults to legacy unmetered construction. A manager returns its existing
    /// shared bank before workers start; permits follow each actual buffer owner.
    /// This does not certify a complete native heap or allocator RSS bound.
    fn byte_bank(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        None
    }

    /// Retain this exact Arc synchronously, before the first RSD probe await.
    /// An error rejects the connection; retain an attached owner for cleanup
    /// even when rejecting it. Core also requests stop on rejection.
    fn attach(&mut self, handle: Arc<TunnelHandle>) -> Result<(), CoreError>;

    /// Transfer the attached cleanup responsibility as the connection returns.
    /// The owner must still retire it if the outer call is later cancelled or
    /// rejected before publication. This is not the publication commit itself.
    fn complete(&mut self);
}

/// Retain the original reservation without a second wrapper allocation. The
/// owned initial-directory route requires the exact caller bank before IO.
#[cfg(feature = "tunnel")]
pub(super) fn reserve_initial<'a>(
    owner: Option<&'a dyn TunnelOwner>,
    bank: Option<&Arc<tokio::sync::Semaphore>>,
) -> Result<Option<TunnelReservationOwner<'a>>, CoreError> {
    let reservation = owner.map(TunnelOwner::reserve_owned).transpose()?;
    if bank.is_some_and(|required| {
        !reservation
            .as_ref()
            .and_then(|reservation| reservation.byte_bank())
            .is_some_and(|original| Arc::ptr_eq(required, &original))
    }) {
        return Err(CoreError::Xpc(
            super::existing_rsd::ExistingRsdProbeError::metadata_limit(),
        ));
    }
    Ok(reservation)
}

/// Stack-only selection for one strategy. The original reservation allocation
/// stays intact; owned bank selection cannot affect public legacy entrypoints.
#[derive(Clone, Copy)]
pub(super) struct InitialOwner<'a, P = super::activation_policy::Immediate> {
    cleanup: Option<&'a dyn TunnelOwner>,
    pub(super) bank: Option<&'a Arc<tokio::sync::Semaphore>>,
    pub(super) policy: P,
}

#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! {
    static AFTER_RESERVATION_FREE: FreeHook = const { std::cell::RefCell::new(None) };
}
// Box completes deallocation (also on unwind) before this observer is dropped,
// and the original byte grant is still held by the following field.
#[cfg(test)]
struct AfterFree;
#[cfg(test)]
impl Drop for AfterFree {
    fn drop(&mut self) {
        AFTER_RESERVATION_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
    }
}

#[cfg(test)]
mod reservation_owner_tests;
impl<'a> InitialOwner<'a> {
    pub(super) fn new(
        cleanup: Option<&'a dyn TunnelOwner>,
        bank: Option<&'a Arc<tokio::sync::Semaphore>>,
    ) -> Self {
        Self {
            cleanup,
            bank,
            policy: super::activation_policy::Immediate,
        }
    }
}
impl<'a, P> InitialOwner<'a, P> {
    pub(super) fn with_policy<Q>(self, policy: Q) -> InitialOwner<'a, Q> {
        InitialOwner {
            cleanup: self.cleanup,
            bank: self.bank,
            policy,
        }
    }
    #[cfg(feature = "tunnel")]
    pub(super) fn reserve(self) -> Result<Option<TunnelReservationOwner<'a>>, CoreError> {
        reserve_initial(self.cleanup, self.bank)
    }
}

#[cfg(all(test, feature = "tunnel"))]
mod initial_owner_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Owner {
        bank: Option<Arc<tokio::sync::Semaphore>>,
        pointer: AtomicUsize,
        dropped: AtomicUsize,
    }
    struct Reservation<'a>(&'a Owner);
    impl TunnelOwner for Owner {
        fn reserve(&self) -> Result<Box<dyn TunnelReservation + '_>, CoreError> {
            let reservation = Box::new(Reservation(self));
            self.pointer
                .store(std::ptr::from_ref(&*reservation) as usize, Ordering::SeqCst);
            Ok(reservation)
        }
    }
    impl TunnelReservation for Reservation<'_> {
        fn byte_bank(&self) -> Option<Arc<tokio::sync::Semaphore>> {
            self.0.bank.clone()
        }
        fn attach(&mut self, _: Arc<TunnelHandle>) -> Result<(), CoreError> {
            panic!("identity preflight must not attach native work")
        }
        fn complete(&mut self) {
            panic!("identity preflight must not complete native work")
        }
    }
    impl Drop for Reservation<'_> {
        fn drop(&mut self) {
            self.0.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn owned_initial_reservation_keeps_original_box_and_requires_original_bank_before_io() {
        let bank = Arc::new(tokio::sync::Semaphore::new(7));
        for matches in [true, false] {
            let original = if matches {
                bank.clone()
            } else {
                Arc::new(tokio::sync::Semaphore::new(7))
            };
            let owner = Owner {
                bank: Some(original),
                pointer: AtomicUsize::new(0),
                dropped: AtomicUsize::new(0),
            };
            let result = InitialOwner::new(Some(&owner), Some(&bank)).reserve();
            if matches {
                let reservation = result.unwrap().unwrap();
                assert_eq!(
                    std::ptr::from_ref(&*reservation) as *const () as usize,
                    owner.pointer.load(Ordering::SeqCst)
                );
                assert_eq!(owner.dropped.load(Ordering::SeqCst), 0);
                drop(reservation);
            } else {
                assert_eq!(
                    result.err().unwrap().class(),
                    crate::ErrorClass::ResourceLimit
                );
            }
            assert_eq!(owner.dropped.load(Ordering::SeqCst), 1);
            assert_eq!(bank.available_permits(), 7);
        }
        assert_eq!(
            InitialOwner::new(None, Some(&bank))
                .reserve()
                .err()
                .unwrap()
                .class(),
            crate::ErrorClass::ResourceLimit
        );
        // Legacy owners still need no bank.
        let owner = Owner {
            bank: None,
            pointer: AtomicUsize::new(0),
            dropped: AtomicUsize::new(0),
        };
        drop(InitialOwner::new(Some(&owner), None).reserve().unwrap());
        assert_eq!(owner.dropped.load(Ordering::SeqCst), 1);
    }
}

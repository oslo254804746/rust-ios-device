//! Complete private native Ledger allocation; reports retain only its identity.
#[cfg(feature = "tunnel-userspace")]
use super::TunnelError;
use super::{Identity, LedgerState};
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{fence, AtomicUsize, Ordering},
};
use tokio::sync::OwnedSemaphorePermit;

#[repr(C)]
struct Allocation {
    strong: AtomicUsize,
    state: ManuallyDrop<LedgerState>,
}

pub(crate) struct Ledger {
    pointer: NonNull<Allocation>,
    _allocation: PhantomData<Allocation>,
}
// SAFETY: the sealed root shares immutable State access; the State's original
// locks/atomics protect mutation. Only the final root moves its Unpin State.
unsafe impl Send for Ledger where LedgerState: Send + Sync {}
unsafe impl Sync for Ledger where LedgerState: Send + Sync {}

struct Destination {
    values: Vec<Allocation>,
    #[cfg(all(test, feature = "tunnel-userspace"))]
    _after_free: AfterFree,
    // Empty destination backing must end before its original whole fee.
    storage: Option<OwnedSemaphorePermit>,
}
impl Destination {
    fn unmetered() -> Self {
        Self {
            values: Vec::with_capacity(1),
            #[cfg(all(test, feature = "tunnel-userspace"))]
            _after_free: AfterFree,
            storage: None,
        }
    }
    #[cfg(feature = "tunnel-userspace")]
    fn from_grant(
        bank: &std::sync::Arc<tokio::sync::Semaphore>,
        storage: OwnedSemaphorePermit,
    ) -> Result<Self, TunnelError> {
        let (storage, values) = crate::device::prepared_rsd::exact_vec_from_grant(bank, 1, storage)
            .map_err(|source| TunnelError::Io(std::io::Error::other(source)))?;
        Ok(Self {
            values,
            #[cfg(all(test, feature = "tunnel-userspace"))]
            _after_free: AfterFree,
            storage: Some(storage),
        })
    }
    fn finish(self, identity: Identity) -> Ledger {
        let Self {
            mut values,
            storage,
            #[cfg(all(test, feature = "tunnel-userspace"))]
            _after_free,
        } = self;
        assert!(values.is_empty() && values.capacity() == 1);
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            state: ManuallyDrop::new(LedgerState::new(storage, identity)),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        #[cfg(all(test, feature = "tunnel-userspace"))]
        mem::forget(_after_free);
        Ledger {
            pointer,
            _allocation: PhantomData,
        }
    }
}
impl Ledger {
    pub(crate) fn new() -> Self {
        let destination = Destination::unmetered();
        destination.finish(Identity::new())
    }
    #[cfg(feature = "tunnel-userspace")]
    pub(crate) fn body_bytes() -> usize {
        mem::size_of::<Allocation>()
    }
    #[cfg(feature = "tunnel-userspace")]
    pub(crate) fn reservation_bytes() -> usize {
        Self::body_bytes() + Identity::body_bytes()
    }
    #[cfg(feature = "tunnel-userspace")]
    pub(crate) fn with_metadata(
        bank: Option<&std::sync::Arc<tokio::sync::Semaphore>>,
    ) -> Result<Self, TunnelError> {
        let Some(bank) = bank else {
            return Ok(Self::new());
        };
        let mut whole = super::super::tun::userspace::runtime_storage::charge(
            Some(bank),
            Self::reservation_bytes(),
        )?
        .expect("original bank and whole Ledger grant");
        let identity = whole
            .split(Identity::body_bytes())
            .expect("admitted independent provenance root");
        // Keep the actual EMPTY root and its whole fee together through the
        // remaining fallible identity allocation. No body fee moves early.
        let destination = Destination::from_grant(bank, whole)?;
        let identity = Identity::from_grant(bank, identity)?;
        Ok(destination.finish(identity))
    }
    fn allocation(&self) -> &Allocation {
        // SAFETY: a live handle retains a positive original strong count.
        unsafe { self.pointer.as_ref() }
    }
}
impl Clone for Ledger {
    fn clone(&self) -> Self {
        if self.allocation().strong.fetch_add(1, Ordering::Relaxed) >= isize::MAX as usize {
            std::process::abort();
        }
        Self {
            pointer: self.pointer,
            _allocation: PhantomData,
        }
    }
}
impl Deref for Ledger {
    type Target = LedgerState;
    fn deref(&self) -> &Self::Target {
        &self.allocation().state
    }
}
impl Drop for Ledger {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        fn require_unpin<T: Unpin>() {}
        require_unpin::<LedgerState>();
        let pointer = self.pointer.as_ptr();
        // SAFETY: the unique final root acquired prior releases, with no Weak,
        // raw owning, mutable or pin escape. Move original State once, free the
        // SAME full allocation, then end its original fields and whole fee.
        let state = unsafe { ManuallyDrop::take(&mut (*pointer).state) };
        unsafe { drop(Vec::from_raw_parts(pointer, 0, 1)) };
        #[cfg(all(test, feature = "tunnel-userspace"))]
        drop(AfterFree);
        drop(state);
    }
}

#[cfg(all(test, feature = "tunnel-userspace"))]
struct AfterFree;
#[cfg(all(test, feature = "tunnel-userspace"))]
impl Drop for AfterFree {
    fn drop(&mut self) {
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
    }
}

#[cfg(all(test, feature = "tunnel-userspace"))]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(all(test, feature = "tunnel-userspace"))]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }
#[cfg(all(test, feature = "tunnel-userspace"))]
pub(super) fn after_free<T>(hook: impl FnMut() + 'static, work: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|hook| hook.borrow_mut().take());
        }
    }
    AFTER_FREE.with(|current| {
        assert!(current.borrow().is_none());
        *current.borrow_mut() = Some(Box::new(hook));
    });
    let _reset = Reset;
    work()
}

#[cfg(all(test, feature = "tunnel-userspace"))]
mod tests {
    use super::*;
    use crate::device::prepared_rsd::{charge, with_allocation_hook};
    use std::{
        alloc::Layout,
        error::Error,
        fmt,
        sync::{atomic::AtomicBool, Arc, Barrier, Mutex},
    };
    use tokio::sync::Semaphore;

    #[test]
    fn native_ledger_root_refuses_foreign_short_long_whole_body_before_any_backing() {
        let bytes = Ledger::body_bytes();
        assert_eq!(bytes, Layout::new::<Allocation>().size());
        assert!(bytes > mem::size_of::<LedgerState>());
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let other = Arc::new(Semaphore::new(2 * bytes));
            let storage = charge(if foreign { &other } else { &bank }, amount).unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("invalid complete Ledger root grant allocated"),
                || {
                    Destination::from_grant(&bank, storage)
                        .err()
                        .expect("invalid root grant accepted")
                },
            );
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), 2 * bytes);
            assert_eq!(other.available_permits(), 2 * bytes);
        }
    }

    #[test]
    fn native_ledger_empty_root_frees_before_its_fee_when_identity_preparation_fails() {
        let bytes = Ledger::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let other = Arc::new(Semaphore::new(Identity::body_bytes()));
        let wrong = charge(&other, Identity::body_bytes()).unwrap();
        let allocations = Arc::new(AtomicUsize::new(0));
        let observed = allocations.clone();
        let observed_bank = bank.clone();
        let credit = bank.clone();
        let result = after_free(
            move || {
                assert_eq!(
                    credit.available_permits(),
                    0,
                    "empty root refunded before its actual backing ended"
                )
            },
            || {
                with_allocation_hook(
                    move |layout, _| {
                        assert_eq!(layout, Layout::new::<Allocation>());
                        assert_eq!(observed_bank.available_permits(), 0);
                        observed.fetch_add(1, Ordering::SeqCst);
                    },
                    || {
                        let destination =
                            Destination::from_grant(&bank, charge(&bank, bytes).unwrap())?;
                        let identity = Identity::from_grant(&bank, wrong)?;
                        Ok::<_, TunnelError>(destination.finish(identity))
                    },
                )
            },
        );
        assert_eq!(
            crate::classify_error(&result.err().expect("foreign identity accepted")),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(allocations.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), bytes);
        assert_eq!(other.available_permits(), Identity::body_bytes());
    }

    struct WorkProbe {
        freed: Arc<AtomicBool>,
        bank: Arc<Semaphore>,
        drops: Arc<AtomicUsize>,
    }
    impl fmt::Debug for WorkProbe {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("WorkProbe")
        }
    }
    impl fmt::Display for WorkProbe {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("original work probe")
        }
    }
    impl Error for WorkProbe {}
    impl Drop for WorkProbe {
        fn drop(&mut self) {
            assert!(
                self.freed.load(Ordering::SeqCst),
                "original cause dropped before actual Ledger backing"
            );
            assert_eq!(
                self.bank.available_permits(),
                Identity::body_bytes(),
                "Ledger body credit preceded original cause destructor"
            );
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn native_ledger_final_closed_root_keeps_state_and_original_cause_until_after_actual_free() {
        let total = Ledger::reservation_bytes();
        let bank = Arc::new(Semaphore::new(total));
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let observed = allocations.clone();
        let observed_bank = bank.clone();
        let ledger = with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(observed_bank.available_permits(), 0);
                observed.lock().unwrap().push((layout, pointer));
            },
            || Ledger::with_metadata(Some(&bank)).unwrap(),
        );
        let allocations = allocations.lock().unwrap();
        assert_eq!(allocations.len(), 2);
        assert_eq!(
            allocations[0],
            (
                Layout::new::<Allocation>(),
                ledger.pointer.as_ptr() as usize
            )
        );
        let state = (&*ledger as *const LedgerState) as usize;
        assert!((allocations[0].1..allocations[0].1 + allocations[0].0.size()).contains(&state));
        let freed = Arc::new(AtomicBool::new(false));
        let drops = Arc::new(AtomicUsize::new(0));
        let original: super::super::Cause = Arc::new(WorkProbe {
            freed: freed.clone(),
            bank: bank.clone(),
            drops: drops.clone(),
        });
        let pointer = Arc::as_ptr(&original) as *const () as usize;
        ledger.record_work(0, original);
        bank.close();
        let kept = with_allocation_hook(
            |_, _| panic!("closed Ledger clone allocated"),
            || ledger.clone(),
        );
        assert_eq!(kept.pointer, ledger.pointer);
        assert_eq!((&*kept as *const LedgerState) as usize, state);
        assert_eq!(
            Arc::as_ptr(
                &kept.outcomes.lock().unwrap().slots[0]
                    .as_ref()
                    .unwrap()
                    .cause
            ) as *const () as usize,
            pointer
        );
        drop(ledger);
        assert_eq!(bank.available_permits(), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let credit = bank.clone();
        let marker = freed.clone();
        after_free(
            move || {
                assert_eq!(
                    credit.available_permits(),
                    0,
                    "original State fee returned before complete root deallocation"
                );
                marker.store(true, Ordering::SeqCst);
            },
            || drop(kept),
        );
        assert!(freed.load(Ordering::SeqCst));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(bank.available_permits(), total);
    }

    #[test]
    fn native_ledger_parallel_last_roots_keep_original_state_and_refund_once() {
        let total = Ledger::reservation_bytes();
        let bank = Arc::new(Semaphore::new(total));
        let ledger = Ledger::with_metadata(Some(&bank)).unwrap();
        let pointer = ledger.pointer.as_ptr() as usize;
        let state = (&*ledger as *const LedgerState) as usize;
        bank.close();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..7 {
                let kept = ledger.clone();
                let barrier = barrier.clone();
                let bank = bank.clone();
                scope.spawn(move || {
                    assert_eq!(kept.pointer.as_ptr() as usize, pointer);
                    assert_eq!((&*kept as *const LedgerState) as usize, state);
                    assert_eq!(bank.available_permits(), 0);
                    barrier.wait();
                    drop(kept);
                });
            }
            barrier.wait();
            drop(ledger);
        });
        assert_eq!(bank.available_permits(), total);
    }
}

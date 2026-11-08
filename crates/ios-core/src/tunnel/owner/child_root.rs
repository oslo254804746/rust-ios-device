//! Exact native child-set root with in-place State destruction.
#[cfg(feature = "tunnel-private-ip")]
use super::TunnelError;
use super::{ChildRole, ChildState, Ledger};
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
    state: ManuallyDrop<ChildState>,
    storage: ManuallyDrop<Option<OwnedSemaphorePermit>>,
}

pub(crate) struct SharedChildTaskSet {
    pointer: NonNull<Allocation>,
    _allocation: PhantomData<Allocation>,
}
// SAFETY: only immutable State access escapes. Its original locks and atomics
// protect mutation; a unique final root drops State in its original location.
unsafe impl Send for SharedChildTaskSet where ChildState: Send + Sync {}
unsafe impl Sync for SharedChildTaskSet where ChildState: Send + Sync {}

struct Destination {
    values: Vec<Allocation>,
    // The unused actual backing ends before returning the whole body fee.
    storage: Option<OwnedSemaphorePermit>,
}
impl Destination {
    fn unmetered() -> Self {
        Self {
            values: Vec::with_capacity(1),
            storage: None,
        }
    }
    #[cfg(feature = "tunnel-private-ip")]
    fn from_grant(
        bank: &std::sync::Arc<tokio::sync::Semaphore>,
        storage: OwnedSemaphorePermit,
    ) -> Result<Self, TunnelError> {
        let (storage, values) = crate::device::prepared_rsd::exact_vec_from_grant(bank, 1, storage)
            .map_err(|source| TunnelError::Io(std::io::Error::other(source)))?;
        Ok(Self {
            values,
            storage: Some(storage),
        })
    }
    fn finish(self, ledger: Ledger, role: ChildRole) -> SharedChildTaskSet {
        // Complete State construction while the empty destination still owns
        // both its actual backing and original grant, including unwind.
        let state = ChildState::new(ledger, role);
        assert!(self.values.is_empty() && self.values.capacity() == 1);
        let Self {
            mut values,
            storage,
        } = self;
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            state: ManuallyDrop::new(state),
            storage: ManuallyDrop::new(storage),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        SharedChildTaskSet {
            pointer,
            _allocation: PhantomData,
        }
    }
}
impl SharedChildTaskSet {
    pub(crate) fn new(ledger: Ledger, role: ChildRole) -> Self {
        Destination::unmetered().finish(ledger, role)
    }
    #[cfg(feature = "tunnel-private-ip")]
    pub(crate) fn body_bytes() -> usize {
        mem::size_of::<Allocation>()
    }
    #[cfg(feature = "tunnel-private-ip")]
    pub(crate) fn with_metadata(
        ledger: Ledger,
        role: ChildRole,
        bank: Option<&std::sync::Arc<tokio::sync::Semaphore>>,
    ) -> Result<Self, TunnelError> {
        let Some(bank) = bank else {
            return Ok(Self::new(ledger, role));
        };
        let storage =
            super::super::tun::userspace::runtime_storage::charge(Some(bank), Self::body_bytes())?
                .expect("original child-set bank and whole grant");
        Ok(Destination::from_grant(bank, storage)?.finish(ledger, role))
    }
    fn allocation(&self) -> &Allocation {
        // SAFETY: each live handle retains a positive original strong count.
        unsafe { self.pointer.as_ref() }
    }
}
impl Clone for SharedChildTaskSet {
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
impl Deref for SharedChildTaskSet {
    type Target = ChildState;
    fn deref(&self) -> &Self::Target {
        &self.allocation().state
    }
}

// Stack-only deallocation owner, installed before State Drop. It also runs
// during State unwind; its original fee field ends after actual deallocation.
struct Release {
    pointer: NonNull<Allocation>,
    _storage: Option<OwnedSemaphorePermit>,
}
impl Drop for Release {
    fn drop(&mut self) {
        // SAFETY: State was dropped in place (also through its unwind glue).
        // The unique original capacity-one block contains no remaining value.
        unsafe { drop(Vec::from_raw_parts(self.pointer.as_ptr(), 0, 1)) };
        #[cfg(all(test, feature = "tunnel-private-ip"))]
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
    }
}
impl Drop for SharedChildTaskSet {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        let pointer = self.pointer.as_ptr();
        // SAFETY: final release acquired every prior owner; no Weak/raw/pin or
        // mutable owning escape exists. Move only the permit, never State.
        let storage = unsafe { ManuallyDrop::take(&mut (*pointer).storage) };
        let _release = Release {
            pointer: self.pointer,
            _storage: storage,
        };
        unsafe { ManuallyDrop::drop(&mut (*pointer).state) };
    }
}

#[cfg(all(test, feature = "tunnel-private-ip"))]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(all(test, feature = "tunnel-private-ip"))]
type StateHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(all(test, feature = "tunnel-private-ip"))]
thread_local! {
    static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) };
    static STATE_DROP: StateHook = const { std::cell::RefCell::new(None) };
}
#[cfg(all(test, feature = "tunnel-private-ip"))]
impl Drop for ChildState {
    fn drop(&mut self) {
        STATE_DROP.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(self as *mut Self as usize);
            }
        });
    }
}
#[cfg(all(test, feature = "tunnel-private-ip"))]
fn observe_drop<T>(
    state: impl FnMut(usize) + 'static,
    free: impl FnMut() + 'static,
    work: impl FnOnce() -> T,
) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            STATE_DROP.with(|hook| hook.borrow_mut().take());
            AFTER_FREE.with(|hook| hook.borrow_mut().take());
        }
    }
    STATE_DROP.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(state));
    });
    AFTER_FREE.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(free));
    });
    let _reset = Reset;
    work()
}

#[cfg(all(test, feature = "tunnel-private-ip"))]
mod tests {
    use super::*;
    use crate::device::prepared_rsd::{charge, with_allocation_hook};
    use std::{
        alloc::Layout,
        panic::{catch_unwind, AssertUnwindSafe},
        sync::{Arc, Barrier, Mutex},
    };
    use tokio::sync::Semaphore;

    #[test]
    fn native_child_root_whole_original_grant_precedes_any_actual_backing() {
        let bytes = SharedChildTaskSet::body_bytes();
        assert_eq!(bytes, Layout::new::<Allocation>().size());
        assert!(bytes > mem::size_of::<ChildState>());
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let other = Arc::new(Semaphore::new(2 * bytes));
            let grant = charge(if foreign { &other } else { &bank }, amount).unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("invalid child root grant allocated"),
                || {
                    Destination::from_grant(&bank, grant)
                        .err()
                        .expect("invalid grant accepted")
                },
            );
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), 2 * bytes);
            assert_eq!(other.available_permits(), 2 * bytes);
        }
        let bank = Arc::new(Semaphore::new(bytes - 1));
        let ledger = Ledger::new();
        let error = with_allocation_hook(
            |_, _| panic!("short child whole grant allocated"),
            || {
                SharedChildTaskSet::with_metadata(
                    ledger.clone(),
                    ChildRole::ProxyClient,
                    Some(&bank),
                )
                .err()
                .expect("short root accepted")
            },
        );
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), bytes - 1);
        assert_eq!(ledger.pending.load(Ordering::Acquire), 0);
    }

    struct StatePanic(Arc<()>);
    #[test]
    fn native_child_last_closed_root_drops_original_state_in_place_then_frees_before_credit_even_on_unwind(
    ) {
        for panic_state in [false, true] {
            let bytes = SharedChildTaskSet::body_bytes();
            let bank = Arc::new(Semaphore::new(bytes));
            let ledger_bytes = Ledger::reservation_bytes();
            let ledger_bank = Arc::new(Semaphore::new(ledger_bytes));
            let ledger = Ledger::with_metadata(Some(&ledger_bank)).unwrap();
            let identity = ledger.identity.clone();
            let allocations = Arc::new(Mutex::new(Vec::new()));
            let observed = allocations.clone();
            let observed_bank = bank.clone();
            let child = with_allocation_hook(
                move |layout, pointer| {
                    assert_eq!(observed_bank.available_permits(), 0);
                    observed.lock().unwrap().push((layout, pointer));
                },
                || {
                    SharedChildTaskSet::with_metadata(ledger, ChildRole::ProxyClient, Some(&bank))
                        .unwrap()
                },
            );
            let root = child.pointer.as_ptr() as usize;
            let state = (&*child as *const ChildState) as usize;
            assert_eq!(
                &*allocations.lock().unwrap(),
                &[(Layout::new::<Allocation>(), root)]
            );
            assert!((root..root + bytes).contains(&state));
            bank.close();
            let kept = with_allocation_hook(
                |_, _| panic!("closed child clone allocated"),
                || child.clone(),
            );
            assert_eq!(kept.pointer, child.pointer);
            drop(child);
            assert_eq!(bank.available_permits(), 0);
            let marker = Arc::new(());
            let panic_marker = marker.clone();
            let stage = Arc::new(AtomicUsize::new(0));
            let drop_stage = stage.clone();
            let free_stage = stage.clone();
            let state_bank = bank.clone();
            let state_ledger_bank = ledger_bank.clone();
            let free_bank = bank.clone();
            let free_ledger_bank = ledger_bank.clone();
            let result = observe_drop(
                move |pointer| {
                    assert_eq!(pointer, state, "State moved out of its original allocation");
                    assert_eq!(state_bank.available_permits(), 0);
                    assert_eq!(state_ledger_bank.available_permits(), 0);
                    assert_eq!(drop_stage.fetch_add(1, Ordering::SeqCst), 0);
                    if panic_state {
                        std::panic::panic_any(StatePanic(panic_marker.clone()));
                    }
                },
                move || {
                    assert_eq!(free_stage.fetch_add(1, Ordering::SeqCst), 1);
                    assert_eq!(
                        free_bank.available_permits(),
                        0,
                        "child credit preceded actual free"
                    );
                    assert_eq!(
                        free_ledger_bank.available_permits(),
                        Ledger::body_bytes(),
                        "State fields had not finished before allocation deallocation"
                    );
                },
                || catch_unwind(AssertUnwindSafe(|| drop(kept))),
            );
            assert_eq!(stage.load(Ordering::SeqCst), 2);
            if panic_state {
                let payload = result
                    .expect_err("original State panic lost")
                    .downcast::<StatePanic>()
                    .unwrap_or_else(|_| panic!("State panic type changed"));
                assert!(Arc::ptr_eq(&payload.0, &marker));
            } else {
                result.unwrap();
            }
            assert_eq!(bank.available_permits(), bytes);
            assert_eq!(ledger_bank.available_permits(), Ledger::body_bytes());
            drop(identity);
            assert_eq!(ledger_bank.available_permits(), ledger_bytes);
        }
    }

    #[test]
    fn native_child_parallel_last_clones_keep_original_state_and_refund_once() {
        let bytes = SharedChildTaskSet::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let child =
            SharedChildTaskSet::with_metadata(Ledger::new(), ChildRole::ProxyClient, Some(&bank))
                .unwrap();
        let root = child.pointer.as_ptr() as usize;
        let state = (&*child as *const ChildState) as usize;
        bank.close();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..7 {
                let kept = child.clone();
                let barrier = barrier.clone();
                let bank = bank.clone();
                scope.spawn(move || {
                    assert_eq!(kept.pointer.as_ptr() as usize, root);
                    assert_eq!((&*kept as *const ChildState) as usize, state);
                    assert_eq!(bank.available_permits(), 0);
                    barrier.wait();
                    drop(kept);
                });
            }
            barrier.wait();
            drop(child);
        });
        assert_eq!(bank.available_permits(), bytes);
    }
}

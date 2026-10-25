//! Independent native-report provenance in a complete declared allocation.
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ptr::NonNull,
    sync::atomic::{fence, AtomicUsize, Ordering},
};
use tokio::sync::OwnedSemaphorePermit;

#[repr(C)]
struct Allocation {
    strong: AtomicUsize,
    storage: ManuallyDrop<Option<OwnedSemaphorePermit>>,
}

pub(super) struct Identity {
    pointer: NonNull<Allocation>,
    _allocation: PhantomData<Allocation>,
}
// SAFETY: the sealed immutable identity shares only an atomic strong count.
// The unique final owner frees its allocation before releasing its fee.
unsafe impl Send for Identity where Allocation: Send + Sync {}
unsafe impl Sync for Identity where Allocation: Send + Sync {}

impl Identity {
    pub(super) fn new() -> Self {
        // Preserve the original unmetered constructor for owners without a bank.
        Self::place(Vec::with_capacity(1), None)
    }
    #[cfg(feature = "tunnel-private-ip")]
    pub(super) fn body_bytes() -> usize {
        mem::size_of::<Allocation>()
    }
    #[cfg(feature = "tunnel-private-ip")]
    pub(super) fn from_grant(
        bank: &std::sync::Arc<tokio::sync::Semaphore>,
        storage: OwnedSemaphorePermit,
    ) -> Result<Self, super::TunnelError> {
        let (storage, values) = crate::device::prepared_rsd::exact_vec_from_grant(bank, 1, storage)
            .map_err(|source| super::TunnelError::Io(std::io::Error::other(source)))?;
        Ok(Self::place(values, Some(storage)))
    }
    fn place(mut values: Vec<Allocation>, storage: Option<OwnedSemaphorePermit>) -> Self {
        assert!(values.is_empty() && values.capacity() == 1);
        values.push(Allocation {
            strong: AtomicUsize::new(1),
            storage: ManuallyDrop::new(storage),
        });
        let pointer = NonNull::from(&mut values[0]);
        mem::forget(values);
        Self {
            pointer,
            _allocation: PhantomData,
        }
    }
    fn allocation(&self) -> &Allocation {
        // SAFETY: each live identity retains a positive original strong count.
        unsafe { self.pointer.as_ref() }
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
}
impl Clone for Identity {
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
impl Drop for Identity {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        let pointer = self.pointer.as_ptr();
        // SAFETY: the final count acquired every prior release, with no Weak,
        // pin, raw ownership or mutation exports. Move the original permit
        // once, then free the SAME entire allocation before its byte credit.
        let storage = unsafe { ManuallyDrop::take(&mut (*pointer).storage) };
        unsafe { drop(Vec::from_raw_parts(pointer, 0, 1)) };
        #[cfg(all(test, feature = "tunnel-private-ip"))]
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
        drop(storage);
    }
}

#[cfg(all(test, feature = "tunnel-private-ip"))]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(all(test, feature = "tunnel-private-ip"))]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }

#[cfg(all(test, feature = "tunnel-private-ip"))]
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

#[cfg(all(test, feature = "tunnel-private-ip"))]
mod tests {
    use super::*;
    use crate::device::prepared_rsd::{charge, with_allocation_hook};
    use std::{
        alloc::Layout,
        sync::{Arc, Barrier, Mutex},
    };
    use tokio::sync::Semaphore;

    #[test]
    fn native_identity_exact_whole_grant_precedes_backing_and_actual_free_precedes_credit() {
        let bytes = Identity::body_bytes();
        assert_eq!(bytes, Layout::new::<Allocation>().size());
        for (foreign, amount) in [(true, bytes), (false, bytes - 1), (false, bytes + 1)] {
            let bank = Arc::new(Semaphore::new(2 * bytes));
            let other = Arc::new(Semaphore::new(2 * bytes));
            let storage = charge(if foreign { &other } else { &bank }, amount).unwrap();
            let error = with_allocation_hook(
                |_, _| panic!("invalid identity grant allocated a backing"),
                || {
                    Identity::from_grant(&bank, storage)
                        .err()
                        .expect("invalid identity grant accepted")
                },
            );
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(bank.available_permits(), 2 * bytes);
            assert_eq!(other.available_permits(), 2 * bytes);
        }
        let bank = Arc::new(Semaphore::new(bytes));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = seen.clone();
        let observed_bank = bank.clone();
        let identity = with_allocation_hook(
            move |layout, pointer| {
                assert_eq!(observed_bank.available_permits(), 0);
                observed.lock().unwrap().push((layout, pointer));
            },
            || Identity::from_grant(&bank, charge(&bank, bytes).unwrap()).unwrap(),
        );
        assert_eq!(
            &*seen.lock().unwrap(),
            &[(
                Layout::new::<Allocation>(),
                identity.pointer.as_ptr() as usize
            )]
        );
        bank.close();
        let kept = with_allocation_hook(
            |_, _| panic!("identity clone allocated a backing"),
            || identity.clone(),
        );
        assert!(identity.ptr_eq(&kept));
        drop(identity);
        assert_eq!(bank.available_permits(), 0);
        let observed_bank = bank.clone();
        after_free(
            move || {
                assert_eq!(
                    observed_bank.available_permits(),
                    0,
                    "identity credit preceded actual deallocation"
                )
            },
            || drop(kept),
        );
        assert_eq!(bank.available_permits(), bytes);
    }

    #[test]
    fn native_identity_parallel_last_clones_keep_original_provenance_and_refund_once() {
        let bytes = Identity::body_bytes();
        let bank = Arc::new(Semaphore::new(bytes));
        let identity = Identity::from_grant(&bank, charge(&bank, bytes).unwrap()).unwrap();
        let pointer = identity.pointer.as_ptr() as usize;
        bank.close();
        let barrier = Arc::new(Barrier::new(8));
        std::thread::scope(|scope| {
            for _ in 0..7 {
                let kept = identity.clone();
                let barrier = barrier.clone();
                let bank = bank.clone();
                scope.spawn(move || {
                    assert_eq!(kept.pointer.as_ptr() as usize, pointer);
                    assert_eq!(bank.available_permits(), 0);
                    barrier.wait();
                    drop(kept);
                });
            }
            barrier.wait();
            drop(identity);
        });
        assert_eq!(bank.available_permits(), bytes);
    }
}

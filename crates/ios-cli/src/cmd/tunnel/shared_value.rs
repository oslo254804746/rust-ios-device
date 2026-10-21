//! Strong-only unpinned values in an exact application-declared allocation.
//! The caller supplies an admitted empty backing and moves its original fee
//! into T. Free the entire backing before final T (and its fee) Drop.
use std::{
    marker::PhantomData,
    mem::{self, ManuallyDrop},
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{fence, AtomicUsize, Ordering},
};

#[repr(C)]
pub(super) struct Allocation<T> {
    strong: AtomicUsize,
    value: ManuallyDrop<T>,
}

pub(super) struct Shared<T: Unpin> {
    pointer: NonNull<Allocation<T>>,
    _allocation: PhantomData<Allocation<T>>,
}
// SAFETY: sealed immutable roots count atomically, as Arc does. There is no
// Weak/raw/mutation/pinning escape; only the last root moves the unpinned T.
unsafe impl<T: Send + Sync + Unpin> Send for Shared<T> {}
unsafe impl<T: Send + Sync + Unpin> Sync for Shared<T> {}

impl<T: Unpin> Shared<T> {
    pub(super) fn body_bytes() -> usize {
        mem::size_of::<Allocation<T>>()
    }
    /// The original capacity-one empty backing is already paid. T owns its
    /// original body grant before publication; this operation cannot allocate.
    pub(super) fn place(value: T, mut backing: Vec<Allocation<T>>) -> Self {
        assert_eq!(backing.len(), 0);
        assert_eq!(backing.capacity(), 1);
        backing.push(Allocation {
            strong: AtomicUsize::new(1),
            value: ManuallyDrop::new(value),
        });
        let pointer = NonNull::from(&mut backing[0]);
        mem::forget(backing);
        Self {
            pointer,
            _allocation: PhantomData,
        }
    }
    fn allocation(&self) -> &Allocation<T> {
        // SAFETY: a live root holds a positive count at the original address.
        unsafe { self.pointer.as_ref() }
    }
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        self.pointer == other.pointer
    }
}
impl<T: Unpin> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        self
    }
}
impl<T: Unpin + std::fmt::Debug> std::fmt::Debug for Shared<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, formatter)
    }
}
impl<T: Unpin + std::fmt::Display> std::fmt::Display for Shared<T> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&**self, formatter)
    }
}
impl<T: Unpin> Clone for Shared<T> {
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
impl<T: Unpin> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.allocation().value
    }
}
impl<T: Unpin> Drop for Shared<T> {
    fn drop(&mut self) {
        if self.allocation().strong.fetch_sub(1, Ordering::Release) != 1 {
            return;
        }
        fence(Ordering::Acquire);
        let pointer = self.pointer.as_ptr();
        // SAFETY: the final count acquired every preceding release. No borrow
        // or pinned/weak/raw owner can survive this sealed final root. T is
        // Unpin and moves once; then the empty Vec frees the SAME full layout.
        let value = unsafe { ManuallyDrop::take(&mut (*pointer).value) };
        unsafe { drop(Vec::from_raw_parts(pointer, 0, 1)) };
        #[cfg(test)]
        AFTER_FREE.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback();
            }
        });
        drop(value);
    }
}

#[cfg(test)]
type FreeHook = std::cell::RefCell<Option<Box<dyn FnMut()>>>;
#[cfg(test)]
thread_local! { static AFTER_FREE: FreeHook = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn after_free<R>(hook: impl FnMut() + 'static, work: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            AFTER_FREE.with(|pending| pending.borrow_mut().take());
        }
    }
    AFTER_FREE.with(|pending| {
        assert!(pending.borrow().is_none());
        *pending.borrow_mut() = Some(Box::new(hook));
    });
    let _reset = Reset;
    work()
}

#[cfg(test)]
mod tests;

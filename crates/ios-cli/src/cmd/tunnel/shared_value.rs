//! Strong-only unpinned values: free Arc allocation before final value Drop.
//! No raw Arc, Weak or pinning interface escapes this sealed wrapper. Arc's
//! private header/layout cost is a separate boundary from payload ownership.
use std::ops::Deref;
use std::sync::Arc;

pub(super) struct Shared<T: Unpin> {
    // Always Some while live; None is only the consuming Drop move state.
    inner: Option<Arc<T>>,
}
impl<T: Unpin> Shared<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            inner: Some(Arc::new(value)),
        }
    }
    #[cfg(test)]
    pub(super) fn get_mut(&mut self) -> Option<&mut T> {
        Arc::get_mut(self.inner.as_mut().expect("live shared value"))
    }
    #[cfg(test)]
    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(
            self.inner.as_ref().expect("live shared value"),
            other.inner.as_ref().expect("live shared value"),
        )
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
        Self {
            inner: Some(Arc::clone(self.inner.as_ref().expect("live shared value"))),
        }
    }
}
impl<T: Unpin> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.inner.as_deref().expect("live shared value")
    }
}
impl<T: Unpin> Drop for Shared<T> {
    fn drop(&mut self) {
        let original = self.inner.take().expect("shared value consumed once");
        // Every clone takes this path; concurrent drops cannot all lose T as
        // try_unwrap(...).ok() could. With no explicit Weak, into_inner frees
        // the original allocation before returning the final unpinned value.
        let value = Arc::into_inner(original);
        drop(value);
    }
}

#[cfg(test)]
mod tests;

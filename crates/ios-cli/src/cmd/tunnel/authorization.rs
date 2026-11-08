//! Actual Authorization bytes retain their grant through the last header clone.
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::http::HeaderValue;
use bytes::Bytes;
use tokio::sync::Semaphore;
use zeroize::Zeroize;

use super::metadata_vec::{exact_backing, AdmittedVec};

struct Payload(AdmittedVec<u8>);
impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        &self.0.values
    }
}
impl Drop for Payload {
    fn drop(&mut self) {
        // Exact len == capacity: erase the entire admitted secret destination
        // before AdmittedVec deallocates its bytes and returns the grant.
        self.0.values.as_mut_slice().zeroize();
        #[cfg(test)]
        ZEROIZED.with(|hook| {
            if let Some(hook) = hook.borrow_mut().as_mut() {
                hook(&self.0.values);
            }
        });
    }
}

pub(super) fn header(token: &str, bank: &Arc<Semaphore>) -> Result<HeaderValue> {
    const PREFIX: &[u8] = b"Bearer ";
    let length = PREFIX.len().checked_add(token.len()).ok_or_else(|| {
        crate::output::resource_decision("upstream authorization layout overflow")
    })?;
    let mut payload = Payload(exact_backing(bank, length)?);
    payload.0.values.extend_from_slice(PREFIX);
    payload.0.values.extend_from_slice(token.as_bytes());
    // Locked http 1.4.0 preserves Bytes directly; bytes 1.11.1 pins this owner
    // without copying its slice. Private Bytes owner/header storage is separate.
    let mut header = HeaderValue::from_maybe_shared(Bytes::from_owner(payload))
        .context("invalid upstream credential header")?;
    header.set_sensitive(true);
    Ok(header)
}

#[cfg(test)]
type ZeroizedHook = std::cell::RefCell<Option<Box<dyn FnMut(&[u8])>>>;
#[cfg(test)]
thread_local! {
    static ZEROIZED: ZeroizedHook = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct HookGuard;
#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        ZEROIZED.with(|hook| hook.borrow_mut().take());
    }
}
#[cfg(test)]
fn on_zeroized(hook: impl FnMut(&[u8]) + 'static) -> HookGuard {
    ZEROIZED.with(|current| {
        let mut current = current.borrow_mut();
        assert!(current.is_none());
        *current = Some(Box::new(hook));
    });
    HookGuard
}

#[cfg(test)]
mod tests;

//! Exact requested storage for the checked current-RSD path.
//! Fees precede allocation and follow the last Bytes clone/slice owner.
//! This is not an allocator/RSS cap; Bytes' private owner Box is MISSING.
use bytes::{Bytes, BytesMut};
use std::{
    alloc::{alloc, Layout},
    io,
    ops::Deref,
    ptr::NonNull,
    sync::Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

fn limit() -> io::Error {
    #[cfg(feature = "tunnel")]
    {
        let crate::XpcError::Io(error) =
            crate::device::existing_rsd::ExistingRsdProbeError::metadata_limit()
        else {
            unreachable!("metadata admission is a typed IO error")
        };
        error
    }
    #[cfg(not(feature = "tunnel"))]
    io::Error::other("raw storage admission limit")
}

fn charge(bank: &Arc<Semaphore>, bytes: usize) -> io::Result<OwnedSemaphorePermit> {
    let permits = u32::try_from(bytes).map_err(|_| limit())?;
    bank.clone()
        .try_acquire_many_owned(permits)
        .map_err(|_| limit())
}

pub(super) struct BoxedOperation<F> {
    future: std::pin::Pin<Box<F>>,
    _fee: Option<OwnedSemaphorePermit>,
}

pub(super) fn boxed<F>(
    operation: F,
    bank: Option<&Arc<Semaphore>>,
) -> io::Result<BoxedOperation<F>> {
    // This is the existing Box allocation, with its concrete payload admitted
    // before allocation. No task, timer, or detached operation is introduced.
    let fee = bank
        .map(|bank| charge(bank, std::mem::size_of_val(&operation)))
        .transpose()?;
    Ok(BoxedOperation {
        future: Box::pin(operation),
        _fee: fee,
    })
}

impl<F: std::future::Future> std::future::Future for BoxedOperation<F> {
    type Output = F::Output;
    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        self.get_mut().future.as_mut().poll(cx)
    }
}

#[cfg(test)]
type PayloadAdmissionHook = std::cell::RefCell<Option<Box<dyn FnMut(usize)>>>;
#[cfg(test)]
thread_local! {
    pub(super) static PAYLOAD_ADMISSION: PayloadAdmissionHook = const { std::cell::RefCell::new(None) };
}

pub(crate) fn exact_vec<T>(
    bank: &Arc<Semaphore>,
    capacity: usize,
) -> io::Result<(OwnedSemaphorePermit, Vec<T>)> {
    let layout = Layout::array::<T>(capacity).map_err(|_| limit())?;
    let fee = charge(bank, layout.size())?;
    if layout.size() == 0 {
        return Ok((fee, Vec::with_capacity(capacity)));
    }
    // SAFETY: exactly T's array layout, capacity and alignment. No initialized
    // element is claimed. Vec deallocates this same requested layout.
    let pointer = NonNull::new(unsafe { alloc(layout) }.cast::<T>()).ok_or_else(limit)?;
    Ok((fee, unsafe {
        Vec::from_raw_parts(pointer.as_ptr(), 0, capacity)
    }))
}

pub(crate) struct OwnerFees {
    _data: OwnedSemaphorePermit,
}

struct ExactBytes {
    bytes: Vec<u8>,
    // Declared after bytes: requested data deallocates before permits release.
    _fees: OwnerFees,
}
impl AsRef<[u8]> for ExactBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

pub(crate) struct MutableBytes {
    pub(crate) bytes: Vec<u8>,
    fees: OwnerFees,
}
impl MutableBytes {
    pub(crate) fn zeroed(bank: &Arc<Semaphore>, len: usize) -> io::Result<Self> {
        #[cfg(test)]
        PAYLOAD_ADMISSION.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(len);
            }
        });
        let (data_fee, mut bytes) = exact_vec(bank, len)?;
        bytes.resize(len, 0);
        Ok(Self {
            bytes,
            fees: OwnerFees { _data: data_fee },
        })
    }
    pub(crate) fn freeze(self) -> Bytes {
        Bytes::from_owner(ExactBytes {
            bytes: self.bytes,
            _fees: self.fees,
        })
    }
    pub(crate) fn into_parts(self) -> (OwnerFees, Vec<u8>) {
        (self.fees, self.bytes)
    }
}

pub(crate) fn freeze_parts(bytes: Vec<u8>, fees: Option<OwnerFees>) -> Bytes {
    match fees {
        Some(fees) => Bytes::from_owner(ExactBytes { bytes, _fees: fees }),
        None => Bytes::from(bytes),
    }
}

/// Legacy BytesMut semantics remain on unbanked paths. Banked grow allocates
/// an exact, explicitly selected geometric capacity under old/new grants.
/// Returned split slices retain their whole allocation grant until final drop.
pub(crate) enum RawBuffer {
    Legacy(BytesMut),
    Banked {
        owned: Option<MutableBytes>,
        shared: Bytes,
        bank: Arc<Semaphore>,
    },
}
impl Default for RawBuffer {
    fn default() -> Self {
        Self::Legacy(BytesMut::new())
    }
}
impl Deref for RawBuffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Legacy(bytes) => bytes,
            Self::Banked { owned, shared, .. } => {
                owned.as_ref().map_or(&shared[..], |bytes| &bytes.bytes[..])
            }
        }
    }
}
impl RawBuffer {
    pub(crate) fn len(&self) -> usize {
        self.deref().len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.deref().is_empty()
    }
    pub(crate) fn new(bank: Option<Arc<Semaphore>>) -> Self {
        match bank {
            Some(bank) => Self::Banked {
                owned: None,
                shared: Bytes::new(),
                bank,
            },
            None => Self::default(),
        }
    }
    pub(crate) fn clear(&mut self) {
        match self {
            Self::Legacy(bytes) => bytes.clear(),
            Self::Banked { owned, shared, .. } => {
                *owned = None;
                *shared = Bytes::new();
            }
        }
    }
    pub(crate) fn split(&mut self) -> Bytes {
        self.split_to(self.len())
    }
    pub(crate) fn split_to(&mut self, n: usize) -> Bytes {
        match self {
            Self::Legacy(bytes) => bytes.split_to(n).freeze(),
            Self::Banked { owned, shared, .. } => {
                if let Some(bytes) = owned.take() {
                    *shared = bytes.freeze();
                }
                shared.split_to(n)
            }
        }
    }
    pub(crate) fn extend_from_slice(&mut self, extra: &[u8]) -> io::Result<()> {
        if extra.is_empty() {
            return Ok(());
        }
        match self {
            Self::Legacy(bytes) => bytes.extend_from_slice(extra),
            Self::Banked {
                owned,
                shared,
                bank,
            } => {
                let current = owned.as_ref().map_or(&shared[..], |bytes| &bytes.bytes[..]);
                let len = current.len().checked_add(extra.len()).ok_or_else(limit)?;
                if owned
                    .as_ref()
                    .is_some_and(|bytes| bytes.bytes.capacity() >= len)
                {
                    // Capacity was admitted before its original exact allocation.
                    owned
                        .as_mut()
                        .expect("owned capacity")
                        .bytes
                        .extend_from_slice(extra);
                    return Ok(());
                }
                let capacity = len.checked_next_power_of_two().ok_or_else(limit)?;
                let mut replacement = MutableBytes::zeroed(bank, capacity)?;
                replacement.bytes[..current.len()].copy_from_slice(current);
                replacement.bytes[current.len()..len].copy_from_slice(extra);
                replacement.bytes.truncate(len);
                *owned = Some(replacement);
                *shared = Bytes::new();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "raw_storage/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "raw_storage/future_tests.rs"]
mod future_tests;

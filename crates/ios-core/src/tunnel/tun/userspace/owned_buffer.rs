//! Application-owned userspace receive storage, from creation to final owner.
//!
//! This fee covers the reported Vec capacity only. Queue backing, allocator
//! overhead, runtime tasks and dependency-private storage have separate owners.

use std::collections::TryReserveError;
use std::num::TryFromIntError;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::TunnelError;

#[derive(Debug)]
pub(crate) enum BufferAdmission {
    Exhausted,
    Overflow(TryFromIntError),
    UnexpectedCapacity,
    Allocation(TryReserveError),
}

impl std::fmt::Display for BufferAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("userspace receive buffer admission failed")
    }
}

impl std::error::Error for BufferAdmission {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Overflow(original) => Some(original),
            Self::Allocation(original) => Some(original),
            _ => None,
        }
    }
}

fn rejected(error: BufferAdmission) -> TunnelError {
    TunnelError::Io(std::io::Error::other(error))
}

/// Non-Clone: one buffer moves from sender to queue, device, token and closure.
/// Exposes slices only; a caller cannot grow the Vec without another admission.
pub(super) struct OwnedBuffer {
    bytes: Vec<u8>,
    // Declared after bytes: release the actual Vec before its capacity fee.
    _charge: Option<OwnedSemaphorePermit>,
}

impl OwnedBuffer {
    pub(super) fn zeroed(len: usize, bank: Option<&Arc<Semaphore>>) -> Result<Self, TunnelError> {
        // Reserve the requested capacity before the allocation or payload read.
        // Refuse an allocator's larger reported capacity before using it; do not
        // obtain a packet and attempt an unreliable later quota top-up.
        let charge = bank
            .map(|bank| {
                let bytes = u32::try_from(len)
                    .map_err(|original| rejected(BufferAdmission::Overflow(original)))?;
                Arc::clone(bank)
                    .try_acquire_many_owned(bytes)
                    .map_err(|_| rejected(BufferAdmission::Exhausted))
            })
            .transpose()?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|original| rejected(BufferAdmission::Allocation(original)))?;
        if charge
            .as_ref()
            .is_some_and(|charge| bytes.capacity() > charge.num_permits())
        {
            return Err(rejected(BufferAdmission::UnexpectedCapacity));
        }
        let charge = charge.map(|mut charge| {
            let exact = charge
                .split(bytes.capacity())
                .expect("pre-admitted actual byte capacity");
            drop(charge);
            exact
        });
        bytes.resize(len, 0);
        Ok(Self {
            bytes,
            _charge: charge,
        })
    }

    #[cfg(test)]
    pub(super) fn capacity(&self) -> usize {
        self.bytes.capacity()
    }
}

impl Deref for OwnedBuffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl DerefMut for OwnedBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receive_actual_capacity_is_prepaid_and_one_byte_short_preserves_bank() {
        let bank = Arc::new(Semaphore::new(4096));
        let packet = OwnedBuffer::zeroed(4096, Some(&bank)).unwrap();
        assert_eq!(packet.capacity(), 4096);
        assert_eq!(bank.available_permits(), 0);
        drop(packet);
        assert_eq!(bank.available_permits(), 4096);
        let bank = Arc::new(Semaphore::new(4095));
        let error = OwnedBuffer::zeroed(4096, Some(&bank)).err().unwrap();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(bank.available_permits(), 4095);
        let TunnelError::Io(error) = error else {
            panic!("typed receive admission error lost");
        };
        assert!(matches!(
            error.get_ref().unwrap().downcast_ref::<BufferAdmission>(),
            Some(BufferAdmission::Exhausted)
        ));
    }

    #[test]
    fn receive_oversized_count_preserves_original_conversion_and_has_no_charge() {
        // Only a host wider than u32 can represent this request. No allocation.
        if let Some(len) = (u32::MAX as usize).checked_add(1) {
            let bank = Arc::new(Semaphore::new(1));
            let error = OwnedBuffer::zeroed(len, Some(&bank)).err().unwrap();
            assert_eq!(bank.available_permits(), 1);
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            let TunnelError::Io(error) = error else {
                panic!("typed receive overflow error lost");
            };
            let original = error.get_ref().unwrap();
            assert!(matches!(
                original.downcast_ref::<BufferAdmission>(),
                Some(BufferAdmission::Overflow(_))
            ));
            assert!(std::error::Error::source(original)
                .unwrap()
                .is::<TryFromIntError>());
        }
    }
}

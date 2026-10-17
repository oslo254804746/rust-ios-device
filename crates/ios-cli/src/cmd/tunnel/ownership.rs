//! One Source call's reservations and handoff before native RSD probing.

use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use ios_core::{CoreError, TunnelHandle, TunnelOwner, TunnelReservation};

use super::lifecycle::Ticket;
use super::retirement::{NativeLease, OwnershipStamp, Registry, Reservation};

/// Applied only before the original Source future is created. Keep the original
/// anyhow allocation/downcast/source when reservation admission is refused.
#[derive(Debug)]
pub(super) struct Admission;
impl std::fmt::Display for Admission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel Source ownership was not admitted")
    }
}

pub(super) fn before_source(error: anyhow::Error) -> anyhow::Error {
    error.context(Admission)
}

pub(super) struct Ownership<'a> {
    registry: Arc<Registry>,
    ticket: Option<&'a Ticket>,
    stamp: OwnershipStamp,
    initial: StdMutex<Option<Reservation>>,
    completed: StdMutex<Option<NativeLease>>,
    input: Option<super::candidate_input::Owner>,
}

impl<'a> Ownership<'a> {
    #[cfg(test)]
    pub(super) fn admitted(registry: Arc<Registry>, ticket: &'a Ticket) -> Result<Self> {
        Self::admitted_with_input(registry, ticket, None)
    }

    pub(super) fn admitted_with_input(
        registry: Arc<Registry>,
        ticket: &'a Ticket,
        input: Option<super::candidate_input::Owner>,
    ) -> Result<Self> {
        let stamp = ticket.ownership_stamp();
        let reservation = ticket
            .commit(|| registry.reserve_with_input(stamp.clone(), input.clone()))
            .map_err(before_source)?;
        Ok(Self {
            registry,
            ticket: Some(ticket),
            stamp,
            initial: StdMutex::new(Some(reservation)),
            completed: StdMutex::new(None),
            input,
        })
    }

    pub(super) fn standalone(registry: Arc<Registry>) -> Result<Self> {
        let stamp = OwnershipStamp::new(Arc::new(()), Arc::new(()));
        let reservation = registry.reserve(stamp.clone())?;
        Ok(Self {
            registry,
            ticket: None,
            stamp,
            initial: StdMutex::new(Some(reservation)),
            completed: StdMutex::new(None),
            input: None,
        })
    }

    /// Select the exact successfully returned runtime. A rejected/missing
    /// handoff retires the completed lease; original Arcs remain in the registry.
    pub(super) fn take_completed(&self, handle: &Arc<TunnelHandle>) -> Result<NativeLease> {
        let lease = self
            .completed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        match lease {
            Some(lease) if lease.matches_runtime(handle) => Ok(lease),
            _ => {
                Err(super::lifecycle::cancelled()
                    .context("native tunnel ownership handoff missing"))
            }
        }
    }

    pub(super) fn stamp(&self) -> &OwnershipStamp {
        &self.stamp
    }
}

fn core_error(error: anyhow::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(error.into_boxed_dyn_error()))
}

impl TunnelOwner for Ownership<'_> {
    fn reserve(&self) -> std::result::Result<Box<dyn TunnelReservation + '_>, CoreError> {
        let allocate = || {
            let initial = self
                .initial
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            match initial {
                Some(initial) => Ok(initial),
                None => self
                    .registry
                    .reserve_with_input(self.stamp.clone(), self.input.clone()),
            }
        };
        // Every additional RemotePairing target is fenced independently. A
        // stale Source call cannot reserve a fresh slot after epoch retirement.
        let reservation = match self.ticket {
            Some(ticket) => ticket.commit(allocate),
            None => allocate(),
        }
        .map_err(core_error)?;
        reservation.start().map_err(core_error)?;
        Ok(Box::new(Attempt {
            scope: self,
            reservation: Some(reservation),
            lease: None,
        }))
    }
}

struct Attempt<'a, 'ticket> {
    scope: &'a Ownership<'ticket>,
    reservation: Option<Reservation>,
    lease: Option<NativeLease>,
}

impl TunnelReservation for Attempt<'_, '_> {
    fn byte_bank(&self) -> Option<Arc<tokio::sync::Semaphore>> {
        self.reservation.as_ref().map(Reservation::byte_bank)
    }

    fn attach(&mut self, handle: Arc<TunnelHandle>) -> std::result::Result<(), CoreError> {
        let reservation = self.reservation.take().ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "native reservation already attached",
            ))
        })?;
        self.lease = Some(reservation.attach(handle));
        Ok(())
    }

    fn complete(&mut self) {
        if let Some(lease) = self.lease.take() {
            // Fixed one-slot successful handoff. Unexpected replacement retires
            // its exact previous runtime outside the scope mutex; no growing log.
            let old = self
                .scope
                .completed
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .replace(lease);
            drop(old);
        }
    }
}

// Attempt Drop retires its acquired lease or releases its empty reservation.
// Ownership Drop retires the successful unpublished lease if not transferred.

#[cfg(test)]
mod buffer_bank_tests {
    use super::*;
    use tokio::sync::Semaphore;

    #[test]
    fn native_source_admission_marker_keeps_original_typed_error_and_source_pointer() {
        #[derive(Debug)]
        struct Original(std::io::Error);
        impl std::fmt::Display for Original {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("original fixture failure")
            }
        }
        impl std::error::Error for Original {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let original = anyhow::Error::new(Original(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "original-source-error",
        )));
        let pointer = original.downcast_ref::<Original>().unwrap() as *const Original as usize;
        let io_pointer =
            &original.downcast_ref::<Original>().unwrap().0 as *const std::io::Error as usize;
        let error = before_source(original);
        assert!(error.is::<Admission>());
        assert_eq!(
            error.downcast_ref::<Original>().unwrap() as *const Original as usize,
            pointer
        );
        let native = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
            .unwrap();
        assert_eq!(native as *const std::io::Error as usize, io_pointer);
        assert_eq!(native.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.root_cause().to_string(), "original-source-error");
    }

    #[test]
    fn native_runtime_bank_is_the_original_registry_bank_before_attachment() {
        let bank = Arc::new(Semaphore::new(
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES,
        ));
        let registry = Registry::with_metadata(&bank).unwrap();
        let baseline = bank.available_permits();
        let owner = Ownership::standalone(Arc::clone(&registry)).unwrap();
        let reservation = owner.reserve().unwrap();
        let supplied = reservation
            .byte_bank()
            .expect("manager bank handoff missing");
        assert!(
            Arc::ptr_eq(&supplied, &bank),
            "runtime received a replacement byte pool"
        );
        let header_fee = supplied.try_acquire_many_owned(40).unwrap();
        assert_eq!(bank.available_permits(), baseline - 40);
        drop(header_fee);
        assert_eq!(bank.available_permits(), baseline);
        drop(reservation);
        drop(owner);
        drop(registry);
        assert_eq!(
            bank.available_permits(),
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
        );
    }
}

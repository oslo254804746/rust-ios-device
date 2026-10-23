//! One Source call's reservations and handoff before native RSD probing.

use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use ios_core::{CoreError, TunnelHandle, TunnelOwner, TunnelReservation, TunnelReservationOwner};
use tokio::sync::OwnedSemaphorePermit;

use super::lifecycle::Ticket;
#[cfg(test)]
use super::retirement::Registry;
use super::retirement::{NativeLease, OwnershipStamp, RegistryOwner, Reservation};

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
    registry: RegistryOwner,
    ticket: Option<&'a Ticket>,
    stamp: OwnershipStamp,
    initial: StdMutex<Option<Reservation>>,
    initial_attempt: StdMutex<Option<super::metadata_vec::AdmittedVec<Attempt<'a, 'a>>>>,
    // Both scope and each legacy Box keep this paid root. A forgotten Box
    // therefore keeps its whole grant even after the scope is destroyed.
    legacy_attempts: StdMutex<Option<LegacyStorageOwner>>,
    completed: StdMutex<Option<NativeLease>>,
    input: Option<super::candidate_input::Owner>,
}

impl<'a> Ownership<'a> {
    #[cfg(test)]
    pub(super) fn admitted(registry: RegistryOwner, ticket: &'a Ticket) -> Result<Self> {
        Self::admitted_with_input(registry, ticket, None)
    }

    pub(super) fn admitted_with_input(
        registry: RegistryOwner,
        ticket: &'a Ticket,
        input: Option<super::candidate_input::Owner>,
    ) -> Result<Self> {
        let stamp = ticket.ownership_stamp();
        let (reservation, storage) = ticket
            .commit(|| {
                let storage = Self::attempt_backing(&registry)?;
                let reservation = registry.reserve_with_input(stamp.clone(), input.clone())?;
                Ok((reservation, storage))
            })
            .map_err(before_source)?;
        Ok(Self {
            registry,
            ticket: Some(ticket),
            stamp,
            initial: StdMutex::new(Some(reservation)),
            initial_attempt: StdMutex::new(Some(storage)),
            legacy_attempts: StdMutex::new(None),
            completed: StdMutex::new(None),
            input,
        })
    }

    pub(super) fn standalone(registry: RegistryOwner) -> Result<Self> {
        let stamp = OwnershipStamp::standalone(&registry);
        let storage = Self::attempt_backing(&registry)?;
        let reservation = registry.reserve(stamp.clone())?;
        Ok(Self {
            registry,
            ticket: None,
            stamp,
            initial: StdMutex::new(Some(reservation)),
            initial_attempt: StdMutex::new(Some(storage)),
            legacy_attempts: StdMutex::new(None),
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

    fn attempt_backing(
        registry: &RegistryOwner,
    ) -> Result<super::metadata_vec::AdmittedVec<Attempt<'a, 'a>>> {
        super::metadata_vec::exact_backing(&registry.byte_bank(), 1)
    }

    #[cfg(test)]
    pub(super) fn attempt_bytes() -> usize {
        std::mem::size_of::<Attempt<'_, '_>>()
    }

    fn legacy_storage(&self) -> Result<LegacyStorageOwner> {
        let mut original = self
            .legacy_attempts
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if original.is_none() {
            let bank = self.registry.byte_bank();
            let storage = super::admission::known_metadata_charge(
                &bank,
                LegacyStorageOwner::allocation_bytes(),
            )?;
            *original = Some(LegacyStorageOwner::from_grant(
                LegacyStorage(StdMutex::new(None)),
                &bank,
                storage,
            )?);
        }
        Ok(original
            .as_ref()
            .expect("prepared legacy grant root")
            .clone())
    }

    fn reserve_attempt(&self, legacy: bool) -> Result<AttemptAllocation<'_>> {
        let allocate = || {
            let legacy = legacy.then(|| self.legacy_storage()).transpose()?;
            // The initial destination was allocated before Source creation.
            // Its empty covariant Vec can adopt this shorter scope borrow.
            let backing: super::metadata_vec::AdmittedVec<Attempt<'_, 'a>> = self
                .initial_attempt
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
                .map(Ok)
                .unwrap_or_else(|| Self::attempt_backing(&self.registry))?;
            let initial = self
                .initial
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
            let reservation = match initial {
                Some(initial) => initial,
                None => self
                    .registry
                    .reserve_with_input(self.stamp.clone(), self.input.clone())?,
            };
            reservation.start()?;
            let super::metadata_vec::AdmittedVec {
                mut values,
                storage,
            } = backing;
            values.push(Attempt {
                scope: self,
                reservation: Some(reservation),
                lease: None,
                _legacy: legacy,
            });
            let pointer = values.as_mut_ptr();
            std::mem::forget(values);
            // SAFETY: this exact destination owns one initialized Attempt,
            // capacity one and Layout::array::<Attempt>(1) equals its Box
            // layout. Nothing can fail between forgetting Vec and adoption.
            let attempt = unsafe { Box::from_raw(pointer) };
            Ok((attempt as Box<dyn TunnelReservation + '_>, storage))
        };
        // Every additional RemotePairing target is fenced independently. A
        // stale Source cannot consume its prepaid destination or a fresh slot.
        match self.ticket {
            Some(ticket) => ticket.commit(allocate),
            None => allocate(),
        }
    }
}

type AttemptAllocation<'a> = (Box<dyn TunnelReservation + 'a>, OwnedSemaphorePermit);

struct LegacyStorage(StdMutex<Option<OwnedSemaphorePermit>>);
type LegacyStorageOwner = super::shared_allocation::Shared<LegacyStorage>;

#[derive(Debug)]
struct ReservationFailure(anyhow::Error);
impl std::fmt::Display for ReservationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("native attempt reservation failed")
    }
}
impl std::error::Error for ReservationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

fn core_error(error: anyhow::Error) -> CoreError {
    CoreError::Io(std::io::Error::other(ReservationFailure(error)))
}

impl TunnelOwner for Ownership<'_> {
    fn reserve(&self) -> std::result::Result<Box<dyn TunnelReservation + '_>, CoreError> {
        let (attempt, storage) = self.reserve_attempt(true).map_err(core_error)?;
        let legacy = self
            .legacy_attempts
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let mut grants = legacy
            .as_ref()
            .expect("legacy Box keeps the same prepared grant root")
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match grants.as_mut() {
            Some(original) => original.merge(storage),
            None => *grants = Some(storage),
        }
        Ok(attempt)
    }

    fn reserve_owned(&self) -> std::result::Result<TunnelReservationOwner<'_>, CoreError> {
        let (attempt, storage) = self.reserve_attempt(false).map_err(core_error)?;
        TunnelReservationOwner::try_from_grant(attempt, storage).map_err(|original| {
            drop(original);
            core_error(crate::output::resource_decision(
                "tunnel native attempt grant mismatch",
            ))
        })
    }
}

struct Attempt<'a, 'ticket> {
    scope: &'a Ownership<'ticket>,
    reservation: Option<Reservation>,
    lease: Option<NativeLease>,
    _legacy: Option<LegacyStorageOwner>,
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
mod attempt_storage_tests;

#[cfg(test)]
mod buffer_bank_tests {
    use super::*;
    use std::error::Error;
    use tokio::sync::Semaphore;

    #[test]
    fn native_reservation_core_error_keeps_original_typed_root_deep_io_and_resource_class() {
        #[derive(Debug)]
        struct Original(std::io::Error);
        impl std::fmt::Display for Original {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("original reservation cause")
            }
        }
        impl std::error::Error for Original {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let original = anyhow::Error::new(Original(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "original-native-reservation-io",
        )));
        let pointer = original.downcast_ref::<Original>().unwrap() as *const Original as usize;
        let io_pointer = &original.downcast_ref::<Original>().unwrap().0 as *const _ as usize;
        let error: anyhow::Error = core_error(original).into();
        assert_eq!(
            error
                .chain()
                .find_map(|cause| cause.downcast_ref::<Original>())
                .unwrap() as *const Original as usize,
            pointer
        );
        let io = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<Original>())
            .unwrap()
            .source()
            .unwrap();
        assert_eq!(
            io.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
            io_pointer
        );
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::PermissionDenied
        );
        let error: anyhow::Error =
            core_error(crate::output::resource_decision("attempt-body-fixture")).into();
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
    }

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
        let owner = Ownership::standalone(registry.clone()).unwrap();
        let reservation = owner.reserve_owned().unwrap();
        let supplied = reservation
            .byte_bank()
            .expect("manager bank handoff missing");
        assert!(
            Arc::ptr_eq(&supplied, &bank),
            "runtime received a replacement byte pool"
        );
        let header_fee = supplied.try_acquire_many_owned(40).unwrap();
        assert_eq!(
            bank.available_permits(),
            baseline - Registry::incarnation_bytes() - std::mem::size_of::<Attempt<'_, '_>>() - 40
        );
        drop(header_fee);
        assert_eq!(
            bank.available_permits(),
            baseline - Registry::incarnation_bytes() - std::mem::size_of::<Attempt<'_, '_>>()
        );
        drop(reservation);
        drop(owner);
        drop(registry);
        assert_eq!(
            bank.available_permits(),
            super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
        );
    }
}

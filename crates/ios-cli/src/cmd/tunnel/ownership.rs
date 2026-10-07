//! One Source call's reservations and handoff before native RSD probing.

use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use ios_core::{CoreError, TunnelHandle, TunnelOwner, TunnelReservation};

use super::lifecycle::Ticket;
use super::retirement::{NativeLease, OwnershipStamp, Registry, Reservation};

pub(super) struct Ownership<'a> {
    registry: Arc<Registry>,
    ticket: Option<&'a Ticket>,
    stamp: OwnershipStamp,
    initial: StdMutex<Option<Reservation>>,
    completed: StdMutex<Option<NativeLease>>,
}

impl<'a> Ownership<'a> {
    pub(super) fn admitted(registry: Arc<Registry>, ticket: &'a Ticket) -> Result<Self> {
        let stamp = ticket.ownership_stamp();
        let reservation = ticket.commit(|| registry.reserve(stamp.clone()))?;
        Ok(Self {
            registry,
            ticket: Some(ticket),
            stamp,
            initial: StdMutex::new(Some(reservation)),
            completed: StdMutex::new(None),
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
                None => self.registry.reserve(self.stamp.clone()),
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

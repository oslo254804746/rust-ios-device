//! Exact selector buffers consume their original whole-owner reservation.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::OwnedSemaphorePermit;

use super::{metadata_vec, AttemptIdentity, Route};

pub(super) struct Copier {
    available: OwnedSemaphorePermit,
    retained: OwnedSemaphorePermit,
}

impl Copier {
    pub(super) fn new(mut storage: OwnedSemaphorePermit, body: usize) -> Result<Self> {
        let retained = storage.split(body).ok_or_else(|| {
            crate::output::resource_decision("tunnel prepaid selector body missing")
        })?;
        Ok(Self {
            available: storage,
            retained,
        })
    }

    pub(super) fn text(&mut self, value: &str) -> Result<String> {
        let bank = Arc::clone(self.available.semaphore());
        let field = self.available.split(value.len()).ok_or_else(|| {
            crate::output::resource_decision("tunnel prepaid selector storage missing")
        })?;
        let allocation = metadata_vec::from_grant(&bank, value.len(), field)?;
        // Already allocated fields can never be spent again by a later copy.
        // Caller binds this owner before all selector locals for partial Drop.
        self.retained.merge(allocation.storage);
        let mut bytes = allocation.values;
        bytes.extend_from_slice(value.as_bytes());
        Ok(String::from_utf8(bytes).expect("copied selector str remains UTF-8"))
    }

    pub(super) fn route(&mut self, value: &Route) -> Result<Route> {
        let host = match value {
            Route::Usbmux => None,
            Route::Usb(host) | Route::Wifi(host) => host.as_deref(),
            Route::Mobdev2(host) => Some(host.as_str()),
        };
        let host = host.map(|host| self.text(host)).transpose()?;
        Ok(match value {
            Route::Usbmux => Route::Usbmux,
            Route::Usb(_) => Route::Usb(host),
            Route::Wifi(_) => Route::Wifi(host),
            Route::Mobdev2(_) => Route::Mobdev2(host.expect("original mobdev2 selector")),
        })
    }

    pub(super) fn identity(&mut self, value: &AttemptIdentity) -> Result<AttemptIdentity> {
        Ok(AttemptIdentity {
            route: self.route(&value.route)?,
            native_id: value.native_id,
        })
    }

    pub(super) fn finish(self) -> OwnedSemaphorePermit {
        let Self {
            available,
            mut retained,
        } = self;
        retained.merge(available);
        retained
    }
}

#[cfg(test)]
mod tests;

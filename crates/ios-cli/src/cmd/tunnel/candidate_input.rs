//! One admitted input shared by the window, table, native owner and original Err.
//! Foreign Source payloads and Arc/anyhow internals need separate creation contracts.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::source::Route;

pub(super) struct Input {
    pub(super) udid: String,
    pub(super) route: Route,
    // The Source API consumes one clone of the original retained route. Reserve
    // that buffer before cloning and retain the reservation through every owner.
    source_route_bytes: usize,
    _storage: OwnedSemaphorePermit,
}

pub(super) fn route_capacity(route: &Route) -> usize {
    match route {
        Route::Usbmux => 0,
        Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::capacity),
        Route::Mobdev2(host) => host.capacity(),
    }
}

fn route_length(route: &Route) -> usize {
    match route {
        Route::Usbmux => 0,
        Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::len),
        Route::Mobdev2(host) => host.len(),
    }
}

impl Input {
    pub(super) fn storage_bank(&self) -> &Arc<Semaphore> {
        self._storage.semaphore()
    }

    pub(super) fn owned(udid: String, route: Route, bank: &Arc<Semaphore>) -> Result<Arc<Self>> {
        super::admission::selectors(
            &udid,
            match &route {
                Route::Usbmux => None,
                Route::Usb(host) | Route::Wifi(host) => host.as_deref(),
                Route::Mobdev2(host) => Some(host.as_str()),
            },
        )?;
        let source_route_bytes = route_length(&route);
        let bytes = std::mem::size_of::<Self>()
            .checked_add(udid.capacity())
            .and_then(|bytes| bytes.checked_add(route_capacity(&route)))
            .and_then(|bytes| bytes.checked_add(source_route_bytes))
            .and_then(|bytes| bytes.checked_add(udid.len()))
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel candidate input size overflow")
            })?;
        let storage = super::admission::known_metadata_charge(bank, bytes)?;
        Ok(Arc::new(Self {
            udid,
            route,
            source_route_bytes,
            _storage: storage,
        }))
    }

    pub(super) fn usb(
        udid: String,
        raw: &mut OwnedSemaphorePermit,
        bank: &Arc<Semaphore>,
    ) -> Result<Arc<Self>> {
        anyhow::ensure!(
            Arc::ptr_eq(raw.semaphore(), bank),
            crate::output::resource_decision("tunnel candidate snapshot bank mismatch")
        );
        let mut storage = super::admission::known_metadata_charge(
            bank,
            std::mem::size_of::<Self>() + udid.len(),
        )?;
        let field = raw.split(udid.capacity()).ok_or_else(|| {
            crate::output::resource_decision("tunnel candidate snapshot field charge missing")
        })?;
        storage.merge(field);
        Ok(Arc::new(Self {
            udid,
            route: Route::Usbmux,
            source_route_bytes: 0,
            _storage: storage,
        }))
    }

    pub(super) fn source_route(&self) -> Result<Route> {
        let route = self.route.clone();
        anyhow::ensure!(
            route_capacity(&route) <= self.source_route_bytes,
            crate::output::resource_decision("tunnel candidate Source clone capacity exceeded")
        );
        Ok(route)
    }

    #[cfg(test)]
    pub(super) fn bytes(udid: &str, route: &Route) -> usize {
        std::mem::size_of::<Self>() + 2 * udid.len() + route_capacity(route) + route_length(route)
    }
}

pub(super) struct SourceInputContext {
    pub(super) input: Arc<Input>,
}

impl std::fmt::Display for SourceInputContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("tunnel Source failure retains admitted input ownership")
    }
}

impl std::fmt::Debug for SourceInputContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests;

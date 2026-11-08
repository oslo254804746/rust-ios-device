//! One admitted input shared by the window, table, native owner and original Err.
//! Foreign Source payloads and Arc/anyhow internals need separate creation contracts.
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::source::Route;

pub(super) type Owner = super::shared_body::Strong<Input>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RouteRef<'a> {
    Usbmux,
    Usb(Option<&'a str>),
    Wifi(Option<&'a str>),
    Mobdev2(&'a str),
}
impl<'a> RouteRef<'a> {
    pub(super) fn host(&self) -> Option<&'a str> {
        match self {
            Self::Usbmux => None,
            Self::Usb(host) | Self::Wifi(host) => *host,
            Self::Mobdev2(host) => Some(host),
        }
    }

    #[cfg(test)]
    pub(super) fn fixture_owned(self) -> Route {
        match self {
            Self::Usbmux => Route::Usbmux,
            Self::Usb(host) => Route::Usb(host.map(str::to_owned)),
            Self::Wifi(host) => Route::Wifi(host.map(str::to_owned)),
            Self::Mobdev2(host) => Route::Mobdev2(host.to_owned()),
        }
    }
}
impl<'a> From<&'a Route> for RouteRef<'a> {
    fn from(route: &'a Route) -> Self {
        match route {
            Route::Usbmux => Self::Usbmux,
            Route::Usb(host) => Self::Usb(host.as_deref()),
            Route::Wifi(host) => Self::Wifi(host.as_deref()),
            Route::Mobdev2(host) => Self::Mobdev2(host),
        }
    }
}
pub(super) struct Input {
    pub(super) udid: String,
    pub(super) route: Route,
    // Retain the original conservative Source text allowance. Production now
    // borrows the original route; this is not a claim of another allocation.
    source_route_bytes: usize,
    outcome: Mutex<Option<super::metadata_vec::AdmittedVec<u8>>>,
    _storage: OwnedSemaphorePermit,
}

pub(super) fn route_capacity(route: &Route) -> usize {
    match route {
        Route::Usbmux => 0,
        Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::capacity),
        Route::Mobdev2(host) => host.capacity(),
    }
}

pub(super) fn route_length(route: &Route) -> usize {
    match route {
        Route::Usbmux => 0,
        Route::Usb(host) | Route::Wifi(host) => host.as_ref().map_or(0, String::len),
        Route::Mobdev2(host) => host.len(),
    }
}

impl Input {
    pub(super) fn body_bytes() -> usize {
        Owner::body_bytes()
    }
    pub(super) fn storage_bank(&self) -> &Arc<Semaphore> {
        self._storage.semaphore()
    }

    pub(super) fn copied(udid: &str, route: RouteRef<'_>, bank: &Arc<Semaphore>) -> Result<Owner> {
        super::admission::selectors(udid, route.host())?;
        let source_route_bytes = route.host().map_or(0, str::len);
        // Keep the conservative Source allowance. The outcome key now has its
        // actual empty destination and independent fee, rather than a reserve.
        let reserved = Self::body_bytes()
            .checked_add(source_route_bytes)
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel candidate input size overflow")
            })?;
        let mut storage = super::admission::known_metadata_charge(bank, reserved)?;
        let body = storage
            .split(Self::body_bytes())
            .expect("prepaid Input body and guard");
        // Bind storage first: on partial failure these actual Strings are
        // destroyed before their merged grants are returned.
        let udid = copy_text(udid, bank, &mut storage)?;
        let host = route
            .host()
            .map(|host| copy_text(host, bank, &mut storage))
            .transpose()?;
        let route = match route {
            RouteRef::Usbmux => Route::Usbmux,
            RouteRef::Usb(_) => Route::Usb(host),
            RouteRef::Wifi(_) => Route::Wifi(host),
            RouteRef::Mobdev2(_) => Route::Mobdev2(host.expect("borrowed mobdev2 host")),
        };
        let outcome = super::metadata_vec::exact_backing(bank, udid.len())?;
        Owner::new(
            Self {
                udid,
                route,
                source_route_bytes,
                outcome: Mutex::new(Some(outcome)),
                _storage: storage,
            },
            bank,
            body,
        )
    }

    #[cfg(test)]
    pub(super) fn owned(
        incoming_udid: String,
        incoming_route: Route,
        bank: &Arc<Semaphore>,
    ) -> Result<Owner> {
        super::admission::selectors(
            &incoming_udid,
            match &incoming_route {
                Route::Usbmux => None,
                Route::Usb(host) | Route::Wifi(host) => host.as_deref(),
                Route::Mobdev2(host) => Some(host.as_str()),
            },
        )?;
        let source_route_bytes = route_length(&incoming_route);
        let bytes = Self::body_bytes()
            .checked_add(incoming_udid.capacity())
            .and_then(|bytes| bytes.checked_add(route_capacity(&incoming_route)))
            .and_then(|bytes| bytes.checked_add(source_route_bytes))
            .ok_or_else(|| {
                crate::output::resource_decision("tunnel candidate input size overflow")
            })?;
        let mut storage = super::admission::known_metadata_charge(bank, bytes)?;
        let body = storage
            .split(Self::body_bytes())
            .expect("prepaid Input body and guard");
        // Local fields must precede refund on an empty destination refusal.
        let udid = incoming_udid;
        let route = incoming_route;
        let outcome = super::metadata_vec::exact_backing(bank, udid.len())?;
        Owner::new(
            Self {
                udid,
                route,
                source_route_bytes,
                outcome: Mutex::new(Some(outcome)),
                _storage: storage,
            },
            bank,
            body,
        )
    }

    pub(super) fn usb(
        incoming_udid: String,
        raw: &mut OwnedSemaphorePermit,
        bank: &Arc<Semaphore>,
    ) -> Result<Owner> {
        anyhow::ensure!(
            Arc::ptr_eq(raw.semaphore(), bank),
            crate::output::resource_decision("tunnel candidate snapshot bank mismatch")
        );
        let mut storage = super::admission::known_metadata_charge(bank, Self::body_bytes())?;
        let body = storage
            .split(Self::body_bytes())
            .expect("prepaid Input body and guard");
        let udid = incoming_udid;
        let outcome = super::metadata_vec::exact_backing(bank, udid.len())?;
        let field = raw.split(udid.capacity()).ok_or_else(|| {
            crate::output::resource_decision("tunnel candidate snapshot field charge missing")
        })?;
        storage.merge(field);
        Owner::new(
            Self {
                udid,
                route: Route::Usbmux,
                source_route_bytes: 0,
                outcome: Mutex::new(Some(outcome)),
                _storage: storage,
            },
            bank,
            body,
        )
    }

    pub(super) fn source_route(&self) -> Result<RouteRef<'_>> {
        anyhow::ensure!(
            route_length(&self.route) <= self.source_route_bytes,
            crate::output::resource_decision("tunnel candidate Source text allowance exceeded")
        );
        Ok((&self.route).into())
    }

    pub(super) fn outcome_key(&self) -> Result<(String, OwnedSemaphorePermit)> {
        let mut pending = self
            .outcome
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let destination = pending.as_ref().ok_or_else(|| {
            crate::output::resource_decision("tunnel candidate outcome key already prepared")
        })?;
        anyhow::ensure!(
            Arc::ptr_eq(destination.storage.semaphore(), self.storage_bank())
                && destination.values.is_empty()
                && destination.values.capacity() == self.udid.len()
                && destination.storage.num_permits() == self.udid.len(),
            crate::output::resource_decision("tunnel candidate outcome destination mismatch")
        );
        let destination = pending
            .take()
            .expect("checked original outcome destination");
        drop(pending);
        // The same original allocation moves out, with its fee outside String.
        let storage = destination.storage;
        let mut bytes = destination.values;
        bytes.extend_from_slice(self.udid.as_bytes());
        Ok((
            String::from_utf8(bytes).expect("copied outcome str remains UTF-8"),
            storage,
        ))
    }

    #[cfg(test)]
    pub(super) fn bytes(udid: &str, route: &Route) -> usize {
        Self::body_bytes() + 2 * udid.len() + route_capacity(route) + route_length(route)
    }
}

fn copy_text(
    value: &str,
    bank: &Arc<Semaphore>,
    storage: &mut OwnedSemaphorePermit,
) -> Result<String> {
    let allocation = super::metadata_vec::exact_backing(bank, value.len())?;
    storage.merge(allocation.storage);
    let mut bytes = allocation.values;
    bytes.extend_from_slice(value.as_bytes());
    Ok(String::from_utf8(bytes).expect("copied candidate str remains UTF-8"))
}

pub(super) struct SourceInputContext {
    pub(super) input: Owner,
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
mod outcome_tests;
#[cfg(test)]
mod tests;

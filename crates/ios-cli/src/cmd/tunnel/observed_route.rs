//! The exact Source observation borrows an admitted input or owns prepaid text.
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::{
    candidate_input::{self, RouteRef},
    metadata_vec,
    source::Route,
};

pub(super) enum Observed {
    Input(Arc<candidate_input::Input>),
    Owned {
        route: Route,
        _storage: Option<OwnedSemaphorePermit>,
    },
}

impl Observed {
    pub(super) fn new<'a>(
        route: impl Into<RouteRef<'a>>,
        input: Option<&Arc<candidate_input::Input>>,
        bank: &Arc<Semaphore>,
    ) -> Result<Self> {
        Self::admit(route.into(), input, bank).map_err(|error| error.context(Admission))
    }

    fn admit(
        route: RouteRef<'_>,
        input: Option<&Arc<candidate_input::Input>>,
        bank: &Arc<Semaphore>,
    ) -> Result<Self> {
        if let Some(input) = input {
            anyhow::ensure!(
                Arc::ptr_eq(input.storage_bank(), bank) && RouteRef::from(&input.route) == route,
                crate::output::resource_decision("tunnel observed route input mismatch")
            );
            return Ok(Self::Input(input.clone()));
        }
        let host = route.host();
        super::admission::selectors("", host)?;
        let mut storage = None;
        let host = match host {
            Some(host) => {
                let allocation = metadata_vec::exact_backing(bank, host.len())?;
                storage = Some(allocation.storage);
                let mut bytes = allocation.values;
                bytes.extend_from_slice(host.as_bytes());
                Some(String::from_utf8(bytes).expect("copied observed str remains UTF-8"))
            }
            None => None,
        };
        let route = match route {
            RouteRef::Usbmux => Route::Usbmux,
            RouteRef::Usb(_) => Route::Usb(host),
            RouteRef::Wifi(_) => Route::Wifi(host),
            RouteRef::Mobdev2(_) => Route::Mobdev2(host.expect("observed mobdev2 host")),
        };
        Ok(Self::Owned {
            route,
            _storage: storage,
        })
    }

    pub(super) fn route(&self) -> &Route {
        match self {
            Self::Input(input) => &input.route,
            Self::Owned { route, .. } => route,
        }
    }

    pub(super) fn text_bytes(&self) -> usize {
        candidate_input::route_capacity(self.route())
    }
}

#[derive(Debug)]
pub(super) struct Admission;
impl std::fmt::Display for Admission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("tunnel observed route destination admission failed")
    }
}

#[cfg(test)]
mod tests;

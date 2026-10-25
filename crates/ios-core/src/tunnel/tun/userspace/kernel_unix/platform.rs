//! Concrete target dispatch. Every successful result belongs to the same
//! original native resource; packet aliases are actual dup'd descriptors.
use std::time::Instant;

#[cfg(target_os = "linux")]
use super::super::native::linux as native;
#[cfg(target_os = "macos")]
use super::super::native::macos as native;

#[allow(unused_imports)]
pub(super) use native::PacketInterrupt as InterruptEvent;
pub(crate) use native::Plan;
pub(super) use native::{
    AddressReadiness, AddressReceipt, Error as NativeError, Identity, MtuReceipt, PacketIo,
    RouteReceipt,
};

#[cfg(test)]
#[path = "local.rs"]
pub(super) mod local;

#[cfg(target_os = "linux")]
type NativeDevice = native::LinuxDevice;
#[cfg(target_os = "macos")]
type NativeDevice = native::MacosDevice;

#[cfg(not(test))]
pub(super) struct Device {
    inner: NativeDevice,
    identity: Option<Identity>,
}

#[cfg(test)]
pub(super) struct Device {
    inner: DeviceInner,
    identity: Option<Identity>,
}

#[cfg(test)]
#[allow(clippy::large_enum_variant)]
pub(super) enum DeviceInner {
    Real(NativeDevice),
    Local(local::LocalDevice),
}

impl Device {
    #[cfg(test)]
    pub(super) fn from_local(local: local::LocalDevice) -> Self {
        Self {
            inner: DeviceInner::Local(local),
            identity: None,
        }
    }

    pub(super) fn allocate(plan: Plan, deadline: Instant) -> Result<Self, NativeError> {
        let inner = NativeDevice::allocate(plan, deadline)?;
        // No native identity/configuration/clock operation follows acquisition.
        Ok(Self {
            #[cfg(not(test))]
            inner,
            #[cfg(test)]
            inner: DeviceInner::Real(inner),
            identity: None,
        })
    }
    pub(super) fn resolve_identity(&mut self, deadline: Instant) -> Result<Identity, NativeError> {
        #[cfg(not(test))]
        let identity = self.inner.resolve_identity(deadline)?;
        #[cfg(test)]
        let identity = match &mut self.inner {
            DeviceInner::Real(inner) => inner.resolve_identity(deadline)?,
            DeviceInner::Local(inner) => inner.resolve_identity(deadline)?,
        };
        self.identity = Some(identity);
        Ok(identity)
    }
    pub(super) fn make_inert(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<(), NativeError> {
        #[cfg(not(test))]
        {
            #[cfg(target_os = "linux")]
            return self.inner.end_session(response, deadline);
            #[cfg(target_os = "macos")]
            return self.inner.make_inert(response, deadline);
        }
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => {
                #[cfg(target_os = "linux")]
                return inner.end_session(response, deadline);
                #[cfg(target_os = "macos")]
                return inner.make_inert(response, deadline);
            }
            DeviceInner::Local(inner) => inner.make_inert(response, deadline),
        }
    }
    pub(super) fn inspect_inert(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<(), NativeError> {
        #[cfg(not(test))]
        return self.inner.inspect_inert(response, deadline).map(|_| ());
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => inner.inspect_inert(response, deadline).map(|_| ()),
            DeviceInner::Local(inner) => inner.inspect_inert(response, deadline),
        }
    }
    pub(super) fn configure_mtu(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<(), NativeError> {
        #[cfg(not(test))]
        return self.inner.configure_mtu(response, deadline);
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => inner.configure_mtu(response, deadline),
            DeviceInner::Local(inner) => inner.configure_mtu(response, deadline),
        }
    }
    pub(super) fn inspect_mtu(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<MtuReceipt, NativeError> {
        #[cfg(not(test))]
        {
            #[cfg(target_os = "linux")]
            return self.inner.inspect_mtu(response, deadline);
            #[cfg(target_os = "macos")]
            {
                let _ = response;
                return self.inner.inspect_mtu(deadline);
            }
        }
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => {
                #[cfg(target_os = "linux")]
                return inner.inspect_mtu(response, deadline);
                #[cfg(target_os = "macos")]
                {
                    let _ = response;
                    return inner.inspect_mtu(deadline);
                }
            }
            DeviceInner::Local(inner) => {
                #[cfg(target_os = "linux")]
                return inner.inspect_mtu(response, deadline);
                #[cfg(target_os = "macos")]
                {
                    let _ = response;
                    return inner.inspect_mtu(deadline);
                }
            }
        }
    }
    pub(super) fn start_session(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<(), NativeError> {
        #[cfg(not(test))]
        return self.inner.start_session(response, deadline);
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => inner.start_session(response, deadline),
            DeviceInner::Local(inner) => inner.start_session(response, deadline),
        }
    }
    pub(super) fn add_address(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<AddressReceipt, NativeError> {
        #[cfg(not(test))]
        return self.inner.add_address(response, deadline);
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => inner.add_address(response, deadline),
            DeviceInner::Local(inner) => inner.add_address(response, deadline),
        }
    }
    pub(super) fn add_route(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<RouteReceipt, NativeError> {
        #[cfg(not(test))]
        return self.inner.add_route(response, deadline);
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => inner.add_route(response, deadline),
            DeviceInner::Local(inner) => inner.add_route(response, deadline),
        }
    }
    pub(super) fn inspect_route(
        &mut self,
        receipt: RouteReceipt,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<(), NativeError> {
        #[cfg(not(test))]
        return self.inner.inspect_route(receipt, response, deadline);
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => inner.inspect_route(receipt, response, deadline),
            DeviceInner::Local(inner) => inner.inspect_route(receipt, response, deadline),
        }
    }
    pub(super) fn address_ready(
        &mut self,
        receipt: AddressReceipt,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<AddressReadiness, NativeError> {
        #[cfg(not(test))]
        {
            #[cfg(target_os = "linux")]
            return self.inner.address_ready(receipt, response, deadline);
            #[cfg(target_os = "macos")]
            {
                let _ = response;
                return self.inner.address_ready(receipt, deadline);
            }
        }
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => {
                #[cfg(target_os = "linux")]
                return inner.address_ready(receipt, response, deadline);
                #[cfg(target_os = "macos")]
                {
                    let _ = response;
                    return inner.address_ready(receipt, deadline);
                }
            }
            DeviceInner::Local(inner) => {
                #[cfg(target_os = "linux")]
                return inner.address_ready(receipt, response, deadline);
                #[cfg(target_os = "macos")]
                {
                    let _ = response;
                    return inner.address_ready(receipt, deadline);
                }
            }
        }
    }
    pub(super) fn prepare_packet_io(&self, deadline: Instant) -> Result<PacketIo, NativeError> {
        #[cfg(not(test))]
        return self.inner.prepare_packet_io(deadline);
        #[cfg(test)]
        match &self.inner {
            DeviceInner::Real(inner) => inner.prepare_packet_io(deadline),
            DeviceInner::Local(inner) => inner.prepare_packet_io(deadline),
        }
    }
    pub(super) fn owns(
        &self,
        identity: Identity,
        address: AddressReceipt,
        route: RouteReceipt,
        mtu: MtuReceipt,
    ) -> bool {
        #[cfg(not(test))]
        {
            #[cfg(target_os = "linux")]
            let derived = self.inner.derived_local_receipt() == Some(address.derived_local());
            #[cfg(not(target_os = "linux"))]
            let derived = true;

            derived
                && self.identity == Some(identity)
                && self.inner.address_receipt() == Some(address)
                && self.inner.installed_route_receipt() == Some(route)
                && self.inner.mtu_receipt() == Some(mtu)
        }
        #[cfg(test)]
        match &self.inner {
            DeviceInner::Real(inner) => {
                #[cfg(target_os = "linux")]
                let derived = inner.derived_local_receipt() == Some(address.derived_local());
                #[cfg(not(target_os = "linux"))]
                let derived = true;

                derived
                    && self.identity == Some(identity)
                    && inner.address_receipt() == Some(address)
                    && inner.installed_route_receipt() == Some(route)
                    && inner.mtu_receipt() == Some(mtu)
            }
            DeviceInner::Local(inner) => {
                self.identity == Some(identity) && inner.owns(identity, address, route, mtu)
            }
        }
    }
    pub(super) fn has_pending_response(&self) -> bool {
        #[cfg(not(test))]
        {
            #[cfg(target_os = "linux")]
            return self.inner.pending_response().is_some();
            #[cfg(target_os = "macos")]
            return false;
        }
        #[cfg(test)]
        match &self.inner {
            DeviceInner::Real(_inner) => {
                #[cfg(target_os = "linux")]
                return _inner.pending_response().is_some();
                #[cfg(target_os = "macos")]
                return false;
            }
            DeviceInner::Local(_inner) => {
                #[cfg(target_os = "linux")]
                return _inner.pending_response().is_some();
                #[cfg(target_os = "macos")]
                return false;
            }
        }
    }
    pub(super) fn undo_step(
        &mut self,
        response: &mut [u8],
        deadline: Instant,
    ) -> Result<bool, (usize, NativeError)> {
        // No configuration API is reachable before this same owner's identity
        // has been resolved. Its genuinely allocated unconfigured fd may be
        // closed after init/packet jobs join even when resolve itself failed.
        if self.identity.is_none() {
            return Ok(true);
        }
        #[cfg(not(test))]
        {
            #[cfg(target_os = "linux")]
            if let Some(error) = self.inner.take_loss_consume_error() {
                return Err((2, error));
            }
            #[cfg(target_os = "macos")]
            self.inner
                .collect_pending_address(deadline)
                .map_err(|error| (1, error))?;
            self.inner
                .collect_pending_mutation(response, deadline)
                .map_err(|error| (2, error))?;
            if let Some(route) = self
                .inner
                .removed_route_receipt()
                .or_else(|| self.inner.installed_route_receipt())
            {
                #[cfg(target_os = "linux")]
                self.inner
                    .remove_owned_route(route, response, deadline)
                    .map_err(|error| (0, error))?;
                #[cfg(target_os = "macos")]
                match self
                    .inner
                    .remove_owned_route(route, response, deadline)
                    .map_err(|error| (0, error))?
                {
                    native::UndoProgress::Complete
                    | native::UndoProgress::PendingAddressRemoval => {}
                    native::UndoProgress::PendingNativeReadback => return Ok(false),
                }
            }
            if let Some(address) = self
                .inner
                .removed_address_receipt()
                .or_else(|| self.inner.address_receipt())
            {
                #[cfg(target_os = "linux")]
                self.inner
                    .remove_owned_address(address, response, deadline)
                    .map_err(|error| (1, error))?;
                #[cfg(target_os = "macos")]
                if self
                    .inner
                    .remove_owned_address(address, response, deadline)
                    .map_err(|error| (1, error))?
                    != native::UndoProgress::Complete
                {
                    return Ok(false);
                }
            }
            self.inner
                .end_session(response, deadline)
                .map_err(|error| (2, error))?;
            Ok(true)
        }
        #[cfg(test)]
        match &mut self.inner {
            DeviceInner::Real(inner) => {
                #[cfg(target_os = "linux")]
                if let Some(error) = inner.take_loss_consume_error() {
                    return Err((2, error));
                }
                #[cfg(target_os = "macos")]
                inner
                    .collect_pending_address(deadline)
                    .map_err(|error| (1, error))?;
                inner
                    .collect_pending_mutation(response, deadline)
                    .map_err(|error| (2, error))?;
                if let Some(route) = inner
                    .removed_route_receipt()
                    .or_else(|| inner.installed_route_receipt())
                {
                    #[cfg(target_os = "linux")]
                    inner
                        .remove_owned_route(route, response, deadline)
                        .map_err(|error| (0, error))?;
                    #[cfg(target_os = "macos")]
                    match inner
                        .remove_owned_route(route, response, deadline)
                        .map_err(|error| (0, error))?
                    {
                        native::UndoProgress::Complete
                        | native::UndoProgress::PendingAddressRemoval => {}
                        native::UndoProgress::PendingNativeReadback => return Ok(false),
                    }
                }
                if let Some(address) = inner
                    .removed_address_receipt()
                    .or_else(|| inner.address_receipt())
                {
                    #[cfg(target_os = "linux")]
                    inner
                        .remove_owned_address(address, response, deadline)
                        .map_err(|error| (1, error))?;
                    #[cfg(target_os = "macos")]
                    if inner
                        .remove_owned_address(address, response, deadline)
                        .map_err(|error| (1, error))?
                        != native::UndoProgress::Complete
                    {
                        return Ok(false);
                    }
                }
                inner
                    .end_session(response, deadline)
                    .map_err(|error| (2, error))?;
                Ok(true)
            }
            DeviceInner::Local(inner) => inner.undo_step(response, deadline),
        }
    }
    #[allow(clippy::result_large_err)]
    pub(super) fn close(self) -> Result<(), (Self, NativeError)> {
        let Self { inner, identity } = self;
        #[cfg(not(test))]
        {
            inner
                .close()
                .map_err(|(inner, error)| (Self { inner, identity }, error))
        }
        #[cfg(test)]
        match inner {
            DeviceInner::Real(inner) => inner.close().map_err(|(inner, error)| {
                (
                    Self {
                        inner: DeviceInner::Real(inner),
                        identity,
                    },
                    error,
                )
            }),
            DeviceInner::Local(inner) => inner.close().map_err(|(inner, error)| {
                (
                    Self {
                        inner: DeviceInner::Local(inner),
                        identity,
                    },
                    error,
                )
            }),
        }
    }
}

pub(super) fn response_capacity() -> usize {
    #[cfg(target_os = "linux")]
    {
        0
    } // First real inert response supplies its exact observed requirement.
    #[cfg(target_os = "macos")]
    {
        native::CONTROL_RESPONSE_CAPACITY
    }
}
pub(super) fn storage_required(error: &NativeError) -> Option<usize> {
    match error {
        NativeError::ResponseStorage { required } => Some(*required),
        _ => None,
    }
}
pub(super) fn is_deadline(error: &NativeError) -> bool {
    matches!(error, NativeError::Deadline)
}
pub(super) fn is_interrupted(error: &NativeError) -> bool {
    matches!(error, NativeError::Interrupted)
}

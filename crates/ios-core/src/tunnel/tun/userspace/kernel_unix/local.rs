//! LocalOps seam for Unix kernel actor testing without real TUN/utun mutation.
//! Packet IO runs over genuine local Unix datagram sockets preserving actual
//! packet boundaries, syscalls, polling, and blocking worker joins.

use std::{
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd},
    os::unix::net::UnixDatagram,
    sync::{Arc, Mutex},
    time::Instant,
};

#[cfg(target_os = "linux")]
use super::super::super::native::linux::PendingResponse;

use super::{
    AddressReadiness, AddressReceipt, Identity, MtuReceipt, NativeError, PacketIo, Plan,
    RouteReceipt,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalStep {
    ResolveIdentity,
    MakeInert,
    InspectInert,
    ConfigureMtu,
    InspectMtu,
    StartSession,
    AddAddress,
    AddRoute,
    InspectRoute,
    AddressReady,
    PreparePacketIo,
    UndoRoute,
    UndoAddress,
    UndoSession,
    Close,
}

#[derive(Default)]
pub struct LocalHook {
    pub fail_at: Option<LocalStep>,
    pub injected_error: Option<NativeError>,
    pub address_readiness: Option<AddressReadiness>,
    pub executed_steps: Vec<LocalStep>,
}

pub struct LocalDevice {
    plan: Plan,
    fd: OwnedFd,
    identity: Option<Identity>,
    address: Option<AddressReceipt>,
    route: Option<RouteReceipt>,
    #[cfg(target_os = "linux")]
    derived_local: Option<RouteReceipt>,
    mtu: Option<MtuReceipt>,
    session: bool,
    hook: Arc<Mutex<LocalHook>>,
}

pub struct LocalHandle {
    pub peer: UnixDatagram,
    pub hook: Arc<Mutex<LocalHook>>,
}

impl LocalHandle {
    pub fn set_fail_at(&self, step: LocalStep, error: NativeError) {
        let mut guard = self.hook.lock().unwrap();
        guard.fail_at = Some(step);
        guard.injected_error = Some(error);
    }

    pub fn set_address_readiness(&self, readiness: AddressReadiness) {
        let mut guard = self.hook.lock().unwrap();
        guard.address_readiness = Some(readiness);
    }

    pub fn executed_steps(&self) -> Vec<LocalStep> {
        self.hook.lock().unwrap().executed_steps.clone()
    }
}

impl LocalDevice {
    pub fn create(plan: Plan) -> Result<(Self, LocalHandle), NativeError> {
        let (sock_a, sock_b) = UnixDatagram::pair().map_err(NativeError::from)?;
        sock_a.set_nonblocking(true).map_err(NativeError::from)?;
        sock_b.set_nonblocking(false).map_err(NativeError::from)?;
        let fd = unsafe { OwnedFd::from_raw_fd(sock_a.into_raw_fd()) };
        let hook = Arc::new(Mutex::new(LocalHook::default()));
        let device = Self {
            plan,
            fd,
            identity: None,
            address: None,
            route: None,
            #[cfg(target_os = "linux")]
            derived_local: None,
            mtu: None,
            session: false,
            hook: hook.clone(),
        };
        let handle = LocalHandle { peer: sock_b, hook };
        Ok((device, handle))
    }

    fn check_step(&self, step: LocalStep) -> Result<(), NativeError> {
        let mut guard = self.hook.lock().unwrap();
        guard.executed_steps.push(step);
        if guard.fail_at == Some(step) {
            guard.fail_at = None;
            let err = guard
                .injected_error
                .take()
                .unwrap_or(NativeError::Conflict("scripted step failure"));
            return Err(err);
        }
        Ok(())
    }

    pub(crate) fn resolve_identity(&mut self, _deadline: Instant) -> Result<Identity, NativeError> {
        self.check_step(LocalStep::ResolveIdentity)?;
        #[cfg(target_os = "linux")]
        let id = Identity::test_new(1, "tuntest0");
        #[cfg(target_os = "macos")]
        let id = Identity::test_new(1, "utuntest0");
        self.identity = Some(id);
        Ok(id)
    }

    pub(crate) fn make_inert(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<(), NativeError> {
        self.check_step(LocalStep::MakeInert)
    }

    pub(crate) fn inspect_inert(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<(), NativeError> {
        self.check_step(LocalStep::InspectInert)
    }

    pub(crate) fn configure_mtu(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<(), NativeError> {
        self.check_step(LocalStep::ConfigureMtu)?;
        let id = self
            .identity
            .ok_or(NativeError::Conflict("unresolved identity"))?;
        self.mtu = Some(MtuReceipt::test_new(id, self.plan.mtu));
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn inspect_mtu(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<MtuReceipt, NativeError> {
        self.check_step(LocalStep::InspectMtu)?;
        self.mtu
            .ok_or(NativeError::Conflict("mtu not yet configured"))
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn inspect_mtu(&mut self, _deadline: Instant) -> Result<MtuReceipt, NativeError> {
        self.check_step(LocalStep::InspectMtu)?;
        self.mtu
            .ok_or(NativeError::Conflict("mtu not yet configured"))
    }

    pub(crate) fn start_session(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<(), NativeError> {
        self.check_step(LocalStep::StartSession)?;
        self.session = true;
        Ok(())
    }

    pub(crate) fn add_address(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<AddressReceipt, NativeError> {
        self.check_step(LocalStep::AddAddress)?;
        let id = self
            .identity
            .ok_or(NativeError::Conflict("unresolved identity"))?;
        #[cfg(target_os = "linux")]
        {
            let derived = RouteReceipt::test_new(id, self.plan.client, 128, 255);
            let receipt = AddressReceipt::test_new(id, self.plan.client, self.plan.prefix, derived);
            self.address = Some(receipt);
            self.derived_local = Some(derived);
            Ok(receipt)
        }
        #[cfg(target_os = "macos")]
        {
            let receipt = AddressReceipt::test_new(id, self.plan.client, self.plan.prefix);
            self.address = Some(receipt);
            Ok(receipt)
        }
    }

    pub(crate) fn add_route(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<RouteReceipt, NativeError> {
        self.check_step(LocalStep::AddRoute)?;
        let id = self
            .identity
            .ok_or(NativeError::Conflict("unresolved identity"))?;
        #[cfg(target_os = "linux")]
        let receipt = RouteReceipt::test_new(id, self.plan.server, self.plan.prefix, 254);
        #[cfg(target_os = "macos")]
        let receipt = RouteReceipt::test_new(
            id,
            super::super::super::native::macos::RouteOrigin::Explicit,
        );
        self.route = Some(receipt);
        Ok(receipt)
    }

    pub(crate) fn inspect_route(
        &mut self,
        receipt: RouteReceipt,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<(), NativeError> {
        self.check_step(LocalStep::InspectRoute)?;
        if self.route != Some(receipt) {
            return Err(NativeError::Conflict("route receipt mismatch on inspect"));
        }
        Ok(())
    }

    pub(crate) fn address_ready(
        &mut self,
        receipt: AddressReceipt,
        #[cfg(target_os = "linux")] _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<AddressReadiness, NativeError> {
        self.check_step(LocalStep::AddressReady)?;
        if self.address != Some(receipt) {
            return Err(NativeError::Conflict("address receipt mismatch on ready"));
        }
        let readiness = self
            .hook
            .lock()
            .unwrap()
            .address_readiness
            .unwrap_or(AddressReadiness::Preferred);
        Ok(readiness)
    }

    pub(crate) fn prepare_packet_io(&self, _deadline: Instant) -> Result<PacketIo, NativeError> {
        self.check_step(LocalStep::PreparePacketIo)?;
        let raw = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if raw < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let dup = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(PacketIo::from_fd(dup, self.plan.mtu))
    }

    pub(crate) fn owns(
        &self,
        identity: Identity,
        address: AddressReceipt,
        route: RouteReceipt,
        mtu: MtuReceipt,
    ) -> bool {
        #[cfg(target_os = "linux")]
        let derived_match = self.derived_local == Some(address.derived_local());
        #[cfg(not(target_os = "linux"))]
        let derived_match = true;

        derived_match
            && self.identity == Some(identity)
            && self.address == Some(address)
            && self.route == Some(route)
            && self.mtu == Some(mtu)
    }

    pub(crate) fn undo_step(
        &mut self,
        _response: &mut [u8],
        _deadline: Instant,
    ) -> Result<bool, (usize, NativeError)> {
        if self.route.is_some() {
            self.check_step(LocalStep::UndoRoute)
                .map_err(|err| (0, err))?;
            self.route = None;
        }
        if self.address.is_some() {
            self.check_step(LocalStep::UndoAddress)
                .map_err(|err| (1, err))?;
            self.address = None;
            #[cfg(target_os = "linux")]
            {
                self.derived_local = None;
            }
        }
        if self.session {
            self.check_step(LocalStep::UndoSession)
                .map_err(|err| (2, err))?;
            self.session = false;
        }
        Ok(true)
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn close(self) -> Result<(), (Self, NativeError)> {
        let failure = {
            let mut guard = self.hook.lock().unwrap();
            guard.executed_steps.push(LocalStep::Close);
            if guard.fail_at == Some(LocalStep::Close) {
                guard.fail_at = None;
                Some(
                    guard
                        .injected_error
                        .take()
                        .unwrap_or(NativeError::Conflict("injected close failure")),
                )
            } else {
                None
            }
        };
        if let Some(err) = failure {
            return Err((self, err));
        }
        drop(self.fd);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn pending_response(&self) -> Option<PendingResponse> {
        None
    }
}

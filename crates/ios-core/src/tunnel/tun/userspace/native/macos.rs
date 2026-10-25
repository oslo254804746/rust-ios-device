//! Synchronous fresh-utun staged native operations. No task is spawned here.
//! The containing paid owner retains the actual blocking jobs and packet
//! interrupt owner and joins them before undo/end_session/close.

mod codec;

use codec::{Address, Message, Query, Request, Row};
use std::{
    error::Error as StdError,
    fmt, io,
    mem::{offset_of, size_of, zeroed},
    net::Ipv6Addr,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    sync::atomic::{AtomicI32, Ordering},
    time::Instant,
};

// XNU route.h uses fixed 32-bit metric fields on both current Darwin targets.
// Never silently parse a different target libc layout as this wire format.
const _: () = {
    assert!(size_of::<libc::rt_msghdr>() == codec::HEADER);
    assert!(offset_of!(libc::rt_msghdr, rtm_flags) == 8);
    assert!(offset_of!(libc::rt_msghdr, rtm_addrs) == 12);
    assert!(offset_of!(libc::rt_msghdr, rtm_pid) == 16);
    assert!(offset_of!(libc::rt_msghdr, rtm_seq) == 20);
    assert!(offset_of!(libc::rt_msghdr, rtm_errno) == 24);
    assert!(offset_of!(libc::rt_msghdr, rtm_rmx) == 36);
    assert!(size_of::<libc::sockaddr_in6>() == 28);
};

pub(crate) const CONTROL_RESPONSE_CAPACITY: usize = codec::RESPONSE_CAPACITY;
const IN6_TENTATIVE: u32 = 0x2;
const IN6_DUPLICATED: u32 = 0x4;
const IN6_NODAD: u32 = 0x20;
const INVALID_ADDRESS_FLAGS: u32 = 0x1 | 0x8 | 0x10 | IN6_NODAD | 0x40 | 0x80 | 0x100 | 0x200;
static NEXT_SEQUENCE: AtomicI32 = AtomicI32::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub client: Ipv6Addr,
    pub server: Ipv6Addr,
    pub mtu: u16,
    pub prefix: u8,
}
impl Plan {
    fn validate(self) -> Result<(), Error> {
        if self.prefix != 64
            || self.mtu < 1280
            || self.client.is_unspecified()
            || self.client.is_multicast()
            || self.client.segments()[0] & 0xffc0 == 0xfe80
            || self.server.is_unspecified()
            || self.server.is_multicast()
            || self.client.octets()[..8] != self.server.octets()[..8]
        {
            return Err(Error::Conflict(
                "invalid authenticated native IPv6 /64 plan",
            ));
        }
        Ok(())
    }
    fn network(self) -> Ipv6Addr {
        let mut bytes = self.client.octets();
        bytes[8..].fill(0);
        Ipv6Addr::from(bytes)
    }
}

#[derive(Debug)]
pub(crate) enum Error {
    Native(io::Error),
    Conflict(&'static str),
    ResponseStorage { required: usize },
    Deadline,
    Interrupted,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(error) => error.fmt(f),
            Self::Conflict(message) => f.write_str(message),
            Self::ResponseStorage { required } => {
                write!(f, "Darwin route control requires {required} prepaid bytes")
            }
            Self::Deadline => f.write_str("native operation deadline elapsed"),
            Self::Interrupted => f.write_str("native packet wait interrupted locally"),
        }
    }
}
impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Native(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Native(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    index: u16,
    name: [libc::c_char; libc::IFNAMSIZ],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AddressReceipt {
    identity: Identity,
    address: Ipv6Addr,
    prefix: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteOrigin {
    Explicit,
    AddressDerived,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteReceipt {
    identity: Identity,
    query: Query,
    origin: RouteOrigin,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MtuReceipt {
    identity: Identity,
    mtu: u16,
}

#[cfg(test)]
impl Identity {
    pub(crate) fn test_new(index: u16, name_str: &str) -> Self {
        let mut name = [0 as libc::c_char; libc::IFNAMSIZ];
        for (i, b) in name_str.bytes().take(libc::IFNAMSIZ - 1).enumerate() {
            name[i] = b as libc::c_char;
        }
        Self { index, name }
    }
}

#[cfg(test)]
impl RouteReceipt {
    pub(crate) fn test_new(identity: Identity, origin: RouteOrigin) -> Self {
        Self {
            identity,
            query: Query {
                destination: std::net::Ipv6Addr::LOCALHOST,
                prefix: 64,
                index: identity.index,
                scoped: false,
            },
            origin,
        }
    }
}

#[cfg(test)]
impl AddressReceipt {
    pub(crate) fn test_new(identity: Identity, address: std::net::Ipv6Addr, prefix: u8) -> Self {
        Self {
            identity,
            address,
            prefix,
        }
    }
}

#[cfg(test)]
impl MtuReceipt {
    pub(crate) fn test_new(identity: Identity, mtu: u16) -> Self {
        Self { identity, mtu }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressReadiness {
    Tentative,
    Preferred,
    Conflict,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UndoProgress {
    Complete,
    PendingAddressRemoval,
    PendingNativeReadback,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Inert {
    identity: Identity,
}

/// Own this value outside a resource write lock before launching packet jobs.
/// It is one-shot: trigger never drains or replaces the actual descriptor.
pub(crate) struct PacketInterrupt {
    reader: OwnedFd,
    writer: OwnedFd,
}
impl PacketInterrupt {
    pub(crate) fn new() -> io::Result<Self> {
        let mut descriptors = [-1; 2];
        // SAFETY: pipe writes exactly two descriptors into this exclusive array.
        if unsafe { libc::pipe(descriptors.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let reader = owned(descriptors[0])?;
        let writer = owned(descriptors[1])?;
        configure_fd(reader.as_raw_fd())?;
        configure_fd(writer.as_raw_fd())?;
        Ok(Self { reader, writer })
    }
    pub(crate) fn trigger(&self) -> io::Result<()> {
        let byte = [1u8];
        // SAFETY: both ends stay owned here, so SIGPIPE cannot follow an owned
        // reader's premature close. The one initialized byte remains borrowed.
        let result = unsafe { libc::write(self.writer.as_raw_fd(), byte.as_ptr().cast(), 1) };
        if result == 1 {
            return Ok(());
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            return Err(error);
        }
        Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "short packet-interrupt write",
        ))
    }
}

/// No packet buffers, response vectors or hidden JoinHandles are allocated by
/// this resource. Caller preadmits the complete value plus exactly 65535 bytes
/// of route response backing before the first control/exposure operation.
pub(crate) struct MacosDevice {
    utun: OwnedFd,
    ioctl: OwnedFd,
    route_control: RouteControl,
    identity: Option<Identity>,
    plan: Plan,
    inert: bool,
    session: bool,
    mtu: Option<MtuReceipt>,
    address: Option<AddressReceipt>,
    address_intent: Option<AddressReceipt>,
    removed_address: Option<AddressReceipt>,
    route: Option<RouteReceipt>,
    route_observed: Option<Row>,
    route_intent: Option<RouteReceipt>,
    removed_route: Option<RouteReceipt>,
    // Expected absence is typed native evidence, not a formatted failure.
    before_address_absence: Option<io::Error>,
    before_global_route_absence: Option<io::Error>,
    before_scoped_route_absence: Option<io::Error>,
    after_route_absence: Option<io::Error>,
    after_address_absence: Option<io::Error>,
}

impl MacosDevice {
    /// The attached actor must store this actual acquired resource before
    /// calling resolve_identity/make_inert or any further fallible operation.
    pub(crate) fn allocate(plan: Plan, deadline: Instant) -> Result<Self, Error> {
        plan.validate()?;
        check_clock(deadline)?;
        // SAFETY: socket has scalar arguments and returns a new control fd.
        let utun = owned(unsafe {
            libc::socket(libc::PF_SYSTEM, libc::SOCK_DGRAM, libc::SYSPROTO_CONTROL)
        })?;
        configure_fd(utun.as_raw_fd())?;
        let mut info: libc::ctl_info = unsafe { zeroed() }; // plain C fields
        let control = b"com.apple.net.utun_control\0";
        for (destination, byte) in info.ctl_name.iter_mut().zip(control) {
            *destination = *byte as libc::c_char;
        }
        native_ioctl(utun.as_raw_fd(), libc::CTLIOCGINFO, &mut info, deadline)?;
        // Ancillary resources are prepared before successful utun connect.
        // Once connect creates the interface, no fallible call intervenes
        // between acquiring that FD and returning its ordinary owned value.
        check_clock(deadline)?;
        let ioctl = owned(unsafe { libc::socket(libc::AF_INET6, libc::SOCK_DGRAM, 0) })?;
        configure_fd(ioctl.as_raw_fd())?;
        let route_control = RouteControl::new()?;
        let address = libc::sockaddr_ctl {
            sc_len: size_of::<libc::sockaddr_ctl>() as u8,
            sc_family: libc::AF_SYSTEM as u8,
            ss_sysaddr: libc::AF_SYS_CONTROL as u16,
            sc_id: info.ctl_id,
            sc_unit: 0,
            sc_reserved: [0; 5],
        };
        check_clock(deadline)?;
        // SAFETY: unit zero asks the kernel to allocate a new utun. No path
        // accepts a name/unit or opens/adopts an already existing interface.
        if unsafe {
            libc::connect(
                utun.as_raw_fd(),
                (&address as *const libc::sockaddr_ctl).cast(),
                size_of::<libc::sockaddr_ctl>() as _,
            )
        } < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        // XNU if_utun.c creates a new interface with IFF_UP. Conservatively
        // retain that live state until actual in-place down succeeds.
        Ok(Self {
            utun,
            ioctl,
            route_control,
            identity: None,
            plan,
            inert: false,
            session: true,
            mtu: None,
            address: None,
            address_intent: None,
            removed_address: None,
            route: None,
            route_observed: None,
            route_intent: None,
            removed_route: None,
            before_address_absence: None,
            before_global_route_absence: None,
            before_scoped_route_absence: None,
            after_route_absence: None,
            after_address_absence: None,
        })
    }

    pub(crate) fn resolve_identity(&mut self, deadline: Instant) -> Result<Identity, Error> {
        check_clock(deadline)?;
        let actual = identity(self.utun.as_raw_fd())?;
        if self.identity.is_some_and(|previous| previous != actual) {
            return Err(Error::Conflict(
                "owned utun identity changed during resolve",
            ));
        }
        self.identity = Some(actual);
        Ok(actual)
    }

    pub(crate) fn make_inert(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        require_response(scratch)?;
        self.same_identity()?;
        if self.address.is_some() || self.address_intent.is_some() || self.route.is_some() {
            return Err(Error::Conflict("cannot make an exposed owner inert"));
        }
        self.set_up(false, deadline)
    }

    pub(crate) fn inspect_inert(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<Inert, Error> {
        require_response(scratch)?;
        self.same_identity()?;
        if self.session
            || self.address.is_some()
            || self.address_intent.is_some()
            || self.route.is_some()
        {
            return Err(Error::Conflict("native owner already exposed"));
        }
        if self.link_flags(deadline)? & libc::IFF_UP as u32 != 0 {
            return Err(Error::Conflict("fresh utun is already up"));
        }
        self.before_address_absence =
            Some(match self.observe_address(self.plan.client, deadline)? {
                AddressObservation::Absent(error) => error,
                AddressObservation::Present(_) => {
                    return Err(Error::Conflict(
                        "authenticated address exists before native staging",
                    ))
                }
            });
        // Refuse any existing exact global prefix before address ioctl: XNU
        // prefix creation may otherwise replace an existing indirect route.
        let global = Query {
            destination: self.plan.network(),
            prefix: self.plan.prefix,
            index: 0,
            scoped: false,
        };
        self.before_global_route_absence =
            Some(match self.get_route(global, scratch, deadline)? {
                RouteObservation::Absent(error) => error,
                RouteObservation::Present(_) => {
                    return Err(Error::Conflict(
                        "global authenticated prefix route already exists",
                    ))
                }
            });
        let scoped = Query {
            index: self.resolved_identity().index,
            scoped: true,
            ..global
        };
        self.before_scoped_route_absence =
            Some(match self.get_route(scoped, scratch, deadline)? {
                RouteObservation::Absent(error) => error,
                RouteObservation::Present(_) => {
                    return Err(Error::Conflict(
                        "fresh utun already has authenticated prefix route",
                    ))
                }
            });
        self.inert = true;
        Ok(Inert {
            identity: self.resolved_identity(),
        })
    }

    pub(crate) fn configure_mtu(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        require_response(scratch)?;
        self.require_inert()?;
        let mut request = self.link_request();
        request.ifr_ifru.ifru_mtu = i32::from(self.plan.mtu);
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iow::<libc::ifreq>(52),
            &mut request,
            deadline,
        )?;
        self.mtu = Some(MtuReceipt {
            identity: self.resolved_identity(),
            mtu: self.plan.mtu,
        });
        let mut readback = self.link_request();
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::ifreq>(51),
            &mut readback,
            deadline,
        )?;
        if unsafe { readback.ifr_ifru.ifru_mtu } != i32::from(self.plan.mtu) {
            return Err(Error::Conflict("native MTU readback differs"));
        }
        Ok(())
    }
    pub(crate) fn start_session(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        require_response(scratch)?;
        self.require_inert()?;
        if self.mtu.is_none() {
            return Err(Error::Conflict("native MTU has not been acknowledged"));
        }
        self.set_up(true, deadline)
    }

    pub(crate) fn add_address(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<AddressReceipt, Error> {
        require_response(scratch)?;
        self.require_inert()?;
        if !self.session || self.address.is_some() || self.address_intent.is_some() {
            return Err(Error::Conflict("address stage is not empty and active"));
        }
        // SIOCAIFADDR_IN6 is an add-or-update ioctl, not an exclusive create.
        // Limit it to this fresh owned utun, require prior and immediate absent
        // readback, and make no atomic administrator-race claim.
        match self.observe_address(self.plan.client, deadline)? {
            AddressObservation::Absent(error) => self.before_address_absence = Some(error),
            AddressObservation::Present(_) => {
                return Err(Error::Conflict("address appeared before owned add"))
            }
        }
        let receipt = self.address_plan();
        let mut request: In6Alias = unsafe { zeroed() }; // all C fields initialized
        request.name = self.resolved_identity().name;
        request.address = socket_address(receipt.address, 0);
        request.mask = socket_address(mask64(), 0);
        request.flags = 0; // deliberately no NODAD or AUTOCONF policy shortcut
        request.lifetime.ia6t_vltime = u32::MAX;
        request.lifetime.ia6t_pltime = u32::MAX;
        // XNU can add the address, then fail adding its prefix. Keep intent
        // before that actual call so an error never loses partial ownership.
        self.address_intent = Some(receipt);
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iow::<In6Alias>(26),
            &mut request,
            deadline,
        )?;
        self.address = Some(receipt);
        self.address_intent = None;
        Ok(receipt)
    }

    pub(crate) fn collect_pending_address(&mut self, deadline: Instant) -> Result<bool, Error> {
        let Some(receipt) = self.address_intent else {
            return Ok(false);
        };
        match self.observe_address(receipt.address, deadline)? {
            AddressObservation::Absent(error) => {
                self.before_address_absence = Some(error);
                self.address_intent = None;
            }
            AddressObservation::Present(row) => {
                if !row.matches(receipt) {
                    return Err(Error::Conflict(
                        "partially added address conflicts with original intent",
                    ));
                }
                self.address = Some(receipt);
                self.address_intent = None;
            }
        }
        Ok(true)
    }

    pub(crate) fn address_ready(
        &self,
        receipt: AddressReceipt,
        deadline: Instant,
    ) -> Result<AddressReadiness, Error> {
        if self.address != Some(receipt) {
            return Err(Error::Conflict("address receipt belongs to another owner"));
        }
        self.same_identity()?;
        let row = match self.observe_address(receipt.address, deadline)? {
            AddressObservation::Present(row) => row,
            AddressObservation::Absent(_) => {
                return Err(Error::Conflict("acknowledged address disappeared"))
            }
        };
        if !row.matches(receipt) || row.flags & IN6_DUPLICATED != 0 {
            return Ok(AddressReadiness::Conflict);
        }
        Ok(if row.flags & IN6_TENTATIVE != 0 {
            AddressReadiness::Tentative
        } else {
            AddressReadiness::Preferred
        })
    }

    pub(crate) fn add_route(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<RouteReceipt, Error> {
        require_response(scratch)?;
        self.same_identity()?;
        if self.address.is_none() || self.route.is_some() || self.route_intent.is_some() {
            return Err(Error::Conflict(
                "route stage is not empty after address add",
            ));
        }
        let query = self.route_query(false);
        // Address creation normally installs the /64 prefix itself. Observe
        // that exact row and distinguish it from an explicit RTM_ADD receipt.
        match self.get_route(query, scratch, deadline)? {
            RouteObservation::Present(row) => {
                self.validate_derived(row, deadline)?;
                let receipt = RouteReceipt {
                    identity: self.resolved_identity(),
                    query: row.query,
                    origin: RouteOrigin::AddressDerived,
                };
                self.route = Some(receipt);
                self.route_observed = Some(row);
                return Ok(receipt);
            }
            RouteObservation::Absent(_) => {}
        }
        let scoped = self.route_query(true);
        if let RouteObservation::Present(row) = self.get_route(scoped, scratch, deadline)? {
            self.validate_derived(row, deadline)?;
            let receipt = RouteReceipt {
                identity: self.resolved_identity(),
                query: row.query,
                origin: RouteOrigin::AddressDerived,
            };
            self.route = Some(receipt);
            self.route_observed = Some(row);
            return Ok(receipt);
        }
        let receipt = RouteReceipt {
            identity: self.resolved_identity(),
            query: scoped,
            origin: RouteOrigin::Explicit,
        };
        let request = self.route_control.request(
            codec::ADD,
            scoped,
            Some(Address::link(self.resolved_identity().index)),
            Some(self.plan.client),
            codec::UP | codec::STATIC,
        )?;
        self.route_intent = Some(receipt);
        let result = self.route_control.exchange(request, scratch, deadline);
        match result {
            Ok(Reply::Ack) => {}
            Ok(_) => return Err(Error::Conflict("unexpected explicit route add result")),
            Err(error) => {
                if self.route_control.pending.is_none() {
                    self.route_intent = None;
                }
                return Err(error);
            }
        }
        self.route = Some(receipt);
        self.route_intent = None;
        Ok(receipt)
    }

    pub(crate) fn inspect_route(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.route != Some(receipt) {
            return Err(Error::Conflict("route receipt belongs to another owner"));
        }
        self.same_identity()?;
        let row = match self.get_route(receipt.query, scratch, deadline)? {
            RouteObservation::Present(row) => row,
            _ => return Err(Error::Conflict("acknowledged route disappeared")),
        };
        if !row.reachable_on(receipt.query) {
            return Err(Error::Conflict("native route reachability differs"));
        }
        if let Some(previous) = self.route_observed {
            if !row.same_owned(previous) {
                return Err(Error::Conflict("owned route was changed"));
            }
        } else if row.gateway.family() != codec::AF_LINK
            || row.gateway.link_index()? != self.resolved_identity().index
            || row.flags & codec::STATIC == 0
        {
            return Err(Error::Conflict("explicit native route readback differs"));
        }
        self.route_observed = Some(row);
        Ok(())
    }

    pub(crate) fn remove_owned_route(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<UndoProgress, Error> {
        require_response(scratch)?;
        if self
            .route_control
            .pending
            .is_some_and(|pending| pending.request.kind == codec::DELETE)
        {
            self.collect_pending_mutation(scratch, deadline)?;
        }
        if self.removed_route == Some(receipt) {
            return self.finish_route_undo(receipt, scratch, deadline);
        }
        self.inspect_route(receipt, scratch, deadline)?;
        if receipt.origin == RouteOrigin::AddressDerived {
            return Ok(UndoProgress::PendingAddressRemoval);
        }
        let observed = self.route_observed.ok_or(Error::Conflict(
            "route was not actually observed before undo",
        ))?;
        let request = self.route_control.request(
            codec::DELETE,
            observed.query,
            Some(observed.gateway),
            None,
            observed.flags,
        )?;
        match self.route_control.exchange(request, scratch, deadline)? {
            Reply::Ack => {}
            _ => return Err(Error::Conflict("unexpected route delete result")),
        }
        self.route = None;
        self.removed_route = Some(receipt);
        self.finish_route_undo(receipt, scratch, deadline)
    }

    pub(crate) fn remove_owned_address(
        &mut self,
        receipt: AddressReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<UndoProgress, Error> {
        require_response(scratch)?;
        if self.removed_address == Some(receipt) {
            return self.finish_address_undo(receipt, scratch, deadline);
        }
        if self.address != Some(receipt)
            || self.address_intent.is_some()
            || self.removed_route.is_some()
        {
            return Err(Error::Conflict("address cleanup is out of order"));
        }
        if let Some(route) = self.route {
            if route.origin != RouteOrigin::AddressDerived {
                return Err(Error::Conflict(
                    "explicit route must be undone before address",
                ));
            }
            self.inspect_route(route, scratch, deadline)?;
        }
        self.same_identity()?;
        let current = match self.observe_address(receipt.address, deadline)? {
            AddressObservation::Present(row) => row,
            _ => return Err(Error::Conflict("owned address disappeared before undo")),
        };
        if !current.matches(receipt) {
            return Err(Error::Conflict("owned address was changed before undo"));
        }
        let mut request = self.address_request(receipt.address);
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iow::<libc::in6_ifreq>(25),
            &mut request,
            deadline,
        )?;
        self.address = None;
        self.removed_address = Some(receipt);
        self.finish_address_undo(receipt, scratch, deadline)
    }

    pub(crate) fn collect_pending_mutation(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<bool, Error> {
        require_response(scratch)?;
        let Some(request) = self.route_control.pending.map(|pending| pending.request) else {
            return Ok(false);
        };
        let result = self.route_control.exchange(request, scratch, deadline);
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if request.kind == codec::ADD && self.route_control.pending.is_none() {
                    self.route_intent = None;
                }
                return Err(error);
            }
        };
        match result {
            Reply::Ack if request.kind == codec::ADD => {
                self.route = Some(self.route_intent.take().unwrap_or(RouteReceipt {
                    identity: self.resolved_identity(),
                    query: request.query,
                    origin: RouteOrigin::Explicit,
                }));
            }
            Reply::Ack if request.kind == codec::DELETE => {
                self.removed_route = Some(self.route.take().unwrap_or(RouteReceipt {
                    identity: self.resolved_identity(),
                    query: request.query,
                    origin: RouteOrigin::Explicit,
                }));
            }
            Reply::Route(_) | Reply::Absent(_) if request.kind == codec::GET => {}
            _ => {
                return Err(Error::Conflict(
                    "pending route result has a different operation type",
                ))
            }
        }
        Ok(true)
    }

    pub(crate) fn end_session(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        require_response(scratch)?;
        if self.address.is_some()
            || self.address_intent.is_some()
            || self.removed_address.is_some()
            || self.route.is_some()
            || self.removed_route.is_some()
            || self.route_control.pending.is_some()
        {
            return Err(Error::Conflict(
                "native undo or control collection remains before session end",
            ));
        }
        self.set_up(false, deadline)
    }
    pub(crate) fn close(self) -> Result<(), (Self, Error)> {
        if (self.session && self.identity.is_some())
            || self.address.is_some()
            || self.address_intent.is_some()
            || self.removed_address.is_some()
            || self.route.is_some()
            || self.route_intent.is_some()
            || self.removed_route.is_some()
            || self.route_control.pending.is_some()
        {
            return Err((
                self,
                Error::Conflict("explicit close requires completed control/undo"),
            ));
        }
        drop(self);
        Ok(())
    }
    pub(crate) fn address_receipt(&self) -> Option<AddressReceipt> {
        self.address
    }
    pub(crate) fn installed_route_receipt(&self) -> Option<RouteReceipt> {
        self.route
    }
    pub(crate) fn removed_address_receipt(&self) -> Option<AddressReceipt> {
        self.removed_address
    }
    pub(crate) fn removed_route_receipt(&self) -> Option<RouteReceipt> {
        self.removed_route
    }
    pub(crate) fn mtu_receipt(&self) -> Option<MtuReceipt> {
        self.mtu
    }
    pub(crate) fn has_pending_control(&self) -> bool {
        self.route_control.pending.is_some() || self.address_intent.is_some()
    }
    pub(crate) fn inspect_mtu(&self, deadline: Instant) -> Result<MtuReceipt, Error> {
        self.same_identity()?;
        let receipt = self
            .mtu
            .ok_or(Error::Conflict("native MTU has not been acknowledged"))?;
        let mut request = self.link_request();
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::ifreq>(51),
            &mut request,
            deadline,
        )?;
        if unsafe { request.ifr_ifru.ifru_mtu } != i32::from(receipt.mtu) {
            return Err(Error::Conflict("native MTU changed after acknowledgement"));
        }
        Ok(receipt)
    }

    /// The actor stores the real returned duplicate before any further native
    /// call. Packet jobs borrow this independent owned socket alias, not a
    /// whole-device read guard that could deadlock a mutating control GET.
    pub(crate) fn prepare_packet_io(&self, deadline: Instant) -> Result<PacketIo, Error> {
        self.same_identity()?;
        check_clock(deadline)?;
        // SAFETY: F_DUPFD_CLOEXEC duplicates this original live utun socket;
        // it creates no interface and adopts no other device by name.
        let fd = owned(unsafe { libc::fcntl(self.utun.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
        Ok(PacketIo {
            fd,
            mtu: self.plan.mtu,
        })
    }
    fn require_inert(&self) -> Result<(), Error> {
        self.same_identity()?;
        if !self.inert {
            return Err(Error::Conflict(
                "fresh native absence proof has not completed",
            ));
        }
        Ok(())
    }
    fn resolved_identity(&self) -> Identity {
        self.identity
            .expect("private operation checked resolved identity before using it")
    }
    fn same_identity(&self) -> Result<(), Error> {
        let expected = self.identity.ok_or(Error::Conflict(
            "acquired utun identity has not been resolved in place",
        ))?;
        if identity(self.utun.as_raw_fd())? != expected {
            return Err(Error::Conflict("owned utun identity changed"));
        }
        Ok(())
    }
    fn address_plan(&self) -> AddressReceipt {
        AddressReceipt {
            identity: self.resolved_identity(),
            address: self.plan.client,
            prefix: self.plan.prefix,
        }
    }
    fn route_query(&self, scoped: bool) -> Query {
        Query {
            destination: self.plan.network(),
            prefix: self.plan.prefix,
            index: self.resolved_identity().index,
            scoped,
        }
    }
    fn link_request(&self) -> libc::ifreq {
        let mut request: libc::ifreq = unsafe { zeroed() };
        request.ifr_name = self.resolved_identity().name;
        request
    }
    fn address_request(&self, address: Ipv6Addr) -> libc::in6_ifreq {
        let mut request: libc::in6_ifreq = unsafe { zeroed() };
        request.ifr_name = self.resolved_identity().name;
        request.ifr_ifru.ifru_addr = socket_address(
            address,
            if address.is_unicast_link_local() {
                u32::from(self.resolved_identity().index)
            } else {
                0
            },
        );
        request
    }
    fn link_flags(&self, deadline: Instant) -> Result<u32, Error> {
        let mut request = self.link_request();
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::ifreq>(17),
            &mut request,
            deadline,
        )?;
        Ok(unsafe { request.ifr_ifru.ifru_flags } as u16 as u32)
    }
    fn set_up(&mut self, up: bool, deadline: Instant) -> Result<(), Error> {
        self.same_identity()?;
        let flags = self.link_flags(deadline)?;
        let mut request = self.link_request();
        request.ifr_ifru.ifru_flags = (if up {
            flags | libc::IFF_UP as u32
        } else {
            flags & !(libc::IFF_UP as u32)
        }) as i16;
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iow::<libc::ifreq>(16),
            &mut request,
            deadline,
        )?;
        self.session = up;
        if (self.link_flags(deadline)? & libc::IFF_UP as u32 != 0) != up {
            return Err(Error::Conflict("native link state readback differs"));
        }
        Ok(())
    }
    fn observe_address(
        &self,
        address: Ipv6Addr,
        deadline: Instant,
    ) -> Result<AddressObservation, Error> {
        let mut actual = self.address_request(address);
        if let Err(error) = native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::in6_ifreq>(33),
            &mut actual,
            deadline,
        ) {
            return match error {
                Error::Native(error) if error.raw_os_error() == Some(libc::EADDRNOTAVAIL) => {
                    Ok(AddressObservation::Absent(error))
                }
                error => Err(error),
            };
        }
        let actual = unsafe { actual.ifr_ifru.ifru_addr };
        if actual.sin6_family != libc::AF_INET6 as u8
            || actual.sin6_len as usize != size_of::<libc::sockaddr_in6>()
        {
            return Err(codec::invalid("unexpected native address readback").into());
        }
        let mut mask = self.address_request(address);
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::in6_ifreq>(37),
            &mut mask,
            deadline,
        )?;
        let mask = unsafe { mask.ifr_ifru.ifru_addr };
        let mut flags = self.address_request(address);
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::in6_ifreq>(73),
            &mut flags,
            deadline,
        )?;
        let flags = unsafe { flags.ifr_ifru.ifru_flags6 } as u32;
        let mut lifetime = self.address_request(address);
        native_ioctl(
            self.ioctl.as_raw_fd(),
            iowr::<libc::in6_ifreq>(81),
            &mut lifetime,
            deadline,
        )?;
        let lifetime = unsafe { lifetime.ifr_ifru.ifru_lifetime };
        Ok(AddressObservation::Present(AddressRow {
            address: Ipv6Addr::from(actual.sin6_addr.s6_addr),
            scope: actual.sin6_scope_id,
            mask: Ipv6Addr::from(mask.sin6_addr.s6_addr),
            flags,
            valid: lifetime.ia6t_vltime,
            preferred: lifetime.ia6t_pltime,
        }))
    }
    fn get_route(
        &mut self,
        query: Query,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<RouteObservation, Error> {
        let request = self
            .route_control
            .request(codec::GET, query, None, None, 0)?;
        match self.route_control.exchange(request, scratch, deadline)? {
            Reply::Route(row) => Ok(RouteObservation::Present(row)),
            Reply::Absent(error) => Ok(RouteObservation::Absent(error)),
            _ => Err(Error::Conflict("unexpected native route GET result")),
        }
    }
    fn validate_derived(&self, row: Row, deadline: Instant) -> Result<(), Error> {
        if !row.reachable_on(self.route_query(row.query.scoped)) {
            return Err(Error::Conflict(
                "address-derived route does not reach authenticated /64",
            ));
        }
        match row.gateway.family() {
            codec::AF_LINK if row.gateway.link_index()? == self.resolved_identity().index => Ok(()),
            codec::AF_INET6 => {
                let (gateway, _) = row.gateway.ip()?;
                if gateway != self.plan.client && !gateway.is_unicast_link_local() {
                    return Err(Error::Conflict(
                        "derived route gateway is not an owned interface address",
                    ));
                }
                match self.observe_address(gateway, deadline)? {
                    AddressObservation::Present(_) => Ok(()),
                    _ => Err(Error::Conflict("derived route gateway address disappeared")),
                }
            }
            _ => Err(Error::Conflict("unexpected derived route gateway")),
        }
    }
    fn finish_route_undo(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<UndoProgress, Error> {
        match self.get_route(receipt.query, scratch, deadline)? {
            RouteObservation::Absent(error) => {
                self.after_route_absence = Some(error);
                self.removed_route = None;
                self.route_observed = None;
                Ok(UndoProgress::Complete)
            }
            RouteObservation::Present(row) => {
                if self
                    .route_observed
                    .is_some_and(|previous| !row.same_owned(previous))
                {
                    return Err(Error::Conflict("route was replaced during undo readback"));
                }
                Ok(UndoProgress::PendingNativeReadback)
            }
        }
    }
    fn finish_address_undo(
        &mut self,
        receipt: AddressReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<UndoProgress, Error> {
        match self.observe_address(receipt.address, deadline)? {
            AddressObservation::Absent(error) => self.after_address_absence = Some(error),
            AddressObservation::Present(_) => return Ok(UndoProgress::PendingNativeReadback),
        }
        if let Some(route) = self.route {
            if route.origin != RouteOrigin::AddressDerived {
                return Err(Error::Conflict("explicit route remains after address undo"));
            }
            match self.get_route(route.query, scratch, deadline)? {
                RouteObservation::Absent(error) => {
                    self.after_route_absence = Some(error);
                    self.route = None;
                    self.route_observed = None;
                }
                RouteObservation::Present(row) => {
                    if self
                        .route_observed
                        .is_some_and(|previous| !row.same_owned(previous))
                    {
                        return Err(Error::Conflict(
                            "derived route changed while address undo was pending",
                        ));
                    }
                    return Ok(UndoProgress::PendingNativeReadback);
                }
            }
        }
        self.removed_address = None;
        Ok(UndoProgress::Complete)
    }
}

/// A real duplicate of the original utun socket. Retain it in its prepaid
/// packet resource slot through every actual read/write job join, then drop
/// it before final control-owner close. Drop alone is not a join witness.
pub(crate) struct PacketIo {
    fd: OwnedFd,
    mtu: u16,
}
impl PacketIo {
    #[cfg(test)]
    pub(crate) fn from_fd(fd: OwnedFd, mtu: u16) -> Self {
        Self { fd, mtu }
    }

    pub(crate) fn recv(
        &self,
        destination: &mut [u8],
        interrupt: &PacketInterrupt,
    ) -> Result<usize, Error> {
        self.recv_inner(destination, interrupt, None)
    }
    pub(crate) fn send(&self, packet: &[u8], interrupt: &PacketInterrupt) -> Result<usize, Error> {
        self.send_inner(packet, interrupt, None)
            .map(|()| packet.len())
    }
    fn recv_inner(
        &self,
        destination: &mut [u8],
        interrupt: &PacketInterrupt,
        deadline: Option<Instant>,
    ) -> Result<usize, Error> {
        if destination.len() < usize::from(self.mtu) + 1 {
            return Err(Error::Conflict(
                "native packet destination requires MTU plus one byte",
            ));
        }
        loop {
            packet_wait(
                self.fd.as_raw_fd(),
                interrupt.reader.as_raw_fd(),
                libc::POLLIN,
                deadline,
            )?;
            let mut family = [0; 4];
            let mut vectors = [
                libc::iovec {
                    iov_base: family.as_mut_ptr().cast(),
                    iov_len: 4,
                },
                libc::iovec {
                    iov_base: destination.as_mut_ptr().cast(),
                    iov_len: destination.len(),
                },
            ];
            let mut message: libc::msghdr = unsafe { zeroed() };
            message.msg_iov = vectors.as_mut_ptr();
            message.msg_iovlen = vectors.len() as _;
            // SAFETY: both exclusive output slices and iovec remain live. The
            // utun AF header is separate from the caller's paid packet backing.
            let length = unsafe { libc::recvmsg(self.fd.as_raw_fd(), &mut message, 0) };
            if length < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(error.into());
            }
            if length < 4
                || length as usize > destination.len() + 4
                || family != (libc::AF_INET6 as u32).to_be_bytes()
                || message.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
            {
                return Err(codec::invalid("truncated or unexpected utun packet family").into());
            }
            let length = length as usize - 4;
            validate_packet(&destination[..length], self.mtu)?;
            return Ok(length);
        }
    }
    fn send_inner(
        &self,
        packet: &[u8],
        interrupt: &PacketInterrupt,
        deadline: Option<Instant>,
    ) -> Result<(), Error> {
        validate_packet(packet, self.mtu)?;
        loop {
            packet_wait(
                self.fd.as_raw_fd(),
                interrupt.reader.as_raw_fd(),
                libc::POLLOUT,
                deadline,
            )?;
            let family = (libc::AF_INET6 as u32).to_be_bytes();
            let vectors = [
                libc::iovec {
                    iov_base: family.as_ptr().cast_mut().cast(),
                    iov_len: 4,
                },
                libc::iovec {
                    iov_base: packet.as_ptr().cast_mut().cast(),
                    iov_len: packet.len(),
                },
            ];
            let mut message: libc::msghdr = unsafe { zeroed() };
            message.msg_iov = vectors.as_ptr().cast_mut();
            message.msg_iovlen = vectors.len() as _;
            // SAFETY: sendmsg only reads these initialized slices during the
            // call. One syscall is one utun packet; a suffix is never resent.
            let length = unsafe { libc::sendmsg(self.fd.as_raw_fd(), &message, 0) };
            if length < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    continue;
                }
                return Err(error.into());
            }
            if length as usize != packet.len() + 4 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "short whole-utun packet send",
                )
                .into());
            }
            return Ok(());
        }
    }
}

#[repr(C)]
struct In6Alias {
    name: [libc::c_char; libc::IFNAMSIZ],
    address: libc::sockaddr_in6,
    destination: libc::sockaddr_in6,
    mask: libc::sockaddr_in6,
    flags: libc::c_int,
    lifetime: libc::in6_addrlifetime,
}
const _: () = assert!(size_of::<In6Alias>() == 128); // XNU's 64-bit in6_aliasreq
const fn iow<T>(number: u8) -> libc::c_ulong {
    0x80000000
        | ((size_of::<T>() as libc::c_ulong & 0x1fff) << 16)
        | ((b'i' as libc::c_ulong) << 8)
        | number as libc::c_ulong
}
const fn iowr<T>(number: u8) -> libc::c_ulong {
    iow::<T>(number) | 0x40000000
}
fn native_ioctl<T>(
    fd: RawFd,
    operation: libc::c_ulong,
    request: &mut T,
    deadline: Instant,
) -> Result<(), Error> {
    check_clock(deadline)?;
    // SAFETY: private callers choose the exact ioctl and corresponding C type;
    // the initialized exclusive request and owned fd live through this call.
    if unsafe { libc::ioctl(fd, operation, request as *mut T) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}
fn socket_address(address: Ipv6Addr, scope: u32) -> libc::sockaddr_in6 {
    let mut result: libc::sockaddr_in6 = unsafe { zeroed() };
    result.sin6_len = size_of::<libc::sockaddr_in6>() as u8;
    result.sin6_family = libc::AF_INET6 as u8;
    result.sin6_addr.s6_addr = address.octets();
    result.sin6_scope_id = scope;
    result
}
fn mask64() -> Ipv6Addr {
    Ipv6Addr::from([
        255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0,
    ])
}
enum AddressObservation {
    Present(AddressRow),
    Absent(io::Error),
}
struct AddressRow {
    address: Ipv6Addr,
    scope: u32,
    mask: Ipv6Addr,
    flags: u32,
    valid: u32,
    preferred: u32,
}
impl AddressRow {
    fn matches(&self, receipt: AddressReceipt) -> bool {
        self.address == receipt.address
            && self.scope == 0
            && self.mask == mask64()
            && receipt.prefix == 64
            && self.valid == u32::MAX
            && self.preferred == u32::MAX
            && self.flags & INVALID_ADDRESS_FLAGS == 0
    }
}
enum RouteObservation {
    Present(Row),
    Absent(io::Error),
}
enum Reply {
    Ack,
    Route(Row),
    Absent(io::Error),
}
#[derive(Clone, Copy)]
struct Pending {
    request: Request,
    sent: bool,
}
struct RouteControl {
    fd: OwnedFd,
    pid: i32,
    pending: Option<Pending>,
}
impl RouteControl {
    fn new() -> io::Result<Self> {
        let fd = owned(unsafe { libc::socket(libc::PF_ROUTE, libc::SOCK_RAW, libc::AF_INET6) })?;
        configure_fd(fd.as_raw_fd())?;
        let enabled = 1i32;
        // SAFETY: this socket needs its own looped-back ACKs; the input is an
        // initialized scalar whose size is passed exactly.
        if unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_USELOOPBACK,
                (&enabled as *const i32).cast(),
                size_of::<i32>() as _,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            pid: unsafe { libc::getpid() },
            pending: None,
        })
    }
    fn request(
        &self,
        kind: u8,
        query: Query,
        gateway: Option<Address>,
        source: Option<Ipv6Addr>,
        flags: u32,
    ) -> io::Result<Request> {
        let sequence = NEXT_SEQUENCE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| codec::invalid("native route sequence exhausted"))?;
        Request::new(kind, self.pid, sequence, query, gateway, source, flags)
    }
    fn exchange(
        &mut self,
        request: Request,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<Reply, Error> {
        require_response(scratch)?;
        let pending = match self.pending {
            Some(pending) => {
                if !pending.request.same_operation(&request) {
                    return Err(Error::Conflict(
                        "original native routing request still needs collection",
                    ));
                }
                pending
            }
            None => {
                let pending = Pending {
                    request,
                    sent: false,
                };
                self.pending = Some(pending);
                pending
            }
        };
        let request = pending.request;
        if !pending.sent {
            loop {
                wait(self.fd.as_raw_fd(), libc::POLLOUT, deadline)?;
                let bytes = request.bytes();
                // SAFETY: PF_ROUTE consumes exactly this borrowed datagram.
                let length =
                    unsafe { libc::write(self.fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
                if length < 0 {
                    let error = io::Error::last_os_error();
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) {
                        continue;
                    }
                    // route_output may return an errno after queueing the
                    // actual ACK. Preserve both original result and request.
                    self.pending
                        .as_mut()
                        .expect("installed original request")
                        .sent = true;
                    return Err(error.into());
                }
                self.pending
                    .as_mut()
                    .expect("installed original request")
                    .sent = true;
                if length as usize != bytes.len() {
                    return Err(codec::invalid("short PF_ROUTE request write").into());
                }
                break;
            }
        }
        loop {
            wait(self.fd.as_raw_fd(), libc::POLLIN, deadline)?;
            let mut vector = libc::iovec {
                iov_base: scratch.as_mut_ptr().cast(),
                iov_len: CONTROL_RESPONSE_CAPACITY,
            };
            let mut header: libc::msghdr = unsafe { zeroed() };
            header.msg_iov = &mut vector;
            header.msg_iovlen = 1;
            // SAFETY: caller's admitted response destination is at least the
            // entire u_short rtm_msglen protocol range, checked before send.
            let length = unsafe { libc::recvmsg(self.fd.as_raw_fd(), &mut header, 0) };
            if length < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    continue;
                }
                return Err(error.into());
            }
            if length as usize > CONTROL_RESPONSE_CAPACITY
                || header.msg_flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
            {
                return Err(codec::invalid(
                    "truncated PF_ROUTE datagram despite protocol-sized backing",
                )
                .into());
            }
            let bytes = &scratch[..length as usize];
            if bytes.len() < 4 || !matches!(bytes[3], codec::ADD | codec::DELETE | codec::GET) {
                continue;
            }
            let message = Message::parse(bytes)?;
            if !message.matches(&request) {
                continue;
            }
            match message.status(&request) {
                Err(error) => {
                    // A valid matched native status completes the request.
                    // Malformed protocol evidence cannot certify that an
                    // already-sent mutation did not take effect.
                    if error.raw_os_error().is_some() {
                        self.pending = None;
                    }
                    if request.kind == codec::GET && error.raw_os_error() == Some(libc::ESRCH) {
                        return Ok(Reply::Absent(error));
                    }
                    return Err(error.into());
                }
                Ok(()) => {}
            }
            self.pending = None;
            return if request.kind == codec::GET {
                Ok(Reply::Route(message.row()?))
            } else {
                Ok(Reply::Ack)
            };
        }
    }
}

fn require_response(scratch: &[u8]) -> Result<(), Error> {
    if scratch.len() < CONTROL_RESPONSE_CAPACITY {
        return Err(Error::ResponseStorage {
            required: CONTROL_RESPONSE_CAPACITY,
        });
    }
    Ok(())
}
fn check_clock(deadline: Instant) -> Result<(), Error> {
    if Instant::now() >= deadline {
        Err(Error::Deadline)
    } else {
        Ok(())
    }
}
fn timeout(deadline: Instant) -> Result<i32, Error> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or(Error::Deadline)?;
    Ok(remaining
        .as_millis()
        .saturating_add(1)
        .min(i32::MAX as u128) as i32)
}
fn wait(fd: RawFd, events: libc::c_short, deadline: Instant) -> Result<(), Error> {
    loop {
        let mut item = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let count = unsafe { libc::poll(&mut item, 1, timeout(deadline)?) };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if item.revents & libc::POLLNVAL != 0 {
            return Err(io::Error::from_raw_os_error(libc::EBADF).into());
        }
        if item.revents & (events | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(());
        }
    }
}
fn packet_wait(
    fd: RawFd,
    interrupt: RawFd,
    events: libc::c_short,
    deadline: Option<Instant>,
) -> Result<(), Error> {
    loop {
        let mut items = [
            libc::pollfd {
                fd,
                events,
                revents: 0,
            },
            libc::pollfd {
                fd: interrupt,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let count = unsafe {
            libc::poll(
                items.as_mut_ptr(),
                2,
                deadline.map(timeout).transpose()?.unwrap_or(-1),
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if items[1].revents != 0 {
            return Err(Error::Interrupted);
        }
        if items[0].revents & libc::POLLNVAL != 0 {
            return Err(io::Error::from_raw_os_error(libc::EBADF).into());
        }
        if items[0].revents & (events | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(());
        }
    }
}
fn owned(raw: RawFd) -> io::Result<OwnedFd> {
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}
fn configure_fd(fd: RawFd) -> io::Result<()> {
    // SAFETY: private caller holds a fresh live fd; F_GET*/F_SET* take only the
    // documented scalar argument. No other task has received the fd yet.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
        let status = libc::fcntl(fd, libc::F_GETFL);
        if status < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFL, status | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
fn identity(fd: RawFd) -> io::Result<Identity> {
    let mut name = [0; libc::IFNAMSIZ];
    let mut length = name.len() as libc::socklen_t;
    // SAFETY: initialized name buffer/length describe this exact output. This
    // gets the name from the live original utun fd, not from an interface scan.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SYSPROTO_CONTROL,
            libc::UTUN_OPT_IFNAME,
            name.as_mut_ptr().cast(),
            &mut length,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize > name.len() || length == 0 || !name[..length as usize].contains(&0) {
        return Err(codec::invalid("unterminated owned utun name"));
    }
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    if index == 0 {
        return Err(io::Error::last_os_error());
    }
    let index = u16::try_from(index)
        .map_err(|_| codec::invalid("utun index exceeds native routing-header field"))?;
    Ok(Identity { index, name })
}
fn validate_packet(packet: &[u8], mtu: u16) -> io::Result<()> {
    if packet.len() < 40
        || packet.len() > usize::from(mtu)
        || packet[0] >> 4 != 6
        || usize::from(u16::from_be_bytes([packet[4], packet[5]])) + 40 != packet.len()
    {
        return Err(codec::invalid("invalid/truncated native IPv6 packet"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

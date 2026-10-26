//! Linux's synchronous staged native adapter.
//!
//! This owner creates only an unnamed, exclusive, non-persistent TUN. It never
//! adopts an existing interface and never shells out. Its caller owns the real
//! blocking-operation joins and pays the complete owner, operation closures,
//! packet buffers and response scratch before launch. No method spawns work.

mod codec;

use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::mem::{size_of, zeroed};
use std::net::Ipv6Addr;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::Instant;

pub(crate) use codec::PendingResponse;
use codec::{Attributes, Message, Messages, Request};

const NEW_LINK: u16 = 16;
const GET_LINK: u16 = 18;
const NEW_ADDRESS: u16 = 20;
const DELETE_ADDRESS: u16 = 21;
const GET_ADDRESS: u16 = 22;
const NEW_ROUTE: u16 = 24;
const DELETE_ROUTE: u16 = 25;
const GET_ROUTE: u16 = 26;
const ADDRESS_TENTATIVE: u32 = 0x40;
const ADDRESS_DAD_FAILED: u32 = 0x08;
const ADDRESS_NO_PREFIX_ROUTE: u32 = 0x200;
const ADDRESS_PERMANENT: u32 = 0x80;
const ROUTE_TABLE: u32 = 254;
const ROUTE_PRIORITY: u32 = 1024;
const ROUTE_PROTOCOL_STATIC: u8 = 4;

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
            || self.server.is_unspecified()
            || self.server.is_multicast()
            || self.client.octets()[..8] != self.server.octets()[..8]
        {
            return Err(Error::Conflict(
                "invalid authenticated IPv6 /64 native plan",
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
    /// This is the original syscall/ACK error object, never a formatted copy.
    Native(io::Error),
    Conflict(&'static str),
    /// The pending datagram remains unread. The owner and mutation receipts
    /// remain live; the caller may retry with actual paid backing of this size.
    ResponseStorage {
        required: usize,
    },
    /// The owned local interrupt descriptor was signalled. This is not a
    /// fabricated syscall errno or the caller's genuine elapsed timer.
    Interrupted,
    Deadline,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(error) => error.fmt(f),
            Self::Conflict(message) => f.write_str(message),
            Self::ResponseStorage { required } => {
                write!(f, "native response requires {required} paid bytes")
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
    index: u32,
    name: [libc::c_char; libc::IFNAMSIZ],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AddressReceipt {
    identity: Identity,
    address: Ipv6Addr,
    prefix: u8,
    flags: u32,
    lifetimes: codec::AddressLifetimes,
    derived_local: RouteReceipt,
}

impl AddressReceipt {
    pub(crate) fn derived_local(&self) -> RouteReceipt {
        self.derived_local
    }
}

/// Linux necessarily derives a local-table /128 route from this address.
/// NOPREFIXROUTE suppresses the connected main-table route; its separate /64
/// route is installed and receipted below. Removing the owned address removes
/// its derived local route; it is never independently deleted by key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RouteReceipt {
    identity: Identity,
    destination: Ipv6Addr,
    prefix: u8,
    table: u32,
    priority: u32,
    protocol: u8,
    scope: u8,
    kind: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressReadiness {
    Tentative,
    Preferred,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Inert {
    pub identity: Identity,
    pub automatic_link_local_seen: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MtuReceipt {
    identity: Identity,
    mtu: u16,
}

#[cfg(test)]
impl Identity {
    pub(crate) fn test_new(index: u32, name_str: &str) -> Self {
        let mut name = [0 as libc::c_char; libc::IFNAMSIZ];
        for (i, b) in name_str.bytes().take(libc::IFNAMSIZ - 1).enumerate() {
            name[i] = b as libc::c_char;
        }
        Self { index, name }
    }
}

#[cfg(test)]
impl RouteReceipt {
    pub(crate) fn test_new(
        identity: Identity,
        destination: Ipv6Addr,
        prefix: u8,
        table: u32,
    ) -> Self {
        Self {
            identity,
            destination,
            prefix,
            table,
            priority: 1024,
            protocol: 3,
            scope: 0,
            kind: 1,
        }
    }
}

#[cfg(test)]
impl AddressReceipt {
    pub(crate) fn test_new(
        identity: Identity,
        address: Ipv6Addr,
        prefix: u8,
        derived_local: RouteReceipt,
    ) -> Self {
        Self {
            identity,
            address,
            prefix,
            flags: 0,
            lifetimes: codec::AddressLifetimes {
                preferred: 0,
                valid: 0,
            },
            derived_local,
        }
    }
}

#[cfg(test)]
impl MtuReceipt {
    pub(crate) fn test_new(identity: Identity, mtu: u16) -> Self {
        Self { identity, mtu }
    }
}

/// All storage here is concrete and inline except the owned kernel resources.
/// Response storage belongs to the operation caller, not an unmetered Vec.
pub(crate) struct LinuxDevice {
    tun: OwnedFd,
    control: Netlink,
    identity: Option<Identity>,
    plan: Plan,
    address: Option<AddressReceipt>,
    route: Option<RouteReceipt>,
    derived_local: Option<RouteReceipt>,
    removed_address: Option<AddressReceipt>,
    removed_route: Option<RouteReceipt>,
    mtu: Option<MtuReceipt>,
    session: bool,
}

impl LinuxDevice {
    /// The actor stores this genuine resource before resolve/readback. All
    /// ancillary controls are prepared before successful TUNSETIFF.
    pub(crate) fn allocate(plan: Plan, deadline: Instant) -> Result<Self, Error> {
        plan.validate()?;
        if Instant::now() >= deadline {
            return Err(Error::Deadline);
        }
        // SAFETY: the constant path is terminated, flags need no variadic mode.
        let raw = unsafe {
            libc::open(
                c"/dev/net/tun".as_ptr(),
                libc::O_RDWR | libc::O_CLOEXEC | libc::O_NONBLOCK,
            )
        };
        let tun = owned(raw)?;
        let control = Netlink::open()?;
        // SAFETY: ifreq is a C plain-data request; zero gives an empty name.
        let mut request: libc::ifreq = unsafe { zeroed() };
        request.ifr_ifru.ifru_flags =
            (libc::IFF_TUN | libc::IFF_NO_PI | libc::IFF_TUN_EXCL) as libc::c_short;
        // Empty name requests a new kernel-assigned name; EXCL forbids adoption.
        // No TUNSETPERSIST or multiqueue call is made anywhere in this owner.
        ioctl(tun.as_raw_fd(), libc::TUNSETIFF as _, &mut request)?;
        Ok(Self {
            tun,
            control,
            identity: None,
            plan,
            address: None,
            route: None,
            derived_local: None,
            removed_address: None,
            removed_route: None,
            mtu: None,
            session: false,
        })
    }

    pub(crate) fn resolve_identity(&mut self, deadline: Instant) -> Result<Identity, Error> {
        if Instant::now() >= deadline {
            return Err(Error::Deadline);
        }
        let actual = identity(self.tun.as_raw_fd())?;
        if self.identity.is_some_and(|previous| previous != actual) {
            return Err(Error::Conflict("owned TUN identity changed during resolve"));
        }
        self.identity = Some(actual);
        Ok(actual)
    }

    fn resolved_identity(&self) -> Identity {
        self.identity
            .expect("private operation checked original resolved TUN identity")
    }

    pub(crate) fn inspect_inert(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<Inert, Error> {
        self.same_identity()?;
        let link = self.link(scratch, deadline)?;
        if link.up || self.session || self.address.is_some() || self.route.is_some() {
            return Err(Error::Conflict("fresh interface is already active"));
        }
        let mut automatic_link_local_seen = false;
        let index = self.resolved_identity().index;
        let client = self.plan.client;
        self.control.dump(
            GET_ADDRESS,
            &[libc::AF_INET6 as u8, 0, 0, 0, 0, 0, 0, 0],
            scratch,
            deadline,
            |message| {
                if message.kind != NEW_ADDRESS {
                    return Ok(());
                }
                let Some(row) = AddressRow::parse(message.payload)? else {
                    return Ok(());
                };
                if row.index == index {
                    if row.address == client {
                        return Err(Error::Conflict("authenticated address already exists"));
                    }
                    if row.address.segments()[0] & 0xffc0 == 0xfe80 {
                        automatic_link_local_seen = true;
                    } else {
                        return Err(Error::Conflict(
                            "fresh interface has an unexpected IPv6 address",
                        ));
                    }
                }
                Ok(())
            },
        )?;
        let wanted = self.route_receipt();
        self.control.dump(
            GET_ROUTE,
            &route_body(wanted),
            scratch,
            deadline,
            |message| {
                if message.kind == NEW_ROUTE {
                    if let Some(row) = RouteRow::parse(message.payload)? {
                        if row.output == index && row.table == ROUTE_TABLE {
                            return Err(Error::Conflict(
                                "fresh interface already has a main-table route",
                            ));
                        }
                    }
                }
                Ok(())
            },
        )?;
        Ok(Inert {
            identity: self.resolved_identity(),
            automatic_link_local_seen,
        })
    }

    pub(crate) fn configure_mtu(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        self.same_identity()?;
        let mut body = [0; 16];
        body[4..8].copy_from_slice(&self.resolved_identity().index.to_ne_bytes());
        let mut request = self.control.request(NEW_LINK, codec::ACK, &body)?;
        request.attribute(4, &u32::from(self.plan.mtu).to_ne_bytes())?;
        self.control.ack(&request, scratch, deadline)?;
        self.mtu = Some(MtuReceipt {
            identity: self.resolved_identity(),
            mtu: self.plan.mtu,
        });
        if self.link(scratch, deadline)?.mtu != u32::from(self.plan.mtu) {
            return Err(Error::Conflict("native MTU readback differs"));
        }
        Ok(())
    }

    /// IFF_UP is a link operation, not proof of address or route publication.
    pub(crate) fn start_session(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.mtu
            != Some(MtuReceipt {
                identity: self.resolved_identity(),
                mtu: self.plan.mtu,
            })
        {
            return Err(Error::Conflict(
                "native session requires the acknowledged planned MTU",
            ));
        }
        self.set_up(true, scratch, deadline)?;
        self.session = true;
        Ok(())
    }

    pub(crate) fn add_address(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<AddressReceipt, Error> {
        self.same_identity()?;
        if !self.session || self.address.is_some() {
            return Err(Error::Conflict("address stage is not empty and active"));
        }
        let receipt = self.address_plan();
        let request =
            self.address_request(NEW_ADDRESS, codec::ACK | codec::CREATE_EXCLUSIVE, receipt)?;
        self.control.ack(&request, scratch, deadline)?;
        // Store the acknowledged mutation before any fallible readback/await.
        self.address = Some(receipt);
        Ok(receipt)
    }

    pub(crate) fn address_ready(
        &mut self,
        receipt: AddressReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<AddressReadiness, Error> {
        if self.address != Some(receipt) {
            return Err(Error::Conflict("address receipt belongs to another stage"));
        }
        self.same_identity()?;
        let row = self
            .address_row(receipt, scratch, deadline)?
            .ok_or(Error::Conflict("acknowledged address disappeared"))?;
        if !row.matches(receipt) || row.flags & ADDRESS_DAD_FAILED != 0 {
            return Ok(AddressReadiness::Conflict);
        }
        if row.flags & ADDRESS_TENTATIVE != 0 {
            return Ok(AddressReadiness::Tentative);
        }
        let local = self
            .route_row(receipt.derived_local, scratch, deadline)?
            .ok_or(Error::Conflict(
                "preferred address lacks its derived local route",
            ))?;
        if !local.exact(receipt.derived_local) {
            return Err(Error::Conflict("derived local route differs"));
        }
        self.derived_local = Some(receipt.derived_local);
        Ok(AddressReadiness::Preferred)
    }

    pub(crate) fn add_route(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<RouteReceipt, Error> {
        self.same_identity()?;
        if self.address.is_none() || self.route.is_some() {
            return Err(Error::Conflict("route stage is not ready"));
        }
        let receipt = self.route_receipt();
        let request =
            self.route_request(NEW_ROUTE, codec::ACK | codec::CREATE_EXCLUSIVE, receipt)?;
        self.control.ack(&request, scratch, deadline)?;
        self.route = Some(receipt);
        Ok(receipt)
    }

    pub(crate) fn inspect_route(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.route != Some(receipt) {
            return Err(Error::Conflict("route receipt belongs to another stage"));
        }
        self.same_identity()?;
        let row = self
            .route_row(receipt, scratch, deadline)?
            .ok_or(Error::Conflict("acknowledged route disappeared"))?;
        if !row.exact(receipt) {
            return Err(Error::Conflict("native route readback differs"));
        }
        Ok(())
    }

    /// Read-before-delete is deliberately not advertised as atomic CAS. The
    /// owned fresh fd and original success receipt limit this cleanup's scope.
    pub(crate) fn remove_owned_route(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.removed_route == Some(receipt) {
            return self.finish_route_undo(receipt, scratch, deadline);
        }
        // A timed-out delete must collect its original ACK, rather than issue
        // another readback that would race/consume that pending mutation.
        if self.control.pending_ack.is_none() {
            self.inspect_route(receipt, scratch, deadline)?;
        }
        let request = self.route_request(DELETE_ROUTE, codec::ACK, receipt)?;
        self.control.ack(&request, scratch, deadline)?;
        self.route = None;
        self.removed_route = Some(receipt);
        self.finish_route_undo(receipt, scratch, deadline)
    }

    pub(crate) fn remove_owned_address(
        &mut self,
        receipt: AddressReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.removed_address == Some(receipt) {
            return self.finish_address_undo(receipt, scratch, deadline);
        }
        if self.route.is_some() || self.removed_route.is_some() || self.address != Some(receipt) {
            return Err(Error::Conflict("address cleanup is out of order"));
        }
        self.same_identity()?;
        if self.control.pending_ack.is_none() {
            let row = self
                .address_row(receipt, scratch, deadline)?
                .ok_or(Error::Conflict("owned address disappeared before undo"))?;
            if !row.matches(receipt) {
                return Err(Error::Conflict("owned address was replaced"));
            }
            if let Some(local) = self.route_row(receipt.derived_local, scratch, deadline)? {
                if !local.exact(receipt.derived_local) {
                    return Err(Error::Conflict("derived local route was replaced"));
                }
            } else if self.derived_local.is_some() {
                return Err(Error::Conflict(
                    "previously observed derived local route disappeared",
                ));
            }
        }
        let request = self.address_request(DELETE_ADDRESS, codec::ACK, receipt)?;
        self.control.ack(&request, scratch, deadline)?;
        self.address = None;
        self.derived_local = None;
        self.removed_address = Some(receipt);
        self.finish_address_undo(receipt, scratch, deadline)
    }

    /// Caller must first signal interruption and collect the actual packet
    /// joins. This method creates no tasks and does not claim to join them.
    pub(crate) fn end_session(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.address.is_some()
            || self.route.is_some()
            || self.removed_address.is_some()
            || self.removed_route.is_some()
        {
            return Err(Error::Conflict(
                "native row undo remains during session close",
            ));
        }
        self.set_up(false, scratch, deadline)?;
        self.session = false;
        Ok(())
    }

    /// A refusal returns this exact owner. It cannot close the live fd while
    /// a caller still needs its original native result or reverse undo.
    #[allow(clippy::result_large_err)]
    pub(crate) fn close(self) -> Result<(), (Self, Error)> {
        if self.session
            || self.address.is_some()
            || self.route.is_some()
            || self.derived_local.is_some()
            || self.removed_address.is_some()
            || self.removed_route.is_some()
            || self.control.pending_ack.is_some()
            || self.control.pending_dump.is_some()
            || self.control.dump_error.is_some()
            || self.control.response.pending().is_some()
            || self.control.loss_consume_error.is_some()
        {
            return Err((
                self,
                Error::Conflict(
                    "explicit close requires collected controls and completed native undo",
                ),
            ));
        }
        drop(self);
        Ok(())
    }

    /// Collect an earlier mutation that reached sendto before its wait timed
    /// out or needed larger paid response backing. Never resend it. A shutdown
    /// owner must call this before choosing which exact reverse receipts exist.
    /// Every success updates this owner before returning to the caller.
    pub(crate) fn collect_pending_mutation(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<bool, Error> {
        let Some(request) = self.control.pending_ack else {
            return Ok(false);
        };
        let kind = codec::u16_at(request.bytes(), 4)?;
        let address = self.address;
        let route = self.route;
        let mtu = if kind == NEW_LINK {
            request_mtu(&request)?
        } else {
            None
        };
        self.control.ack(&request, scratch, deadline)?;
        match kind {
            NEW_ADDRESS => self.address = Some(self.address_plan()),
            DELETE_ADDRESS => {
                self.address = None;
                self.derived_local = None;
                self.removed_address = address;
            }
            NEW_ROUTE => self.route = Some(self.route_receipt()),
            DELETE_ROUTE => {
                self.route = None;
                self.removed_route = route;
            }
            NEW_LINK => {
                if let Some(mtu) = mtu {
                    self.mtu = Some(MtuReceipt {
                        identity: self.resolved_identity(),
                        mtu,
                    });
                }
                let body = &request.bytes()[codec::HEADER..];
                let change = u32::from_ne_bytes(
                    body[12..16]
                        .try_into()
                        .expect("private link request has a complete fixed body"),
                );
                if change & libc::IFF_UP as u32 != 0 {
                    let flags = u32::from_ne_bytes(
                        body[8..12]
                            .try_into()
                            .expect("private link request has a complete fixed body"),
                    );
                    self.session = flags & libc::IFF_UP as u32 != 0;
                }
            }
            _ => unreachable!("only private typed constructors can create a pending mutation"),
        }
        Ok(true)
    }

    pub(crate) fn address_receipt(&self) -> Option<AddressReceipt> {
        self.address
    }
    pub(crate) fn installed_route_receipt(&self) -> Option<RouteReceipt> {
        self.route
    }
    pub(crate) fn derived_local_receipt(&self) -> Option<RouteReceipt> {
        self.derived_local
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

    pub(crate) fn inspect_mtu(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<MtuReceipt, Error> {
        self.same_identity()?;
        let receipt = self
            .mtu
            .ok_or(Error::Conflict("native MTU has not been acknowledged"))?;
        if self.link(scratch, deadline)?.mtu != u32::from(receipt.mtu) {
            return Err(Error::Conflict("native MTU changed after acknowledgement"));
        }
        Ok(receipt)
    }

    /// Returning this typed state needs no admission or allocation. In
    /// particular a closed-zero caller keeps this same owner and original
    /// receipts; it does not retry with an invented receive cap or report undo
    /// complete while an exact observed datagram remains unread.
    pub(crate) fn pending_response(&self) -> Option<PendingResponse> {
        self.control.response.pending()
    }

    pub(crate) fn take_loss_consume_error(&mut self) -> Option<Error> {
        self.control.loss_consume_error.take()
    }

    /// The event stays signalled, so every in-flight/subsequent packet call can
    /// leave. Keeping this fd in the same owner prevents an fd-reuse wake race.
    pub(crate) fn prepare_packet_io(&self, deadline: Instant) -> Result<PacketIo, Error> {
        self.same_identity()?;
        if Instant::now() >= deadline {
            return Err(Error::Deadline);
        }
        // SAFETY: dup preserves this original open TUN file description and
        // creates no second interface. The actor stores the result immediately.
        let fd = owned(unsafe { libc::fcntl(self.tun.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) })?;
        Ok(PacketIo {
            fd,
            mtu: self.plan.mtu,
        })
    }
    fn same_identity(&self) -> Result<(), Error> {
        let expected = self.identity.ok_or(Error::Conflict(
            "acquired TUN identity has not been resolved in place",
        ))?;
        if identity(self.tun.as_raw_fd())? != expected {
            return Err(Error::Conflict("owned TUN identity changed"));
        }
        Ok(())
    }

    fn link(&mut self, scratch: &mut [u8], deadline: Instant) -> Result<LinkRow, Error> {
        let identity = self.resolved_identity();
        let mut found = None;
        self.control.dump(
            GET_LINK,
            &[
                libc::AF_UNSPEC as u8,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            ],
            scratch,
            deadline,
            |message| {
                if message.kind != NEW_LINK {
                    return Ok(());
                }
                if let Some(row) = LinkRow::parse(message.payload, identity)? {
                    if found.replace(row).is_some() {
                        return Err(Error::Conflict("duplicate owned link readback"));
                    }
                }
                Ok(())
            },
        )?;
        found.ok_or(Error::Conflict("owned link is absent"))
    }

    fn set_up(&mut self, up: bool, scratch: &mut [u8], deadline: Instant) -> Result<(), Error> {
        self.same_identity()?;
        let mut body = [0; 16];
        body[4..8].copy_from_slice(&self.resolved_identity().index.to_ne_bytes());
        body[8..12].copy_from_slice(&(if up { libc::IFF_UP as u32 } else { 0 }).to_ne_bytes());
        body[12..16].copy_from_slice(&(libc::IFF_UP as u32).to_ne_bytes());
        let request = self.control.request(NEW_LINK, codec::ACK, &body)?;
        self.control.ack(&request, scratch, deadline)?;
        self.session = up;
        if self.link(scratch, deadline)?.up != up {
            return Err(Error::Conflict("native link-state readback differs"));
        }
        Ok(())
    }

    fn address_request(
        &mut self,
        kind: u16,
        flags: u16,
        receipt: AddressReceipt,
    ) -> Result<Request, Error> {
        let mut body = [libc::AF_INET6 as u8, receipt.prefix, 0, 0, 0, 0, 0, 0];
        body[4..8].copy_from_slice(&receipt.identity.index.to_ne_bytes());
        let mut request = self.control.request(kind, flags, &body)?;
        request.attribute(1, &receipt.address.octets())?;
        request.attribute(2, &receipt.address.octets())?;
        request.attribute(8, &receipt.flags.to_ne_bytes())?;
        Ok(request)
    }

    fn route_receipt(&self) -> RouteReceipt {
        RouteReceipt {
            identity: self.resolved_identity(),
            destination: self.plan.network(),
            prefix: self.plan.prefix,
            table: ROUTE_TABLE,
            priority: ROUTE_PRIORITY,
            protocol: ROUTE_PROTOCOL_STATIC,
            scope: 0,
            kind: 1,
        }
    }

    fn address_plan(&self) -> AddressReceipt {
        AddressReceipt {
            identity: self.resolved_identity(),
            address: self.plan.client,
            prefix: self.plan.prefix,
            flags: ADDRESS_NO_PREFIX_ROUTE,
            lifetimes: codec::AddressLifetimes::INFINITE,
            derived_local: RouteReceipt {
                identity: self.resolved_identity(),
                destination: self.plan.client,
                prefix: 128,
                table: 255,
                priority: 0,
                protocol: 2,
                scope: 0,
                kind: 2,
            },
        }
    }

    fn finish_route_undo(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.route_row(receipt, scratch, deadline)?.is_some() {
            return Err(Error::Conflict(
                "deleted route still exists or was replaced",
            ));
        }
        self.removed_route = None;
        Ok(())
    }

    fn finish_address_undo(
        &mut self,
        receipt: AddressReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.address_row(receipt, scratch, deadline)?.is_some() {
            return Err(Error::Conflict(
                "deleted address still exists or was replaced",
            ));
        }
        if self
            .route_row(receipt.derived_local, scratch, deadline)?
            .is_some()
        {
            return Err(Error::Conflict(
                "derived local route remains after address undo",
            ));
        }
        self.removed_address = None;
        Ok(())
    }

    fn route_request(
        &mut self,
        kind: u16,
        flags: u16,
        receipt: RouteReceipt,
    ) -> Result<Request, Error> {
        let mut request = self.control.request(kind, flags, &route_body(receipt))?;
        request.attribute(1, &receipt.destination.octets())?;
        request.attribute(4, &receipt.identity.index.to_ne_bytes())?;
        request.attribute(6, &receipt.priority.to_ne_bytes())?;
        Ok(request)
    }

    fn address_row(
        &mut self,
        receipt: AddressReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<Option<AddressRow>, Error> {
        let mut found = None;
        self.control.dump(
            GET_ADDRESS,
            &[libc::AF_INET6 as u8, 0, 0, 0, 0, 0, 0, 0],
            scratch,
            deadline,
            |message| {
                if message.kind == NEW_ADDRESS {
                    if let Some(row) = AddressRow::parse(message.payload)? {
                        if row.index == receipt.identity.index
                            && row.address == receipt.address
                            && found.replace(row).is_some()
                        {
                            return Err(Error::Conflict("duplicate address readback"));
                        }
                    }
                }
                Ok(())
            },
        )?;
        Ok(found)
    }

    fn route_row(
        &mut self,
        receipt: RouteReceipt,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<Option<RouteRow>, Error> {
        let mut found = None;
        self.control.dump(
            GET_ROUTE,
            &route_body(receipt),
            scratch,
            deadline,
            |message| {
                if message.kind == NEW_ROUTE {
                    if let Some(row) = RouteRow::parse(message.payload)? {
                        if row.destination == receipt.destination
                            && row.prefix == receipt.prefix
                            && row.output == receipt.identity.index
                            && row.table == receipt.table
                            && found.replace(row).is_some()
                        {
                            return Err(Error::Conflict("duplicate route readback"));
                        }
                    }
                }
                Ok(())
            },
        )?;
        Ok(found)
    }
}

pub(crate) struct PacketInterrupt {
    fd: OwnedFd,
}
impl PacketInterrupt {
    pub(crate) fn new() -> io::Result<Self> {
        // SAFETY: this creates the one real external packet interrupt event.
        let fd = owned(unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) })?;
        Ok(Self { fd })
    }
    pub(crate) fn trigger(&self) -> io::Result<()> {
        let one = 1u64.to_ne_bytes();
        // SAFETY: the eventfd and the initialized eight-byte input are live.
        let result = unsafe { libc::write(self.fd.as_raw_fd(), one.as_ptr().cast(), one.len()) };
        if result == one.len() as isize {
            return Ok(());
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                Ok(())
            } else {
                Err(error)
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short interrupt event write",
            ))
        }
    }
}
/// Genuine alias of the same original TUN file description. Jobs own no
/// control-device borrow while waiting; the actor drops this only after joins.
pub(crate) struct PacketIo {
    fd: OwnedFd,
    mtu: u16,
}
impl PacketIo {
    #[cfg(test)]
    pub(crate) fn test_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&self.fd)
    }
    #[cfg(test)]
    pub(crate) fn from_fd(fd: OwnedFd, mtu: u16) -> Self {
        Self { fd, mtu }
    }

    /// Receive into the caller's already paid MTU+1 destination. No copy or
    /// allocation occurs. The extra byte plus IPv6 length detects truncation.
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
                "packet destination needs MTU plus one byte",
            ));
        }
        loop {
            packet_wait(
                self.fd.as_raw_fd(),
                interrupt.fd.as_raw_fd(),
                libc::POLLIN,
                deadline,
            )?;
            // SAFETY: the live TUN fd writes only within the unique slice.
            let result = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    destination.as_mut_ptr().cast(),
                    destination.len(),
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    continue;
                }
                return Err(error.into());
            }
            let length = result as usize;
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
                interrupt.fd.as_raw_fd(),
                libc::POLLOUT,
                deadline,
            )?;
            // SAFETY: packet remains borrowed for this single whole-packet call.
            let result =
                unsafe { libc::write(self.fd.as_raw_fd(), packet.as_ptr().cast(), packet.len()) };
            if result < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    continue;
                }
                return Err(error.into());
            }
            if result as usize != packet.len() {
                return Err(
                    io::Error::new(io::ErrorKind::WriteZero, "short native packet write").into(),
                );
            }
            return Ok(());
        }
    }
}

fn route_body(receipt: RouteReceipt) -> [u8; 12] {
    [
        libc::AF_INET6 as u8,
        receipt.prefix,
        0,
        0,
        receipt.table as u8,
        receipt.protocol,
        receipt.scope,
        receipt.kind,
        0,
        0,
        0,
        0,
    ]
}

fn request_mtu(request: &Request) -> io::Result<Option<u16>> {
    let body = request
        .bytes()
        .get(codec::HEADER + 16..)
        .ok_or_else(|| codec::invalid("short private link request"))?;
    let mut mtu = None;
    for attribute in Attributes::new(body) {
        let (kind, value) = attribute?;
        if kind == 4 {
            if value.len() != 4 || mtu.is_some() {
                return Err(codec::invalid("invalid private MTU request"));
            }
            mtu = Some(
                u16::try_from(codec::u32_at(value, 0)?)
                    .map_err(|_| codec::invalid("private MTU exceeds plan representation"))?,
            );
        }
    }
    Ok(mtu)
}

fn owned(raw: RawFd) -> io::Result<OwnedFd> {
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: each caller passes a newly returned fd exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

fn ioctl<T>(fd: RawFd, operation: libc::c_ulong, request: &mut T) -> io::Result<()> {
    // SAFETY: each private caller selects the ioctl for exactly its C request
    // type and holds a live descriptor plus exclusive initialized request.
    if unsafe { libc::ioctl(fd, operation as _, request as *mut T) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn identity(tun: RawFd) -> io::Result<Identity> {
    // SAFETY: these ifreq values are plain C data with valid all-zero state.
    let mut tun_request: libc::ifreq = unsafe { zeroed() };
    ioctl(tun, libc::TUNGETIFF as _, &mut tun_request)?;
    if !tun_request.ifr_name.contains(&0) {
        return Err(codec::invalid("unterminated owned TUN name"));
    }
    // SAFETY: TUNGETIFF initialized this union's flag field.
    let flags = unsafe { tun_request.ifr_ifru.ifru_flags } as libc::c_int;
    if flags & (libc::IFF_TUN | libc::IFF_NO_PI) != libc::IFF_TUN | libc::IFF_NO_PI
        || flags & libc::IFF_PERSIST != 0
    {
        return Err(codec::invalid("owned TUN mode changed"));
    }
    // SAFETY: the prior check proves this fixed C name is NUL-terminated.
    let index = unsafe { libc::if_nametoindex(tun_request.ifr_name.as_ptr()) };
    if index == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity {
        index,
        name: tun_request.ifr_name,
    })
}

fn validate_packet(packet: &[u8], mtu: u16) -> io::Result<()> {
    if packet.len() < 40 || packet.len() > usize::from(mtu) || packet[0] >> 4 != 6 {
        return Err(codec::invalid("invalid or truncated native IPv6 packet"));
    }
    let length = u16::from_be_bytes([packet[4], packet[5]]) as usize + 40;
    if length != packet.len() {
        return Err(codec::invalid("native IPv6 declared length differs"));
    }
    Ok(())
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
        let mut pollfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // SAFETY: one initialized pollfd remains exclusive for this call.
        let result = unsafe { libc::poll(&mut pollfd, 1, timeout(deadline)?) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if result == 0 {
            continue;
        }
        if pollfd.revents & libc::POLLNVAL != 0 {
            return Err(io::Error::from_raw_os_error(libc::EBADF).into());
        }
        if pollfd.revents & (events | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(());
        }
    }
}

fn packet_wait(
    tun: RawFd,
    interrupt: RawFd,
    events: libc::c_short,
    deadline: Option<Instant>,
) -> Result<(), Error> {
    loop {
        let mut fds = [
            libc::pollfd {
                fd: tun,
                events,
                revents: 0,
            },
            libc::pollfd {
                fd: interrupt,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: this initialized two-element array remains live and unique.
        let result = unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                2,
                deadline.map(timeout).transpose()?.unwrap_or(-1),
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if fds[1].revents != 0 {
            return Err(Error::Interrupted);
        }
        if fds[0].revents & libc::POLLNVAL != 0 {
            return Err(io::Error::from_raw_os_error(libc::EBADF).into());
        }
        if fds[0].revents & (events | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(());
        }
    }
}

struct Netlink {
    fd: OwnedFd,
    port: u32,
    sequence: u32,
    pending_ack: Option<Request>,
    pending_dump: Option<Request>,
    dump_error: Option<Error>,
    response: codec::ResponseState,
    loss_consume_error: Option<Error>,
}

impl Netlink {
    fn open() -> io::Result<Self> {
        // SAFETY: socket has only scalar arguments and returns a new fd.
        let fd = owned(unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::NETLINK_ROUTE,
            )
        })?;
        // SAFETY: sockaddr_nl's padding is initialized, pid/groups zero request
        // an assigned port and no multicast subscriptions.
        let mut address: libc::sockaddr_nl = unsafe { zeroed() };
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        // SAFETY: pointer and size describe exactly the live sockaddr_nl.
        if unsafe {
            libc::bind(
                fd.as_raw_fd(),
                (&address as *const libc::sockaddr_nl).cast(),
                size_of::<libc::sockaddr_nl>() as _,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut length = size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        // SAFETY: getsockname writes only within this initialized value.
        if unsafe {
            libc::getsockname(
                fd.as_raw_fd(),
                (&mut address as *mut libc::sockaddr_nl).cast(),
                &mut length,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        if length as usize != size_of::<libc::sockaddr_nl>() || address.nl_pid == 0 {
            return Err(codec::invalid("invalid assigned netlink port"));
        }
        Ok(Self {
            fd,
            port: address.nl_pid,
            sequence: 0,
            pending_ack: None,
            pending_dump: None,
            dump_error: None,
            response: codec::ResponseState::default(),
            loss_consume_error: None,
        })
    }

    fn request(&mut self, kind: u16, flags: u16, body: &[u8]) -> io::Result<Request> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| codec::invalid("netlink sequence exhausted"))?;
        Request::new(kind, flags, self.sequence, self.port, body)
    }

    fn send(&self, request: &Request, deadline: Instant) -> Result<(), Error> {
        // SAFETY: plain sockaddr_nl initialized for the kernel (pid zero).
        let mut kernel: libc::sockaddr_nl = unsafe { zeroed() };
        kernel.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        loop {
            wait(self.fd.as_raw_fd(), libc::POLLOUT, deadline)?;
            let bytes = request.bytes();
            // SAFETY: all borrowed addresses/input stay live during sendto.
            let result = unsafe {
                libc::sendto(
                    self.fd.as_raw_fd(),
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    0,
                    (&kernel as *const libc::sockaddr_nl).cast(),
                    size_of::<libc::sockaddr_nl>() as _,
                )
            };
            if result < 0 {
                let error = io::Error::last_os_error();
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    continue;
                }
                return Err(error.into());
            }
            if result as usize != bytes.len() {
                return Err(codec::invalid("short netlink datagram send").into());
            }
            return Ok(());
        }
    }

    fn receive<'a>(
        &mut self,
        scratch: &'a mut [u8],
        sequence: u32,
        deadline: Instant,
    ) -> Result<&'a [u8], Error> {
        if self.loss_consume_error.is_some() {
            return Err(Error::Conflict(
                "original loss-consume result must be collected before another native receive",
            ));
        }
        loop {
            wait(self.fd.as_raw_fd(), libc::POLLIN, deadline)?;
            let mut header = [0; codec::HEADER];
            let (required, sender, flags) = receive_datagram(
                self.fd.as_raw_fd(),
                &mut header,
                libc::MSG_PEEK | libc::MSG_TRUNC,
            )?;
            if sender == 0 && required < codec::HEADER {
                return Err(codec::invalid("short kernel netlink datagram").into());
            }
            if sender == 0 && codec::u16_at(&header, 4)? == codec::OVERRUN {
                // The loss is already acquired. Consume this exact queued
                // datagram so future drain can progress; a later consume error
                // remains additional evidence and cannot replace ENOBUFS.
                let (original, cleanup) =
                    codec::consume_loss(io::Error::from_raw_os_error(libc::ENOBUFS), || {
                        self.discard(deadline)
                    });
                if self.loss_consume_error.is_none() {
                    self.loss_consume_error = cleanup;
                }
                return Err(original.into());
            }
            if sender != 0
                || codec::u32_at(&header, 8)? != sequence
                || codec::u32_at(&header, 12)? != self.port
            {
                // Discard a whole unrelated datagram without retaining its
                // rows or allocating storage proportional to host inventory.
                self.discard(deadline)?;
                continue;
            }
            if flags & libc::MSG_CTRUNC != 0 {
                return Err(codec::invalid("truncated netlink ancillary data").into());
            }
            if let Err(pending) = self.response.check(
                required,
                scratch.len(),
                sequence,
                codec::u16_at(&header, 4)?,
            ) {
                return Err(Error::ResponseStorage {
                    required: pending.required,
                });
            }
            let (length, sender, flags) = receive_datagram(self.fd.as_raw_fd(), scratch, 0)?;
            self.response.consumed();
            if sender != 0
                || length != required
                || flags & (libc::MSG_TRUNC | libc::MSG_CTRUNC) != 0
            {
                return Err(codec::invalid("truncated or replaced netlink datagram").into());
            }
            return Ok(&scratch[..length]);
        }
    }

    fn discard(&self, deadline: Instant) -> Result<(), Error> {
        loop {
            match receive_datagram(self.fd.as_raw_fd(), &mut [], libc::MSG_TRUNC) {
                Ok(_) => return Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    wait(self.fd.as_raw_fd(), libc::POLLIN, deadline)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn ack(
        &mut self,
        request: &Request,
        scratch: &mut [u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        if self.pending_dump.is_some() {
            self.consume_dump(scratch, deadline, |_| Ok(()))?;
        }
        let request = match self.pending_ack {
            Some(pending) => {
                if !pending.same_operation(request) {
                    return Err(Error::Conflict(
                        "another native mutation still awaits its original ACK",
                    ));
                }
                pending
            }
            None => {
                self.send(request, deadline)?;
                self.pending_ack = Some(*request);
                *request
            }
        };
        let sequence = codec::u32_at(request.bytes(), 8)?;
        loop {
            for message in Messages::new(self.receive(scratch, sequence, deadline)?) {
                let message = message?;
                if message.sequence != sequence || message.port != self.port {
                    continue;
                }
                if message.flags & codec::DUMP_INTERRUPTED != 0 {
                    return Err(io::Error::from_raw_os_error(libc::EINTR).into());
                }
                match message.kind {
                    codec::ERROR => {
                        let result = request
                            .ack(message.payload, message.flags)
                            .map_err(Error::from);
                        self.pending_ack = None;
                        return result;
                    }
                    codec::OVERRUN => {
                        return Err(io::Error::from_raw_os_error(libc::ENOBUFS).into())
                    }
                    _ => {
                        return Err(codec::invalid(
                            "unexpected message type for native mutation ACK",
                        )
                        .into())
                    }
                }
            }
        }
    }

    fn dump(
        &mut self,
        kind: u16,
        body: &[u8],
        scratch: &mut [u8],
        deadline: Instant,
        mut row: impl FnMut(Message<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        if self.pending_ack.is_some() {
            return Err(Error::Conflict(
                "native mutation ACK must be collected before readback",
            ));
        }
        // A previous ResponseStorage/deadline kept the old dump alive. Drain
        // that exact request before starting a fresh complete observation;
        // resuming only its suffix would lose already-seen target rows.
        if self.pending_dump.is_some() {
            self.consume_dump(scratch, deadline, |_| Ok(()))?;
        }
        let request = self.request(kind, codec::DUMP, body)?;
        self.send(&request, deadline)?;
        self.pending_dump = Some(request);
        self.consume_dump(scratch, deadline, &mut row)
    }

    fn consume_dump(
        &mut self,
        scratch: &mut [u8],
        deadline: Instant,
        mut row: impl FnMut(Message<'_>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let request = self
            .pending_dump
            .ok_or(Error::Conflict("no original dump to collect"))?;
        let sequence = codec::u32_at(request.bytes(), 8)?;
        let expected_kind = codec::u16_at(request.bytes(), 4)?
            .checked_sub(2)
            .ok_or_else(|| codec::invalid("invalid pending dump operation"))?;
        loop {
            let bytes = match self.receive(scratch, sequence, deadline) {
                Ok(bytes) => bytes,
                Err(error) => return Err(self.dump_error.take().unwrap_or(error)),
            };
            for message in Messages::new(bytes) {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        if self.dump_error.is_none() {
                            self.dump_error = Some(error.into());
                        }
                        continue;
                    }
                };
                if message.sequence != sequence || message.port != self.port {
                    continue;
                }
                if message.flags & codec::DUMP_INTERRUPTED != 0 && self.dump_error.is_none() {
                    self.dump_error = Some(io::Error::from_raw_os_error(libc::EINTR).into());
                }
                match message.kind {
                    codec::DONE => {
                        let completion = dump_completion(message.payload);
                        self.pending_dump = None;
                        return match self.dump_error.take() {
                            Some(error) => Err(error),
                            None => completion,
                        };
                    }
                    codec::ERROR => {
                        let result = request
                            .ack(message.payload, message.flags)
                            .map_err(Error::from);
                        if let Err(error) = result {
                            self.pending_dump = None;
                            return Err(self.dump_error.take().unwrap_or(error));
                        }
                    }
                    codec::OVERRUN => {
                        if self.dump_error.is_none() {
                            self.dump_error =
                                Some(io::Error::from_raw_os_error(libc::ENOBUFS).into());
                        }
                    }
                    _ => {
                        if self.dump_error.is_none() {
                            let result = if message.kind != expected_kind {
                                Err(codec::invalid("wrong native readback response type").into())
                            } else if message.flags & codec::MULTI == 0 {
                                Err(codec::invalid("dump row lacks multipart flag").into())
                            } else {
                                row(message)
                            };
                            if let Err(error) = result {
                                self.dump_error = Some(error);
                            }
                        }
                    }
                }
            }
        }
    }
}

fn dump_completion(payload: &[u8]) -> Result<(), Error> {
    if payload.is_empty() {
        return Ok(());
    }
    let status = codec::u32_at(payload, 0)? as i32;
    if status < 0 && status != i32::MIN {
        return Err(io::Error::from_raw_os_error(-status).into());
    }
    if status != 0 {
        return Err(codec::invalid("invalid netlink dump completion status").into());
    }
    // Extended ACK attributes may accompany NLMSG_DONE as well.
    for attribute in Attributes::new(&payload[4..]) {
        attribute?;
    }
    Ok(())
}

fn receive_datagram(
    fd: RawFd,
    destination: &mut [u8],
    flags: libc::c_int,
) -> io::Result<(usize, u32, libc::c_int)> {
    // SAFETY: zero initializes all optional msghdr pointers and sockaddr_nl
    // padding. The iovec covers exactly the exclusive caller destination.
    let mut sender: libc::sockaddr_nl = unsafe { zeroed() };
    let mut vector = libc::iovec {
        iov_base: destination.as_mut_ptr().cast(),
        iov_len: destination.len(),
    };
    // SAFETY: msghdr is plain C data with a valid all-zero starting value.
    let mut message: libc::msghdr = unsafe { zeroed() };
    message.msg_name = (&mut sender as *mut libc::sockaddr_nl).cast();
    message.msg_namelen = size_of::<libc::sockaddr_nl>() as _;
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    // SAFETY: all output pointers remain live and unique for recvmsg.
    let result = unsafe { libc::recvmsg(fd, &mut message, flags) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if message.msg_namelen as usize != size_of::<libc::sockaddr_nl>()
        || sender.nl_family != libc::AF_NETLINK as libc::sa_family_t
    {
        return Err(codec::invalid("invalid netlink sender address"));
    }
    Ok((result as usize, sender.nl_pid, message.msg_flags))
}

#[derive(Clone, Copy)]
struct LinkRow {
    mtu: u32,
    up: bool,
}

impl LinkRow {
    fn parse(bytes: &[u8], identity: Identity) -> Result<Option<Self>, Error> {
        if bytes.len() < 16 {
            return Err(codec::invalid("short link row").into());
        }
        if codec::u32_at(bytes, 4)? != identity.index {
            return Ok(None);
        }
        let mut name = None;
        let mut mtu = None;
        for attribute in Attributes::new(&bytes[16..]) {
            let (kind, value) = attribute?;
            match kind {
                3 => {
                    if name.replace(value).is_some() {
                        return Err(codec::invalid("duplicate link name").into());
                    }
                }
                4 => {
                    if value.len() != 4 || mtu.replace(codec::u32_at(value, 0)?).is_some() {
                        return Err(codec::invalid("invalid link MTU").into());
                    }
                }
                _ => {}
            }
        }
        let length = identity
            .name
            .iter()
            .position(|v| *v == 0)
            .ok_or_else(|| codec::invalid("owned name lacks terminator"))?;
        let name = name.ok_or_else(|| codec::invalid("link name missing"))?;
        if name.len() != length + 1
            || name[length] != 0
            || name[..length]
                .iter()
                .zip(identity.name[..length].iter())
                .any(|(a, b)| *a != *b as u8)
        {
            return Err(Error::Conflict("link name no longer matches owned fd"));
        }
        Ok(Some(Self {
            mtu: mtu.ok_or_else(|| codec::invalid("link MTU missing"))?,
            up: codec::u32_at(bytes, 8)? & libc::IFF_UP as u32 != 0,
        }))
    }
}

#[derive(Clone, Copy)]
struct AddressRow {
    index: u32,
    address: Ipv6Addr,
    prefix: u8,
    scope: u8,
    flags: u32,
    has_peer: bool,
    lifetimes: Option<codec::AddressLifetimes>,
}

impl AddressRow {
    fn parse(bytes: &[u8]) -> io::Result<Option<Self>> {
        if bytes.len() < 8 {
            return Err(codec::invalid("short address row"));
        }
        if bytes[0] != libc::AF_INET6 as u8 {
            return Ok(None);
        }
        let mut local = None;
        let mut address = None;
        let mut flags = u32::from(bytes[2]);
        let mut full_flags = false;
        let mut lifetimes = None;
        for attribute in Attributes::new(&bytes[8..]) {
            let (kind, value) = attribute?;
            match kind {
                1 | 2 => {
                    let value: [u8; 16] = value
                        .try_into()
                        .map_err(|_| codec::invalid("invalid IPv6 address attribute"))?;
                    let slot = if kind == 1 { &mut address } else { &mut local };
                    if slot.replace(Ipv6Addr::from(value)).is_some() {
                        return Err(codec::invalid("duplicate IPv6 address attribute"));
                    }
                }
                8 => {
                    if value.len() != 4 || full_flags {
                        return Err(codec::invalid("invalid IPv6 flags attribute"));
                    }
                    flags = codec::u32_at(value, 0)?;
                    full_flags = true;
                }
                6 => {
                    if lifetimes
                        .replace(codec::AddressLifetimes::parse(value)?)
                        .is_some()
                    {
                        return Err(codec::invalid("duplicate IFA_CACHEINFO"));
                    }
                }
                _ => {}
            }
        }
        let has_peer = matches!((local, address), (Some(local), Some(address)) if local != address);
        Ok(Some(Self {
            index: codec::u32_at(bytes, 4)?,
            address: local
                .or(address)
                .ok_or_else(|| codec::invalid("IPv6 address missing"))?,
            prefix: bytes[1],
            scope: bytes[3],
            flags,
            has_peer,
            lifetimes,
        }))
    }

    fn matches(self, receipt: AddressReceipt) -> bool {
        self.index == receipt.identity.index && self.address == receipt.address
            && self.prefix == receipt.prefix && self.scope == 0 && !self.has_peer
            && self.flags & ADDRESS_NO_PREFIX_ROUTE != 0
            && self.flags & ADDRESS_PERMANENT != 0
            && self.lifetimes == Some(receipt.lifetimes)
            // NODAD/OPTIMISTIC/DEPRECATED must not become a DAD-ready proof.
            && self.flags & (0x02 | 0x04 | 0x20) == 0
    }
}

#[derive(Clone, Copy)]
struct RouteRow {
    destination: Ipv6Addr,
    prefix: u8,
    output: u32,
    table: u32,
    priority: u32,
    protocol: u8,
    scope: u8,
    kind: u8,
    has_gateway: bool,
    extra_key: bool,
}

impl RouteRow {
    fn parse(bytes: &[u8]) -> io::Result<Option<Self>> {
        if bytes.len() < 12 {
            return Err(codec::invalid("short route row"));
        }
        if bytes[0] != libc::AF_INET6 as u8 {
            return Ok(None);
        }
        let mut result = Self {
            destination: Ipv6Addr::UNSPECIFIED,
            prefix: bytes[1],
            output: 0,
            table: u32::from(bytes[4]),
            priority: 0,
            protocol: bytes[5],
            scope: bytes[6],
            kind: bytes[7],
            has_gateway: false,
            extra_key: bytes[2] != 0 || bytes[3] != 0 || codec::u32_at(bytes, 8)? != 0,
        };
        let mut seen = 0u32;
        for attribute in Attributes::new(&bytes[12..]) {
            let (kind, value) = attribute?;
            if kind < 32 {
                let bit = 1u32 << kind;
                if seen & bit != 0 {
                    return Err(codec::invalid("duplicate route attribute"));
                }
                seen |= bit;
            }
            match kind {
                1 => {
                    result.destination = Ipv6Addr::from(
                        <[u8; 16]>::try_from(value)
                            .map_err(|_| codec::invalid("invalid route destination"))?,
                    );
                }
                4 | 6 | 15 => {
                    if value.len() != 4 {
                        return Err(codec::invalid("invalid route scalar attribute"));
                    }
                    let scalar = codec::u32_at(value, 0)?;
                    match kind {
                        4 => result.output = scalar,
                        6 => result.priority = scalar,
                        _ => result.table = scalar,
                    }
                }
                5 => result.has_gateway = true,
                2 | 3 | 7 | 8 | 9 | 18 | 21 | 22 | 30 => result.extra_key = true,
                _ => {}
            }
        }
        if result.prefix > 128 || result.prefix != 0 && seen & (1 << 1) == 0 {
            return Err(codec::invalid("invalid route prefix"));
        }
        Ok(Some(result))
    }

    fn exact(self, receipt: RouteReceipt) -> bool {
        self.destination == receipt.destination
            && self.prefix == receipt.prefix
            && self.output == receipt.identity.index
            && self.table == receipt.table
            && self.priority == receipt.priority
            && self.protocol == receipt.protocol
            && self.scope == receipt.scope
            && self.kind == receipt.kind
            && !self.has_gateway
            && !self.extra_key
    }
}

#[cfg(test)]
mod tests;

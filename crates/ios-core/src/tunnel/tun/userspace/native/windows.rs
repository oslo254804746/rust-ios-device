//! Synchronous, creation-owned Windows exposure and packet operations.
//!
//! This adapter creates only a new Wintun adapter. It never opens an existing
//! adapter, invokes netsh, deletes the shared driver, or spawns hidden tasks.
//! The caller owns admission, blocking-operation joins, the attempt clock and
//! rollback orchestration. Keep this object alive until all packet/control
//! operations have actually joined before ending its session or closing it.
use std::error::Error;
use std::ffi::c_void;
use std::fmt;
use std::io;
use std::net::Ipv6Addr;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{self, NonNull};

use libloading::Library;
use rand::RngCore;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_BUFFER_OVERFLOW, ERROR_NOT_FOUND, ERROR_NO_MORE_ITEMS, HANDLE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceLuidToGuid, ConvertInterfaceLuidToIndex, CreateIpForwardEntry2,
    CreateUnicastIpAddressEntry, DeleteIpForwardEntry2, DeleteUnicastIpAddressEntry, GetIfEntry2,
    GetIpForwardEntry2, GetIpInterfaceEntry, GetUnicastIpAddressEntry, InitializeIpForwardEntry,
    InitializeIpInterfaceEntry, InitializeUnicastIpAddressEntry, SetIpInterfaceEntry, MIB_IF_ROW2,
    MIB_IPFORWARD_ROW2, MIB_IPINTERFACE_ROW, MIB_UNICASTIPADDRESS_ROW,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Networking::WinSock::{
    IpDadStatePreferred, IpDadStateTentative, IpPrefixOriginManual, IpSuffixOriginManual,
    NlroManual, AF_INET6, IN6_ADDR, IN6_ADDR_0, MIB_IPPROTO_NETMGMT, SOCKADDR_IN6, SOCKADDR_IN6_0,
    SOCKADDR_INET,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
};

const NAME_LEN: usize = 37; // "ios-" + 32 hex digits + NUL
const MAX_PACKET: usize = 0xffff;
const MIN_RING: u32 = 0x2_0000;
const MAX_RING: u32 = 0x400_0000;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan {
    pub(crate) client: Ipv6Addr,
    pub(crate) server: Ipv6Addr,
    pub(crate) mtu: u16,
    pub(crate) prefix: u8,
}
impl Plan {
    fn validate(self) -> Result<Self, NativeError> {
        if self.mtu < 1280 || self.prefix != 64 {
            return Err(NativeError::Invalid("IPv6 MTU or prefix"));
        }
        if self.client.is_unspecified()
            || self.client.is_multicast()
            || self.server.is_unspecified()
            || self.server.is_multicast()
            || network(self.client, self.prefix) != network(self.server, self.prefix)
        {
            return Err(NativeError::Invalid("authenticated IPv6 endpoint network"));
        }
        Ok(self)
    }
}

/// Original status returned by the named API, with its original IO conversion.
/// IP Helper statuses are never replaced with GetLastError.
#[derive(Debug)]
pub(crate) struct NativeCallError {
    operation: &'static str,
    status: u32,
    original: io::Error,
}
impl NativeCallError {
    fn new(operation: &'static str, status: u32) -> Self {
        Self {
            operation,
            status,
            original: io::Error::from_raw_os_error(status as i32),
        }
    }
    #[cfg(test)]
    pub(crate) fn operation(&self) -> &'static str {
        self.operation
    }
    pub(crate) fn status(&self) -> u32 {
        self.status
    }
    #[cfg(test)]
    pub(crate) fn original(&self) -> &io::Error {
        &self.original
    }
}
impl fmt::Display for NativeCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} failed with Windows status {}",
            self.operation, self.status
        )
    }
}
impl Error for NativeCallError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.original)
    }
}

#[derive(Debug)]
pub(crate) enum NativeError {
    Call(NativeCallError),
    Loader(libloading::Error),
    Random(rand::Error),
    Invalid(&'static str),
    Conflict(&'static str),
    DadState(i32),
    Closed,
    SessionNotStarted,
    Interrupted,
    PacketTooLarge { length: usize, capacity: usize },
    UnexpectedWait(u32),
}
impl fmt::Display for NativeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(error) => fmt::Display::fmt(error, f),
            Self::Loader(_) => f.write_str("load Wintun API"),
            Self::Random(_) => f.write_str("generate Wintun adapter name"),
            Self::Invalid(field) => write!(f, "invalid native tunnel {field}"),
            Self::Conflict(field) => {
                write!(f, "native tunnel ownership/readback conflict: {field}")
            }
            Self::DadState(state) => write!(f, "native IPv6 DAD state is {state}"),
            Self::Closed => f.write_str("native tunnel interface is closed"),
            Self::SessionNotStarted => f.write_str("native tunnel packet session is not started"),
            Self::Interrupted => f.write_str("native tunnel operation was locally interrupted"),
            Self::PacketTooLarge { length, capacity } => write!(
                f,
                "native packet length {length} exceeds destination {capacity}"
            ),
            Self::UnexpectedWait(value) => write!(f, "unexpected native wait result {value}"),
        }
    }
}
impl Error for NativeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Call(error) => Some(error),
            Self::Loader(error) => Some(error),
            Self::Random(error) => Some(error),
            _ => None,
        }
    }
}
fn status(operation: &'static str, value: u32) -> Result<(), NativeError> {
    if value == 0 {
        Ok(())
    } else {
        Err(NativeError::Call(NativeCallError::new(operation, value)))
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Identity {
    luid: NET_LUID_LH,
    index: u32,
    guid: GUID,
}
impl Identity {
    pub(crate) fn same(&self, other: Self) -> bool {
        identity_eq(*self, other)
    }
    #[cfg(test)]
    pub(crate) fn luid(&self) -> u64 {
        // SAFETY: NET_LUID is initialized by WintunGetAdapterLUID; Value is its
        // documented complete integer representation.
        unsafe { self.luid.Value }
    }
}
impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowsTunnelIdentity")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

type CreateAdapter = unsafe extern "system" fn(*const u16, *const u16, *const GUID) -> *mut c_void;
type CloseAdapter = unsafe extern "system" fn(*mut c_void);
type GetAdapterLuid = unsafe extern "system" fn(*mut c_void, *mut NET_LUID_LH);
type StartSession = unsafe extern "system" fn(*mut c_void, u32) -> *mut c_void;
type EndSession = unsafe extern "system" fn(*mut c_void);
type GetReadWaitEvent = unsafe extern "system" fn(*mut c_void) -> HANDLE;
type ReceivePacket = unsafe extern "system" fn(*mut c_void, *mut u32) -> *mut u8;
type ReleaseReceivePacket = unsafe extern "system" fn(*mut c_void, *const u8);
type AllocateSendPacket = unsafe extern "system" fn(*mut c_void, u32) -> *mut u8;
type SendPacket = unsafe extern "system" fn(*mut c_void, *const u8);

#[derive(Clone, Copy)]
struct WintunFns {
    create: CreateAdapter,
    close: CloseAdapter,
    luid: GetAdapterLuid,
    start: StartSession,
    end: EndSession,
    read_event: GetReadWaitEvent,
    receive: ReceivePacket,
    release: ReleaseReceivePacket,
    allocate: AllocateSendPacket,
    send: SendPacket,
}
#[derive(Clone, Copy)]
struct IpFns {
    index: unsafe extern "system" fn(*const NET_LUID_LH, *mut u32) -> u32,
    guid: unsafe extern "system" fn(*const NET_LUID_LH, *mut GUID) -> u32,
    interface: unsafe extern "system" fn(*mut MIB_IF_ROW2) -> u32,
    init_ip: unsafe extern "system" fn(*mut MIB_IPINTERFACE_ROW),
    get_ip: unsafe extern "system" fn(*mut MIB_IPINTERFACE_ROW) -> u32,
    set_ip: unsafe extern "system" fn(*mut MIB_IPINTERFACE_ROW) -> u32,
    init_address: unsafe extern "system" fn(*mut MIB_UNICASTIPADDRESS_ROW),
    create_address: unsafe extern "system" fn(*const MIB_UNICASTIPADDRESS_ROW) -> u32,
    get_address: unsafe extern "system" fn(*mut MIB_UNICASTIPADDRESS_ROW) -> u32,
    delete_address: unsafe extern "system" fn(*const MIB_UNICASTIPADDRESS_ROW) -> u32,
    init_route: unsafe extern "system" fn(*mut MIB_IPFORWARD_ROW2),
    create_route: unsafe extern "system" fn(*const MIB_IPFORWARD_ROW2) -> u32,
    get_route: unsafe extern "system" fn(*mut MIB_IPFORWARD_ROW2) -> u32,
    delete_route: unsafe extern "system" fn(*const MIB_IPFORWARD_ROW2) -> u32,
}
impl IpFns {
    fn native() -> Self {
        Self {
            index: ConvertInterfaceLuidToIndex,
            guid: ConvertInterfaceLuidToGuid,
            interface: GetIfEntry2,
            init_ip: InitializeIpInterfaceEntry,
            get_ip: GetIpInterfaceEntry,
            set_ip: SetIpInterfaceEntry,
            init_address: InitializeUnicastIpAddressEntry,
            create_address: CreateUnicastIpAddressEntry,
            get_address: GetUnicastIpAddressEntry,
            delete_address: DeleteUnicastIpAddressEntry,
            init_route: InitializeIpForwardEntry,
            create_route: CreateIpForwardEntry2,
            get_route: GetIpForwardEntry2,
            delete_route: DeleteIpForwardEntry2,
        }
    }
}
struct Api {
    wintun: WintunFns,
    ip: IpFns,
    // Always Some in production. Tests inject only the private function seam.
    // The DLL outlives all adapter/session calls, including Drop.
    _library: Option<Library>,
}
impl Api {
    fn load() -> Result<Self, NativeError> {
        // SAFETY: this is the installed Wintun DLL ABI. Every required symbol
        // below has the exact WINAPI declaration in the official wintun.h.
        let library = unsafe { Library::new("wintun.dll") }.map_err(NativeError::Loader)?;
        macro_rules! symbol {
            ($name:literal, $ty:ty) => {{
                // SAFETY: the fixed symbol name and its exact header signature
                // are paired here; _library retains every copied function.
                *unsafe { library.get::<$ty>(concat!($name, "\0").as_bytes()) }
                    .map_err(NativeError::Loader)?
            }};
        }
        let wintun = WintunFns {
            create: symbol!("WintunCreateAdapter", CreateAdapter),
            close: symbol!("WintunCloseAdapter", CloseAdapter),
            luid: symbol!("WintunGetAdapterLUID", GetAdapterLuid),
            start: symbol!("WintunStartSession", StartSession),
            end: symbol!("WintunEndSession", EndSession),
            read_event: symbol!("WintunGetReadWaitEvent", GetReadWaitEvent),
            receive: symbol!("WintunReceivePacket", ReceivePacket),
            release: symbol!("WintunReleaseReceivePacket", ReleaseReceivePacket),
            allocate: symbol!("WintunAllocateSendPacket", AllocateSendPacket),
            send: symbol!("WintunSendPacket", SendPacket),
        };
        Ok(Self {
            wintun,
            ip: IpFns::native(),
            _library: Some(library),
        })
    }
}

struct Session {
    handle: NonNull<c_void>,
    // Borrowed from WintunGetReadWaitEvent. Never CloseHandle this event.
    read_event: HANDLE,
}
#[derive(Clone, Copy)]
pub(crate) struct AddressReceipt {
    identity: Identity,
    adapter: NonNull<c_void>,
    desired: MIB_UNICASTIPADDRESS_ROW,
    route_absent_before: MIB_IPFORWARD_ROW2,
    created_at: Option<i64>,
}
#[derive(Clone, Copy)]
pub(crate) struct RouteReceipt {
    identity: Identity,
    adapter: NonNull<c_void>,
    desired: MIB_IPFORWARD_ROW2,
}
/// Observation of an address-derived connected route. This is deliberately
/// distinct from a successful explicit route Create receipt. Undo it by undoing
/// the original owned address, never by deleting an already-existing route.
#[derive(Clone, Copy)]
pub(crate) struct DerivedRouteObservation {
    address: AddressReceipt,
    absent_before_address: MIB_IPFORWARD_ROW2,
    observed: MIB_IPFORWARD_ROW2,
}
// SAFETY: all raw pointers in this observation are non-owning receipt identity
// values only; no pointer is dereferenced or used to call native code.
unsafe impl Send for DerivedRouteObservation {}
unsafe impl Sync for DerivedRouteObservation {}

// SAFETY: receipts never dereference the adapter pointer. It is a non-owning
// identity check against this object's retained actual adapter, not a handle.
unsafe impl Send for AddressReceipt {}
unsafe impl Sync for AddressReceipt {}
unsafe impl Send for RouteReceipt {}
unsafe impl Sync for RouteReceipt {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressReadiness {
    Preferred,
    Tentative,
    Conflict { dad_state: i32 },
}
pub(crate) struct InertObservation {
    pub(crate) identity: Identity,
}

/// Ordinary owned resource. Control/close methods require exclusive access;
/// concurrent packet methods borrow the live session and never end it.
pub(crate) struct WindowsDevice {
    api: Api,
    adapter: Option<NonNull<c_void>>,
    identity: Identity,
    identity_ready: bool,
    plan: Plan,
    session: Option<Session>,
    address: Option<AddressReceipt>,
    route: Option<RouteReceipt>,
}
// SAFETY: Wintun packet functions are documented thread safe. Control and
// session mutation require &mut self, excluding outstanding shared borrows.
// Api's DLL is retained through final Drop and no raw pointer is exported.
unsafe impl Send for WindowsDevice {}
unsafe impl Sync for WindowsDevice {}

impl WindowsDevice {
    /// Return the genuine fresh adapter before the next fallible identity
    /// operation. The staged owner stores this value in its paid destination
    /// before calling resolve_identity, so failure cannot discard its resource.
    pub(crate) fn allocate(plan: Plan) -> Result<Self, NativeError> {
        let plan = plan.validate()?;
        let mut entropy = [0; 16];
        rand::rngs::OsRng
            .try_fill_bytes(&mut entropy)
            .map_err(NativeError::Random)?;
        Self::allocate_with_api(plan, Api::load()?, name(&entropy))
    }

    #[cfg(test)]
    fn create_with_api(plan: Plan, api: Api, name: [u16; NAME_LEN]) -> Result<Self, NativeError> {
        let mut device = Self::allocate_with_api(plan, api, name)?;
        device.resolve_identity()?;
        Ok(device)
    }

    fn allocate_with_api(plan: Plan, api: Api, name: [u16; NAME_LEN]) -> Result<Self, NativeError> {
        let plan = plan.validate()?;
        let tunnel_type = [b'i' as u16, b'O' as u16, b'S' as u16, 0];
        // SAFETY: the two fixed stack arrays are NUL-terminated UTF-16. NULL
        // requests a new system GUID. This facade has no OpenAdapter symbol.
        let raw = unsafe { (api.wintun.create)(name.as_ptr(), tunnel_type.as_ptr(), ptr::null()) };
        let Some(adapter) = NonNull::new(raw) else {
            // GetLastError belongs to this pointer-returning Wintun API.
            return Err(NativeError::Call(NativeCallError::new(
                "WintunCreateAdapter",
                unsafe { GetLastError() },
            )));
        };
        // Establish RAII before any subsequent fallible conversion/readback.
        let mut device = Self {
            api,
            adapter: Some(adapter),
            identity: Identity {
                luid: NET_LUID_LH { Value: 0 },
                index: 0,
                guid: GUID {
                    data1: 0,
                    data2: 0,
                    data3: 0,
                    data4: [0; 8],
                },
            },
            identity_ready: false,
            plan,
            session: None,
            address: None,
            route: None,
        };
        // SAFETY: adapter is this genuine newly created handle; destinations
        // are correctly sized writable fields retained in the same object.
        unsafe { (device.api.wintun.luid)(adapter.as_ptr(), &mut device.identity.luid) };
        Ok(device)
    }

    pub(crate) fn resolve_identity(&mut self) -> Result<(), NativeError> {
        self.live()?;
        if self.identity_ready {
            return self.inspect_identity().map(|_| ());
        }
        // SAFETY: this object owns the live adapter and its original LUID;
        // both conversion destinations are correctly sized exclusive fields.
        let code = unsafe { (self.api.ip.index)(&self.identity.luid, &mut self.identity.index) };
        status("ConvertInterfaceLuidToIndex", code)?;
        // SAFETY: same owned LUID and writable GUID field as above.
        let code = unsafe { (self.api.ip.guid)(&self.identity.luid, &mut self.identity.guid) };
        status("ConvertInterfaceLuidToGuid", code)?;
        self.inspect_identity_row()?;
        self.identity_ready = true;
        Ok(())
    }

    fn live(&self) -> Result<NonNull<c_void>, NativeError> {
        self.adapter.ok_or(NativeError::Closed)
    }
    /// Local ownership validation only; the proof's actual native readbacks
    /// were acquired by init. This neither queries OS state nor certifies ACK.
    pub(crate) fn owns_ready_address(
        &self,
        identity: Identity,
        address: &AddressReceipt,
        mtu: u16,
    ) -> bool {
        self.identity_ready
            && identity_eq(self.identity, identity)
            && self.plan.mtu == mtu
            && self.owns_address(address).is_ok()
    }
    pub(crate) fn owns_ready_route(&self, route: &RouteReceipt) -> bool {
        self.owns_route(route).is_ok()
    }
    pub(crate) fn owns_ready_derived(&self, observed: &DerivedRouteObservation) -> bool {
        self.route.is_none()
            && self.owns_address(&observed.address).is_ok()
            && route_matches(
                &observed.absent_before_address,
                &observed.address.route_absent_before,
            )
    }

    pub(crate) fn inspect_identity(&self) -> Result<Identity, NativeError> {
        self.live()?;
        if !self.identity_ready {
            return Err(NativeError::Conflict(
                "native identity has not been resolved",
            ));
        }
        self.inspect_identity_row()
    }

    fn inspect_identity_row(&self) -> Result<Identity, NativeError> {
        self.live()?;
        let mut row = MIB_IF_ROW2 {
            InterfaceLuid: self.identity.luid,
            InterfaceIndex: self.identity.index,
            ..MIB_IF_ROW2::default()
        };
        // SAFETY: this initialized row is writable for the duration of the call.
        status("GetIfEntry2", unsafe { (self.api.ip.interface)(&mut row) })?;
        if !same_identity(
            self.identity,
            row.InterfaceLuid,
            row.InterfaceIndex,
            row.InterfaceGuid,
        ) {
            return Err(NativeError::Conflict("interface LUID/GUID/index"));
        }
        Ok(self.identity)
    }

    /// Checks the authenticated endpoint's exact keys, not all system rows.
    /// A new adapter may have automatic OS link-local state.
    pub(crate) fn inspect_inert(&self) -> Result<InertObservation, NativeError> {
        self.inspect_identity()?;
        if self.session.is_some() || self.address.is_some() || self.route.is_some() {
            return Err(NativeError::Conflict("interface already staged or active"));
        }
        let mut address = self.address_row();
        // SAFETY: address is a valid exact-key query row for this live adapter.
        let code = unsafe { (self.api.ip.get_address)(&mut address) };
        if code == 0 {
            return Err(NativeError::Conflict("endpoint address already exists"));
        }
        if code != ERROR_NOT_FOUND {
            status("GetUnicastIpAddressEntry(inert)", code)?;
        }
        let mut route = self.route_row();
        // SAFETY: route contains the exact target prefix and this live LUID.
        let code = unsafe { (self.api.ip.get_route)(&mut route) };
        if code == 0 {
            return Err(NativeError::Conflict("endpoint route already exists"));
        }
        if code != ERROR_NOT_FOUND {
            status("GetIpForwardEntry2(inert)", code)?;
        }
        Ok(InertObservation {
            identity: self.identity,
        })
    }

    pub(crate) fn configure_mtu(&mut self) -> Result<(), NativeError> {
        self.inspect_identity()?;
        let mut row = self.ip_row();
        // SAFETY: row is an initialized writable query for this live interface.
        status("GetIpInterfaceEntry(MTU)", unsafe {
            (self.api.ip.get_ip)(&mut row)
        })?;
        row.NlMtu = u32::from(self.plan.mtu);
        // SAFETY: preserve returned settings and change only this owned
        // interface's IPv6 MTU. The API returns its own native status.
        status("SetIpInterfaceEntry(MTU)", unsafe {
            (self.api.ip.set_ip)(&mut row)
        })
    }
    pub(crate) fn inspect_mtu(&self) -> Result<(), NativeError> {
        self.inspect_identity()?;
        let mut row = self.ip_row();
        // SAFETY: the initialized row is writable for this exact-key query.
        status("GetIpInterfaceEntry(readback)", unsafe {
            (self.api.ip.get_ip)(&mut row)
        })?;
        if !same_luid(row.InterfaceLuid, self.identity.luid)
            || row.InterfaceIndex != self.identity.index
            || row.Family != AF_INET6
            || row.NlMtu != u32::from(self.plan.mtu)
        {
            return Err(NativeError::Conflict("IPv6 MTU readback"));
        }
        Ok(())
    }

    pub(crate) fn start_session(&mut self, ring_capacity: u32) -> Result<(), NativeError> {
        self.inspect_identity()?;
        let adapter = self.live()?;
        if self.session.is_some() {
            return Err(NativeError::Conflict("session already started"));
        }
        if !(MIN_RING..=MAX_RING).contains(&ring_capacity) || !ring_capacity.is_power_of_two() {
            return Err(NativeError::Invalid("Wintun ring capacity"));
        }
        // SAFETY: adapter is live and no other session exists in this owner.
        let raw = unsafe { (self.api.wintun.start)(adapter.as_ptr(), ring_capacity) };
        let Some(handle) = NonNull::new(raw) else {
            return Err(NativeError::Call(NativeCallError::new(
                "WintunStartSession",
                unsafe { GetLastError() },
            )));
        };
        // Persist the genuine session before inspecting its borrowed event.
        self.session = Some(Session {
            handle,
            read_event: ptr::null_mut(),
        });
        // SAFETY: the newly started session remains alive in this owner.
        let event = unsafe { (self.api.wintun.read_event)(handle.as_ptr()) };
        if event.is_null() {
            let error = NativeError::Call(NativeCallError::new("WintunGetReadWaitEvent", unsafe {
                GetLastError()
            }));
            self.end_session();
            return Err(error);
        }
        self.session
            .as_mut()
            .expect("stored original session")
            .read_event = event;
        Ok(())
    }

    /// On success the receipt is durable here before returning. Readiness and
    /// exact readback are separate operations, so failure cannot lose undo.
    pub(crate) fn add_address(&mut self) -> Result<AddressReceipt, NativeError> {
        let adapter = self.live()?;
        self.inspect_identity()?;
        if self.address.is_some() {
            return Err(NativeError::Conflict("address already owned"));
        }
        // Capture a real exact-key absence observation before the successful
        // address Create can cause Windows to install a connected route.
        let mut route_absent_before = self.route_row();
        // SAFETY: this initialized exact prefix query targets our live LUID.
        let code = unsafe { (self.api.ip.get_route)(&mut route_absent_before) };
        if code == 0 {
            return Err(NativeError::Conflict("route existed before address Create"));
        }
        if code != ERROR_NOT_FOUND {
            status("GetIpForwardEntry2(before address)", code)?;
        }
        let desired = self.address_row();
        // SAFETY: initialized exact address row, live original interface.
        // Already-exists is an original error; never Set/overwrite/adopt it.
        status("CreateUnicastIpAddressEntry", unsafe {
            (self.api.ip.create_address)(&desired)
        })?;
        let mut receipt = AddressReceipt {
            identity: self.identity,
            adapter,
            desired,
            route_absent_before,
            created_at: None,
        };
        self.address = Some(receipt);
        let mut observed = desired;
        // SAFETY: retain the Create receipt before this exact-key readback can
        // fail; caller can still close the owned interface on that failure.
        status("GetUnicastIpAddressEntry(after Create)", unsafe {
            (self.api.ip.get_address)(&mut observed)
        })?;
        if !address_matches(&desired, &observed) {
            return Err(NativeError::Conflict("created address readback changed"));
        }
        receipt.created_at = Some(observed.CreationTimeStamp);
        self.address = Some(receipt);
        Ok(receipt)
    }
    pub(crate) fn address_receipt(&self) -> Option<AddressReceipt> {
        self.address
    }

    pub(crate) fn address_ready(
        &self,
        receipt: &AddressReceipt,
    ) -> Result<AddressReadiness, NativeError> {
        self.owns_address(receipt)?;
        self.inspect_identity()?;
        let mut row = receipt.desired;
        // SAFETY: stored receipt supplies the initialized exact query key.
        status("GetUnicastIpAddressEntry(readback)", unsafe {
            (self.api.ip.get_address)(&mut row)
        })?;
        if !address_matches(&receipt.desired, &row) {
            return Err(NativeError::Conflict("address row changed"));
        }
        if receipt.created_at != Some(row.CreationTimeStamp) {
            return Err(NativeError::Conflict("address creation timestamp changed"));
        }
        Ok(if row.DadState == IpDadStatePreferred {
            AddressReadiness::Preferred
        } else if row.DadState == IpDadStateTentative {
            AddressReadiness::Tentative
        } else {
            AddressReadiness::Conflict {
                dad_state: row.DadState,
            }
        })
    }

    pub(crate) fn add_route(&mut self) -> Result<RouteReceipt, NativeError> {
        let adapter = self.live()?;
        self.inspect_identity()?;
        if self.route.is_some() {
            return Err(NativeError::Conflict("route already owned"));
        }
        let desired = self.route_row();
        // SAFETY: exact initialized prefix/next-hop row for this live LUID.
        // A kernel-derived connected route may already exist after address add;
        // that status is refused and never reclassified as our Create receipt.
        status("CreateIpForwardEntry2", unsafe {
            (self.api.ip.create_route)(&desired)
        })?;
        let receipt = RouteReceipt {
            identity: self.identity,
            adapter,
            desired,
        };
        self.route = Some(receipt);
        Ok(receipt)
    }
    pub(crate) fn route_receipt(&self) -> Option<RouteReceipt> {
        self.route
    }

    /// Windows may install the /prefix on-link route as part of this owner's
    /// address Create. This exact observation never adopts an explicit Create
    /// receipt and never grants permission for DeleteIpForwardEntry2.
    pub(crate) fn inspect_address_route(
        &self,
        address: &AddressReceipt,
    ) -> Result<DerivedRouteObservation, NativeError> {
        self.owns_address(address)?;
        self.inspect_identity()?;
        let mut observed = address.route_absent_before;
        // SAFETY: retained real pre-address query supplies this exact key.
        status("GetIpForwardEntry2(address-derived)", unsafe {
            (self.api.ip.get_route)(&mut observed)
        })?;
        if !route_key_matches(&address.route_absent_before, &observed) {
            return Err(NativeError::Conflict("address-derived route key changed"));
        }
        Ok(DerivedRouteObservation {
            address: *address,
            absent_before_address: address.route_absent_before,
            observed,
        })
    }

    pub(crate) fn inspect_derived_route(
        &self,
        observation: &DerivedRouteObservation,
    ) -> Result<(), NativeError> {
        self.owns_address(&observation.address)?;
        self.inspect_identity()?;
        if !route_key_matches(&observation.absent_before_address, &observation.observed) {
            return Err(NativeError::Conflict("address-derived observation key"));
        }
        let mut current = observation.observed;
        // SAFETY: same owned address and exact observed route key remain live.
        status("GetIpForwardEntry2(derived readback)", unsafe {
            (self.api.ip.get_route)(&mut current)
        })?;
        if !route_matches(&observation.observed, &current) {
            return Err(NativeError::Conflict("address-derived route changed"));
        }
        Ok(())
    }

    pub(crate) fn inspect_route(&self, receipt: &RouteReceipt) -> Result<(), NativeError> {
        self.owns_route(receipt)?;
        self.inspect_identity()?;
        let mut row = receipt.desired;
        // SAFETY: receipt provides the initialized exact route key.
        status("GetIpForwardEntry2(readback)", unsafe {
            (self.api.ip.get_route)(&mut row)
        })?;
        if !route_matches(&receipt.desired, &row) {
            return Err(NativeError::Conflict("route row changed"));
        }
        Ok(())
    }

    /// Exact receipt/readback guard only; Windows has no atomic compare-delete
    /// exposed here. A concurrent external writer can still race this pair.
    pub(crate) fn remove_route(&mut self, receipt: &RouteReceipt) -> Result<(), NativeError> {
        self.owns_route(receipt)?;
        self.inspect_identity()?;
        let mut row = receipt.desired;
        // SAFETY: exact initialized query row from this owner's Create receipt.
        let code = unsafe { (self.api.ip.get_route)(&mut row) };
        if code == ERROR_NOT_FOUND {
            self.route = None;
            return Ok(());
        }
        status("GetIpForwardEntry2(undo)", code)?;
        if !route_matches(&receipt.desired, &row) {
            return Err(NativeError::Conflict("refuse deletion of changed route"));
        }
        // SAFETY: only this exact successfully created/read-back row is removed.
        status("DeleteIpForwardEntry2", unsafe {
            (self.api.ip.delete_route)(&row)
        })?;
        self.route = None;
        Ok(())
    }
    pub(crate) fn remove_address(&mut self, receipt: &AddressReceipt) -> Result<(), NativeError> {
        self.owns_address(receipt)?;
        self.inspect_identity()?;
        let mut row = receipt.desired;
        // SAFETY: exact initialized query row from this owner's Create receipt.
        let code = unsafe { (self.api.ip.get_address)(&mut row) };
        if code == ERROR_NOT_FOUND {
            self.address = None;
            return Ok(());
        }
        status("GetUnicastIpAddressEntry(undo)", code)?;
        if !address_matches(&receipt.desired, &row) {
            return Err(NativeError::Conflict("refuse deletion of changed address"));
        }
        if receipt.created_at != Some(row.CreationTimeStamp) {
            return Err(NativeError::Conflict(
                "refuse deletion without original address timestamp",
            ));
        }
        // SAFETY: this exact owned address row passed the readback guard.
        status("DeleteUnicastIpAddressEntry", unsafe {
            (self.api.ip.delete_address)(&row)
        })?;
        self.address = None;
        Ok(())
    }

    fn owns_address(&self, receipt: &AddressReceipt) -> Result<(), NativeError> {
        let adapter = self.live()?;
        if receipt.adapter != adapter
            || !identity_eq(receipt.identity, self.identity)
            || !self
                .address
                .as_ref()
                .is_some_and(|owned| address_matches(&owned.desired, &receipt.desired))
        {
            return Err(NativeError::Conflict(
                "address receipt belongs to another owner",
            ));
        }
        Ok(())
    }
    fn owns_route(&self, receipt: &RouteReceipt) -> Result<(), NativeError> {
        let adapter = self.live()?;
        if receipt.adapter != adapter
            || !identity_eq(receipt.identity, self.identity)
            || !self
                .route
                .as_ref()
                .is_some_and(|owned| route_matches(&owned.desired, &receipt.desired))
        {
            return Err(NativeError::Conflict(
                "route receipt belongs to another owner",
            ));
        }
        Ok(())
    }

    fn ip_row(&self) -> MIB_IPINTERFACE_ROW {
        let mut row = MIB_IPINTERFACE_ROW::default();
        // SAFETY: writable correctly sized row; initializer performs no IO.
        unsafe { (self.api.ip.init_ip)(&mut row) };
        row.Family = AF_INET6;
        row.InterfaceLuid = self.identity.luid;
        row.InterfaceIndex = self.identity.index;
        row
    }
    fn address_row(&self) -> MIB_UNICASTIPADDRESS_ROW {
        let mut row = MIB_UNICASTIPADDRESS_ROW::default();
        // SAFETY: writable correctly sized row; initializer performs no IO.
        unsafe { (self.api.ip.init_address)(&mut row) };
        row.Address = sockaddr(self.plan.client, self.identity.index);
        row.InterfaceLuid = self.identity.luid;
        row.InterfaceIndex = self.identity.index;
        row.PrefixOrigin = IpPrefixOriginManual;
        row.SuffixOrigin = IpSuffixOriginManual;
        row.ValidLifetime = u32::MAX;
        row.PreferredLifetime = u32::MAX;
        row.OnLinkPrefixLength = self.plan.prefix;
        row.SkipAsSource = false;
        row
    }
    fn route_row(&self) -> MIB_IPFORWARD_ROW2 {
        let mut row = MIB_IPFORWARD_ROW2::default();
        // SAFETY: writable correctly sized row; initializer performs no IO.
        unsafe { (self.api.ip.init_route)(&mut row) };
        row.InterfaceLuid = self.identity.luid;
        row.InterfaceIndex = self.identity.index;
        row.DestinationPrefix.Prefix = sockaddr(
            network(self.plan.client, self.plan.prefix),
            self.identity.index,
        );
        row.DestinationPrefix.PrefixLength = self.plan.prefix;
        row.NextHop = sockaddr(Ipv6Addr::UNSPECIFIED, 0);
        row.SitePrefixLength = self.plan.prefix;
        row.ValidLifetime = u32::MAX;
        row.PreferredLifetime = u32::MAX;
        row.Metric = 0;
        row.Protocol = MIB_IPPROTO_NETMGMT;
        row.Origin = NlroManual;
        row.Loopback = false;
        row.AutoconfigureAddress = false;
        row.Publish = false;
        row.Immortal = true;
        row
    }

    fn session(&self) -> Result<&Session, NativeError> {
        self.live()?;
        self.session.as_ref().ok_or(NativeError::SessionNotStarted)
    }
    pub(crate) fn try_recv(&self, destination: &mut [u8]) -> Result<usize, NativeError> {
        let session = self.session()?;
        let mut length = 0;
        // SAFETY: session is live for this shared borrow; length is writable.
        let raw = unsafe { (self.api.wintun.receive)(session.handle.as_ptr(), &mut length) };
        let Some(packet) = NonNull::new(raw) else {
            return Err(NativeError::Call(NativeCallError::new(
                "WintunReceivePacket",
                unsafe { GetLastError() },
            )));
        };
        let _release = Received {
            f: self.api.wintun.release,
            session: session.handle,
            packet,
        };
        let length = length as usize;
        if length > destination.len() {
            return Err(NativeError::PacketTooLarge {
                length,
                capacity: destination.len(),
            });
        }
        // SAFETY: Wintun provides length readable bytes until release; the
        // caller's distinct writable slice is large enough. No Vec copy exists.
        unsafe { ptr::copy_nonoverlapping(packet.as_ptr(), destination.as_mut_ptr(), length) };
        Ok(length)
    }
    pub(crate) fn wait_readable(&self, interrupt: &InterruptEvent) -> Result<(), NativeError> {
        let session = self.session()?;
        let handles = [session.read_event, interrupt.handle()];
        // SAFETY: both handles remain live for the blocking call. This borrowed
        // Wintun event is never closed, and the caller can trigger interruption.
        let value = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
        match value {
            WAIT_OBJECT_0 => Ok(()),
            value if value == WAIT_OBJECT_0 + 1 => Err(NativeError::Interrupted),
            WAIT_FAILED => Err(NativeError::Call(NativeCallError::new(
                "WaitForMultipleObjects",
                unsafe { GetLastError() },
            ))),
            value => Err(NativeError::UnexpectedWait(value)),
        }
    }
    pub(crate) fn recv(
        &self,
        destination: &mut [u8],
        interrupt: &InterruptEvent,
    ) -> Result<usize, NativeError> {
        loop {
            if interrupt.signalled()? {
                return Err(NativeError::Interrupted);
            }
            match self.try_recv(destination) {
                Err(NativeError::Call(error)) if error.status == ERROR_NO_MORE_ITEMS => {
                    self.wait_readable(interrupt)?
                }
                result => return result,
            }
        }
    }
    pub(crate) fn try_send(&self, packet: &[u8]) -> Result<usize, NativeError> {
        let session = self.session()?;
        if packet.is_empty() || packet.len() > MAX_PACKET {
            return Err(NativeError::Invalid("Wintun packet length"));
        }
        // SAFETY: length is representable and within Wintun's documented limit.
        let raw =
            unsafe { (self.api.wintun.allocate)(session.handle.as_ptr(), packet.len() as u32) };
        let Some(destination) = NonNull::new(raw) else {
            return Err(NativeError::Call(NativeCallError::new(
                "WintunAllocateSendPacket",
                unsafe { GetLastError() },
            )));
        };
        // SAFETY: Wintun allocated exactly this writable packet length. No await
        // or cancellation gap separates allocation from the required Send.
        unsafe {
            ptr::copy_nonoverlapping(packet.as_ptr(), destination.as_ptr(), packet.len());
            (self.api.wintun.send)(session.handle.as_ptr(), destination.as_ptr());
        }
        Ok(packet.len())
    }
    pub(crate) fn send(
        &self,
        packet: &[u8],
        interrupt: &InterruptEvent,
    ) -> Result<usize, NativeError> {
        loop {
            if interrupt.signalled()? {
                return Err(NativeError::Interrupted);
            }
            match self.try_send(packet) {
                Err(NativeError::Call(error)) if error.status == ERROR_BUFFER_OVERFLOW => {
                    // Wintun exposes a read event, no writable event. Preserve
                    // this same packet while waiting on the actual stop event.
                    if interrupt.wait(1)? {
                        return Err(NativeError::Interrupted);
                    }
                }
                result => return result,
            }
        }
    }

    /// Caller must collect every actual packet/control join first.
    pub(crate) fn end_session(&mut self) {
        if let Some(session) = self.session.take() {
            // SAFETY: &mut excludes every packet/session borrow; original DLL
            // remains alive. EndSession owns its borrowed read event's lifetime.
            unsafe { (self.api.wintun.end)(session.handle.as_ptr()) };
        }
    }
    /// Closes only this genuine CreateAdapter result; no global driver delete.
    /// Rows on this fresh owned interface disappear with its native lifetime.
    pub(crate) fn close(&mut self) {
        self.end_session();
        if let Some(adapter) = self.adapter.take() {
            // SAFETY: exclusive ownership, closed at most once, DLL still live.
            unsafe { (self.api.wintun.close)(adapter.as_ptr()) };
        }
        self.address = None;
        self.route = None;
        self.identity_ready = false;
    }
}
impl Drop for WindowsDevice {
    fn drop(&mut self) {
        self.close();
    }
}

struct Received {
    f: ReleaseReceivePacket,
    session: NonNull<c_void>,
    packet: NonNull<u8>,
}
impl Drop for Received {
    fn drop(&mut self) {
        // SAFETY: this is the original outstanding receive from this session;
        // the enclosing shared WindowsDevice borrow keeps the session alive.
        unsafe { (self.f)(self.session.as_ptr(), self.packet.as_ptr()) };
    }
}

/// An actual external, one-shot stop event. Caller admits its complete owner
/// before starting operations and retains it until every blocking join returns.
type SignalEvent = unsafe extern "system" fn(HANDLE) -> i32;
pub(crate) struct InterruptEvent(OwnedHandle, SignalEvent);
impl InterruptEvent {
    pub(crate) fn new() -> Result<Self, NativeError> {
        // SAFETY: no inherited security/name, manual reset, initially unsignaled.
        let raw = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if raw.is_null() {
            return Err(NativeError::Call(NativeCallError::new(
                "CreateEventW",
                unsafe { GetLastError() },
            )));
        }
        // SAFETY: CreateEventW returned this new uniquely owned HANDLE.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw) }, SetEvent))
    }
    fn handle(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
    pub(crate) fn trigger(&self) -> Result<(), NativeError> {
        // SAFETY: this owned event remains live through &self.
        if unsafe { (self.1)(self.handle()) } != 0 {
            return Ok(());
        }
        Err(NativeError::Call(NativeCallError::new(
            "SetEvent",
            unsafe { GetLastError() },
        )))
    }
    fn wait(&self, milliseconds: u32) -> Result<bool, NativeError> {
        // SAFETY: the owned event is live throughout this wait.
        match unsafe { WaitForSingleObject(self.handle(), milliseconds) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(NativeError::Call(NativeCallError::new(
                "WaitForSingleObject",
                unsafe { GetLastError() },
            ))),
            value => Err(NativeError::UnexpectedWait(value)),
        }
    }
    pub(crate) fn signalled(&self) -> Result<bool, NativeError> {
        self.wait(0)
    }
    #[cfg(test)]
    pub(crate) fn with_signal(signal: SignalEvent) -> Result<Self, NativeError> {
        let mut event = Self::new()?;
        event.1 = signal;
        Ok(event)
    }
    #[cfg(test)]
    pub(crate) fn force_signal(&self) -> Result<(), NativeError> {
        // A failing old-version test still recovers actual joins using the
        // real API and this exact still-owned valid event; no handle is closed.
        if unsafe { SetEvent(self.handle()) } != 0 {
            return Ok(());
        }
        Err(NativeError::Call(NativeCallError::new(
            "SetEvent(test recovery)",
            unsafe { GetLastError() },
        )))
    }
}

fn name(entropy: &[u8; 16]) -> [u16; NAME_LEN] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = [0; NAME_LEN];
    for (out, byte) in name.iter_mut().zip(b"ios-") {
        *out = u16::from(*byte);
    }
    for (index, byte) in entropy.iter().enumerate() {
        name[4 + 2 * index] = u16::from(HEX[(byte >> 4) as usize]);
        name[5 + 2 * index] = u16::from(HEX[(byte & 15) as usize]);
    }
    name
}
fn network(address: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    Ipv6Addr::from(u128::from(address) & mask)
}
fn sockaddr(address: Ipv6Addr, index: u32) -> SOCKADDR_INET {
    SOCKADDR_INET {
        Ipv6: SOCKADDR_IN6 {
            sin6_family: AF_INET6,
            sin6_port: 0,
            sin6_flowinfo: 0,
            sin6_addr: IN6_ADDR {
                u: IN6_ADDR_0 {
                    Byte: address.octets(),
                },
            },
            Anonymous: SOCKADDR_IN6_0 {
                sin6_scope_id: if address.segments()[0] & 0xffc0 == 0xfe80 {
                    index
                } else {
                    0
                },
            },
        },
    }
}
fn socket_eq(left: &SOCKADDR_INET, right: &SOCKADDR_INET) -> bool {
    // SAFETY: family is the shared union discriminant. IPv6 fields are read
    // only after checking both discriminants, and their storage is initialized.
    unsafe {
        left.si_family == AF_INET6
            && right.si_family == AF_INET6
            && left.Ipv6.sin6_addr.u.Byte == right.Ipv6.sin6_addr.u.Byte
            && left.Ipv6.Anonymous.sin6_scope_id == right.Ipv6.Anonymous.sin6_scope_id
            && left.Ipv6.sin6_port == right.Ipv6.sin6_port
            && left.Ipv6.sin6_flowinfo == right.Ipv6.sin6_flowinfo
    }
}
fn same_luid(left: NET_LUID_LH, right: NET_LUID_LH) -> bool {
    // SAFETY: both LUIDs are fully initialized integer representations.
    unsafe { left.Value == right.Value }
}
fn guid_eq(left: GUID, right: GUID) -> bool {
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}
fn same_identity(expected: Identity, luid: NET_LUID_LH, index: u32, guid: GUID) -> bool {
    same_luid(expected.luid, luid) && expected.index == index && guid_eq(expected.guid, guid)
}
fn identity_eq(left: Identity, right: Identity) -> bool {
    same_identity(left, right.luid, right.index, right.guid)
}
fn address_matches(left: &MIB_UNICASTIPADDRESS_ROW, right: &MIB_UNICASTIPADDRESS_ROW) -> bool {
    same_luid(left.InterfaceLuid, right.InterfaceLuid)
        && left.InterfaceIndex == right.InterfaceIndex
        && socket_eq(&left.Address, &right.Address)
        && left.OnLinkPrefixLength == right.OnLinkPrefixLength
        && left.PrefixOrigin == right.PrefixOrigin
        && left.SuffixOrigin == right.SuffixOrigin
        && left.SkipAsSource == right.SkipAsSource
        && left.ValidLifetime == right.ValidLifetime
        && left.PreferredLifetime == right.PreferredLifetime
}
fn route_key_matches(left: &MIB_IPFORWARD_ROW2, right: &MIB_IPFORWARD_ROW2) -> bool {
    same_luid(left.InterfaceLuid, right.InterfaceLuid)
        && left.InterfaceIndex == right.InterfaceIndex
        && socket_eq(
            &left.DestinationPrefix.Prefix,
            &right.DestinationPrefix.Prefix,
        )
        && left.DestinationPrefix.PrefixLength == right.DestinationPrefix.PrefixLength
        && socket_eq(&left.NextHop, &right.NextHop)
}
fn route_matches(left: &MIB_IPFORWARD_ROW2, right: &MIB_IPFORWARD_ROW2) -> bool {
    route_key_matches(left, right)
        && left.Metric == right.Metric
        && left.Protocol == right.Protocol
        && left.Origin == right.Origin
        && left.SitePrefixLength == right.SitePrefixLength
        && left.Loopback == right.Loopback
        && left.AutoconfigureAddress == right.AutoconfigureAddress
        && left.Publish == right.Publish
        && left.Immortal == right.Immortal
        && left.ValidLifetime == right.ValidLifetime
        && left.PreferredLifetime == right.PreferredLifetime
}

#[cfg(test)]
pub(crate) mod scripted_runtime;
#[cfg(test)]
mod tests;

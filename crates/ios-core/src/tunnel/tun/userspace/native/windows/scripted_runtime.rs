//! Scripted Wintun/IP Helper table for connected runtime tests.
//! No DLL, adapter, address or route operation is invoked. Only genuine local
//! manual-reset event handles model the blocking readiness/stop wait contract.
use super::*;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex},
};
use windows_sys::Win32::Foundation::{
    SetLastError, ERROR_INVALID_DATA, ERROR_OBJECT_ALREADY_EXISTS,
};
use windows_sys::Win32::System::Threading::ResetEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Create,
    Resolve,
    Identity,
    GetMtu,
    SetMtu,
    StartSession,
    AddAddress,
    GetAddress,
    AddRoute,
    GetRoute,
    RemoveRoute,
    RemoveAddress,
    Read,
    AllocateSend,
    Send,
    ReleaseReceive,
    EndSession,
    Close,
}
struct Data {
    calls: Vec<Operation>,
    faults: Vec<(Operation, u32)>,
    blocked: Option<Operation>,
    released: bool,
    auto_route: bool,
    tentative: bool,
    mtu: u32,
    address: Option<MIB_UNICASTIPADDRESS_ROW>,
    route: Option<MIB_IPFORWARD_ROW2>,
    incoming: VecDeque<Vec<u8>>,
    received: HashMap<usize, Vec<u8>>,
    sending: HashMap<usize, Vec<u8>>,
    sent: VecDeque<Vec<u8>>,
}
struct State {
    data: Mutex<Data>,
    wake: Condvar,
    readable: InterruptEvent,
}
#[derive(Clone)]
pub(crate) struct Script(Arc<State>);
thread_local! { static ALLOCATING: RefCell<Option<Arc<State>>> = const { RefCell::new(None) }; }
struct AllocatingGuard;
impl Drop for AllocatingGuard {
    fn drop(&mut self) {
        ALLOCATING.with(|slot| slot.borrow_mut().take());
    }
}
impl Script {
    pub(crate) fn new() -> Result<Self, NativeError> {
        Ok(Self(Arc::new(State {
            data: Mutex::new(Data {
                calls: Vec::new(),
                faults: Vec::new(),
                blocked: None,
                released: false,
                auto_route: true,
                tentative: false,
                mtu: 1500,
                address: None,
                route: None,
                incoming: VecDeque::new(),
                received: HashMap::new(),
                sending: HashMap::new(),
                sent: VecDeque::new(),
            }),
            wake: Condvar::new(),
            readable: InterruptEvent::new()?,
        })))
    }
    pub(crate) fn allocator(
        &self,
    ) -> impl FnOnce(Plan) -> Result<WindowsDevice, NativeError> + Send + 'static {
        let state = self.0.clone();
        move |plan| {
            let plan = plan.validate()?;
            ALLOCATING.with(|slot| {
                assert!(
                    slot.borrow().is_none(),
                    "one scripted allocation on this worker"
                );
                *slot.borrow_mut() = Some(state);
            });
            let _guard = AllocatingGuard;
            WindowsDevice::allocate_with_api(plan, api(), name(&[0x51; 16]))
        }
    }
    pub(crate) fn fail(&self, operation: Operation, status: u32) {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .faults
            .push((operation, status));
    }
    pub(crate) fn block(&self, operation: Operation) {
        let mut data = self.0.data.lock().unwrap_or_else(|p| p.into_inner());
        data.blocked = Some(operation);
        data.released = false;
    }
    pub(crate) fn unblock(&self) {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .released = true;
        self.0.wake.notify_all();
    }
    pub(crate) fn tentative(&self, value: bool) {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .tentative = value;
    }
    pub(crate) fn automatic_route(&self, value: bool) {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .auto_route = value;
    }
    pub(crate) fn count(&self, operation: Operation) -> usize {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .calls
            .iter()
            .filter(|value| **value == operation)
            .count()
    }
    pub(crate) fn calls(&self) -> Vec<Operation> {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .calls
            .clone()
    }
    pub(crate) fn queue_received(&self, packet: Vec<u8>) -> Result<(), NativeError> {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .incoming
            .push_back(packet);
        self.0.readable.trigger()
    }
    pub(crate) fn take_sent(&self) -> Option<Vec<u8>> {
        self.0
            .data
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .sent
            .pop_front()
    }
    pub(crate) async fn wait_for(&self, operation: Operation) {
        while self.count(operation) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }
}
impl State {
    fn enter(&self, operation: Operation) -> u32 {
        let mut data = self.data.lock().unwrap_or_else(|p| p.into_inner());
        data.calls.push(operation);
        self.wake.notify_all();
        while data.blocked == Some(operation) && !data.released {
            data = self.wake.wait(data).unwrap_or_else(|p| p.into_inner());
        }
        data.faults
            .iter()
            .find(|(kind, _)| *kind == operation)
            .map_or(0, |(_, status)| *status)
    }
}
unsafe fn state<'a>(handle: *mut c_void) -> &'a State {
    // SAFETY: scripted adapter and session handles each retain one genuine Arc
    // raw strong owner. Every caller keeps its actual device/session borrowed.
    unsafe { &*handle.cast::<State>() }
}
unsafe fn row_state<'a>(luid: NET_LUID_LH) -> &'a State {
    // SAFETY: only this table supplies these LUIDs, from its retained adapter's
    // original Arc pointer; they are never supplied to a native networking API.
    unsafe { state(luid.Value as usize as *mut c_void) }
}
fn scripted_guid() -> GUID {
    GUID {
        data1: 0x51,
        data2: 0x52,
        data3: 0x53,
        data4: [0x54; 8],
    }
}
fn native_failure(status: u32) {
    // SAFETY: this is the documented thread-local error channel of mocked
    // pointer-returning Wintun functions, never an IP Helper return substitute.
    unsafe { SetLastError(status) };
}
unsafe extern "system" fn create(_: *const u16, _: *const u16, _: *const GUID) -> *mut c_void {
    let owner = ALLOCATING.with(|slot| {
        slot.borrow()
            .as_ref()
            .expect("scripted allocating context")
            .clone()
    });
    let status = owner.enter(Operation::Create);
    if status != 0 {
        native_failure(status);
        return ptr::null_mut();
    }
    Arc::into_raw(owner).cast_mut().cast()
}
unsafe extern "system" fn close(handle: *mut c_void) {
    // SAFETY: this exact adapter raw owner was produced by create once.
    let owner = unsafe { Arc::from_raw(handle.cast::<State>()) };
    owner.enter(Operation::Close);
}
unsafe extern "system" fn luid(handle: *mut c_void, out: *mut NET_LUID_LH) {
    // SAFETY: the facade passes its correctly sized writable field.
    unsafe {
        *out = NET_LUID_LH {
            Value: handle as usize as u64,
        }
    };
}
unsafe extern "system" fn start(handle: *mut c_void, _: u32) -> *mut c_void {
    // SAFETY: the real scripted adapter owner remains live.
    let owner = unsafe { state(handle) };
    let status = owner.enter(Operation::StartSession);
    if status != 0 {
        native_failure(status);
        return ptr::null_mut();
    }
    // SAFETY: create's raw Arc owner is live, so retain a distinct session owner.
    unsafe { Arc::increment_strong_count(handle.cast::<State>()) };
    handle
}
unsafe extern "system" fn end(handle: *mut c_void) {
    // SAFETY: this is the distinct original raw strong owner from start.
    let owner = unsafe { Arc::from_raw(handle.cast::<State>()) };
    owner.enter(Operation::EndSession);
}
unsafe extern "system" fn read_event(handle: *mut c_void) -> HANDLE {
    // SAFETY: the session's strong owner retains the borrowed genuine event.
    unsafe { state(handle) }.readable.handle()
}
unsafe extern "system" fn receive(handle: *mut c_void, size: *mut u32) -> *mut u8 {
    // SAFETY: original session owner stays live.
    let owner = unsafe { state(handle) };
    let status = owner.enter(Operation::Read);
    if status != 0 {
        native_failure(status);
        return ptr::null_mut();
    }
    let mut data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    let Some(mut packet) = data.incoming.pop_front() else {
        // SAFETY: this genuine event belongs to the retained scripted state.
        unsafe { ResetEvent(owner.readable.handle()) };
        native_failure(ERROR_NO_MORE_ITEMS);
        return ptr::null_mut();
    };
    if packet.is_empty() || packet.len() > MAX_PACKET {
        native_failure(ERROR_INVALID_DATA);
        return ptr::null_mut();
    }
    let pointer = packet.as_mut_ptr();
    // SAFETY: the facade's size destination is writable.
    unsafe { *size = packet.len() as u32 };
    data.received.insert(pointer as usize, packet);
    pointer
}
unsafe extern "system" fn release(handle: *mut c_void, packet: *const u8) {
    // SAFETY: session and packet are the original table's live owners.
    let owner = unsafe { state(handle) };
    owner.enter(Operation::ReleaseReceive);
    owner
        .data
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .received
        .remove(&(packet as usize));
}
unsafe extern "system" fn allocate_send(handle: *mut c_void, len: u32) -> *mut u8 {
    // SAFETY: original session strong owner is retained.
    let owner = unsafe { state(handle) };
    let status = owner.enter(Operation::AllocateSend);
    if status != 0 {
        native_failure(status);
        return ptr::null_mut();
    }
    let mut packet = vec![0; len as usize];
    let pointer = packet.as_mut_ptr();
    owner
        .data
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .sending
        .insert(pointer as usize, packet);
    pointer
}
unsafe extern "system" fn send(handle: *mut c_void, pointer: *const u8) {
    // SAFETY: original session owner is retained.
    let owner = unsafe { state(handle) };
    owner.enter(Operation::Send);
    let mut data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(packet) = data.sending.remove(&(pointer as usize)) {
        data.sent.push_back(packet);
    }
}
unsafe extern "system" fn index(input: *const NET_LUID_LH, out: *mut u32) -> u32 {
    // SAFETY: the initialized original fake LUID and writable index are valid.
    let owner = unsafe { row_state(*input) };
    let status = owner.enter(Operation::Resolve);
    if status == 0 {
        unsafe { *out = 42 };
    }
    status
}
unsafe extern "system" fn guid(_: *const NET_LUID_LH, out: *mut GUID) -> u32 {
    // SAFETY: facade supplies a correctly sized writable GUID destination.
    unsafe { *out = scripted_guid() };
    0
}
unsafe extern "system" fn interface(out: *mut MIB_IF_ROW2) -> u32 {
    // SAFETY: facade passes an initialized writable row with original fake LUID.
    let owner = unsafe { row_state((*out).InterfaceLuid) };
    let status = owner.enter(Operation::Identity);
    if status == 0 {
        unsafe {
            (*out).InterfaceIndex = 42;
            (*out).InterfaceGuid = scripted_guid();
        }
    }
    status
}
unsafe extern "system" fn init_ip(out: *mut MIB_IPINTERFACE_ROW) {
    // SAFETY: correctly sized writable output.
    unsafe { *out = MIB_IPINTERFACE_ROW::default() };
}
unsafe extern "system" fn get_ip(out: *mut MIB_IPINTERFACE_ROW) -> u32 {
    // SAFETY: initialized writable query row supplied by facade.
    let owner = unsafe { row_state((*out).InterfaceLuid) };
    let status = owner.enter(Operation::GetMtu);
    if status == 0 {
        unsafe { (*out).NlMtu = owner.data.lock().unwrap_or_else(|p| p.into_inner()).mtu };
    }
    status
}
unsafe extern "system" fn set_ip(input: *mut MIB_IPINTERFACE_ROW) -> u32 {
    // SAFETY: initialized row remains live for this synchronous call.
    let owner = unsafe { row_state((*input).InterfaceLuid) };
    let status = owner.enter(Operation::SetMtu);
    if status == 0 {
        owner.data.lock().unwrap_or_else(|p| p.into_inner()).mtu = unsafe { (*input).NlMtu };
    }
    status
}
unsafe extern "system" fn init_address(out: *mut MIB_UNICASTIPADDRESS_ROW) {
    // SAFETY: correctly sized writable output.
    unsafe { *out = MIB_UNICASTIPADDRESS_ROW::default() };
}
unsafe extern "system" fn create_address(input: *const MIB_UNICASTIPADDRESS_ROW) -> u32 {
    // SAFETY: initialized input row retained by caller.
    let owner = unsafe { row_state((*input).InterfaceLuid) };
    let status = owner.enter(Operation::AddAddress);
    if status != 0 {
        return status;
    }
    let mut data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    if data.address.is_some() {
        return ERROR_OBJECT_ALREADY_EXISTS;
    }
    let mut address = unsafe { *input };
    address.CreationTimeStamp = 100;
    address.DadState = IpDadStatePreferred;
    data.address = Some(address);
    if data.auto_route {
        let mut route = MIB_IPFORWARD_ROW2 {
            InterfaceLuid: address.InterfaceLuid,
            InterfaceIndex: 42,
            ..MIB_IPFORWARD_ROW2::default()
        };
        let client = unsafe { Ipv6Addr::from(address.Address.Ipv6.sin6_addr.u.Byte) };
        route.DestinationPrefix.Prefix = sockaddr(network(client, address.OnLinkPrefixLength), 42);
        route.DestinationPrefix.PrefixLength = address.OnLinkPrefixLength;
        route.NextHop = sockaddr(Ipv6Addr::UNSPECIFIED, 0);
        route.ValidLifetime = u32::MAX;
        route.PreferredLifetime = u32::MAX;
        data.route = Some(route);
    }
    0
}
unsafe extern "system" fn get_address(out: *mut MIB_UNICASTIPADDRESS_ROW) -> u32 {
    // SAFETY: initialized writable query row.
    let owner = unsafe { row_state((*out).InterfaceLuid) };
    let status = owner.enter(Operation::GetAddress);
    if status != 0 {
        return status;
    }
    let data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    let Some(mut address) = data.address else {
        return ERROR_NOT_FOUND;
    };
    address.DadState = if data.tentative {
        IpDadStateTentative
    } else {
        IpDadStatePreferred
    };
    unsafe { *out = address };
    0
}
unsafe extern "system" fn delete_address(input: *const MIB_UNICASTIPADDRESS_ROW) -> u32 {
    // SAFETY: initialized input retained through the call.
    let owner = unsafe { row_state((*input).InterfaceLuid) };
    let status = owner.enter(Operation::RemoveAddress);
    if status != 0 {
        return status;
    }
    let mut data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    data.address = None;
    if data.auto_route {
        data.route = None;
    }
    0
}
unsafe extern "system" fn init_route(out: *mut MIB_IPFORWARD_ROW2) {
    // SAFETY: correctly sized writable output.
    unsafe { *out = MIB_IPFORWARD_ROW2::default() };
}
unsafe extern "system" fn create_route(input: *const MIB_IPFORWARD_ROW2) -> u32 {
    // SAFETY: initialized input row.
    let owner = unsafe { row_state((*input).InterfaceLuid) };
    let status = owner.enter(Operation::AddRoute);
    if status != 0 {
        return status;
    }
    let mut data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    if data.route.is_some() {
        return ERROR_OBJECT_ALREADY_EXISTS;
    }
    data.route = Some(unsafe { *input });
    0
}
unsafe extern "system" fn get_route(out: *mut MIB_IPFORWARD_ROW2) -> u32 {
    // SAFETY: initialized writable query row.
    let owner = unsafe { row_state((*out).InterfaceLuid) };
    let status = owner.enter(Operation::GetRoute);
    if status != 0 {
        return status;
    }
    let data = owner.data.lock().unwrap_or_else(|p| p.into_inner());
    let Some(route) = data.route else {
        return ERROR_NOT_FOUND;
    };
    unsafe { *out = route };
    0
}
unsafe extern "system" fn delete_route(input: *const MIB_IPFORWARD_ROW2) -> u32 {
    // SAFETY: initialized input row retained through the call.
    let owner = unsafe { row_state((*input).InterfaceLuid) };
    let status = owner.enter(Operation::RemoveRoute);
    if status != 0 {
        return status;
    }
    owner.data.lock().unwrap_or_else(|p| p.into_inner()).route = None;
    0
}
fn api() -> Api {
    Api {
        wintun: WintunFns {
            create,
            close,
            luid,
            start,
            end,
            read_event,
            receive,
            release,
            allocate: allocate_send,
            send,
        },
        ip: IpFns {
            index,
            guid,
            interface,
            init_ip,
            get_ip,
            set_ip,
            init_address,
            create_address,
            get_address,
            delete_address,
            init_route,
            create_route,
            get_route,
            delete_route,
        },
        _library: None,
    }
}

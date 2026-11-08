//! Scripted function-table tests. No DLL or native networking is invoked.
use super::*;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::ERROR_OBJECT_ALREADY_EXISTS;

static SERIAL: Mutex<()> = Mutex::new(());
static SCRIPT: Mutex<Script> = Mutex::new(Script::new());

struct Script {
    index_status: u32,
    address_status: u32,
    address_read_status: u32,
    auto_route: bool,
    address: Option<MIB_UNICASTIPADDRESS_ROW>,
    route: Option<MIB_IPFORWARD_ROW2>,
    address_mismatch: bool,
    route_mismatch: bool,
    mtu: u32,
    close_count: usize,
    delete_address_count: usize,
    delete_route_count: usize,
    release_count: usize,
    sent: usize,
    packet: [u8; 8],
    send_buffer: [u8; 64],
    order: Vec<&'static str>,
}
impl Script {
    const fn new() -> Self {
        Self {
            index_status: 0,
            address_status: 0,
            address_read_status: 0,
            auto_route: false,
            address: None,
            route: None,
            address_mismatch: false,
            route_mismatch: false,
            mtu: 1500,
            close_count: 0,
            delete_address_count: 0,
            delete_route_count: 0,
            release_count: 0,
            sent: 0,
            packet: [1, 2, 3, 4, 5, 6, 7, 8],
            send_buffer: [0; 64],
            order: Vec::new(),
        }
    }
}
fn plan() -> Plan {
    Plan {
        client: "fd12:3456:789a:bcde::2".parse().unwrap(),
        server: "fd12:3456:789a:bcde::1".parse().unwrap(),
        mtu: 1280,
        prefix: 64,
    }
}
fn guid() -> GUID {
    GUID {
        data1: 1,
        data2: 2,
        data3: 3,
        data4: [4; 8],
    }
}
fn adapter() -> *mut c_void {
    NonNull::<u8>::dangling().as_ptr().cast()
}

unsafe extern "system" fn create_adapter(
    _: *const u16,
    _: *const u16,
    _: *const GUID,
) -> *mut c_void {
    SCRIPT.lock().unwrap().order.push("create");
    adapter()
}
unsafe extern "system" fn close_adapter(_: *mut c_void) {
    let mut script = SCRIPT.lock().unwrap();
    script.close_count += 1;
    script.order.push("close");
}
unsafe extern "system" fn adapter_luid(_: *mut c_void, output: *mut NET_LUID_LH) {
    // SAFETY: the private adapter seam passes a writable output field.
    unsafe { *output = NET_LUID_LH { Value: 42 } };
}
unsafe extern "system" fn start_session(_: *mut c_void, _: u32) -> *mut c_void {
    SCRIPT.lock().unwrap().order.push("start");
    adapter()
}
unsafe extern "system" fn end_session(_: *mut c_void) {
    SCRIPT.lock().unwrap().order.push("end");
}
unsafe extern "system" fn read_event(_: *mut c_void) -> HANDLE {
    adapter()
}
unsafe extern "system" fn receive_packet(_: *mut c_void, size: *mut u32) -> *mut u8 {
    let mut script = SCRIPT.lock().unwrap();
    // SAFETY: the scripted caller provides a writable size field.
    unsafe { *size = script.packet.len() as u32 };
    script.packet.as_mut_ptr()
}
unsafe extern "system" fn release_packet(_: *mut c_void, _: *const u8) {
    SCRIPT.lock().unwrap().release_count += 1;
}
unsafe extern "system" fn allocate_packet(_: *mut c_void, _: u32) -> *mut u8 {
    SCRIPT.lock().unwrap().send_buffer.as_mut_ptr()
}
unsafe extern "system" fn send_packet(_: *mut c_void, _: *const u8) {
    SCRIPT.lock().unwrap().sent += 1;
}
unsafe extern "system" fn index(_: *const NET_LUID_LH, output: *mut u32) -> u32 {
    let script = SCRIPT.lock().unwrap();
    if script.index_status != 0 {
        return script.index_status;
    }
    // SAFETY: the seam's caller supplies a writable native index destination.
    unsafe { *output = 7 };
    0
}
unsafe extern "system" fn interface_guid(_: *const NET_LUID_LH, output: *mut GUID) -> u32 {
    // SAFETY: the seam's caller supplies a writable native GUID destination.
    unsafe { *output = guid() };
    0
}
unsafe extern "system" fn interface(output: *mut MIB_IF_ROW2) -> u32 {
    // SAFETY: exact query seam receives a correctly sized writable row.
    unsafe {
        (*output).InterfaceLuid = NET_LUID_LH { Value: 42 };
        (*output).InterfaceIndex = 7;
        (*output).InterfaceGuid = guid();
    }
    0
}
unsafe extern "system" fn init_ip(output: *mut MIB_IPINTERFACE_ROW) {
    // SAFETY: initializer seam receives a writable correctly sized row.
    unsafe { *output = MIB_IPINTERFACE_ROW::default() };
}
unsafe extern "system" fn get_ip(output: *mut MIB_IPINTERFACE_ROW) -> u32 {
    // SAFETY: query seam receives a correctly sized writable initialized row.
    unsafe { (*output).NlMtu = SCRIPT.lock().unwrap().mtu };
    0
}
unsafe extern "system" fn set_ip(input: *mut MIB_IPINTERFACE_ROW) -> u32 {
    // SAFETY: the input row remains live for this synchronous seam call.
    SCRIPT.lock().unwrap().mtu = unsafe { (*input).NlMtu };
    0
}
unsafe extern "system" fn init_address(output: *mut MIB_UNICASTIPADDRESS_ROW) {
    // SAFETY: initializer seam receives a writable correctly sized row.
    unsafe { *output = MIB_UNICASTIPADDRESS_ROW::default() };
}
unsafe extern "system" fn create_address(input: *const MIB_UNICASTIPADDRESS_ROW) -> u32 {
    let mut script = SCRIPT.lock().unwrap();
    if script.address_status != 0 {
        return script.address_status;
    }
    if script.address.is_some() {
        return ERROR_OBJECT_ALREADY_EXISTS;
    }
    // SAFETY: input is live and initialized for this synchronous call.
    let mut address = unsafe { *input };
    address.DadState = IpDadStatePreferred;
    address.CreationTimeStamp = 100;
    script.address = Some(address);
    if script.auto_route {
        let mut route = MIB_IPFORWARD_ROW2 {
            InterfaceLuid: address.InterfaceLuid,
            InterfaceIndex: address.InterfaceIndex,
            ..MIB_IPFORWARD_ROW2::default()
        };
        // SAFETY: test input is an initialized IPv6 address row.
        let ip = unsafe { Ipv6Addr::from(address.Address.Ipv6.sin6_addr.u.Byte) };
        route.DestinationPrefix.Prefix = sockaddr(network(ip, address.OnLinkPrefixLength), 7);
        route.DestinationPrefix.PrefixLength = address.OnLinkPrefixLength;
        route.NextHop = sockaddr(Ipv6Addr::UNSPECIFIED, 0);
        route.ValidLifetime = u32::MAX;
        route.PreferredLifetime = u32::MAX;
        script.route = Some(route);
    }
    0
}
unsafe extern "system" fn get_address(output: *mut MIB_UNICASTIPADDRESS_ROW) -> u32 {
    let script = SCRIPT.lock().unwrap();
    if script.address_read_status != 0 {
        return script.address_read_status;
    }
    let Some(mut address) = script.address else {
        return ERROR_NOT_FOUND;
    };
    if script.address_mismatch {
        address.OnLinkPrefixLength = 63;
    }
    // SAFETY: query seam receives a correctly sized writable row.
    unsafe { *output = address };
    0
}
unsafe extern "system" fn delete_address(_: *const MIB_UNICASTIPADDRESS_ROW) -> u32 {
    let mut script = SCRIPT.lock().unwrap();
    script.delete_address_count += 1;
    script.address = None;
    if script.auto_route {
        script.route = None;
    }
    0
}
unsafe extern "system" fn init_route(output: *mut MIB_IPFORWARD_ROW2) {
    // SAFETY: initializer seam receives a writable correctly sized row.
    unsafe { *output = MIB_IPFORWARD_ROW2::default() };
}
unsafe extern "system" fn create_route(input: *const MIB_IPFORWARD_ROW2) -> u32 {
    let mut script = SCRIPT.lock().unwrap();
    if script.route.is_some() {
        return ERROR_OBJECT_ALREADY_EXISTS;
    }
    // SAFETY: the original initialized row is live for this synchronous call.
    script.route = Some(unsafe { *input });
    0
}
unsafe extern "system" fn get_route(output: *mut MIB_IPFORWARD_ROW2) -> u32 {
    let script = SCRIPT.lock().unwrap();
    let Some(mut route) = script.route else {
        return ERROR_NOT_FOUND;
    };
    if script.route_mismatch {
        route.Metric += 1;
    }
    // SAFETY: query seam receives a correctly sized writable row.
    unsafe { *output = route };
    0
}
unsafe extern "system" fn delete_route(_: *const MIB_IPFORWARD_ROW2) -> u32 {
    let mut script = SCRIPT.lock().unwrap();
    script.delete_route_count += 1;
    script.route = None;
    0
}
fn api() -> Api {
    Api {
        wintun: WintunFns {
            create: create_adapter,
            close: close_adapter,
            luid: adapter_luid,
            start: start_session,
            end: end_session,
            read_event,
            receive: receive_packet,
            release: release_packet,
            allocate: allocate_packet,
            send: send_packet,
        },
        ip: IpFns {
            index,
            guid: interface_guid,
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
fn device() -> WindowsDevice {
    WindowsDevice::create_with_api(plan(), api(), name(&[0x17; 16])).unwrap()
}

#[test]
fn stack_utf16_name_and_prefix_codec_preserve_the_original_64_network() {
    let name = name(&[0xab; 16]);
    assert_eq!(&name[..4], &[105, 111, 115, 45]);
    assert!(name[4..36].chunks_exact(2).all(|chunk| chunk == [97, 98]));
    assert_eq!(name[36], 0);
    let planned = plan();
    let prefix: Ipv6Addr = "fd12:3456:789a:bcde::".parse().unwrap();
    assert_eq!(network(planned.client, planned.prefix), prefix);
    assert_eq!(network(planned.server, planned.prefix), prefix);
    assert!(socket_eq(
        &sockaddr(planned.client, 7),
        &sockaddr(planned.client, 9)
    ));
    let local: Ipv6Addr = "fe80::1".parse().unwrap();
    assert!(!socket_eq(&sockaddr(local, 7), &sockaddr(local, 9)));
}

#[test]
fn direct_return_status_retains_the_same_original_io_source() {
    let error = NativeCallError::new("scripted native return", 0xf000_0042);
    assert_eq!(error.status(), 0xf000_0042);
    assert_eq!(error.original().raw_os_error(), Some(0xf000_0042u32 as i32));
    let outer = NativeError::Call(error);
    let original = outer
        .source()
        .unwrap()
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    // The original is moved into the stable outer object, so observe its final
    // location and ensure repeated traversal returns that exact same object.
    let retained = original as *const io::Error;
    assert_eq!(
        outer
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<io::Error>()
            .unwrap() as *const io::Error,
        retained
    );
    assert_eq!(
        outer
            .source()
            .unwrap()
            .downcast_ref::<NativeCallError>()
            .unwrap()
            .operation(),
        "scripted native return"
    );
}

#[test]
fn plan_rejects_non_64_prefix_before_any_native_factory() {
    for prefix in [0, 1, 63, 65, 128] {
        let mut invalid = plan();
        invalid.prefix = prefix;
        assert!(matches!(invalid.validate(), Err(NativeError::Invalid(_))));
    }
}

#[test]
fn creation_conversion_failure_retains_direct_status_and_closes_actual_adapter_once() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    SCRIPT.lock().unwrap().index_status = 5;
    let error = match WindowsDevice::create_with_api(plan(), api(), name(&[0; 16])) {
        Err(error) => error,
        Ok(_) => panic!("conversion failure lost"),
    };
    let NativeError::Call(error) = error else {
        panic!("original native status lost")
    };
    assert_eq!(error.operation(), "ConvertInterfaceLuidToIndex");
    assert_eq!(error.status(), 5);
    assert_eq!(error.original().raw_os_error(), Some(5));
    let script = SCRIPT.lock().unwrap();
    assert_eq!(script.close_count, 1);
    assert_eq!(script.order, ["create", "close"]);
}

#[test]
fn genuine_allocation_is_durable_in_paid_owner_before_closed_zero_conversion_failure() {
    use crate::tunnel::tun::userspace::runtime_storage::RuntimeRoot;
    use std::sync::{Arc, RwLock};
    use tokio::sync::Semaphore;
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    SCRIPT.lock().unwrap().index_status = 5;
    let total = 1024 * 1024;
    let bank = Arc::new(Semaphore::new(total));
    let resource =
        RuntimeRoot::new(RwLock::<Option<WindowsDevice>>::new(None), Some(&bank)).unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let device = WindowsDevice::allocate_with_api(plan(), api(), name(&[0; 16])).unwrap();
    assert!(!device.identity_ready);
    *resource.write().unwrap() = Some(device);
    assert_eq!(SCRIPT.lock().unwrap().close_count, 0);
    let error = resource
        .write()
        .unwrap()
        .as_mut()
        .unwrap()
        .resolve_identity()
        .unwrap_err();
    let NativeError::Call(cause) = &error else {
        panic!("original direct native status lost")
    };
    assert_eq!(cause.status(), 5);
    assert_eq!(cause.original().raw_os_error(), Some(5));
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(SCRIPT.lock().unwrap().close_count, 0);
    {
        let device = resource.read().unwrap();
        let device = device.as_ref().unwrap();
        assert!(device.live().is_ok());
        assert!(matches!(
            device.inspect_identity(),
            Err(NativeError::Conflict(_))
        ));
        assert!(device.address_receipt().is_none() && device.route_receipt().is_none());
    }
    let mut original = resource.write().unwrap().take().unwrap();
    original.close();
    assert_eq!(SCRIPT.lock().unwrap().close_count, 1);
    drop((original, resource, error, held));
    assert_eq!(bank.available_permits(), total);
    assert_eq!(SCRIPT.lock().unwrap().close_count, 1);
}

#[test]
fn exact_inert_mtu_address_and_explicit_route_readback_keep_undo_receipts() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    let mut device = device();
    assert_eq!(device.inspect_inert().unwrap().identity.luid(), 42);
    device.configure_mtu().unwrap();
    device.inspect_mtu().unwrap();
    let address = device.add_address().unwrap();
    assert_eq!(
        device.address_ready(&address).unwrap(),
        AddressReadiness::Preferred
    );
    let route = device.add_route().unwrap();
    device.inspect_route(&route).unwrap();
    device.remove_route(&route).unwrap();
    device.remove_address(&address).unwrap();
    assert!(device.address_receipt().is_none());
    assert!(device.route_receipt().is_none());
    let script = SCRIPT.lock().unwrap();
    assert_eq!(script.delete_route_count, 1);
    assert_eq!(script.delete_address_count, 1);
    drop(script);
    device.close();
    device.close();
    assert_eq!(SCRIPT.lock().unwrap().close_count, 1);
}

#[test]
fn conflicting_address_create_never_becomes_owned_or_deleted() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    SCRIPT.lock().unwrap().address_status = ERROR_OBJECT_ALREADY_EXISTS;
    let mut device = device();
    let error = match device.add_address() {
        Err(error) => error,
        Ok(_) => panic!("conflict adopted"),
    };
    let NativeError::Call(error) = error else {
        panic!("native conflict status lost")
    };
    assert_eq!(error.status(), ERROR_OBJECT_ALREADY_EXISTS);
    assert!(device.address_receipt().is_none());
    assert_eq!(SCRIPT.lock().unwrap().delete_address_count, 0);
}

#[test]
fn failed_post_create_readback_keeps_receipt_and_original_error_until_owned_close() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    let mut device = device();
    SCRIPT.lock().unwrap().address_read_status = 5;
    let error = match device.add_address() {
        Err(error) => error,
        Ok(_) => panic!("readback failure lost"),
    };
    let NativeError::Call(error) = error else {
        panic!("native readback source lost")
    };
    assert_eq!(error.operation(), "GetUnicastIpAddressEntry(after Create)");
    assert_eq!(error.status(), 5);
    let original = error.original() as *const io::Error;
    let receipt = device
        .address_receipt()
        .expect("successful Create receipt retained");
    SCRIPT.lock().unwrap().address_read_status = 0;
    assert!(matches!(
        device.remove_address(&receipt),
        Err(NativeError::Conflict(_))
    ));
    assert_eq!(SCRIPT.lock().unwrap().delete_address_count, 0);
    device.close();
    assert_eq!(error.original() as *const io::Error, original);
    assert_eq!(error.original().raw_os_error(), Some(5));
    assert_eq!(SCRIPT.lock().unwrap().close_count, 1);
}

#[test]
fn changed_address_or_route_readback_refuses_deletion_and_keeps_original_receipt() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    let mut device = device();
    let address = device.add_address().unwrap();
    let route = device.add_route().unwrap();
    SCRIPT.lock().unwrap().route_mismatch = true;
    assert!(matches!(
        device.remove_route(&route),
        Err(NativeError::Conflict(_))
    ));
    assert!(device.route_receipt().is_some());
    SCRIPT.lock().unwrap().address_mismatch = true;
    assert!(matches!(
        device.remove_address(&address),
        Err(NativeError::Conflict(_))
    ));
    assert!(device.address_receipt().is_some());
    let script = SCRIPT.lock().unwrap();
    assert_eq!(script.delete_route_count, 0);
    assert_eq!(script.delete_address_count, 0);
}

#[test]
fn address_derived_route_is_observed_without_explicit_create_or_delete_ownership() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    SCRIPT.lock().unwrap().auto_route = true;
    let mut device = device();
    device.inspect_inert().unwrap();
    let address = device.add_address().unwrap();
    let error = match device.add_route() {
        Err(error) => error,
        Ok(_) => panic!("existing route adopted"),
    };
    let NativeError::Call(error) = error else {
        panic!("native existing-route status lost")
    };
    assert_eq!(error.status(), ERROR_OBJECT_ALREADY_EXISTS);
    assert!(device.route_receipt().is_none());
    let observation = device.inspect_address_route(&address).unwrap();
    device.inspect_derived_route(&observation).unwrap();
    device.remove_address(&address).unwrap();
    assert!(SCRIPT.lock().unwrap().route.is_none());
    assert_eq!(SCRIPT.lock().unwrap().delete_route_count, 0);
}

#[test]
fn caller_buffers_release_received_packet_and_real_session_before_adapter_close() {
    let _serial = SERIAL.lock().unwrap();
    *SCRIPT.lock().unwrap() = Script::new();
    let mut device = device();
    device.start_session(MIN_RING).unwrap();
    let mut short = [0; 4];
    assert!(matches!(
        device.try_recv(&mut short),
        Err(NativeError::PacketTooLarge {
            length: 8,
            capacity: 4
        })
    ));
    assert_eq!(SCRIPT.lock().unwrap().release_count, 1);
    let mut destination = [0; 8];
    assert_eq!(device.try_recv(&mut destination).unwrap(), 8);
    assert_eq!(destination, [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(device.try_send(&[9, 10, 11, 12]).unwrap(), 4);
    {
        let script = SCRIPT.lock().unwrap();
        assert_eq!(script.release_count, 2);
        assert_eq!(script.sent, 1);
        assert_eq!(&script.send_buffer[..4], &[9, 10, 11, 12]);
    }
    device.close();
    let script = SCRIPT.lock().unwrap();
    assert_eq!(script.order, ["create", "start", "end", "close"]);
    assert_eq!(script.close_count, 1);
}

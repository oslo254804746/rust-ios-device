//! ios-ffi: C FFI for iOS device discovery, lockdown queries, and CoreDevice tunnel functionality.
//!
//! This library keeps the tunnel APIs intact while adding a narrow device handle
//! for synchronous lockdown queries.
//! Once the tunnel is up, use any standard C networking library
//! (or system calls) to talk to the device.
//!
//! # Quick start (C)
//! ```c
//! #include "ios_rs.h"
//!
//! ios_runtime_init();
//!
//! // List devices
//! IosDevice *devices = NULL;
//! size_t count = 0;
//! ios_list_devices(&devices, &count);
//! const char *udid = devices[0].udid;
//!
//! // Start tunnel
//! IosTunnel *tunnel = NULL;
//! if (ios_start_tunnel(udid, IOS_TUN_USERSPACE, &tunnel) != 0) { ... }
//!
//! // Use the tunnel
//! char addr[64];
//! ios_tunnel_server_address(tunnel, addr, sizeof(addr));
//! uint16_t rsd_port = ios_tunnel_rsd_port(tunnel);
//! uint16_t proxy_port = ios_tunnel_userspace_port(tunnel);
//!
//! // Cleanup
//! ios_tunnel_close(tunnel);
//! ios_free_devices(devices, count);
//! ```

use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::os::raw::{c_char, c_int};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use tokio::runtime::Runtime;

static RUNTIME: Lazy<Runtime> =
    Lazy::new(|| Runtime::new().expect("failed to create tokio runtime"));

// ── Types exported to C ───────────────────────────────────────────────────────

/// Connected iOS device info.
#[repr(C)]
pub struct IosDevice {
    /// Null-terminated UDID string (must be freed via ios_free_devices).
    pub udid: *mut c_char,
    pub device_id: u32,
    /// Null-terminated connection type ("USB", "Network").
    pub connection_type: *mut c_char,
}

/// Tunnel mode.
#[repr(C)]
#[allow(non_camel_case_types)]
pub enum IosTunMode {
    /// Userspace smoltcp tunnel (no root/admin required). Recommended.
    IOS_TUN_USERSPACE = 0,
    /// Kernel TUN device (requires root/admin).
    IOS_TUN_KERNEL = 1,
}

/// Opaque tunnel handle. Created by ios_start_tunnel, destroyed by ios_tunnel_close.
#[repr(C)]
pub struct IosTunnel {
    server_address: String,
    rsd_port: u16,
    userspace_port: u16,
    /// Holds the ConnectedDevice (and TunnelHandle) alive.
    _device: Mutex<Option<ios_core::ConnectedDevice>>,
}

/// Opaque non-tunnel device handle. Created by ios_device_open, destroyed by ios_device_close.
#[repr(C)]
pub struct IosDeviceHandle {
    _device: Mutex<Option<Arc<ios_core::ConnectedDevice>>>,
}

const IOS_SUCCESS: c_int = 0;
const IOS_ERR_NULL: c_int = 1;
const IOS_ERR_UTF8: c_int = 2;
const IOS_ERR_CORE: c_int = 3;
const IOS_ERR_ALLOC: c_int = 4;
const IOS_ERR_STATE: c_int = 5;

// ── Runtime ───────────────────────────────────────────────────────────────────

/// Initialize the ios-rs async runtime.
/// Must be called once before any other function.
/// Safe to call multiple times (idempotent).
#[no_mangle]
pub extern "C" fn ios_runtime_init() {
    Lazy::force(&RUNTIME);
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("writing to string cannot fail");
    }
    out
}

fn plist_value_to_json(value: &plist::Value) -> serde_json::Value {
    match value {
        plist::Value::String(s) => serde_json::Value::String(s.clone()),
        plist::Value::Boolean(b) => serde_json::Value::Bool(*b),
        plist::Value::Integer(n) => {
            if let Some(i) = n.as_signed() {
                serde_json::json!(i)
            } else if let Some(u) = n.as_unsigned() {
                serde_json::json!(u)
            } else {
                serde_json::Value::Null
            }
        }
        plist::Value::Real(f) => serde_json::json!(f),
        plist::Value::Data(bytes) => serde_json::Value::String(hex_encode(bytes)),
        plist::Value::Date(date) => serde_json::Value::String(date.to_xml_format()),
        plist::Value::Uid(uid) => serde_json::Value::Number(serde_json::Number::from(uid.get())),
        plist::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(plist_value_to_json).collect())
        }
        plist::Value::Dictionary(values) => {
            let mut map = serde_json::Map::new();
            for (key, value) in values {
                map.insert(key.clone(), plist_value_to_json(value));
            }
            serde_json::Value::Object(map)
        }
        _ => serde_json::Value::Null,
    }
}

fn to_owned_c_string(value: String) -> Result<*mut c_char, c_int> {
    CString::new(value)
        .map(|s| s.into_raw())
        .map_err(|_| IOS_ERR_ALLOC)
}

unsafe fn write_owned_c_string(out: *mut *mut c_char, value: String) -> c_int {
    if out.is_null() {
        return IOS_ERR_NULL;
    }
    match to_owned_c_string(value) {
        Ok(ptr) => {
            *out = ptr;
            IOS_SUCCESS
        }
        Err(code) => code,
    }
}

unsafe fn device_from_handle(
    handle: *const IosDeviceHandle,
) -> Result<Arc<ios_core::ConnectedDevice>, c_int> {
    if handle.is_null() {
        return Err(IOS_ERR_NULL);
    }
    let guard = (*handle)._device.lock().map_err(|_| IOS_ERR_STATE)?;
    guard.as_ref().cloned().ok_or(IOS_ERR_STATE)
}

fn ios_devices_into_raw(devices: Vec<IosDevice>) -> (*mut IosDevice, usize) {
    let boxed = devices.into_boxed_slice();
    let count = boxed.len();
    let ptr = Box::into_raw(boxed) as *mut IosDevice;
    (ptr, count)
}

unsafe fn drop_ios_devices_allocation(devices: *mut IosDevice, count: usize) {
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
        devices, count,
    )));
}

// ── Device listing ────────────────────────────────────────────────────────────

/// List all connected iOS devices.
///
/// On success: fills `*devices_out` with a newly-allocated array of `IosDevice`,
/// sets `*count_out` to the number of elements, and returns 0.
/// On error: returns non-zero; `*devices_out` and `*count_out` are unchanged.
///
/// Free the result with `ios_free_devices(*devices_out, *count_out)`.
///
/// # Safety
///
/// Caller must pass valid, non-null pointers for `devices_out` and `count_out`.
#[no_mangle]
pub unsafe extern "C" fn ios_list_devices(
    devices_out: *mut *mut IosDevice,
    count_out: *mut usize,
) -> c_int {
    let result = RUNTIME.block_on(ios_core::list_devices());
    match result {
        Err(_) => 1,
        Ok(devs) => {
            let count = devs.len();
            let mut c_devs: Vec<IosDevice> = Vec::with_capacity(count);
            for d in devs {
                let udid = match CString::new(d.udid) {
                    Ok(s) => s.into_raw(),
                    Err(_) => continue, // skip devices with invalid UDIDs
                };
                let connection_type = match CString::new(d.connection_type) {
                    Ok(s) => s.into_raw(),
                    Err(_) => {
                        // Free the already-allocated udid before skipping
                        drop(CString::from_raw(udid));
                        continue;
                    }
                };
                c_devs.push(IosDevice {
                    udid,
                    device_id: d.device_id,
                    connection_type,
                });
            }
            let (ptr, actual_count) = ios_devices_into_raw(c_devs);
            *devices_out = ptr;
            *count_out = actual_count;
            0
        }
    }
}

/// Free a device list returned by `ios_list_devices`.
///
/// # Safety
///
/// Caller must pass a pointer previously returned by `ios_list_devices` with the correct count.
#[no_mangle]
pub unsafe extern "C" fn ios_free_devices(devices: *mut IosDevice, count: usize) {
    if devices.is_null() {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(devices, count);
    for d in &mut *slice {
        if !d.udid.is_null() {
            drop(CString::from_raw(d.udid));
        }
        if !d.connection_type.is_null() {
            drop(CString::from_raw(d.connection_type));
        }
    }
    drop_ios_devices_allocation(devices, count);
}

// ── Device handle ────────────────────────────────────────────────────────────

/// Open a non-tunnel device handle for synchronous lockdown queries.
///
/// The handle uses the existing pairing records but does not create a tunnel.
/// Free the result with `ios_device_close`.
///
/// # Safety
///
/// Caller must pass a valid null-terminated `udid` string and a non-null `out` pointer.
#[no_mangle]
pub unsafe extern "C" fn ios_device_open(
    udid: *const c_char,
    out: *mut *mut IosDeviceHandle,
) -> c_int {
    if udid.is_null() || out.is_null() {
        return IOS_ERR_NULL;
    }
    let udid_str = match CStr::from_ptr(udid).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return IOS_ERR_UTF8,
    };
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    match RUNTIME.block_on(ios_core::connect(&udid_str, opts)) {
        Ok(device) => {
            let handle = Box::new(IosDeviceHandle {
                _device: Mutex::new(Some(Arc::new(device))),
            });
            *out = Box::into_raw(handle);
            IOS_SUCCESS
        }
        Err(_) => IOS_ERR_CORE,
    }
}

/// Close and free a handle created by `ios_device_open`.
///
/// # Safety
///
/// Caller must pass a handle previously returned by `ios_device_open`, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_device_close(handle: *mut IosDeviceHandle) {
    if handle.is_null() {
        return;
    }
    let mut handle = Box::from_raw(handle);
    if let Ok(guard) = handle._device.get_mut() {
        guard.take();
    }
    drop(handle);
}

/// Get the device product version as a newly-allocated UTF-8 C string.
///
/// Free the returned string with `ios_free_string`.
///
/// # Safety
///
/// Caller must pass a valid `IosDeviceHandle` pointer and a non-null `out` pointer.
#[no_mangle]
pub unsafe extern "C" fn ios_get_product_version(
    handle: *const IosDeviceHandle,
    out: *mut *mut c_char,
) -> c_int {
    if out.is_null() {
        return IOS_ERR_NULL;
    }
    let device = match device_from_handle(handle) {
        Ok(device) => device,
        Err(code) => return code,
    };
    let version = match RUNTIME.block_on(device.product_version()) {
        Ok(version) => version.to_string(),
        Err(_) => return IOS_ERR_CORE,
    };
    write_owned_c_string(out, version)
}

/// Get a lockdown value as compact JSON in a newly-allocated UTF-8 C string.
///
/// Pass `key = NULL` to fetch the full lockdown dictionary.
/// Free the returned string with `ios_free_string`.
///
/// # Safety
///
/// Caller must pass a valid `IosDeviceHandle` pointer, an optional null-terminated `key`, and a non-null `out` pointer.
#[no_mangle]
pub unsafe extern "C" fn ios_get_lockdown_value_json(
    handle: *const IosDeviceHandle,
    key: *const c_char,
    out: *mut *mut c_char,
) -> c_int {
    if out.is_null() {
        return IOS_ERR_NULL;
    }
    let key_owned = if key.is_null() {
        None
    } else {
        match CStr::from_ptr(key).to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => return IOS_ERR_UTF8,
        }
    };
    let device = match device_from_handle(handle) {
        Ok(device) => device,
        Err(code) => return code,
    };
    let plist_value = match RUNTIME.block_on(device.lockdown_get_value(key_owned.as_deref())) {
        Ok(value) => value,
        Err(_) => return IOS_ERR_CORE,
    };
    let json = plist_value_to_json(&plist_value);
    let json_text = match serde_json::to_string(&json) {
        Ok(text) => text,
        Err(_) => return IOS_ERR_CORE,
    };
    write_owned_c_string(out, json_text)
}

/// Free a UTF-8 C string returned by ios_get_product_version or ios_get_lockdown_value_json.
///
/// # Safety
///
/// Caller must pass a string previously returned by an `ios_get_*` function, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    drop(CString::from_raw(s));
}

// ── Tunnel ────────────────────────────────────────────────────────────────────

/// Start a CDTunnel to the given device.
///
/// `udid`  – null-terminated device UDID.
/// `mode`  – `IOS_TUN_USERSPACE` (recommended) or `IOS_TUN_KERNEL`.
/// `out`   – on success, receives a newly-allocated `IosTunnel*`.
///
/// Returns 0 on success, non-zero on error.
/// Free the tunnel with `ios_tunnel_close`.
///
/// # Safety
///
/// Caller must pass a valid null-terminated `udid` string and a non-null `out` pointer.
#[no_mangle]
pub unsafe extern "C" fn ios_start_tunnel(
    udid: *const c_char,
    mode: IosTunMode,
    out: *mut *mut IosTunnel,
) -> c_int {
    if udid.is_null() || out.is_null() {
        return 1;
    }
    let udid_str = match CStr::from_ptr(udid).to_str() {
        Ok(s) => s.to_string(),
        Err(_) => return 2,
    };
    let tun_mode = match mode {
        IosTunMode::IOS_TUN_KERNEL => ios_core::tunnel::TunMode::Kernel,
        _ => ios_core::tunnel::TunMode::Userspace,
    };
    let opts = ios_core::device::ConnectOptions {
        tun_mode,
        pair_record_path: None,
        skip_tunnel: false,
    };
    match RUNTIME.block_on(ios_core::connect(&udid_str, opts)) {
        Err(_) => 3,
        Ok(device) => {
            let server_address = device.server_address().unwrap_or("").to_string();
            let rsd_port = device
                .tunnel
                .as_ref()
                .map(|t| t.info.server_rsd_port)
                .unwrap_or(0);
            let userspace_port = device.userspace_port().unwrap_or(0);
            let tunnel = Box::new(IosTunnel {
                server_address,
                rsd_port,
                userspace_port,
                _device: Mutex::new(Some(device)),
            });
            *out = Box::into_raw(tunnel);
            0
        }
    }
}

/// Close and free a tunnel created by `ios_start_tunnel`.
///
/// # Safety
///
/// Caller must pass a tunnel pointer previously returned by `ios_start_tunnel`, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_close(tunnel: *mut IosTunnel) {
    if tunnel.is_null() {
        return;
    }
    let mut t = Box::from_raw(tunnel);
    // Drop the ConnectedDevice first (cancels tunnel), then drop the box
    if let Ok(guard) = t._device.get_mut() {
        guard.take();
    }
    drop(t);
}

/// Get the device's tunnel IPv6 address (null-terminated string).
///
/// `buf_len` must be at least 46 bytes (max IPv6 string length + null).
/// Returns 0 on success.
///
/// # Safety
///
/// Caller must pass a valid `IosTunnel` pointer and a `buf` with at least `buf_len` bytes.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_server_address(
    tunnel: *const IosTunnel,
    buf: *mut c_char,
    buf_len: usize,
) -> c_int {
    if tunnel.is_null() || buf.is_null() {
        return 1;
    }
    let addr = &(*tunnel).server_address;
    let bytes = addr.as_bytes();
    if bytes.len() + 1 > buf_len {
        return 2;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
    *buf.add(bytes.len()) = 0;
    0
}

/// Get the RSD port (Remote Service Discovery, iOS 17+).
/// Returns 0 if the tunnel is not established or RSD is not available.
///
/// # Safety
///
/// Caller must pass a valid `IosTunnel` pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_rsd_port(tunnel: *const IosTunnel) -> u16 {
    if tunnel.is_null() {
        return 0;
    }
    (*tunnel).rsd_port
}

/// Get the local userspace proxy port (for IOS_TUN_USERSPACE mode).
///
/// Connect to `127.0.0.1:<port>` and send:
///
/// - 16 bytes: device IPv6 address (from ios_tunnel_server_address)
/// - 4 bytes:  target port as little-endian uint32
///
/// Then use the socket as a direct TCP connection to that service.
///
/// Returns 0 in kernel mode or if not available.
///
/// # Safety
///
/// Caller must pass a valid `IosTunnel` pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_userspace_port(tunnel: *const IosTunnel) -> u16 {
    if tunnel.is_null() {
        return 0;
    }
    (*tunnel).userspace_port
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_value_to_json_encodes_data_as_hex() {
        let value = plist::Value::Data(vec![0x00, 0xAB, 0xCD]);
        let json = plist_value_to_json(&value);
        assert_eq!(json, serde_json::Value::String("00abcd".to_string()));
    }

    #[test]
    fn to_owned_c_string_round_trips() {
        let ptr = to_owned_c_string("15.7.1".to_string()).unwrap();
        let text = unsafe { CString::from_raw(ptr).into_string().unwrap() };
        assert_eq!(text, "15.7.1");
    }

    #[test]
    fn ios_devices_raw_roundtrip_does_not_depend_on_vec_capacity() {
        let mut devices = Vec::with_capacity(8);
        devices.push(IosDevice {
            udid: std::ptr::null_mut(),
            device_id: 7,
            connection_type: std::ptr::null_mut(),
        });

        let (ptr, count) = ios_devices_into_raw(devices);

        assert_eq!(count, 1);
        unsafe {
            assert_eq!((*ptr).device_id, 7);
            drop_ios_devices_allocation(ptr, count);
        }
    }
}

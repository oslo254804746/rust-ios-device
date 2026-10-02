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
//! #include <stdio.h>
//!
//! int main(void) {
//!     if (ios_runtime_init_checked() != IOS_SUCCESS) return 1;
//!     IosDevice *devices = NULL;
//!     size_t count = 0;
//!     int status = ios_list_devices_checked(&devices, &count);
//!     if (status != IOS_SUCCESS) return 1;
//!     if (count == 0) puts("no device found");
//!     for (size_t i = 0; i < count; ++i)
//!         printf("%s %s\n", devices[i].udid, devices[i].connection_type);
//!     if (ios_free_devices_checked(&devices, &count) != IOS_SUCCESS) return 1;
//!     return 0;
//! }
//! ```

use ios_core::plist_json::{self, DataEncoding, PlistJsonPolicy};
use ios_core::ErrorClass;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::{Arc, LazyLock, Mutex};

use tokio::runtime::Runtime;

#[cfg(test)]
mod lifecycle_contract_tests;

static RUNTIME: LazyLock<std::io::Result<Runtime>> = LazyLock::new(Runtime::new);

thread_local! {
    static LAST_ERROR_CLASS: Cell<Option<ErrorClass>> = const { Cell::new(None) };
}

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
    rsd_services_json: String,
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
const IOS_ERR_RUNTIME: c_int = 6;
const IOS_ERR_PANIC: c_int = 7;
const IOS_ERR_BUFFER_TOO_SMALL: c_int = 8;

// ── Runtime ───────────────────────────────────────────────────────────────────

/// Initialize the ios-rs async runtime.
/// Initialize before device/tunnel operations; the error-class getter needs no runtime.
/// Safe to call multiple times (idempotent).
/// No result/category is reported; use [`ios_runtime_init_checked`] for a status.
#[no_mangle]
pub extern "C" fn ios_runtime_init() {
    ffi_void(|| {
        LazyLock::force(&RUNTIME);
    });
}

/// Initialize the shared runtime with an int status/caller-thread category.
///
/// Success clears the category. A cached ordinary initialization IO error returns
/// IOS_ERR_RUNTIME with its typed category; a caught unwind is IOS_ERR_PANIC/
/// unknown. Repeated calls reuse the same initialization result, without retrying
/// it. No device is opened. This does not suppress a process-installed panic hook
/// or recover from abort, invalid memory access or allocation failure that aborts.
#[no_mangle]
pub extern "C" fn ios_runtime_init_checked() -> c_int {
    ffi_status(|| match runtime() {
        Ok(_) => IOS_SUCCESS,
        Err(code) => code,
    })
}

/// Stable typed category of this caller thread's last int-status operation.
///
/// Returns a borrowed static NUL-terminated string; never free or modify it.
/// "none" means no int-status call on this thread yet, or its last one succeeded.
/// Void/port/getter calls do not change it. A category alone grants no retry or
/// cleanup guarantee. The pointer remains valid while the library is loaded.
#[no_mangle]
pub extern "C" fn ios_last_error_class() -> *const c_char {
    LAST_ERROR_CLASS.with(|state| {
        let code = match state.get() {
            None => c"none",
            Some(ErrorClass::Timeout) => c"timeout",
            Some(ErrorClass::Cancelled) => c"cancelled",
            Some(ErrorClass::Disconnected) => c"disconnected",
            Some(ErrorClass::ServiceMissing) => c"service_missing",
            Some(ErrorClass::PermissionDenied) => c"permission_denied",
            Some(ErrorClass::Protocol) => c"protocol",
            Some(ErrorClass::InvalidInput) => c"invalid_input",
            Some(ErrorClass::Unsupported) => c"unsupported",
            Some(ErrorClass::DeviceNotFound) => c"device_not_found",
            Some(ErrorClass::NotFound) => c"not_found",
            Some(ErrorClass::ResourceLimit) => c"resource_limit",
            Some(ErrorClass::Conflict) => c"conflict",
            Some(ErrorClass::Precondition) => c"precondition",
            Some(ErrorClass::ServiceFailure) => c"service_failure",
            Some(ErrorClass::Io) => c"io",
            Some(_) => c"unknown",
        };
        code.as_ptr()
    })
}

fn error_status(code: c_int, error: &(dyn std::error::Error + 'static)) -> c_int {
    LAST_ERROR_CLASS.with(|state| state.set(Some(ios_core::classify_error(error))));
    code
}

fn runtime() -> Result<&'static Runtime, c_int> {
    runtime_from(&RUNTIME)
}

fn runtime_from(initialization: &std::io::Result<Runtime>) -> Result<&Runtime, c_int> {
    initialization
        .as_ref()
        .map_err(|error| error_status(IOS_ERR_RUNTIME, error))
}

fn ffi_status(f: impl FnOnce() -> c_int + std::panic::UnwindSafe) -> c_int {
    LAST_ERROR_CLASS.with(|state| state.set(None));
    let code = std::panic::catch_unwind(f).unwrap_or(IOS_ERR_PANIC);
    LAST_ERROR_CLASS.with(|state| {
        if code == IOS_SUCCESS {
            state.set(None);
        } else if code == IOS_ERR_PANIC || state.get().is_none() {
            state.set(Some(match code {
                IOS_ERR_NULL | IOS_ERR_UTF8 => ErrorClass::InvalidInput,
                IOS_ERR_STATE => ErrorClass::Precondition,
                IOS_ERR_BUFFER_TOO_SMALL => ErrorClass::ResourceLimit,
                // IOS_ERR_ALLOC also represents an embedded NUL, not a proven
                // memory limit. Keep lost/unrecognized information unknown.
                _ => ErrorClass::Unknown,
            }));
        }
    });
    code
}

fn ffi_void(f: impl FnOnce() + std::panic::UnwindSafe) {
    let _ = std::panic::catch_unwind(f);
}

fn ffi_u16(f: impl FnOnce() -> u16 + std::panic::UnwindSafe) -> u16 {
    std::panic::catch_unwind(f).unwrap_or(0)
}

unsafe fn consume_pointer<T>(owner: *mut *mut T, release: impl FnOnce(*mut T) -> c_int) -> c_int {
    if owner.is_null() {
        return IOS_ERR_NULL;
    }
    let pointer = *owner;
    *owner = std::ptr::null_mut();
    if pointer.is_null() {
        IOS_SUCCESS
    } else {
        release(pointer)
    }
}

fn plist_value_to_json(value: &plist::Value) -> serde_json::Value {
    plist_json::to_json(value, PlistJsonPolicy::new(DataEncoding::HexString))
}

fn encode_rsd_services_json(
    rsd: Option<&ios_core::RsdHandshake>,
) -> Result<String, serde_json::Error> {
    let services = rsd
        .map(|rsd| {
            rsd.services
                .iter()
                .map(|(name, descriptor)| {
                    (
                        name.clone(),
                        serde_json::json!({
                            "port": descriptor.port,
                            "features": descriptor.features,
                        }),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    serde_json::to_string(&services)
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
    *out = std::ptr::null_mut();
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

#[derive(Clone, Copy)]
enum ListProjection {
    LegacySkip,
    Checked,
}

struct OwnedCDevice {
    udid: CString,
    device_id: u32,
    connection_type: CString,
}

fn project_device_list(
    devices: Vec<ios_core::DeviceInfo>,
    projection: ListProjection,
) -> Result<Vec<OwnedCDevice>, std::io::Error> {
    let mut projected = Vec::with_capacity(devices.len());
    for device in devices {
        let convert = || -> Result<OwnedCDevice, std::ffi::NulError> {
            Ok(OwnedCDevice {
                udid: CString::new(device.udid)?,
                device_id: device.device_id,
                connection_type: CString::new(device.connection_type)?,
            })
        };
        match convert() {
            Ok(row) => projected.push(row),
            Err(error) => match projection {
                ListProjection::LegacySkip => continue,
                ListProjection::Checked => {
                    // Received device data cannot be represented by this ABI.
                    // All prepared strings still have Rust ownership here.
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, error));
                }
            },
        }
    }
    Ok(projected)
}

fn validate_device_list_outputs(
    devices_out: *mut *mut IosDevice,
    count_out: *mut usize,
) -> Result<(), c_int> {
    if devices_out.is_null() || count_out.is_null() {
        return Err(IOS_ERR_NULL);
    }
    Ok(())
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
/// On error: returns non-zero; non-null output pointers are cleared before fallible work.
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
    list_devices_with_projection(devices_out, count_out, ListProjection::LegacySkip)
}

/// List received device records without silently skipping C encoding failures.
/// Returns IOS_ERR_CORE/Protocol and cleared outputs if a record contains NUL.
/// Uses the existing array layout and ios_free_devices ownership on success.
/// This validates the C projection, not completeness of discovery upstream.
///
/// # Safety
/// Caller must provide valid output pointers, as for ios_list_devices.
#[no_mangle]
pub unsafe extern "C" fn ios_list_devices_checked(
    devices_out: *mut *mut IosDevice,
    count_out: *mut usize,
) -> c_int {
    list_devices_with_projection(devices_out, count_out, ListProjection::Checked)
}

unsafe fn list_devices_with_projection(
    devices_out: *mut *mut IosDevice,
    count_out: *mut usize,
    projection: ListProjection,
) -> c_int {
    ffi_status(|| {
        if !devices_out.is_null() {
            *devices_out = std::ptr::null_mut();
        }
        if !count_out.is_null() {
            *count_out = 0;
        }
        if let Err(code) = validate_device_list_outputs(devices_out, count_out) {
            return code;
        }
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        let result = runtime.block_on(ios_core::list_devices());
        match result {
            Err(error) => error_status(IOS_ERR_CORE, &error),
            Ok(devs) => {
                let rows = match project_device_list(devs, projection) {
                    Ok(rows) => rows,
                    Err(error) => return error_status(IOS_ERR_CORE, &error),
                };
                // Reserve before any CString loses Rust ownership.
                let mut c_devs = Vec::with_capacity(rows.len());
                for row in rows {
                    c_devs.push(IosDevice {
                        udid: row.udid.into_raw(),
                        device_id: row.device_id,
                        connection_type: row.connection_type.into_raw(),
                    });
                }
                let (ptr, actual_count) = ios_devices_into_raw(c_devs);
                *devices_out = ptr;
                *count_out = actual_count;
                IOS_SUCCESS
            }
        }
    })
}

/// Free a device list returned by `ios_list_devices`.
///
/// # Safety
///
/// Caller must pass a pointer previously returned by `ios_list_devices` with the correct count.
#[no_mangle]
pub unsafe extern "C" fn ios_free_devices(devices: *mut IosDevice, count: usize) {
    ffi_void(|| free_device_list(devices, count));
}

unsafe fn free_device_list(devices: *mut IosDevice, count: usize) {
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

/// Consume/free a device list and reset the caller's pointer/count.
///
/// Both owner pointers are required; otherwise neither is modified and the call
/// returns IOS_ERR_NULL/invalid_input. Once accepted, they are cleared before
/// release, including on a caught unwind. A null list is a successful no-op.
/// Failure after consumption grants no safe retry or full-cleanup guarantee.
///
/// # Safety
///
/// Owner pointers must be null or valid, writable, distinct and non-overlapping
/// with the list. A non-null list must be a live allocation returned by an ios
/// list function with its exact count. Do not concurrently use/free it or retain
/// aliases for later cleanup.
#[no_mangle]
pub unsafe extern "C" fn ios_free_devices_checked(
    devices: *mut *mut IosDevice,
    count: *mut usize,
) -> c_int {
    ffi_status(|| {
        if devices.is_null() || count.is_null() {
            return IOS_ERR_NULL;
        }
        let allocation = *devices;
        let length = *count;
        *devices = std::ptr::null_mut();
        *count = 0;
        free_device_list(allocation, length);
        IOS_SUCCESS
    })
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
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = std::ptr::null_mut();
        if udid.is_null() {
            return IOS_ERR_NULL;
        }
        let udid_str = match CStr::from_ptr(udid).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return IOS_ERR_UTF8,
        };
        let opts = ios_core::device::ConnectOptions {
            tun_mode: ios_core::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        };
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        match runtime.block_on(ios_core::connect(&udid_str, opts)) {
            Ok(device) => {
                let handle = Box::new(IosDeviceHandle {
                    _device: Mutex::new(Some(Arc::new(device))),
                });
                *out = Box::into_raw(handle);
                IOS_SUCCESS
            }
            Err(error) => error_status(IOS_ERR_CORE, &error),
        }
    })
}

/// Close and free a handle created by `ios_device_open`.
///
/// # Safety
///
/// Caller must pass a handle previously returned by `ios_device_open`, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_device_close(handle: *mut IosDeviceHandle) {
    ffi_void(|| {
        let _ = close_device_handle(handle);
    });
}

unsafe fn close_device_handle(handle: *mut IosDeviceHandle) -> c_int {
    if handle.is_null() {
        return IOS_SUCCESS;
    }
    let mut handle = Box::from_raw(handle);
    let status = match handle._device.get_mut() {
        Ok(guard) => {
            guard.take();
            IOS_SUCCESS
        }
        Err(_) => IOS_ERR_STATE,
    };
    drop(handle);
    status
}

/// Consume/close a device handle and clear the caller's pointer before release.
///
/// A null owner pointer is IOS_ERR_NULL/invalid_input. A null owned handle is a
/// successful no-op. Poisoned state returns IOS_ERR_STATE/precondition after
/// consuming/dropping the handle; a caught unwind is IOS_ERR_PANIC/unknown.
/// No consumed handle may be retried, even on failure. No Apple cleanup is
/// acknowledged; other owners may retain the ConnectedDevice.
///
/// # Safety
///
/// `owner` must be null or writable pointer storage. Its non-null value must be a
/// live handle returned by ios_device_open, owned exclusively for this release.
/// Storage must not overlap the handle. Do not concurrently access/close it or
/// later use/free any retained alias.
#[no_mangle]
pub unsafe extern "C" fn ios_device_close_checked(owner: *mut *mut IosDeviceHandle) -> c_int {
    ffi_status(|| consume_pointer(owner, |handle| close_device_handle(handle)))
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
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = std::ptr::null_mut();
        let device = match device_from_handle(handle) {
            Ok(device) => device,
            Err(code) => return code,
        };
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        let version = match runtime.block_on(device.product_version()) {
            Ok(version) => version.to_string(),
            Err(error) => return error_status(IOS_ERR_CORE, &error),
        };
        write_owned_c_string(out, version)
    })
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
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = std::ptr::null_mut();
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
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        let plist_value = match runtime.block_on(device.lockdown_get_value(key_owned.as_deref())) {
            Ok(value) => value,
            Err(error) => return error_status(IOS_ERR_CORE, &error),
        };
        let json = plist_value_to_json(&plist_value);
        let json_text = match serde_json::to_string(&json) {
            Ok(text) => text,
            Err(error) => return error_status(IOS_ERR_CORE, &error),
        };
        write_owned_c_string(out, json_text)
    })
}

/// Free a UTF-8 C string returned by an ios-rs string-output function.
///
/// # Safety
///
/// Caller must pass a string previously returned by an `ios_get_*` function, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_free_string(s: *mut c_char) {
    ffi_void(|| free_string(s));
}

unsafe fn free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

/// Consume/free a library-owned C string and clear its caller pointer.
///
/// A null owner is IOS_ERR_NULL/invalid_input; a null string is a successful
/// no-op. The owner is cleared before release, even on a caught unwind. No safe
/// retry or completed cleanup follows from a failure after consumption.
///
/// # Safety
///
/// `owner` must be null or writable pointer storage containing null or a live
/// string returned by this library. Storage must not overlap the string. Do not
/// concurrently use/free it, modify its terminator or later use/free a retained
/// alias.
#[no_mangle]
pub unsafe extern "C" fn ios_free_string_checked(owner: *mut *mut c_char) -> c_int {
    ffi_status(|| {
        consume_pointer(owner, |string| {
            free_string(string);
            IOS_SUCCESS
        })
    })
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
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = std::ptr::null_mut();
        if udid.is_null() {
            return IOS_ERR_NULL;
        }
        let udid_str = match CStr::from_ptr(udid).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return IOS_ERR_UTF8,
        };
        let tun_mode = match mode {
            IosTunMode::IOS_TUN_KERNEL => ios_core::TunMode::Kernel,
            _ => ios_core::TunMode::Userspace,
        };
        let opts = ios_core::device::ConnectOptions {
            tun_mode,
            pair_record_path: None,
            skip_tunnel: false,
        };
        let runtime = match runtime() {
            Ok(runtime) => runtime,
            Err(code) => return code,
        };
        match runtime.block_on(ios_core::connect(&udid_str, opts)) {
            Err(error) => error_status(IOS_ERR_CORE, &error),
            Ok(device) => {
                let server_address = device.server_address().unwrap_or("").to_string();
                let rsd_port = device.rsd_port().unwrap_or(0);
                let userspace_port = device.userspace_port().unwrap_or(0);
                let rsd_services_json = match encode_rsd_services_json(device.rsd()) {
                    Ok(json) => json,
                    Err(error) => return error_status(IOS_ERR_CORE, &error),
                };
                let tunnel = Box::new(IosTunnel {
                    server_address,
                    rsd_port,
                    userspace_port,
                    rsd_services_json,
                    _device: Mutex::new(Some(device)),
                });
                *out = Box::into_raw(tunnel);
                IOS_SUCCESS
            }
        }
    })
}

/// Close and free a tunnel created by `ios_start_tunnel`.
///
/// # Safety
///
/// Caller must pass a tunnel pointer previously returned by `ios_start_tunnel`, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_close(tunnel: *mut IosTunnel) {
    ffi_void(|| {
        let _ = close_tunnel(tunnel);
    });
}

unsafe fn close_tunnel(tunnel: *mut IosTunnel) -> c_int {
    if tunnel.is_null() {
        return IOS_SUCCESS;
    }
    let mut tunnel = Box::from_raw(tunnel);
    let status = match tunnel._device.get_mut() {
        Ok(guard) => {
            guard.take();
            IOS_SUCCESS
        }
        Err(_) => IOS_ERR_STATE,
    };
    drop(tunnel);
    status
}

/// Consume/close a tunnel handle and clear the caller's pointer before release.
///
/// Status/ownership rules match [`ios_device_close_checked`]. Local owner drop
/// can request cancellation, but does not join background tasks or acknowledge
/// orderly device/Apple cleanup. A consumed handle cannot be retried on failure.
///
/// # Safety
///
/// `owner` must be null or writable pointer storage. Its non-null value must be a
/// live handle returned by ios_start_tunnel, owned exclusively for this release.
/// Storage must not overlap the handle. Do not concurrently access/close it or
/// later use/free any retained alias.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_close_checked(owner: *mut *mut IosTunnel) -> c_int {
    ffi_status(|| consume_pointer(owner, |tunnel| close_tunnel(tunnel)))
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
    ffi_status(|| {
        if tunnel.is_null() || buf.is_null() {
            return IOS_ERR_NULL;
        }
        let addr = &(*tunnel).server_address;
        let bytes = addr.as_bytes();
        if bytes.len() + 1 > buf_len {
            return IOS_ERR_BUFFER_TOO_SMALL;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
        *buf.add(bytes.len()) = 0;
        IOS_SUCCESS
    })
}

/// Get the RSD port (Remote Service Discovery, iOS 17+).
/// Returns 0 if the tunnel is not established or RSD is not available.
/// Use [`ios_tunnel_rsd_port_checked`] to distinguish NULL from a missing port.
///
/// # Safety
///
/// Caller must pass a valid `IosTunnel` pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_rsd_port(tunnel: *const IosTunnel) -> u16 {
    ffi_u16(|| {
        if tunnel.is_null() {
            return 0;
        }
        (*tunnel).rsd_port
    })
}

unsafe fn checked_tunnel_port(
    tunnel: *const IosTunnel,
    out: *mut u16,
    select: fn(&IosTunnel) -> u16,
) -> c_int {
    if out.is_null() {
        return IOS_ERR_NULL;
    }
    *out = 0;
    if tunnel.is_null() {
        return IOS_ERR_NULL;
    }
    let port = select(&*tunnel);
    if port == 0 {
        return IOS_ERR_STATE;
    }
    *out = port;
    IOS_SUCCESS
}

/// Get the cached RSD port with an int status and caller-thread error category.
///
/// A non-null output is cleared to zero before checking the handle. NULL inputs
/// return IOS_ERR_NULL/invalid_input; a zero cached port returns
/// IOS_ERR_STATE/precondition. Success writes a nonzero port and clears the
/// category. This snapshot query requires no runtime or network operation; it
/// does not prove current reachability or a device's support for an operation.
///
/// # Safety
///
/// `tunnel` must be null or a live handle. `out` must be null or point to writable
/// u16 storage that does not overlap the handle. Do not concurrently close it.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_rsd_port_checked(
    tunnel: *const IosTunnel,
    out: *mut u16,
) -> c_int {
    ffi_status(|| checked_tunnel_port(tunnel, out, |handle| handle.rsd_port))
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
/// Use [`ios_tunnel_userspace_port_checked`] for an explicit status/category.
///
/// # Safety
///
/// Caller must pass a valid `IosTunnel` pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_userspace_port(tunnel: *const IosTunnel) -> u16 {
    ffi_u16(|| {
        if tunnel.is_null() {
            return 0;
        }
        (*tunnel).userspace_port
    })
}

/// Get the cached local userspace proxy port with an int status/category.
///
/// Like [`ios_tunnel_rsd_port_checked`], this clears a non-null output, rejects
/// NULL inputs, and returns IOS_ERR_STATE/precondition when its cached port is
/// zero. It does not infer kernel mode, disconnect or explicit unsupported.
/// A nonzero port is a snapshot, not a reachability check.
///
/// # Safety
///
/// `tunnel` must be null or a live handle. `out` must be null or point to writable
/// u16 storage that does not overlap the handle. Do not concurrently close it.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_userspace_port_checked(
    tunnel: *const IosTunnel,
    out: *mut u16,
) -> c_int {
    ffi_status(|| checked_tunnel_port(tunnel, out, |handle| handle.userspace_port))
}

/// Get the RSD service directory as a newly allocated compact JSON object.
///
/// The object maps service names to `{ "port": number, "features": string[] }`.
/// Services that do not advertise capability metadata have an empty `features`
/// array. Free the returned string with [`ios_free_string`].
///
/// # Safety
///
/// Caller must pass a valid `IosTunnel` pointer and a non-null output pointer.
#[no_mangle]
pub unsafe extern "C" fn ios_tunnel_rsd_services_json(
    tunnel: *const IosTunnel,
    out: *mut *mut c_char,
) -> c_int {
    ffi_status(|| {
        if out.is_null() {
            return IOS_ERR_NULL;
        }
        *out = std::ptr::null_mut();
        if tunnel.is_null() {
            return IOS_ERR_NULL;
        }
        write_owned_c_string(out, (*tunnel).rsd_services_json.clone())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_class() -> &'static str {
        unsafe { CStr::from_ptr(ios_last_error_class()) }
            .to_str()
            .unwrap()
    }

    #[test]
    fn checked_list_projection_rejects_nul_while_legacy_keeps_valid_rows() {
        fn row(id: u32, udid: &str, connection_type: &str) -> ios_core::DeviceInfo {
            ios_core::DeviceInfo {
                device_id: id,
                udid: udid.into(),
                connection_type: connection_type.into(),
                product_id: 0,
            }
        }
        for bad in [
            row(2, "private\0invalid-device", "USB"),
            row(2, "private-invalid-type", "private\0invalid-type"),
        ] {
            let input = vec![
                row(1, "owned-valid-before", "USB"),
                bad,
                row(3, "owned-valid-after", "Network"),
            ];
            let legacy = project_device_list(input.clone(), ListProjection::LegacySkip).unwrap();
            assert_eq!(legacy.len(), 2);
            assert_eq!(legacy[0].udid.to_bytes(), b"owned-valid-before");
            assert_eq!(legacy[1].device_id, 3);
            let error = match project_device_list(input, ListProjection::Checked) {
                Err(error) => error,
                Ok(_) => panic!("unencodable row was silently accepted"),
            };
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert_eq!(
                error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<std::ffi::NulError>()
                    .unwrap()
                    .nul_position(),
                7
            );
            assert_eq!(error_status(IOS_ERR_CORE, &error), IOS_ERR_CORE);
            assert_eq!(last_class(), "protocol");
        }
        for projection in [ListProjection::LegacySkip, ListProjection::Checked] {
            assert!(project_device_list(vec![], projection).unwrap().is_empty());
            let rows =
                project_device_list(vec![row(42, "owned-Unicode-设备", "Network")], projection)
                    .unwrap();
            assert_eq!(rows[0].udid.to_bytes(), "owned-Unicode-设备".as_bytes());
            assert_eq!(rows[0].connection_type.to_bytes(), b"Network");
        }
    }

    #[test]
    fn checked_list_null_outputs_fail_before_runtime_or_device_work() {
        unsafe {
            let mut devices = std::ptr::NonNull::<IosDevice>::dangling().as_ptr();
            let mut count = 99;
            assert_eq!(
                ios_list_devices_checked(std::ptr::null_mut(), &mut count),
                IOS_ERR_NULL
            );
            assert_eq!(count, 0);
            assert_eq!(last_class(), "invalid_input");
            assert_eq!(
                ios_list_devices_checked(&mut devices, std::ptr::null_mut()),
                IOS_ERR_NULL
            );
            assert!(devices.is_null());
            assert_eq!(last_class(), "invalid_input");
        }
    }

    #[test]
    fn ffi_category_preserves_typed_sources_and_status_without_prose_lookup() {
        let secret = "synthetic-ffi-private-native-source";
        let io = std::io::Error::new(std::io::ErrorKind::TimedOut, secret);
        for status in [IOS_ERR_CORE, IOS_ERR_RUNTIME] {
            assert_eq!(
                ffi_status(std::panic::AssertUnwindSafe(|| error_status(status, &io))),
                status
            );
            assert_eq!(last_class(), "timeout");
        }
        assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
        assert_eq!(io.to_string(), secret);
        let cases: [(Box<dyn std::error::Error>, &str); 3] = [
            (Box::new(ios_core::OperationCancelled::new()), "cancelled"),
            (
                Box::new(std::io::Error::from(std::io::ErrorKind::Interrupted)),
                "io",
            ),
            (
                Box::new(ios_core::CoreError::Other(
                    "permission denied timeout cancelled".into(),
                )),
                "unknown",
            ),
        ];
        for (error, class) in cases {
            assert_eq!(
                ffi_status(std::panic::AssertUnwindSafe(|| error_status(
                    IOS_ERR_CORE,
                    error.as_ref()
                ))),
                IOS_ERR_CORE
            );
            assert_eq!(last_class(), class);
        }
    }

    #[test]
    fn ffi_category_tracks_status_success_without_void_or_port_reset() {
        let mut output = 1usize as *mut c_char;
        assert_eq!(
            unsafe { ios_get_product_version(std::ptr::null(), &mut output) },
            IOS_ERR_NULL
        );
        assert!(output.is_null());
        assert_eq!(last_class(), "invalid_input");
        let saved = ios_last_error_class();
        unsafe {
            ios_free_string(std::ptr::null_mut());
            ios_free_devices(std::ptr::null_mut(), 0);
            ios_device_close(std::ptr::null_mut());
            ios_tunnel_close(std::ptr::null_mut());
            assert_eq!(ios_tunnel_rsd_port(std::ptr::null()), 0);
            assert_eq!(ios_tunnel_userspace_port(std::ptr::null()), 0);
        }
        assert_eq!(last_class(), "invalid_input");

        let handle = IosDeviceHandle {
            _device: Mutex::new(None),
        };
        assert_eq!(
            unsafe { ios_get_product_version(&handle, &mut output) },
            IOS_ERR_STATE
        );
        assert_eq!(last_class(), "precondition");
        let tunnel = IosTunnel {
            server_address: "fd00::8".into(),
            rsd_port: 1234,
            userspace_port: 4321,
            rsd_services_json: "embedded\0NUL".into(),
            _device: Mutex::new(None),
        };
        assert_eq!(
            unsafe { ios_tunnel_rsd_services_json(&tunnel, &mut output) },
            IOS_ERR_ALLOC
        );
        assert!(output.is_null());
        assert_eq!(
            last_class(),
            "unknown",
            "ALLOC does not prove allocation exhaustion"
        );
        let mut small = [0; 1];
        assert_eq!(
            unsafe { ios_tunnel_server_address(&tunnel, small.as_mut_ptr(), small.len()) },
            IOS_ERR_BUFFER_TOO_SMALL
        );
        assert_eq!(last_class(), "resource_limit");
        let mut enough = [0; 46];
        assert_eq!(
            unsafe { ios_tunnel_server_address(&tunnel, enough.as_mut_ptr(), enough.len()) },
            IOS_SUCCESS
        );
        assert_eq!(last_class(), "none");
        assert_eq!(
            unsafe { CStr::from_ptr(saved) }.to_str().unwrap(),
            "invalid_input",
            "borrowed code remains valid after later calls"
        );
    }

    #[test]
    fn checked_ports_report_null_missing_and_snapshot_without_changing_legacy() {
        type Checked = unsafe extern "C" fn(*const IosTunnel, *mut u16) -> c_int;
        type Legacy = unsafe extern "C" fn(*const IosTunnel) -> u16;
        let queries: [(Checked, Legacy); 2] = [
            (ios_tunnel_rsd_port_checked, ios_tunnel_rsd_port),
            (ios_tunnel_userspace_port_checked, ios_tunnel_userspace_port),
        ];
        for (rsd_port, userspace_port) in [
            (1234, 4321),
            (0, 4321),
            (1234, 0),
            (0, 0),
            (1, u16::MAX),
            (u16::MAX, 1),
        ] {
            // An owned metadata fixture, not a constructed physical connection.
            let tunnel = IosTunnel {
                server_address: "fd00::8".into(),
                rsd_port,
                userspace_port,
                rsd_services_json: "{}".into(),
                _device: Mutex::new(None),
            };
            for ((checked, legacy), expected) in queries.into_iter().zip([rsd_port, userspace_port])
            {
                ffi_status(|| {
                    error_status(
                        IOS_ERR_CORE,
                        &std::io::Error::from(std::io::ErrorKind::TimedOut),
                    )
                });
                let saved = ios_last_error_class();
                assert_eq!(unsafe { legacy(&tunnel) }, expected);
                assert_eq!(unsafe { legacy(std::ptr::null()) }, 0);
                assert_eq!(last_class(), "timeout");

                let mut output = 99;
                assert_eq!(
                    unsafe { checked(&tunnel, &mut output) },
                    if expected == 0 {
                        IOS_ERR_STATE
                    } else {
                        IOS_SUCCESS
                    }
                );
                assert_eq!(output, expected);
                assert_eq!(
                    last_class(),
                    if expected == 0 {
                        "precondition"
                    } else {
                        "none"
                    }
                );
                assert_eq!(unsafe { CStr::from_ptr(saved) }.to_bytes(), b"timeout");

                output = 99;
                assert_eq!(
                    unsafe { checked(std::ptr::null(), &mut output) },
                    IOS_ERR_NULL
                );
                assert_eq!(output, 0);
                assert_eq!(last_class(), "invalid_input");
                for handle in [&tunnel as *const IosTunnel, std::ptr::null()] {
                    assert_eq!(
                        unsafe { checked(handle, std::ptr::null_mut()) },
                        IOS_ERR_NULL
                    );
                    assert_eq!(last_class(), "invalid_input");
                }
            }
        }
    }

    #[test]
    fn checked_port_category_is_local_to_actual_exported_caller() {
        ffi_status(|| {
            error_status(
                IOS_ERR_CORE,
                &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            )
        });
        let observed = std::thread::spawn(|| {
            let initial = last_class();
            let mut port = 99;
            let status = unsafe { ios_tunnel_rsd_port_checked(std::ptr::null(), &mut port) };
            (initial, status, port, last_class())
        })
        .join()
        .unwrap();
        assert_eq!(observed, ("none", IOS_ERR_NULL, 0, "invalid_input"));
        assert_eq!(last_class(), "permission_denied");
    }

    #[test]
    fn ffi_category_is_caller_thread_local() {
        ffi_status(|| {
            error_status(
                IOS_ERR_CORE,
                &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            )
        });
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = [
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::InvalidInput,
        ]
        .into_iter()
        .map(|kind| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let initial = last_class();
                let status = ffi_status(|| error_status(IOS_ERR_CORE, &std::io::Error::from(kind)));
                barrier.wait();
                let observed = last_class();
                barrier.wait();
                let success = ffi_status(|| IOS_SUCCESS);
                (initial, status, observed, success, last_class())
            })
        })
        .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            results,
            vec![
                ("none", IOS_ERR_CORE, "timeout", IOS_SUCCESS, "none"),
                ("none", IOS_ERR_CORE, "invalid_input", IOS_SUCCESS, "none")
            ]
        );
        assert_eq!(last_class(), "permission_denied");
    }

    #[test]
    fn ffi_category_codes_match_rust_and_borrow_static_strings() {
        let classes = [
            ErrorClass::Timeout,
            ErrorClass::Cancelled,
            ErrorClass::Disconnected,
            ErrorClass::ServiceMissing,
            ErrorClass::PermissionDenied,
            ErrorClass::Protocol,
            ErrorClass::InvalidInput,
            ErrorClass::Unsupported,
            ErrorClass::DeviceNotFound,
            ErrorClass::NotFound,
            ErrorClass::ResourceLimit,
            ErrorClass::Conflict,
            ErrorClass::Precondition,
            ErrorClass::ServiceFailure,
            ErrorClass::Io,
            ErrorClass::Unknown,
        ];
        for class in classes {
            LAST_ERROR_CLASS.with(|state| state.set(Some(class)));
            let saved = ios_last_error_class();
            assert_eq!(last_class(), class.as_str());
            ffi_status(|| IOS_SUCCESS);
            assert_eq!(
                unsafe { CStr::from_ptr(saved) }.to_str().unwrap(),
                class.as_str()
            );
        }
    }

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

    #[test]
    fn validate_device_list_outputs_rejects_null_pointers() {
        let mut devices = std::ptr::null_mut();
        let mut count = 0usize;

        assert_eq!(
            validate_device_list_outputs(std::ptr::null_mut(), &mut count),
            Err(IOS_ERR_NULL)
        );
        assert_eq!(
            validate_device_list_outputs(&mut devices, std::ptr::null_mut()),
            Err(IOS_ERR_NULL)
        );
    }

    #[test]
    fn validate_device_list_outputs_accepts_valid_pointers() {
        let mut devices = std::ptr::null_mut();
        let mut count = 0usize;

        assert_eq!(
            validate_device_list_outputs(&mut devices, &mut count),
            Ok(())
        );
    }

    #[test]
    fn ffi_status_converts_panics_to_status_code() {
        let code = ffi_status(|| {
            error_status(IOS_ERR_CORE, &ios_core::OperationCancelled::new());
            panic!("ffi panic boundary")
        });

        assert_eq!(code, IOS_ERR_PANIC);
        assert_eq!(last_class(), "unknown");
    }

    #[test]
    fn device_open_clears_out_pointer_on_error() {
        let mut out = 0x1usize as *mut IosDeviceHandle;
        let code = unsafe { ios_device_open(std::ptr::null(), &mut out) };

        assert_eq!(code, IOS_ERR_NULL);
        assert!(out.is_null());
    }

    #[test]
    fn list_devices_clears_outputs_on_error() {
        let code = unsafe { ios_list_devices(std::ptr::null_mut(), std::ptr::null_mut()) };

        assert_eq!(code, IOS_ERR_NULL);
    }

    #[test]
    fn status_codes_are_distinct() {
        let codes = [
            IOS_SUCCESS,
            IOS_ERR_NULL,
            IOS_ERR_UTF8,
            IOS_ERR_CORE,
            IOS_ERR_ALLOC,
            IOS_ERR_STATE,
            IOS_ERR_RUNTIME,
            IOS_ERR_PANIC,
            IOS_ERR_BUFFER_TOO_SMALL,
        ];

        for (index, code) in codes.iter().enumerate() {
            assert!(!codes[..index].contains(code));
        }
    }

    #[test]
    fn tunnel_server_address_uses_named_error_codes() {
        let mut buffer = [0; 4];
        assert_eq!(
            unsafe {
                ios_tunnel_server_address(std::ptr::null(), buffer.as_mut_ptr(), buffer.len())
            },
            IOS_ERR_NULL
        );

        let tunnel = IosTunnel {
            server_address: "fd00::1234".to_string(),
            rsd_port: 1234,
            userspace_port: 4321,
            rsd_services_json: "{}".to_string(),
            _device: Mutex::new(None),
        };

        assert_eq!(
            unsafe { ios_tunnel_server_address(&tunnel, buffer.as_mut_ptr(), buffer.len()) },
            IOS_ERR_BUFFER_TOO_SMALL
        );
    }

    #[test]
    fn tunnel_rsd_services_json_returns_owned_string() {
        let tunnel = IosTunnel {
            server_address: "fd00::1234".to_string(),
            rsd_port: 1234,
            userspace_port: 4321,
            rsd_services_json:
                r#"{"com.apple.coredevice.appservice":{"features":["listapps"],"port":1234}}"#
                    .to_string(),
            _device: Mutex::new(None),
        };
        let mut output = std::ptr::null_mut();

        assert_eq!(
            unsafe { ios_tunnel_rsd_services_json(&tunnel, &mut output) },
            IOS_SUCCESS
        );
        let value: serde_json::Value = serde_json::from_str(
            unsafe { CStr::from_ptr(output) }
                .to_str()
                .expect("FFI output should be UTF-8"),
        )
        .expect("FFI output should be JSON");
        assert_eq!(
            value["com.apple.coredevice.appservice"]["port"],
            serde_json::json!(1234)
        );
        assert_eq!(
            value["com.apple.coredevice.appservice"]["features"],
            serde_json::json!(["listapps"])
        );
        unsafe { ios_free_string(output) };
    }

    #[test]
    fn encode_rsd_services_json_is_complete_and_stable() {
        let rsd = ios_core::RsdHandshake {
            udid: "test".into(),
            services: std::collections::HashMap::from([
                (
                    "z.service".into(),
                    ios_core::ServiceDescriptor {
                        port: 2,
                        features: vec!["feature.z".into()],
                    },
                ),
                ("a.service".into(), ios_core::ServiceDescriptor::new(1)),
            ]),
        };

        assert_eq!(
            encode_rsd_services_json(Some(&rsd)).unwrap(),
            r#"{"a.service":{"features":[],"port":1},"z.service":{"features":["feature.z"],"port":2}}"#
        );
        assert_eq!(encode_rsd_services_json(None).unwrap(), "{}");
    }

    #[test]
    fn tunnel_rsd_services_json_validates_and_clears_outputs() {
        let mut output = 1usize as *mut c_char;
        assert_eq!(
            unsafe { ios_tunnel_rsd_services_json(std::ptr::null(), &mut output) },
            IOS_ERR_NULL
        );
        assert!(output.is_null());

        let tunnel = IosTunnel {
            server_address: "fd00::1234".to_string(),
            rsd_port: 1234,
            userspace_port: 4321,
            rsd_services_json: "{}".to_string(),
            _device: Mutex::new(None),
        };
        assert_eq!(
            unsafe { ios_tunnel_rsd_services_json(&tunnel, std::ptr::null_mut()) },
            IOS_ERR_NULL
        );
    }
}

#[cfg(test)]
mod plist_contract;

#[cfg(test)]
#[test]
fn plist_json_legacy_contract() {
    crate::plist_contract::check(
        plist_value_to_json(&crate::plist_contract::value()),
        serde_json::json!("007fff"),
        serde_json::json!(""),
        true,
    );
}

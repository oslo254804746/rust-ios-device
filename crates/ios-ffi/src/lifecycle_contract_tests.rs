//! Owned projection/consumption fixtures; no physical device or Apple cleanup.

use super::*;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};

fn last_class() -> &'static str {
    unsafe { CStr::from_ptr(ios_last_error_class()) }
        .to_str()
        .unwrap()
}

fn seed_timeout() {
    ffi_status(|| {
        error_status(
            IOS_ERR_CORE,
            &std::io::Error::from(std::io::ErrorKind::TimedOut),
        )
    });
}

fn device_handle() -> Box<IosDeviceHandle> {
    Box::new(IosDeviceHandle {
        _device: Mutex::new(None),
    })
}

fn tunnel_handle() -> Box<IosTunnel> {
    Box::new(IosTunnel {
        server_address: "fd00::8".into(),
        rsd_port: 1234,
        userspace_port: 4321,
        rsd_services_json: "{\"owned\":{\"port\":1234,\"features\":[]}}".into(),
        _device: Mutex::new(None),
    })
}

#[test]
fn cached_native_runtime_failure_keeps_category_and_original_result() {
    let secret = "synthetic-private-ffi-runtime-source";
    for (kind, expected) in [
        (std::io::ErrorKind::TimedOut, "timeout"),
        (std::io::ErrorKind::PermissionDenied, "permission_denied"),
        (std::io::ErrorKind::Other, "io"),
    ] {
        let initialization = Err(std::io::Error::new(kind, secret));
        let invoked = std::cell::Cell::new(0);
        for _ in 0..2 {
            assert_eq!(
                ffi_status(AssertUnwindSafe(|| match runtime_from(&initialization) {
                    Ok(_) => {
                        invoked.set(invoked.get() + 1);
                        IOS_SUCCESS
                    }
                    Err(code) => code,
                })),
                IOS_ERR_RUNTIME
            );
            assert_eq!(last_class(), expected);
            assert!(!last_class().contains(secret));
        }
        assert_eq!(invoked.get(), 0);
        let original = initialization.as_ref().unwrap_err();
        assert_eq!(original.kind(), kind);
        assert_eq!(original.to_string(), secret);
    }
}

#[test]
fn actual_checked_runtime_success_reuses_result_and_legacy_keeps_category() {
    for _ in 0..2 {
        seed_timeout();
        let saved = ios_last_error_class();
        ios_runtime_init();
        assert_eq!(last_class(), "timeout");
        assert_eq!(ios_runtime_init_checked(), IOS_SUCCESS);
        assert_eq!(last_class(), "none");
        assert_eq!(unsafe { CStr::from_ptr(saved) }.to_bytes(), b"timeout");
    }
    let owned = tokio::runtime::Builder::new_current_thread().build();
    assert!(std::ptr::eq(
        runtime_from(&owned).unwrap(),
        owned.as_ref().unwrap()
    ));
}

#[test]
fn checked_cleanup_distinguishes_null_owner_from_empty_owned_resource() {
    unsafe {
        assert_eq!(ios_free_string_checked(std::ptr::null_mut()), IOS_ERR_NULL);
        assert_eq!(last_class(), "invalid_input");
        assert_eq!(ios_device_close_checked(std::ptr::null_mut()), IOS_ERR_NULL);
        assert_eq!(last_class(), "invalid_input");
        assert_eq!(ios_tunnel_close_checked(std::ptr::null_mut()), IOS_ERR_NULL);
        assert_eq!(last_class(), "invalid_input");
        assert_eq!(
            ios_free_devices_checked(std::ptr::null_mut(), std::ptr::null_mut()),
            IOS_ERR_NULL
        );
        assert_eq!(last_class(), "invalid_input");

        let mut string = std::ptr::null_mut();
        let mut device = std::ptr::null_mut();
        let mut tunnel = std::ptr::null_mut();
        let mut devices = std::ptr::null_mut();
        let mut count = 99;
        for _ in 0..2 {
            seed_timeout();
            assert_eq!(ios_free_string_checked(&mut string), IOS_SUCCESS);
            assert!(string.is_null());
            assert_eq!(last_class(), "none");
            seed_timeout();
            assert_eq!(ios_device_close_checked(&mut device), IOS_SUCCESS);
            assert!(device.is_null());
            assert_eq!(last_class(), "none");
            seed_timeout();
            assert_eq!(ios_tunnel_close_checked(&mut tunnel), IOS_SUCCESS);
            assert!(tunnel.is_null());
            assert_eq!(last_class(), "none");
            seed_timeout();
            assert_eq!(
                ios_free_devices_checked(&mut devices, &mut count),
                IOS_SUCCESS
            );
            assert!(devices.is_null());
            assert_eq!(count, 0);
            assert_eq!(last_class(), "none");
        }
    }
}

#[test]
fn checked_list_and_string_consume_library_allocations_and_preserve_rejected_owners() {
    let rows = ["owned-first", "owned-设备"]
        .into_iter()
        .enumerate()
        .map(|(index, udid)| IosDevice {
            udid: CString::new(udid).unwrap().into_raw(),
            device_id: index as u32,
            connection_type: CString::new("USB").unwrap().into_raw(),
        })
        .collect();
    let (mut devices, mut count) = ios_devices_into_raw(rows);
    let original = devices;
    unsafe {
        assert_eq!(
            ios_free_devices_checked(std::ptr::null_mut(), &mut count),
            IOS_ERR_NULL
        );
        assert_eq!(count, 2);
        assert_eq!(last_class(), "invalid_input");
        assert_eq!(
            ios_free_devices_checked(&mut devices, std::ptr::null_mut()),
            IOS_ERR_NULL
        );
        assert_eq!(devices, original);
        assert_eq!(last_class(), "invalid_input");
        assert_eq!(
            CStr::from_ptr((*devices.add(1)).udid).to_str().unwrap(),
            "owned-设备"
        );
        assert_eq!(
            ios_free_devices_checked(&mut devices, &mut count),
            IOS_SUCCESS
        );
        assert!(devices.is_null());
        assert_eq!(count, 0);
        assert_eq!(last_class(), "none");
        assert_eq!(
            ios_free_devices_checked(&mut devices, &mut count),
            IOS_SUCCESS
        );

        let (mut empty, mut empty_count) = ios_devices_into_raw(vec![]);
        assert!(!empty.is_null());
        assert_eq!(
            ios_free_devices_checked(&mut empty, &mut empty_count),
            IOS_SUCCESS
        );
        assert!(empty.is_null());
        assert_eq!(empty_count, 0);

        let mut string = std::ptr::null_mut();
        let tunnel = tunnel_handle();
        assert_eq!(
            ios_tunnel_rsd_services_json(&*tunnel, &mut string),
            IOS_SUCCESS
        );
        assert_eq!(
            CStr::from_ptr(string).to_str().unwrap(),
            tunnel.rsd_services_json
        );
        assert_eq!(ios_free_string_checked(&mut string), IOS_SUCCESS);
        assert!(string.is_null());
        assert_eq!(last_class(), "none");
        assert_eq!(ios_free_string_checked(&mut string), IOS_SUCCESS);
    }
}

#[test]
fn actual_checked_handles_are_consumed_even_after_poisoned_state() {
    for poisoned in [false, true] {
        let device = device_handle();
        let tunnel = tunnel_handle();
        if poisoned {
            assert!(std::panic::catch_unwind(|| {
                let _guard = device._device.lock().unwrap();
                panic!("owned device state poison");
            })
            .is_err());
            assert!(std::panic::catch_unwind(|| {
                let _guard = tunnel._device.lock().unwrap();
                panic!("owned tunnel state poison");
            })
            .is_err());
        }
        let mut device = Box::into_raw(device);
        let mut tunnel = Box::into_raw(tunnel);
        let expected = if poisoned { IOS_ERR_STATE } else { IOS_SUCCESS };
        let category = if poisoned { "precondition" } else { "none" };
        unsafe {
            assert_eq!(ios_device_close_checked(&mut device), expected);
            assert!(device.is_null());
            assert_eq!(last_class(), category);
            let saved = ios_last_error_class();
            assert_eq!(ios_tunnel_close_checked(&mut tunnel), expected);
            assert!(tunnel.is_null());
            assert_eq!(last_class(), category);
            assert_eq!(ios_device_close_checked(&mut device), IOS_SUCCESS);
            assert_eq!(last_class(), "none");
            assert_eq!(ios_tunnel_close_checked(&mut tunnel), IOS_SUCCESS);
            assert_eq!(last_class(), "none");
            assert_eq!(CStr::from_ptr(saved).to_str().unwrap(), category);
        }
    }
    seed_timeout();
    unsafe {
        ios_device_close(Box::into_raw(device_handle()));
        ios_tunnel_close(Box::into_raw(tunnel_handle()));
        ios_free_string(to_owned_c_string("owned-string".into()).unwrap());
        let (empty, count) = ios_devices_into_raw(vec![]);
        ios_free_devices(empty, count);
    }
    assert_eq!(last_class(), "timeout");
}

#[test]
fn consumption_clears_owner_before_release_failure_or_destructor_unwind() {
    struct OwnedDrop {
        drops: Arc<AtomicUsize>,
        panic_on_drop: bool,
    }
    impl Drop for OwnedDrop {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            if self.panic_on_drop {
                panic!("owned destructor unwind");
            }
        }
    }
    for panic_on_drop in [false, true] {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut owner = Box::into_raw(Box::new(OwnedDrop {
            drops: Arc::clone(&drops),
            panic_on_drop,
        }));
        let storage = &mut owner as *mut *mut OwnedDrop;
        let status = ffi_status(|| unsafe {
            consume_pointer(storage, |pointer| {
                assert!((*storage).is_null(), "clear before invoking release");
                drop(Box::from_raw(pointer));
                IOS_ERR_STATE
            })
        });
        assert_eq!(
            status,
            if panic_on_drop {
                IOS_ERR_PANIC
            } else {
                IOS_ERR_STATE
            }
        );
        assert_eq!(
            last_class(),
            if panic_on_drop {
                "unknown"
            } else {
                "precondition"
            }
        );
        assert!(owner.is_null());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            ffi_status(AssertUnwindSafe(|| unsafe {
                consume_pointer(storage, |_| {
                    calls.set(calls.get() + 1);
                    IOS_ERR_STATE
                })
            })),
            IOS_SUCCESS
        );
        assert_eq!(calls.get(), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(last_class(), "none");
    }
}

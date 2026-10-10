//! C-owner/ABI decisions; linked wire/TLS evidence is a separate explicit gate.
#![allow(clippy::needless_borrow)]
use super::*;

#[path = "../../../ios-core/src/lockdown/checked_session/test_assets.rs"]
mod assets;

fn class() -> &'static str {
    unsafe { CStr::from_ptr(ios_last_error_class()) }
        .to_str()
        .unwrap()
}

fn pair() -> Vec<u8> {
    let mut dictionary = plist::Dictionary::new();
    for (key, bytes) in [
        ("DeviceCertificate", &b""[..]),
        ("HostCertificate", assets::TEST_CERT_PEM),
        ("HostPrivateKey", assets::TEST_KEY_PEM),
        ("RootCertificate", &b""[..]),
    ] {
        dictionary.insert(key.into(), plist::Value::Data(bytes.to_vec()));
    }
    dictionary.insert("HostID".into(), "owned-ffi-host".into());
    dictionary.insert("SystemBUID".into(), "owned-ffi-buid".into());
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &dictionary).unwrap();
    bytes
}

unsafe fn session(address: &str) -> *mut IosDeviceSession {
    let bytes = pair();
    let udid = "owned-ffi-session-device";
    let config = IosDeviceSessionConfig {
        udid: udid.as_ptr().cast(),
        udid_len: udid.len(),
        endpoint: address.as_ptr().cast(),
        endpoint_len: address.len(),
        pair_record: bytes.as_ptr(),
        pair_record_len: bytes.len(),
        concurrent_operations: 1,
    };
    let mut handle = std::ptr::null_mut();
    assert_eq!(ios_device_session_create(&config, &mut handle), IOS_SUCCESS);
    handle
}

#[test]
fn zero_logs_require_valid_checked_owner_and_never_open_a_socket() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("tcp://{}", listener.local_addr().unwrap());
    unsafe {
        let mut session = session(&endpoint);
        let mut handle = std::ptr::null_mut();
        assert_eq!(
            ios_device_handle_from_session(session, &mut handle),
            IOS_SUCCESS
        );
        let mut output = std::ptr::null_mut();
        assert_eq!(
            ios_device_syslog_read_json(handle, 0, 1000, &mut output),
            IOS_SUCCESS
        );
        assert_eq!(CStr::from_ptr(output).to_bytes(), b"[]");
        assert_eq!(ios_free_string_checked(&mut output), IOS_SUCCESS);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(ios_device_services_close(handle, 1000), IOS_SUCCESS);
        assert_eq!(
            ios_device_syslog_read_json(handle, 0, 1000, &mut output),
            IOS_ERR_CORE
        );
        assert_eq!(class(), "precondition");
        assert!(output.is_null());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(ios_device_close_checked(&mut handle), IOS_SUCCESS);
        assert_eq!(
            ios_device_session_destroy_checked(&mut session),
            IOS_SUCCESS
        );
    }
}

#[test]
fn null_owners_output_clearing_and_repeated_checked_destroy() {
    unsafe {
        let mut session = std::ptr::null_mut();
        let mut cancel = std::ptr::null_mut();
        let mut text = c"old-output".as_ptr().cast_mut();
        assert_eq!(
            ios_device_session_create(std::ptr::null(), &mut session),
            IOS_ERR_NULL
        );
        assert!(session.is_null());
        assert_eq!(
            ios_device_session_retain(std::ptr::null(), &mut session),
            IOS_ERR_NULL
        );
        assert_eq!(
            ios_operation_cancel_retain(std::ptr::null(), &mut cancel),
            IOS_ERR_NULL
        );
        assert_eq!(
            ios_device_session_query_json(
                std::ptr::null(),
                7,
                std::ptr::null(),
                std::ptr::null(),
                &mut text
            ),
            IOS_ERR_NULL
        );
        assert!(text.is_null());
        assert_eq!(class(), "invalid_input");
        assert_eq!(
            ios_device_session_close(std::ptr::null(), 100),
            IOS_ERR_NULL
        );
        assert_eq!(ios_operation_cancel(std::ptr::null()), IOS_ERR_NULL);
        assert_eq!(
            ios_device_query_options_default(std::ptr::null_mut()),
            IOS_ERR_NULL
        );
        assert_eq!(
            ios_device_session_destroy_checked(std::ptr::null_mut()),
            IOS_ERR_NULL
        );
        assert_eq!(
            ios_operation_cancel_destroy_checked(std::ptr::null_mut()),
            IOS_ERR_NULL
        );
        for _ in 0..2 {
            assert_eq!(
                ios_device_session_destroy_checked(&mut session),
                IOS_SUCCESS
            );
            assert_eq!(
                ios_operation_cancel_destroy_checked(&mut cancel),
                IOS_SUCCESS
            );
            assert!(session.is_null() && cancel.is_null());
            assert_eq!(class(), "none");
        }
    }
}

#[test]
fn invalid_fields_limits_lengths_and_endpoint_are_refused_before_source() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let route = format!("tcp://{}", listener.local_addr().unwrap());
    unsafe {
        let mut handle = session(&route);
        let mut cancel = std::ptr::null_mut();
        assert_eq!(ios_operation_cancel_create(&mut cancel), IOS_SUCCESS);
        let options = IosDeviceQueryOptions::default();
        for fields in [0, 8, u32::MAX] {
            let mut output = c"old-output".as_ptr().cast_mut();
            assert_eq!(
                ios_device_session_query_json(handle, fields, &options, cancel, &mut output),
                IOS_ERR_CORE
            );
            assert!(output.is_null());
            assert_eq!(class(), "invalid_input");
        }
        let mut invalid = options;
        invalid.total_ms = u64::MAX;
        let mut output = std::ptr::null_mut();
        assert_eq!(
            ios_device_session_query_json(handle, 7, &invalid, cancel, &mut output),
            IOS_ERR_CORE
        );
        assert_eq!(class(), "invalid_input");
        assert_eq!(ios_device_session_close(handle, u64::MAX), IOS_ERR_CORE);
        assert!(!(&(*handle).state).device.is_closed());
        assert!(selector(7).is_ok()); // Actual finite wire keys are checked by linked C.
        for text in [
            "tcp://localhost:1",
            "tcp://127.0.0.1:0",
            "tcp://127.0.0.1",
            "other://127.0.0.1:1",
        ] {
            assert!(endpoint(text).is_err());
            assert_eq!(class(), "invalid_input");
        }
        let oversized = vec![b'x'; MAX_INPUT_TEXT + 1];
        assert!(input_text(oversized.as_ptr().cast(), oversized.len()).is_err());
        assert_eq!(class(), "resource_limit");
        assert!(input_text([255u8].as_ptr().cast(), 1).is_err());
        assert!(input_text(b"a\0b".as_ptr().cast(), 3).is_err());
        assert_eq!(ios_device_session_destroy_checked(&mut handle), IOS_SUCCESS);
        assert_eq!(
            ios_operation_cancel_destroy_checked(&mut cancel),
            IOS_SUCCESS
        );
    }
    assert!(listener
        .accept()
        .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn create_checks_pair_byte_and_local_bounds_before_runtime_or_source() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let route = format!("tcp://{}", listener.local_addr().unwrap());
    let udid = "owned-ffi-session-device";
    let bytes = pair();
    let mut config = IosDeviceSessionConfig {
        udid: udid.as_ptr().cast(),
        udid_len: udid.len(),
        endpoint: route.as_ptr().cast(),
        endpoint_len: route.len(),
        pair_record: bytes.as_ptr(),
        pair_record_len: bytes.len(),
        concurrent_operations: 1,
    };
    unsafe {
        let mut handle = std::ptr::null_mut();
        for bound in [0, 5, u32::MAX] {
            config.concurrent_operations = bound;
            assert_eq!(
                ios_device_session_create(&config, &mut handle),
                IOS_ERR_CORE
            );
            assert!(handle.is_null());
            assert_eq!(class(), "invalid_input");
        }
        config.concurrent_operations = 4;
        config.pair_record_len = 0;
        assert_eq!(
            ios_device_session_create(&config, &mut handle),
            IOS_ERR_CORE
        );
        assert_eq!(class(), "invalid_input");
        let oversized = vec![0; MAX_PAIR_BYTES + 1];
        config.pair_record = oversized.as_ptr();
        config.pair_record_len = oversized.len();
        assert_eq!(
            ios_device_session_create(&config, &mut handle),
            IOS_ERR_CORE
        );
        assert!(handle.is_null());
        assert_eq!(class(), "resource_limit");
        let malformed = b"not a serialized pairing plist";
        config.pair_record = malformed.as_ptr();
        config.pair_record_len = malformed.len();
        assert_eq!(
            ios_device_session_create(&config, &mut handle),
            IOS_ERR_CORE
        );
        assert!(handle.is_null());
        assert_eq!(class(), "invalid_input"); // Retained native from_bytes cause.
        config.pair_record = bytes.as_ptr();
        config.pair_record_len = bytes.len();
        let owned_runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _entered = owned_runtime.enter();
        // Valid create is also runtime-independent, and accepts the upper bound.
        assert_eq!(ios_device_session_create(&config, &mut handle), IOS_SUCCESS);
        assert_eq!(ios_device_session_destroy_checked(&mut handle), IOS_SUCCESS);
    }
    assert!(listener
        .accept()
        .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn retain_one_alias_does_not_close_and_last_alias_cancels_original_prepared_job() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let route = format!("tcp://{}", listener.local_addr().unwrap());
    unsafe {
        let mut handle = session(&route);
        let mut alias = std::ptr::null_mut();
        assert_eq!(ios_device_session_retain(handle, &mut alias), IOS_SUCCESS);
        assert_ne!(alias, handle);
        assert!(Arc::ptr_eq(&(*handle).state, &(*alias).state));
        assert_eq!(ios_device_session_destroy_checked(&mut alias), IOS_SUCCESS);
        assert!(!(&(*handle).state).device.is_closed());
        let prepared = (&(*handle).state)
            .device
            .prepare_query(
                DeviceQuery::product_type(),
                DeviceQueryLimits::new(1024, 1024).unwrap(),
                OperationOptions::new(Duration::from_secs(1)),
                OperationCancel::new(),
            )
            .unwrap();
        assert_eq!((&(*handle).state).device.active(), 1);
        assert_eq!(ios_device_session_destroy_checked(&mut handle), IOS_SUCCESS);
        let result = runtime().unwrap().block_on(prepared);
        assert_eq!(
            ios_core::classify_error(&result.unwrap_err()),
            ErrorClass::Cancelled
        );
    }
    assert!(listener
        .accept()
        .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock));
}

#[test]
fn close_timeout_retains_closed_state_then_real_lease_drop_allows_repeated_close() {
    unsafe {
        let mut handle = session("tcp://127.0.0.1:1");
        let prepared = (&(*handle).state)
            .device
            .prepare_query(
                DeviceQuery::device_name(),
                DeviceQueryLimits::new(1024, 1024).unwrap(),
                OperationOptions::new(Duration::from_secs(1)),
                OperationCancel::new(),
            )
            .unwrap();
        assert_eq!(ios_device_session_close(handle, 0), IOS_ERR_CORE);
        assert_eq!(class(), "timeout");
        assert!((&(*handle).state).device.is_closed());
        assert_eq!((&(*handle).state).device.active(), 1);
        drop(prepared); // Actual lease release, not a fake completion counter.
        assert_eq!((&(*handle).state).device.active(), 0);
        for _ in 0..2 {
            assert_eq!(ios_device_session_close(handle, 0), IOS_SUCCESS);
        }
        assert_eq!(ios_device_session_destroy_checked(&mut handle), IOS_SUCCESS);
    }
}

#[test]
fn independently_owned_cancel_can_be_signalled_by_original_os_thread() {
    unsafe {
        let mut cancel = std::ptr::null_mut();
        let mut alias = std::ptr::null_mut();
        assert_eq!(ios_operation_cancel_create(&mut cancel), IOS_SUCCESS);
        assert_eq!(ios_operation_cancel_retain(cancel, &mut alias), IOS_SUCCESS);
        let snapshot = (*cancel).cancel.clone();
        assert_eq!(
            ios_operation_cancel_destroy_checked(&mut cancel),
            IOS_SUCCESS
        );
        assert!(!snapshot.is_cancelled());
        // Transfer the exclusively owned Box into its original worker thread.
        let owned = Box::from_raw(alias);
        let worker = std::thread::spawn(move || {
            let mut pointer = Box::into_raw(owned);
            let status = ios_operation_cancel(pointer);
            let category = class();
            let released = ios_operation_cancel_destroy_checked(&mut pointer);
            (status, category, released, pointer.is_null())
        });
        let result = worker.join().unwrap();
        assert!(snapshot.is_cancelled());
        assert_eq!(result, (IOS_SUCCESS, "none", IOS_SUCCESS, true));
    }
}

#[test]
fn entered_runtime_refusal_has_no_admitted_lease_and_close_remains_sealed() {
    unsafe {
        let mut handle = session("tcp://127.0.0.1:1");
        let mut cancel = std::ptr::null_mut();
        assert_eq!(ios_operation_cancel_create(&mut cancel), IOS_SUCCESS);
        let owned_runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        {
            let _entered = owned_runtime.enter();
            let mut text = std::ptr::null_mut();
            assert_eq!(
                ios_device_session_query_json(
                    handle,
                    7,
                    &IosDeviceQueryOptions::default(),
                    cancel,
                    &mut text
                ),
                IOS_ERR_RUNTIME
            );
            assert!(text.is_null());
            assert_eq!(class(), "precondition");
            assert_eq!((&(*handle).state).device.active(), 0);
            assert_eq!(ios_device_session_close(handle, 100), IOS_ERR_RUNTIME);
            assert!((&(*handle).state).device.is_closed());
        }
        assert_eq!(ios_device_session_close(handle, 100), IOS_SUCCESS);
        assert_eq!(ios_device_session_destroy_checked(&mut handle), IOS_SUCCESS);
        assert_eq!(
            ios_operation_cancel_destroy_checked(&mut cancel),
            IOS_SUCCESS
        );
    }
}

//! Representative external consumer: no private modules or device I/O.

use std::collections::HashMap;
use std::future::Future;

use ios_core::{
    ConnectOptions, CoreError, DeviceEvent, DeviceInfo, RsdHandshake, ServiceDescriptor, TunMode,
    TunnelInfo, XpcMessage, XpcValue,
};

#[test]
fn root_xpc_codec_error_retains_native_source_and_exhaustive_family() {
    use ios_core::{
        decode_xpc_message as decode_message, encode_xpc_message as encode_message, XpcError,
    };
    use std::error::Error;
    fn family(error: &XpcError) -> &'static str {
        match error {
            XpcError::Io(_) => "io",
            XpcError::NotConnected => "not-connected",
            XpcError::ServiceNotFound(_) => "service-not-found",
            XpcError::Tls(_) => "protocol",
        }
    }
    let message = XpcMessage {
        flags: 1,
        msg_id: 42,
        body: Some(XpcValue::String("owned-private".into())),
    };
    let valid = encode_message(&message).unwrap();
    assert_eq!(decode_message(valid.clone()).unwrap().body, message.body);
    let mut malformed = valid.to_vec();
    let position = malformed
        .windows(b"owned-private".len())
        .position(|s| s == b"owned-private")
        .unwrap();
    malformed[position] = 0xff;
    let error = decode_message(bytes::Bytes::from(malformed)).unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Protocol
    );
    let XpcError::Io(outer) = &error else {
        unreachable!()
    };
    assert!(outer.source().unwrap().is::<std::str::Utf8Error>());
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[test]
fn root_backup_required_file_error_keeps_native_io_and_public_family() {
    use ios_core::{
        backup2::{initialize_backup_directory, load_backup_applications, Mobilebackup2Error},
        classify_error, ErrorClass,
    };
    use std::error::Error;
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-backup-host-{}", uuid::Uuid::new_v4())),
    );
    let info = plist::Dictionary::from_iter([(
        "Applications",
        plist::Value::Dictionary(plist::Dictionary::new()),
    )]);
    let layout = initialize_backup_directory(&root.0, "owned-private-device", &info, true).unwrap();
    assert_eq!(
        load_backup_applications(&root.0, "owned-private-device").unwrap(),
        info.get("Applications").cloned()
    );
    let path = layout.device_directory.join("Status.plist");
    std::fs::remove_file(&path).unwrap();
    let expected = std::fs::symlink_metadata(&path).unwrap_err();
    let error = load_backup_applications(&root.0, "owned-private-device").unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::NotFound);
    let Mobilebackup2Error::Io(outer) = &error else {
        panic!("expected native IO family");
    };
    assert_eq!(outer.kind(), std::io::ErrorKind::NotFound);
    let original = outer
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(original.raw_os_error(), expected.raw_os_error());
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    assert!(layout.device_directory.join("Info.plist").is_file());
    assert!(!path.exists());
}

#[test]
fn root_mux_constructor_keeps_its_typed_error_future() {
    fn signature<E, F>(future: F)
    where
        E: std::error::Error + Send + Sync + 'static,
        F: Future<Output = Result<ios_core::MuxClient, E>>,
    {
        drop(future);
    }
    // External type check only; no daemon endpoint or device is contacted.
    signature(ios_core::MuxClient::connect());
}

#[cfg(feature = "backup2-manifest")]
#[test]
fn root_backup_metadata_error_keeps_exhaustive_family_and_native_sqlite_source() {
    use ios_core::{
        backup2::{list_backup_entries, Mobilebackup2Error},
        classify_error, ErrorClass,
    };
    use std::error::Error;
    fn family(error: &Mobilebackup2Error) -> &'static str {
        match error {
            Mobilebackup2Error::DeviceLink(_) => "device-link",
            Mobilebackup2Error::Io(_) => "io",
            Mobilebackup2Error::Plist(_) => "plist",
            Mobilebackup2Error::Protocol(_) => "protocol",
        }
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<Mobilebackup2Error>();
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-backup-{}", uuid::Uuid::new_v4())),
    );
    let device = root.0.join("owned-device");
    std::fs::create_dir_all(&device).unwrap();
    std::fs::write(device.join("Info.plist"), b"owned-info").unwrap();
    std::fs::write(device.join("Status.plist"), b"owned-status").unwrap();
    plist::to_file_xml(
        device.join("Manifest.plist"),
        &plist::Dictionary::from_iter([
            ("IsEncrypted", plist::Value::Boolean(false)),
            (
                "Lockdown",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ProductVersion",
                    "17.0",
                )])),
            ),
        ]),
    )
    .unwrap();
    std::fs::write(device.join("Manifest.db"), b"owned-private not a database").unwrap();
    let error = list_backup_entries(&root.0, "owned-device", None).unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    let Mobilebackup2Error::Io(source) = &error else {
        unreachable!()
    };
    let original = source
        .source()
        .unwrap()
        .downcast_ref::<rusqlite::Error>()
        .unwrap();
    assert_eq!(
        original.sqlite_error_code(),
        Some(rusqlite::ErrorCode::NotADatabase)
    );
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
}

#[cfg(feature = "backup2-manifest")]
#[test]
fn root_legacy_backup_rejection_keeps_native_utf8_bytes_without_public_variant_changes() {
    use ios_core::{
        backup2::{list_backup_entries, Mobilebackup2Error},
        classify_error, ErrorClass,
    };
    use std::error::Error;
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-mbdb-{}", uuid::Uuid::new_v4())),
    );
    let device = root.0.join("owned-device");
    std::fs::create_dir_all(&device).unwrap();
    std::fs::write(device.join("Info.plist"), b"owned-info").unwrap();
    std::fs::write(device.join("Status.plist"), b"owned-status").unwrap();
    plist::to_file_xml(
        device.join("Manifest.plist"),
        &plist::Dictionary::from_iter([
            ("IsEncrypted", plist::Value::Boolean(false)),
            (
                "Lockdown",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ProductVersion",
                    "10.2",
                )])),
            ),
        ]),
    )
    .unwrap();
    let bytes = [
        b"mbdb\x05\x00".as_slice(),
        b"\x00\x0eowned-private\xff".as_slice(),
    ]
    .concat();
    std::fs::write(device.join("Manifest.mbdb"), bytes).unwrap();
    let error = list_backup_entries(&root.0, "owned-device", None).unwrap_err();
    assert!(matches!(&error, Mobilebackup2Error::Io(_)));
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    assert!(!format!("{error:?}").contains("owned-private"));
    let mut current: Option<&(dyn Error + 'static)> = Some(&error);
    let mut found = false;
    for _ in 0..32 {
        let Some(source) = current else { break };
        if let Some(native) = source.downcast_ref::<std::string::FromUtf8Error>() {
            assert_eq!(native.as_bytes(), b"owned-private\xff");
            found = true;
            break;
        }
        current = source.source();
    }
    assert!(
        found,
        "native UTF8 source must remain reachable from root API"
    );
}

#[tokio::test]
async fn default_lockdown_root_api_retains_sources_and_exhaustive_errors() {
    use ios_core::{
        classify_error, ErrorClass, LockdownClient, LockdownError, PairRecord, PairRecordError,
    };
    use std::{error::Error, io};
    use tokio::io::AsyncWriteExt;
    fn family(error: &LockdownError) -> &'static str {
        match error {
            LockdownError::Io(_) => "io",
            LockdownError::Protocol(_) => "protocol",
            LockdownError::PairRecord(_) => "pair-record",
        }
    }
    fn record_family(error: &PairRecordError) -> &'static str {
        match error {
            PairRecordError::NotFound(_) => "missing",
            PairRecordError::Read { .. } => "read",
            PairRecordError::Parse(_) => "parse",
        }
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    fn signature<F: Future<Output = Result<LockdownClient, LockdownError>>>(future: F) {
        drop(future);
    }
    assert_error::<LockdownError>();
    assert_error::<PairRecordError>();
    let record = PairRecord {
        device_certificate: Vec::new(),
        host_certificate: b"-----BEGIN CERTIFICATE-----\nowned-private!\n-----END CERTIFICATE-----"
            .to_vec(),
        host_private_key: Vec::new().into(),
        root_certificate: Vec::new(),
        host_id: "owned-host".into(),
        system_buid: "owned-buid".into(),
        wifi_mac_address: None,
    };
    let (stream, peer) = tokio::io::duplex(64);
    // Root constructor is type-checked but not polled/session-established.
    signature(LockdownClient::connect_with_stream(stream, &record));
    drop(peer);
    let (stream, peer) = tokio::io::duplex(64);
    let error = ios_core::wrap_service_tls(stream, &record)
        .await
        .unwrap_err();
    drop(peer);
    assert_eq!(family(&error), "io");
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(source) = &error else {
        unreachable!()
    };
    assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
    assert!(source.source().unwrap().is::<io::Error>());
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));

    let (mut stream, mut peer) = tokio::io::duplex(64);
    let payload = b"<plist><dict>";
    peer.write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .unwrap();
    peer.write_all(payload).await.unwrap();
    let error = ios_core::recv_lockdown::<_, plist::Value>(&mut stream)
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    let LockdownError::Io(source) = &error else {
        unreachable!()
    };
    assert!(source.source().unwrap().is::<plist::Error>());
    assert_eq!(
        record_family(&PairRecordError::Parse("owned-private".into())),
        "parse"
    );
}

#[cfg(feature = "supervised-pair")]
#[tokio::test]
async fn supervised_pairing_root_api_keeps_native_sources_and_record_fields() {
    use ios_core::{
        classify_error, pair_supervised, save_pair_record, ErrorClass, FullPairRecord,
        LockdownError,
    };
    use std::{error::Error, io};
    let (mut stream, peer) = tokio::io::duplex(64);
    let error = pair_supervised(
        &mut stream,
        b"owned-private invalid DER",
        "owned-password",
        "owned-buid",
    )
    .await
    .unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(source) = &error else {
        panic!("root native cause lost")
    };
    assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
    assert!(source.source().unwrap().is::<openssl::error::ErrorStack>());
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    let record = FullPairRecord {
        device_certificate: vec![1],
        host_certificate: vec![2],
        host_private_key: vec![3],
        root_certificate: vec![4],
        root_private_key: vec![5],
        host_id: "owned-host".into(),
        system_buid: "owned-buid".into(),
    };
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let owned = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-supervised-root-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir(&owned.0).unwrap();
    let path = owned.0.join("owned-private\0.plist");
    let error = save_pair_record(&record, b"escrow", None, &path).unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(source) = &error else {
        panic!("root native IO lost")
    };
    assert!(source.source().unwrap().is::<io::Error>());
    drop(peer);
}

#[cfg(feature = "mcinstall")]
#[tokio::test]
async fn mcinstall_root_error_family_and_feature_decisions_remain_observable() {
    use ios_core::{
        classify_error,
        mcinstall::{McInstallClient, McInstallError},
        ErrorClass,
    };
    use std::{error::Error, io};
    fn family(error: &McInstallError) -> &'static str {
        match error {
            McInstallError::Io(_) => "io",
            McInstallError::Plist(_) => "plist",
            McInstallError::Protocol(_) => "protocol",
            McInstallError::Crypto(_) => "crypto",
            McInstallError::PasscodeSet => "passcode",
        }
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<McInstallError>();
    let (stream, peer) = tokio::io::duplex(64);
    let mut client = McInstallClient::new(stream);
    let error = client
        .escalate_with_p12(b"owned-private invalid DER", "owned-password")
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    let McInstallError::Io(source) = &error else {
        unreachable!()
    };
    #[cfg(feature = "supervised-pair")]
    {
        assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
        assert!(source.source().unwrap().is::<openssl::error::ErrorStack>());
    }
    #[cfg(not(feature = "supervised-pair"))]
    {
        assert_eq!(source.kind(), io::ErrorKind::Other);
        assert_eq!(classify_error(&error), ErrorClass::Unsupported);
        assert!(source.source().unwrap().is::<CoreError>());
    }
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    assert_eq!(
        family(&McInstallError::Crypto("owned-private".into())),
        "crypto"
    );
    assert_eq!(
        classify_error(&McInstallError::PasscodeSet),
        ErrorClass::Precondition
    );
    drop(peer);
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
#[test]
fn manual_pairing_root_api_preserves_its_exhaustive_error_and_future() {
    use ios_core::{pair_new_device, PairedCredentials, PairingTransportError};
    use std::{error::Error, net::Ipv6Addr};
    fn family(error: &PairingTransportError) -> &'static str {
        match error {
            PairingTransportError::Io(_) => "io",
            PairingTransportError::Xpc(_) => "xpc",
            PairingTransportError::Rsd(_) => "rsd",
            PairingTransportError::Crypto(_) => "crypto",
            PairingTransportError::Failed(_) => "failed",
            PairingTransportError::Rejected(_) => "rejected",
            PairingTransportError::MissingField(_) => "missing",
            PairingTransportError::UnexpectedType(_) => "type",
            PairingTransportError::ServiceNotFound => "service-missing",
        }
    }
    fn signature<F: Future<Output = Result<PairedCredentials, PairingTransportError>>>(future: F) {
        drop(future);
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<PairingTransportError>();
    // The real root API future is type-checked but never polled/device-connected.
    signature(pair_new_device(Ipv6Addr::LOCALHOST));
    assert_eq!(
        family(&PairingTransportError::ServiceNotFound),
        "service-missing"
    );
}

#[cfg(feature = "mobileactivation")]
#[test]
fn deactivation_fallback_observation_is_public_borrowed_and_thread_safe() {
    use ios_core::{DeactivationFallbackError, ErrorClass};
    fn assert_error<T: std::error::Error + Send + Sync + 'static>() {}
    assert_error::<DeactivationFallbackError>();
    let _: fn(&DeactivationFallbackError) -> &CoreError = DeactivationFallbackError::service_error;
    let _: fn(&DeactivationFallbackError) -> &CoreError = DeactivationFallbackError::fallback_error;
    let _: fn(&DeactivationFallbackError) -> ErrorClass = DeactivationFallbackError::service_class;
    let _: fn(&DeactivationFallbackError) -> ErrorClass = DeactivationFallbackError::fallback_class;
}

// These enums are currently exhaustive. Adding a variant/non_exhaustive is a
// downstream source change, even if every in-workspace caller still compiles.
fn error_family(error: &CoreError) -> &'static str {
    match error {
        CoreError::Io(_) => "io",
        CoreError::Mux(_) => "mux",
        CoreError::Lockdown(_) => "lockdown",
        CoreError::PairRecord(_) => "pair-record",
        CoreError::Tunnel(_) => "tunnel",
        CoreError::Xpc(_) => "xpc",
        CoreError::Json(_) => "json",
        CoreError::Protocol(_) => "protocol",
        CoreError::DeviceNotFound(_) => "device-not-found",
        CoreError::Unsupported(_) => "unsupported",
        CoreError::Other(_) => "other",
    }
}

#[cfg(feature = "apps")]
#[tokio::test]
async fn checked_install_observer_is_additive_and_keeps_error_enum_exhaustive() {
    use ios_core::apps::{
        install_ipa, install_ipa_with_checked_progress, CheckedProgressCallback, ZipConduitError,
    };
    fn family(error: &ZipConduitError) -> &'static str {
        match error {
            ZipConduitError::Io(_) => "io",
            ZipConduitError::Plist(_) => "plist",
            ZipConduitError::Protocol(_) => "protocol",
            ZipConduitError::Zip(_) => "zip",
            ZipConduitError::Install(_) => "install",
        }
    }
    let path = std::env::temp_dir().join(format!("ios-uncreated-{}.ipa", uuid::Uuid::new_v4()));
    let (mut stream, _peer) = tokio::io::duplex(64);
    let callback: ios_core::apps::zipconduit::ProgressCallback = Box::new(|_, _| {});
    let old = install_ipa(&mut stream, &path, Some(callback));
    drop(old);
    let callback: CheckedProgressCallback =
        Box::new(|_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
    let error = install_ipa_with_checked_progress(&mut stream, &path, Some(callback))
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    let ZipConduitError::Io(native) = error else {
        panic!("local source lost");
    };
    assert_eq!(native.kind(), std::io::ErrorKind::NotFound);
}

#[cfg(feature = "webinspector")]
#[test]
fn webinspector_facade_models_clients_and_sessions_remain_public() {
    use ios_core::webinspector::{
        Application, ApplicationPage, AutomationAvailability, AutomationSession, By,
        InspectorSession, Page, WebInspectorClient, WebInspectorEvent, WirType,
    };
    let application = Application {
        id: "PID:42".into(),
        bundle_identifier: "com.example.synthetic".into(),
        pid: 42,
        name: "Synthetic".into(),
        availability: AutomationAvailability::Available,
        is_active: true,
        is_proxy: false,
        is_ready: true,
        host_application_identifier: None,
    };
    let page = Page {
        id: 1,
        listing_key: "page-1".into(),
        page_type: WirType::WebPage,
        title: Some("Synthetic page".into()),
        url: None,
        automation_is_paired: None,
        automation_name: None,
        automation_version: None,
        automation_session_id: None,
        automation_connection_id: None,
    };
    let row = ApplicationPage { application, page };
    assert_eq!(
        serde_json::to_value(&row).unwrap()["page"]["page_type"],
        "web_page"
    );
    let _: ios_core::services::webinspector::WebInspectorEvent =
        WebInspectorEvent::ConnectedDrivers;
    let _: ios_core::services::webinspector::By = By::CssSelector;
    let (stream, _peer) = tokio::io::duplex(64);
    let mut client = WebInspectorClient::with_connection_id(stream, "synthetic-connection");
    let mut inspector = InspectorSession::with_session_id("PID:42", 1, "synthetic-session");
    let mut automation = AutomationSession::with_session_id(
        "PID:42",
        "com.example.synthetic",
        "synthetic-automation",
    );
    automation.set_implicit_wait_timeout(std::time::Duration::ZERO);
    assert_eq!(client.connection_id(), "synthetic-connection");
    assert_eq!(inspector.session_id(), "synthetic-session");
    assert_eq!(automation.session_id(), "synthetic-automation");
    // Check old method paths without polling any service I/O.
    drop(client.start(std::time::Duration::ZERO));
    drop(client.start_preserving_errors(std::time::Duration::ZERO));
    drop(inspector.attach_unpaused(&mut client, true, std::time::Duration::ZERO));
    drop(automation.attach(&mut client, std::time::Duration::ZERO));
    drop(automation.find_element(&mut client, By::CssSelector, ".synthetic"));
    let _browser = ios_core::webinspector::cdp_browser::BrowserCdpState::new("synthetic-browser");
}

#[cfg(feature = "webinspector")]
#[tokio::test]
async fn public_webinspector_start_preserves_typed_io_without_changing_legacy_api() {
    use ios_core::webinspector::{WebInspectorClient, WebInspectorError};
    // Existing callers may exhaustively match this enum; the additive API
    // does not require a new variant or a fallback in their old match.
    fn family(error: &WebInspectorError) -> &'static str {
        match error {
            WebInspectorError::Io(_) => "io",
            WebInspectorError::Plist(_) => "plist",
            WebInspectorError::Json(_) => "json",
            WebInspectorError::Disabled => "disabled",
            WebInspectorError::Protocol(_) => "protocol",
            WebInspectorError::Timeout(_) => "timeout",
        }
    }
    for preserve in [false, true] {
        let (stream, peer) = tokio::io::duplex(64);
        drop(peer);
        let mut client = WebInspectorClient::with_connection_id(stream, "synthetic");
        let duration = std::time::Duration::from_millis(20);
        let error = if preserve {
            client.start_preserving_errors(duration).await
        } else {
            client.start(duration).await
        }
        .unwrap_err();
        assert_eq!(family(&error), if preserve { "io" } else { "disabled" });
        assert_eq!(
            ios_core::classify_error(&error),
            if preserve {
                ios_core::ErrorClass::Disconnected
            } else {
                ios_core::ErrorClass::Precondition
            }
        );
    }
}

fn event_id(event: DeviceEvent) -> u32 {
    match event {
        DeviceEvent::Attached(DeviceInfo { device_id, .. })
        | DeviceEvent::Detached { device_id, .. } => device_id,
    }
}

#[cfg(feature = "tunnel")]
#[tokio::test]
async fn public_xpc_client_retains_native_h2_failure_in_existing_error_enum() {
    use ios_core::{classify_error, ErrorClass, XpcClient, XpcError};
    use std::error::Error;
    use std::io::ErrorKind;

    let (stream, peer) = tokio::io::duplex(64);
    drop(peer);
    let error = match XpcClient::connect_stream(stream).await {
        Err(error) => error,
        Ok(_) => panic!("closed transport connected"),
    };
    // The existing exhaustive error enum is unchanged. The native IO source
    // crosses the public root API instead of becoming a TLS/protocol string.
    match &error {
        XpcError::Io(outer) => assert_eq!(outer.kind(), ErrorKind::BrokenPipe),
        XpcError::NotConnected | XpcError::ServiceNotFound(_) | XpcError::Tls(_) => {
            panic!("native H2 failure flattened")
        }
    }
    assert_eq!(classify_error(&error), ErrorClass::Disconnected);
    let mut current: Option<&(dyn Error + 'static)> = error.source();
    let mut io_sources = 0;
    for _ in 0..32 {
        let Some(source) = current else { break };
        if let Some(io) = source.downcast_ref::<std::io::Error>() {
            assert_eq!(io.kind(), ErrorKind::BrokenPipe);
            io_sources += 1;
        }
        current = source.source();
    }
    assert_eq!(io_sources, 2, "outer adapter and original transport IO");
}

#[test]
fn public_error_classes_preserve_existing_exhaustive_core_matches() {
    use ios_core::{classify_error, ErrorClass};
    let error = CoreError::Io(std::io::Error::from(std::io::ErrorKind::TimedOut));
    assert_eq!(error_family(&error), "io");
    assert_eq!(error.class(), ErrorClass::Timeout);
    assert_eq!(classify_error(&error).as_str(), "timeout");
    let code = match error.class() {
        ErrorClass::Timeout => "wait budget",
        _ => "other or future category",
    };
    assert_eq!(code, "wait budget");
    let error = CoreError::Xpc(ios_core::XpcError::ServiceNotFound(
        "synthetic service".into(),
    ));
    assert_eq!(error_family(&error), "xpc");
    assert_eq!(classify_error(&error), ErrorClass::ServiceMissing);
    assert_eq!(
        serde_json::to_value(error.class()).unwrap(),
        "service_missing"
    );
}

#[test]
fn public_cancellation_marker_is_additive_and_distinct_from_plain_interrupted_io() {
    let marker: ios_core::error::OperationCancelled = ios_core::OperationCancelled::new();
    assert_eq!(ios_core::classify_error(&marker).as_str(), "cancelled");
    assert!(std::error::Error::source(&marker).is_none());
    assert_eq!(marker.to_string(), "operation cancelled");
    assert_eq!(format!("{marker:?}"), "OperationCancelled");
    let io = CoreError::Io(std::io::Error::from(std::io::ErrorKind::Interrupted));
    assert_eq!(error_family(&io), "io");
    assert_eq!(io.class().as_str(), "io");
}

#[test]
fn public_rsd_evidence_keeps_unknown_metadata_distinct_from_missing_service() {
    use ios_core::RsdFeatureEvidence as Evidence;
    // The new observation is borrowed and the legacy directory getter remains.
    let _read: fn(
        &ios_core::ConnectedDevice,
    ) -> (&ios_core::InitialRsdProbe, Option<&RsdHandshake>) =
        |device| (device.initial_rsd_probe(), device.rsd());
    let state: ios_core::device::InitialRsdProbeState =
        ios_core::InitialRsdProbeState::NotAttempted;
    assert_eq!(serde_json::to_value(state).unwrap(), "not_attempted");
    let _future_state = match state {
        ios_core::InitialRsdProbeState::NotAttempted => "no initial request",
        ios_core::InitialRsdProbeState::Available => "initial directory",
        ios_core::InitialRsdProbeState::Failed => "initial failure",
        _ => "future observation",
    };
    let legacy = ServiceDescriptor::new(1234);
    assert!(legacy.supports_feature("requested"));
    assert_eq!(
        legacy.feature_evidence("requested"),
        Evidence::MetadataUnavailable
    );
    let directory = RsdHandshake {
        udid: "synthetic-device".into(),
        services: HashMap::from([
            ("legacy".into(), legacy),
            (
                "modern".into(),
                ServiceDescriptor {
                    port: 2345,
                    features: vec!["requested".into()],
                },
            ),
            ("modern.shim.remote".into(), ServiceDescriptor::new(3456)),
            (
                "shim-only.shim.remote".into(),
                ServiceDescriptor {
                    port: 4567,
                    features: vec!["requested".into()],
                },
            ),
        ]),
    };
    assert_eq!(
        directory.feature_evidence("missing", "requested"),
        Evidence::ServiceMissing
    );
    assert_eq!(
        directory.feature_evidence("legacy", "requested"),
        Evidence::MetadataUnavailable
    );
    assert_eq!(
        directory.supports_feature("legacy", "requested"),
        Some(true)
    );
    assert_eq!(
        directory.feature_evidence("modern", "requested"),
        Evidence::Advertised
    );
    assert_eq!(
        directory.feature_evidence("modern", "other"),
        Evidence::NotAdvertised
    );
    assert_eq!(
        directory.resolved_feature_evidence("modern", "requested"),
        Evidence::Advertised
    );
    assert_eq!(
        directory.feature_evidence("shim-only", "requested"),
        Evidence::ServiceMissing
    );
    assert_eq!(
        directory.resolved_feature_evidence("shim-only", "requested"),
        Evidence::Advertised
    );
    assert_eq!(
        serde_json::to_value(Evidence::MetadataUnavailable).unwrap(),
        "metadata_unavailable"
    );
}

#[test]
fn public_plist_conversion_policies_select_historical_data_formats() {
    use ios_core::plist_json::{into_json, to_json, DataEncoding, PlistJsonPolicy};
    let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("data".to_string(), plist::Value::Data(vec![0, 127, 255])),
        (
            "uid".to_string(),
            plist::Value::Uid(plist::Uid::new(u64::MAX)),
        ),
    ]));
    let hex = PlistJsonPolicy::new(DataEncoding::HexString);
    assert_eq!(
        to_json(&value, hex),
        serde_json::json!({"data":"007fff","uid":u64::MAX})
    );
    let numeric = PlistJsonPolicy::new(DataEncoding::NumericArray);
    assert_eq!(
        to_json(&value, numeric),
        serde_json::json!({"data":[0,127,255],"uid":u64::MAX})
    );
    let legacy = hex.without_date_and_uid().with_zero_integer_fallback();
    assert_eq!(
        into_json(value, legacy),
        serde_json::json!({"data":"007fff","uid":null})
    );
}

fn mode_name(mode: TunMode) -> &'static str {
    match mode {
        TunMode::Kernel => "kernel",
        TunMode::Userspace => "userspace",
    }
}

#[test]
fn backup_facade_types_functions_and_client_methods_remain_public() {
    use ios_core::backup2::{
        BackupDirectoryLayout, BackupOptions, BackupResult, Mobilebackup2Client, RestoreOptions,
        RestoreResult, VersionExchange,
    };
    let layout = BackupDirectoryLayout {
        root: "synthetic-backup".into(),
        device_directory: "synthetic-backup/source".into(),
        target_identifier: "source".into(),
    };
    let version = VersionExchange {
        device_link_version: 300,
        protocol_version: 2.1,
        local_versions: vec![2.0, 2.1],
    };
    let _: BackupResult = BackupResult {
        layout: layout.clone(),
        device_link_version: version.device_link_version,
        protocol_version: version.protocol_version,
    };
    let _: RestoreResult = RestoreResult {
        layout: layout.clone(),
        device_link_version: 300,
        protocol_version: 2.1,
    };
    let restore = RestoreOptions {
        system: false,
        reboot: true,
        copy: false,
        settings: true,
        remove: false,
        password: None,
        source_identifier: None,
    };
    let options = BackupOptions {
        filter: None,
        patch_manifest: false,
        password: None,
    };
    assert_eq!(ios_core::backup2::BackupSelection::NAMES.len(), 6);
    let _initialize = ios_core::backup2::initialize_backup_directory;
    let _encrypted = ios_core::services::backup2::backup_is_encrypted;
    let (stream, _peer) = tokio::io::duplex(64);
    let mut client = Mobilebackup2Client::new(stream);
    let info = plist::Dictionary::new();
    // Type-check method paths without polling or touching a device/filesystem.
    drop(client.version_exchange());
    drop(client.backup_with_options(&layout.root, "source", false, &info, options));
    drop(client.restore(&layout.root, "source", restore));
    drop(client.info(&layout.root, "source", None));
    drop(client.list(&layout.root, "source", None));
    #[cfg(feature = "backup2-manifest")]
    {
        let _list = ios_core::backup2::list_backup_entries;
        let _patch = ios_core::backup2::patch_backup_directory;
        let _unback = ios_core::backup2::unback_backup;
        let _extract = ios_core::backup2::extract_backup_file;
    }
}

#[test]
fn published_struct_literals_and_root_paths_remain_usable() {
    let options = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    assert_eq!(mode_name(options.tun_mode), "userspace");
    let device = DeviceInfo {
        udid: "synthetic-downstream-device".into(),
        device_id: 42,
        connection_type: "USB".into(),
        product_id: 1234,
    };
    assert_eq!(event_id(DeviceEvent::Attached(device.clone())), 42);
    assert_eq!(
        event_id(DeviceEvent::Detached {
            udid: device.udid,
            device_id: 42
        }),
        42
    );
    let info = TunnelInfo {
        server_address: "::1".into(),
        server_rsd_port: 58783,
        client_address: "::1".into(),
        client_mtu: 1280,
    };
    let (handle, receiver) = ios_core::TunnelHandle::new("synthetic".into(), info, None);
    assert!(handle.is_alive());
    drop(handle);
    assert!(receiver.has_changed().is_err());
    assert_eq!(
        error_family(&CoreError::Unsupported("synthetic".into())),
        "unsupported"
    );
}

#[test]
fn public_service_metadata_preserves_absent_and_unadvertised_distinction() {
    let metadata = ServiceDescriptor {
        port: 1234,
        features: Vec::new(),
    };
    assert_eq!(ServiceDescriptor::new(1234), metadata);
    let rsd = RsdHandshake {
        udid: "synthetic-rsd".into(),
        services: HashMap::from([("synthetic.service".into(), metadata)]),
    };
    assert_eq!(
        rsd.supports_feature("synthetic.service", "synthetic.feature"),
        Some(true)
    );
    assert_eq!(rsd.supports_feature("missing", "synthetic.feature"), None);
    assert_eq!(
        rsd.get_service_features("synthetic.service"),
        Some([].as_slice())
    );
}

#[test]
fn public_async_signatures_compile_without_polling_device_operations() {
    fn returns<T>(_: impl Future<Output = Result<T, CoreError>>) {}
    returns::<Vec<DeviceInfo>>(ios_core::list_devices());
    returns::<ios_core::ConnectedDevice>(ios_core::connect("synthetic", ConnectOptions::default()));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect(
        "synthetic",
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::connect_direct_usb_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect_direct_usb_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::connect_tcp_lockdown_tunnel(
        "synthetic",
        "127.0.0.1",
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect_tcp_lockdown_tunnel(
        "synthetic",
        "127.0.0.1",
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::connect_remote_pairing_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect_remote_pairing_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    #[cfg(feature = "mdns")]
    {
        returns::<Vec<ios_core::PairedMobdev2Device>>(ios_core::discover_paired_mobdev2_devices());
        returns::<Vec<ios_core::PairedMobdev2Device>>(
            ios_core::device::discover_paired_mobdev2_devices(),
        );
    }
    // Futures are dropped before their first poll; this does not discover a
    // device, establish a connection or certify cancellation after any I/O.
    drop(ios_core::watch_devices());
}

#[test]
fn public_xpc_message_shape_and_codec_roundtrip_remain_compatible() {
    let message = XpcMessage {
        flags: ios_core::xpc_message_flags::ALWAYS_SET,
        msg_id: 42,
        body: Some(XpcValue::String("synthetic".into())),
    };
    let encoded = ios_core::encode_xpc_message(&message).unwrap();
    let decoded = ios_core::decode_xpc_message(encoded).unwrap();
    assert_eq!(decoded.flags, message.flags);
    assert_eq!(decoded.msg_id, 42);
    assert_eq!(decoded.body.unwrap().as_str(), Some("synthetic"));
}

#[cfg(all(feature = "icon_service", feature = "screen_capture"))]
#[test]
fn service_feature_aliases_retain_identical_public_types() {
    fn icon_identity(
        value: ios_core::icon_service::IconServiceClient,
    ) -> ios_core::iconservice::IconServiceClient {
        value
    }
    fn screen_identity(
        value: ios_core::screen_capture::ScreenCaptureServiceClient,
    ) -> ios_core::screencapture::ScreenCaptureServiceClient {
        value
    }
    let _ = icon_identity;
    let _ = screen_identity;
}

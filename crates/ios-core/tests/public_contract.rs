//! Representative external consumer: no private modules or device I/O.

use std::collections::HashMap;
use std::future::Future;

use ios_core::{
    ConnectOptions, CoreError, DeviceEvent, DeviceInfo, RsdHandshake, ServiceDescriptor, TunMode,
    TunnelInfo, XpcMessage, XpcValue,
};

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

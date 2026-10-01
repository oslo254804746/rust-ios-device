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

fn event_id(event: DeviceEvent) -> u32 {
    match event {
        DeviceEvent::Attached(DeviceInfo { device_id, .. })
        | DeviceEvent::Detached { device_id, .. } => device_id,
    }
}

fn mode_name(mode: TunMode) -> &'static str {
    match mode {
        TunMode::Kernel => "kernel",
        TunMode::Userspace => "userspace",
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

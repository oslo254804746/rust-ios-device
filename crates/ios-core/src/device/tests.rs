use std::path::PathBuf;

#[test]
fn retained_metadata_charge_includes_spare_tcp_and_native_identity_capacities() {
    let mut device = super::ConnectedDevice {
        info: crate::discovery::DeviceInfo {
            udid: "metadata-capacity-fixture".into(),
            device_id: 7,
            connection_type: "Network".into(),
            product_id: 0,
        },
        tunnel: None,
        rsd: None,
        #[cfg(feature = "tunnel")]
        owned_initial_rsd: None,
        #[cfg(feature = "tunnel")]
        owned_initial_bank: None,
        #[cfg(feature = "tunnel")]
        prepared_rsd: None,
        #[cfg(feature = "tunnel")]
        owned_prepared_rsd: None,
        initial_rsd_probe: super::InitialRsdProbe::skipped(
            super::InitialRsdProbeSkipReason::TunnelSkipped,
        ),
        pair_record: None,
        lockdown_transport: super::LockdownTransport::Usbmux { device_id: 7 },
    };
    let initial = device.retained_metadata_bytes().unwrap();
    let mut host = String::with_capacity(64 * 1024);
    host.push_str("192.0.2.70");
    let host_charge = host.capacity();
    device.lockdown_transport = super::LockdownTransport::Tcp { host };
    assert_eq!(
        device.retained_metadata_bytes(),
        Some(initial + host_charge)
    );

    let mut udid = String::with_capacity(4096);
    udid.push_str("metadata-capacity-fixture");
    let mut server_address = String::with_capacity(8192);
    server_address.push_str("fd00::1");
    let mut client_address = String::with_capacity(16384);
    client_address.push_str("fd00::2");
    let native_charge = std::mem::size_of::<crate::TunnelHandle>()
        + udid.capacity()
        + server_address.capacity()
        + client_address.capacity();
    let (handle, _receiver) = crate::TunnelHandle::new(
        udid,
        crate::TunnelInfo {
            server_address,
            server_rsd_port: 58783,
            client_address,
            client_mtu: 1280,
        },
        None,
    );
    device.tunnel = Some(std::sync::Arc::new(handle));
    assert_eq!(
        device.retained_metadata_bytes(),
        Some(initial + host_charge + native_charge)
    );
}

#[test]
fn device_connection_debug_hides_credential_paths_and_network_identity() {
    let secret = "synthetic-connection-private-identity";
    let options = super::ConnectOptions {
        pair_record_path: Some(PathBuf::from(secret)),
        ..Default::default()
    };
    let paired = super::PairedMobdev2Device {
        udid: secret.into(),
        host: secret.into(),
    };
    let transport = super::LockdownTransport::Tcp {
        host: secret.into(),
    };
    let output =
        format!("{options:?} {options:#?} {paired:?} {paired:#?} {transport:?} {transport:#?}");
    assert!(!output.contains(secret));
    assert!(output.contains("pair_record_configured"));
    assert_eq!(paired.udid, secret);
    assert_eq!(options.pair_record_path, Some(PathBuf::from(secret)));
}

#[tokio::test]
async fn connect_service_with_timeout_zero_budget_rejects_without_io() {
    let device = super::ConnectedDevice {
        info: crate::discovery::DeviceInfo {
            udid: "zero-budget-device".into(),
            device_id: 1,
            connection_type: "USB".into(),
            product_id: 0,
        },
        tunnel: None,
        rsd: None,
        #[cfg(feature = "tunnel")]
        owned_initial_rsd: None,
        #[cfg(feature = "tunnel")]
        owned_initial_bank: None,
        #[cfg(feature = "tunnel")]
        prepared_rsd: None,
        #[cfg(feature = "tunnel")]
        owned_prepared_rsd: None,
        initial_rsd_probe: super::InitialRsdProbe::skipped(
            super::InitialRsdProbeSkipReason::TunnelSkipped,
        ),
        pair_record: None,
        lockdown_transport: super::LockdownTransport::Usbmux { device_id: 1 },
    };

    let result = device
        .connect_service_with_timeout("com.apple.test.service", std::time::Duration::ZERO)
        .await;
    match result {
        Err(crate::CoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
        other => panic!("expected TimedOut io error, got: {:?}", other.is_err()),
    }
}

use super::*;

/// Construct a device for an explicit loopback-only workflow fixture.  The
/// directory is obtained through the real RSD handshake; this handle does not
/// own a native tunnel and must not be used as evidence of native retirement.
#[doc(hidden)]
pub async fn workflow_loopback_fixture(
    udid: &str,
    rsd_port: u16,
) -> Result<ConnectedDevice, crate::CoreError> {
    if udid.is_empty() || rsd_port == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "fixture requires a target and loopback RSD port",
        )
        .into());
    }
    let directory = crate::xpc::rsd::handshake(std::net::Ipv6Addr::LOCALHOST, rsd_port).await?;
    if directory.udid != udid {
        return Err(crate::CoreError::Protocol(
            "fixture RSD identity mismatch".into(),
        ));
    }
    let (rsd, initial_rsd_probe) =
        rsd_observation::RsdProbeOutcome::Completed(Ok(directory)).into_parts();
    let (handle, _receiver) = TunnelHandle::new(
        udid.to_owned(),
        crate::TunnelInfo {
            server_address: "::1".into(),
            server_rsd_port: rsd_port,
            client_address: "::1".into(),
            client_mtu: 1280,
        },
        None,
    );
    Ok(ConnectedDevice {
        info: DeviceInfo {
            udid: udid.to_owned(),
            device_id: 0,
            connection_type: "Loopback fixture".into(),
            product_id: 0,
        },
        tunnel: Some(Arc::new(handle)),
        rsd,
        owned_initial_rsd: None,
        owned_initial_bank: None,
        owned_prepared_rsd: None,
        prepared_rsd: None,
        initial_rsd_probe,
        pair_record: None,
        lockdown_transport: LockdownTransport::Tcp { host: "::1".into() },
    })
}

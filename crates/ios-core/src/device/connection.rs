//! Public connection entry points and strategy selection.

use super::lockdown_transport::{
    connect_via_lockdown_transport, load_pair_record, require_pair_record, try_load_pair_record,
};
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use super::tunnel_connect::{
    connect_via_direct_rsd_target, connect_via_remote_pairing_target, discover_direct_rsd_targets,
    discover_remote_pairing_targets,
};
use super::{
    ConnectOptions, ConnectedDevice, InitialRsdProbe, InitialRsdProbeSkipReason, LockdownTransport,
    TunnelOwner,
};
use crate::discovery::DeviceInfo;
use crate::error::CoreError;
use crate::mux::MuxClient;
#[cfg(all(feature = "tunnel", feature = "mdns"))]
use std::time::Duration;

pub async fn connect(udid: &str, opts: ConnectOptions) -> Result<ConnectedDevice, CoreError> {
    connect_inner(udid, opts, None).await
}

/// Connect with caller cleanup ownership established before native activation
/// and the initial RSD probe. Each strategy attempt reserves separately.
pub async fn connect_with_owner(
    udid: &str,
    opts: ConnectOptions,
    owner: &dyn TunnelOwner,
) -> Result<ConnectedDevice, CoreError> {
    connect_inner(udid, opts, Some(owner)).await
}

async fn connect_inner(
    udid: &str,
    opts: ConnectOptions,
    owner: Option<&dyn TunnelOwner>,
) -> Result<ConnectedDevice, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    let dev = select_mux_device(devices, udid)
        .ok_or_else(|| CoreError::DeviceNotFound(udid.to_string()))?;

    let info = DeviceInfo {
        udid: dev.serial_number.clone(),
        device_id: dev.device_id,
        connection_type: dev.connection_type.clone(),
        product_id: dev.product_id,
    };

    let pair_record = load_pair_record(udid, opts.pair_record_path.as_deref())?;
    connect_via_lockdown_transport(
        info,
        pair_record,
        LockdownTransport::Usbmux {
            device_id: dev.device_id,
        },
        opts,
        owner,
    )
    .await
}

pub async fn connect_direct_usb_tunnel(
    udid: &str,
    rsd_ip: Option<&str>,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    connect_direct_usb_tunnel_inner(udid, rsd_ip, opts, None).await
}

/// Direct USB connection with per-target caller cleanup reservations.
pub async fn connect_direct_usb_tunnel_with_owner(
    udid: &str,
    rsd_ip: Option<&str>,
    opts: ConnectOptions,
    owner: &dyn TunnelOwner,
) -> Result<ConnectedDevice, CoreError> {
    connect_direct_usb_tunnel_inner(udid, rsd_ip, opts, Some(owner)).await
}

async fn connect_direct_usb_tunnel_inner(
    udid: &str,
    rsd_ip: Option<&str>,
    opts: ConnectOptions,
    owner: Option<&dyn TunnelOwner>,
) -> Result<ConnectedDevice, CoreError> {
    let mut mux = MuxClient::connect().await?;
    let devices = mux.list_devices().await?;
    let dev = select_mux_device(devices, udid)
        .ok_or_else(|| CoreError::DeviceNotFound(udid.to_string()))?;
    let pair_record = try_load_pair_record(udid, opts.pair_record_path.as_deref());
    let info = DeviceInfo {
        udid: dev.serial_number.clone(),
        device_id: dev.device_id,
        connection_type: dev.connection_type.clone(),
        product_id: dev.product_id,
    };
    let lockdown_transport = LockdownTransport::Usbmux {
        device_id: dev.device_id,
    };

    if opts.skip_tunnel {
        let pair_record =
            require_pair_record(pair_record, udid, "direct USB lockdown access requires")?;
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            initial_rsd_probe: InitialRsdProbe::skipped(InitialRsdProbeSkipReason::TunnelSkipped),
            pair_record: Some(pair_record),
            lockdown_transport,
        });
    }

    #[cfg(not(all(feature = "tunnel", feature = "mdns")))]
    {
        let _ = (rsd_ip, owner);
        Err(CoreError::Unsupported(
            "direct USB tunnel support requires ios-core features 'tunnel' and 'mdns'".into(),
        ))
    }

    #[cfg(all(feature = "tunnel", feature = "mdns"))]
    {
        let targets = discover_direct_rsd_targets(udid, rsd_ip).await?;
        if targets.is_empty() {
            return Err(CoreError::Unsupported(format!(
                "no _remoted target matched device {}",
                crate::redaction::diagnostic_id(udid)
            )));
        }

        let mut last_error = None;
        for target in targets {
            match connect_via_direct_rsd_target(
                info.clone(),
                pair_record.clone(),
                lockdown_transport.clone(),
                opts.clone(),
                target,
                owner,
            )
            .await
            {
                Ok(device) => return Ok(device),
                Err(err) => last_error = Some(err),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            CoreError::Unsupported(format!(
                "no direct RSD target produced a tunnel for device {}",
                crate::redaction::diagnostic_id(udid)
            ))
        }))
    }
}

/// Connect only to Bonjour advertisements recognized by existing RemotePairing
/// trust records. An empty identifier selects a single matched device; several
/// devices require an explicit identifier or host filter. Legacy records without
/// a peer altIRK remain usable through direct RSD, but cannot identify Wi-Fi adverts.
pub async fn connect_remote_pairing_tunnel(
    udid: &str,
    host: Option<&str>,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    connect_remote_pairing_tunnel_inner(udid, host, opts, None).await
}

/// Existing-trust RemotePairing connection with per-target cleanup ownership.
/// A target timing out after activation retains its owner for caller cleanup;
/// a subsequent target must acquire a new reservation before transport work.
pub async fn connect_remote_pairing_tunnel_with_owner(
    udid: &str,
    host: Option<&str>,
    opts: ConnectOptions,
    owner: &dyn TunnelOwner,
) -> Result<ConnectedDevice, CoreError> {
    connect_remote_pairing_tunnel_inner(udid, host, opts, Some(owner)).await
}

async fn connect_remote_pairing_tunnel_inner(
    udid: &str,
    host: Option<&str>,
    opts: ConnectOptions,
    owner: Option<&dyn TunnelOwner>,
) -> Result<ConnectedDevice, CoreError> {
    let pair_record = try_load_pair_record(udid, opts.pair_record_path.as_deref());
    let info = DeviceInfo {
        udid: udid.to_string(),
        device_id: 0,
        connection_type: "Network".into(),
        product_id: 0,
    };

    if opts.skip_tunnel {
        let pair_record =
            require_pair_record(pair_record, udid, "remote pairing lockdown access requires")?;
        return Ok(ConnectedDevice {
            info,
            tunnel: None,
            rsd: None,
            initial_rsd_probe: InitialRsdProbe::skipped(InitialRsdProbeSkipReason::TunnelSkipped),
            pair_record: Some(pair_record),
            lockdown_transport: LockdownTransport::Tcp {
                host: host.unwrap_or_default().to_string(),
            },
        });
    }

    #[cfg(not(all(feature = "tunnel", feature = "mdns")))]
    {
        let _ = (host, owner);
        Err(CoreError::Unsupported(
            "remote pairing tunnel support requires ios-core features 'tunnel' and 'mdns'".into(),
        ))
    }

    #[cfg(all(feature = "tunnel", feature = "mdns"))]
    {
        let targets = discover_remote_pairing_targets(udid, host).await?;
        if targets.is_empty() {
            return Err(CoreError::Unsupported(
                "no RemotePairing target matched the requested device and host".into(),
            ));
        }

        let mut last_error = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        for (remote_identifier, remote_host, port) in targets {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            let mut remote_info = info.clone();
            remote_info.udid = remote_identifier.clone();
            let candidate_pair_record = if udid.is_empty() {
                try_load_pair_record(&remote_identifier, opts.pair_record_path.as_deref())
            } else {
                pair_record.clone()
            };
            match tokio::time::timeout_at(
                (tokio::time::Instant::now() + Duration::from_secs(10)).min(deadline),
                connect_via_remote_pairing_target(
                    remote_info,
                    candidate_pair_record,
                    opts.clone(),
                    &remote_identifier,
                    &remote_host,
                    port,
                    owner,
                ),
            )
            .await
            {
                Ok(Ok(device)) => return Ok(device),
                Ok(Err(err)) => last_error = Some(err),
                Err(_) => {
                    last_error = Some(CoreError::Protocol(
                        "remote pairing connection timed out".into(),
                    ))
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            CoreError::Unsupported(
                "no remote pairing target produced a tunnel for the requested device".into(),
            )
        }))
    }
}

pub async fn connect_tcp_lockdown_tunnel(
    udid: &str,
    host: &str,
    opts: ConnectOptions,
) -> Result<ConnectedDevice, CoreError> {
    connect_tcp_lockdown_tunnel_inner(udid, host, opts, None).await
}

/// TCP lockdown connection with caller ownership before initial RSD probing.
pub async fn connect_tcp_lockdown_tunnel_with_owner(
    udid: &str,
    host: &str,
    opts: ConnectOptions,
    owner: &dyn TunnelOwner,
) -> Result<ConnectedDevice, CoreError> {
    connect_tcp_lockdown_tunnel_inner(udid, host, opts, Some(owner)).await
}

async fn connect_tcp_lockdown_tunnel_inner(
    udid: &str,
    host: &str,
    opts: ConnectOptions,
    owner: Option<&dyn TunnelOwner>,
) -> Result<ConnectedDevice, CoreError> {
    let pair_record = load_pair_record(udid, opts.pair_record_path.as_deref())?;
    let info = DeviceInfo {
        udid: udid.to_string(),
        device_id: 0,
        connection_type: "Network".into(),
        product_id: 0,
    };
    connect_via_lockdown_transport(
        info,
        pair_record,
        LockdownTransport::Tcp {
            host: host.to_string(),
        },
        opts,
        owner,
    )
    .await
}

fn select_mux_device(
    devices: Vec<crate::mux::MuxDevice>,
    udid: &str,
) -> Option<crate::mux::MuxDevice> {
    let mut fallback = None;

    for device in devices {
        if device.serial_number != udid {
            continue;
        }

        let is_usb = device.connection_type.eq_ignore_ascii_case("USB");
        fallback = Some(device);

        if is_usb {
            return fallback;
        }
    }

    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_mux_device_prefers_usb_when_multiple_transports_match() {
        let selected = select_mux_device(
            vec![
                crate::mux::MuxDevice {
                    device_id: 7,
                    serial_number: "test-udid".into(),
                    connection_type: "Network".into(),
                    product_id: 0,
                },
                crate::mux::MuxDevice {
                    device_id: 8,
                    serial_number: "test-udid".into(),
                    connection_type: "USB".into(),
                    product_id: 0,
                },
            ],
            "test-udid",
        )
        .expect("matching device should be selected");

        assert_eq!(selected.device_id, 8);
        assert_eq!(selected.connection_type, "USB");
    }

    #[test]
    fn select_mux_device_falls_back_to_non_usb_match() {
        let selected = select_mux_device(
            vec![crate::mux::MuxDevice {
                device_id: 9,
                serial_number: "test-udid".into(),
                connection_type: "Network".into(),
                product_id: 0,
            }],
            "test-udid",
        )
        .expect("network-only match should still be selected");

        assert_eq!(selected.device_id, 9);
        assert_eq!(selected.connection_type, "Network");
    }
}

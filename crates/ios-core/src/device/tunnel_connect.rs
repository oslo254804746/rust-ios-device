// Connection-strategy orchestration for direct RSD and remote pairing.
//
// This file owns discovery, authentication, and strategy selection. Wire
// message construction and response validation live in `protocol.rs`; the
// shared CDTunnel lifecycle lives in `tunnel_activation.rs`.

#[cfg(feature = "tunnel")]
#[path = "tunnel_connect/protocol.rs"]
mod protocol;
#[cfg(feature = "tunnel")]
#[path = "tunnel_connect/credentials.rs"]
mod credentials;
#[cfg(feature = "tunnel")]
#[path = "tunnel_connect/pairing.rs"]
mod pairing;
#[cfg(feature = "tunnel")]
use pairing::{establish_direct_tunnel_stream, establish_remote_pairing_tunnel_stream};

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn discover_direct_rsd_targets(
    udid: &str,
    ip_filter: Option<&str>,
) -> Result<Vec<MdnsDevice>, CoreError> {
    let stream = crate::discovery::discover_mdns().await?;
    tokio::pin!(stream);

    let deadline = Instant::now() + DIRECT_RSD_DISCOVERY_TIMEOUT;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }

        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(device)) => {
                let ip = device.ipv6.to_string();
                if ip_filter.map(|filter| filter != ip).unwrap_or(false) {
                    continue;
                }

                let key = (device.ipv6, device.rsd_port);
                if !seen.insert(key) {
                    continue;
                }

                targets.push(device);
            }
            Ok(None) | Err(_) => break,
        }
    }

    targets.sort_by_key(|device| {
        if device.udid == udid {
            0
        } else if device.udid.is_empty() {
            1
        } else {
            2
        }
    });
    Ok(targets)
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn discover_remote_pairing_targets(
    udid: &str,
    host_filter: Option<&str>,
) -> Result<Vec<(String, u16)>, CoreError> {
    let services = browse_remotepairing(MOBDEV2_DISCOVERY_TIMEOUT).await?;
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for service in services {
        let Some(host) = preferred_lockdown_address(&service.addresses) else {
            continue;
        };
        if host_filter.map(|filter| filter != host).unwrap_or(false) {
            continue;
        }

        let key = (host.to_string(), service.port);
        if seen.insert(key.clone()) {
            targets.push(key);
        }
    }

    if targets.is_empty() {
        return Err(CoreError::Unsupported(format!(
            "no browse_remotepairing target matched udid={udid} host={host_filter:?}"
        )));
    }

    Ok(targets)
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn connect_via_direct_rsd_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
    target: MdnsDevice,
) -> Result<ConnectedDevice, CoreError> {
    let rsd = rsd_handshake(target.ipv6, target.rsd_port).await?;
    if rsd.udid != info.udid {
        return Err(CoreError::Protocol(format!(
            "direct RSD target {} resolved to unexpected udid {}",
            target.ipv6, rsd.udid
        )));
    }

    let service_port = rsd
        .get_port(crate::pairing_transport::UNTRUSTED_SERVICE_NAME)
        .ok_or_else(|| {
            CoreError::Unsupported(format!(
                "direct RSD target {} does not expose {}",
                target.ipv6,
                crate::pairing_transport::UNTRUSTED_SERVICE_NAME
            ))
        })?;
    let direct_stream = establish_direct_tunnel_stream(target.ipv6, service_port).await?;

    activate_tunnel(
        TunnelConnection::new(info, pair_record, lockdown_transport, "direct RSD"),
        direct_stream,
        opts.tun_mode,
    )
    .await
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
async fn connect_via_remote_pairing_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    opts: ConnectOptions,
    remote_identifier: &str,
    host: &str,
    port: u16,
) -> Result<ConnectedDevice, CoreError> {
    let remote_stream = establish_remote_pairing_tunnel_stream(remote_identifier, host, port).await?;

    activate_tunnel(
        TunnelConnection::new(
            info,
            pair_record,
            LockdownTransport::Tcp {
                host: host.to_string(),
            },
            "remote pairing",
        ),
        remote_stream,
        opts.tun_mode,
    )
    .await
}

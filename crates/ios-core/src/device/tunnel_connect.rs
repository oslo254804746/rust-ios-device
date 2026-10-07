//! Direct-RSD and RemotePairing discovery and authenticated strategies.

#[cfg(feature = "mdns")]
use super::discovery_match::ordered_lockdown_addresses;
#[cfg(feature = "mdns")]
use super::tunnel_activation::{activate_tunnel, TunnelConnection};
#[cfg(feature = "mdns")]
use super::{ConnectOptions, ConnectedDevice, LockdownTransport};
#[cfg(feature = "mdns")]
use crate::discovery::{browse_remotepairing, BonjourService, DeviceInfo, MdnsDevice};
#[cfg(feature = "mdns")]
use crate::error::CoreError;
#[cfg(feature = "mdns")]
use crate::lockdown::pair_record::PairRecord;
#[cfg(feature = "mdns")]
use crate::xpc::rsd::handshake as rsd_handshake;
#[cfg(feature = "mdns")]
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[cfg(feature = "mdns")]
use tokio_stream::StreamExt;

#[cfg(feature = "mdns")]
const DIRECT_RSD_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(feature = "mdns")]
const REMOTE_PAIRING_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

// Connection-strategy orchestration for direct RSD and remote pairing.
//
// This file owns discovery, authentication, and strategy selection. Wire
// message construction and response validation live in `protocol.rs`; the
// shared CDTunnel lifecycle lives in `tunnel_activation.rs`.

#[cfg(feature = "tunnel")]
mod credentials;
#[cfg(feature = "tunnel")]
mod pairing;
#[cfg(feature = "tunnel")]
mod protocol;
#[cfg(feature = "mdns")]
use pairing::{establish_direct_tunnel_stream, establish_remote_pairing_tunnel_stream};

#[cfg(test)]
mod source_contract_tests;

#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub(super) async fn discover_direct_rsd_targets(
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
pub(super) async fn discover_remote_pairing_targets(
    udid: &str,
    host_filter: Option<&str>,
) -> Result<Vec<(String, String, u16)>, CoreError> {
    let keys = tokio::task::spawn_blocking(credentials::load_remote_discovery_keys)
        .await
        .map_err(|error| super::error_context::retain("load remote discovery keys", error))??;
    if keys.is_empty() {
        return Err(CoreError::Unsupported(
            "no existing RemotePairing record has a usable peer altIRK; direct RSD remains available for older records".into()
        ));
    }
    let services = browse_remotepairing(REMOTE_PAIRING_DISCOVERY_TIMEOUT).await?;
    let targets = match_remote_pairing_targets(&services, &keys, udid, host_filter)?;
    if targets.is_empty() {
        return Err(CoreError::Unsupported(
            "no RemotePairing advertisement matched the existing trust records and filters".into(),
        ));
    }
    Ok(targets)
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
fn match_remote_pairing_targets(
    services: &[BonjourService],
    keys: &[(String, zeroize::Zeroizing<[u8; 16]>)],
    requested_identifier: &str,
    host_filter: Option<&str>,
) -> Result<Vec<(String, String, u16)>, CoreError> {
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut matched_identifiers = std::collections::HashSet::new();
    for service in services {
        // Match against every record before applying the requested identifier:
        // narrowing first would conceal duplicate discovery keys.
        let mut matched = keys.iter().filter(|(_, key)| {
            crate::discovery::auth::remote_pairing_matches(&service.properties, key)
        });
        let Some((identifier, _)) = matched.next() else {
            continue;
        };
        if matched.any(|(other, _)| other != identifier) {
            return Err(CoreError::Protocol(
                "ambiguous RemotePairing advertisement matches multiple trust records".into(),
            ));
        }
        if !requested_identifier.is_empty() && identifier != requested_identifier {
            continue;
        }
        if service.port == 0 {
            continue;
        }
        for host in ordered_lockdown_addresses(&service.addresses)
            .into_iter()
            .take(4)
        {
            if host_filter.is_some_and(|filter| filter != host) {
                continue;
            }
            matched_identifiers.insert(identifier);
            if requested_identifier.is_empty() && matched_identifiers.len() > 1 {
                return Err(CoreError::Unsupported("multiple paired RemotePairing devices found; specify a device identifier or host".into()));
            }
            let key = (identifier.clone(), host.to_string(), service.port);
            if targets.len() < 16 && seen.insert(key.clone()) {
                targets.push(key);
            }
        }
    }
    Ok(targets)
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub(super) async fn connect_via_direct_rsd_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    lockdown_transport: LockdownTransport,
    opts: ConnectOptions,
    target: MdnsDevice,
    owner: Option<&dyn super::TunnelOwner>,
) -> Result<ConnectedDevice, CoreError> {
    let reservation = owner.map(super::TunnelOwner::reserve).transpose()?;
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
        reservation,
    )
    .await
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
pub(super) async fn connect_via_remote_pairing_target(
    info: DeviceInfo,
    pair_record: Option<Arc<PairRecord>>,
    opts: ConnectOptions,
    remote_identifier: &str,
    host: &str,
    port: u16,
    owner: Option<&dyn super::TunnelOwner>,
) -> Result<ConnectedDevice, CoreError> {
    let reservation = owner.map(super::TunnelOwner::reserve).transpose()?;
    let remote_stream =
        establish_remote_pairing_tunnel_stream(remote_identifier, host, port).await?;

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
        reservation,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::super::lockdown_transport::ProxyStream;
    use super::super::tunnel_activation::TunnelTransport;
    use super::credentials::*;
    use super::pairing::{GuardedTunnelStream, RemotePairingControlChannel};
    #[cfg(feature = "mdns")]
    use super::*;
    use crate::credentials::{PersistedCredentials, RemotePairingRecord};
    use crate::lockdown::pairing::HostIdentity;
    use crate::xpc::XpcClient;
    #[cfg(feature = "mdns")]
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn temp_test_dir(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ios_core_device_{label}_{unique}"))
    }

    #[cfg(feature = "tunnel")]
    fn make_remote_pair_record(identity: &HostIdentity) -> RemotePairingRecord {
        RemotePairingRecord {
            public_key: identity.public_key_bytes(),
            private_key: identity.private_key_bytes(),
            remote_unlock_host_key: None,
            peer_alt_irk: None,
        }
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_transport_accepts_each_connection_strategy() {
        fn assert_transport<T: TunnelTransport>() {}

        // Lockdown's CoreDeviceProxy wrapper, direct RSD's XPC control-channel
        // guard, and remote pairing's framed control-channel guard all enter
        // the same activation path. A future protocol can exercise it with an
        // in-memory duplex stream before hardware integration exists.
        assert_transport::<ProxyStream>();
        assert_transport::<GuardedTunnelStream<XpcClient>>();
        assert_transport::<GuardedTunnelStream<RemotePairingControlChannel>>();
        assert_transport::<tokio::io::DuplexStream>();
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn load_remote_pairing_credentials_accepts_legacy_ios_rs_without_private_key_hex() {
        let base_dir = temp_test_dir("legacy_ios_rs");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let identity = HostIdentity::generate();

        make_remote_pair_record(&identity)
            .save_for_identifier(&ios_rs_dir, remote_identifier)
            .unwrap();
        PersistedCredentials {
            remote_identifier: Some(remote_identifier.into()),
            host_identifier: identity.identifier.clone(),
            host_public_key_hex: hex::encode(identity.public_key_bytes()),
            host_private_key_hex: None,
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&ios_rs_dir)
        .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            "unused-hostname",
        )
        .expect("legacy ios-rs credentials should load from remote pair record");

        assert_eq!(loaded.host_identity.identifier, identity.identifier);
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn load_remote_pairing_credentials_prefers_ios_rs_over_pymobiledevice3() {
        let base_dir = temp_test_dir("prefers_ios_rs");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let ios_rs_identity = HostIdentity::generate();
        let fallback_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier("example-host"),
            &[0x44; 32],
        )
        .unwrap();

        make_remote_pair_record(&ios_rs_identity)
            .save_for_identifier(&ios_rs_dir, remote_identifier)
            .unwrap();
        PersistedCredentials {
            remote_identifier: Some(remote_identifier.into()),
            host_identifier: ios_rs_identity.identifier.clone(),
            host_public_key_hex: hex::encode(ios_rs_identity.public_key_bytes()),
            host_private_key_hex: Some(hex::encode(ios_rs_identity.private_key_bytes())),
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&ios_rs_dir)
        .unwrap();
        make_remote_pair_record(&fallback_identity)
            .save_for_identifier(&pymobiledevice3_dir, remote_identifier)
            .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            "example-host",
        )
        .expect("ios-rs credentials should take precedence");

        assert_eq!(loaded.host_identity.identifier, ios_rs_identity.identifier);
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            ios_rs_identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn load_remote_pairing_credentials_falls_back_to_pymobiledevice3_remote_record() {
        let base_dir = temp_test_dir("pymobiledevice3_fallback");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let hostname = "example-host";
        let expected_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier(hostname),
            &[0x22; 32],
        )
        .unwrap();

        make_remote_pair_record(&expected_identity)
            .save_for_identifier(&pymobiledevice3_dir, remote_identifier)
            .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            hostname,
        )
        .expect("pymobiledevice3 remote record should be usable as fallback");

        assert_eq!(
            loaded.host_identity.identifier,
            pymobiledevice3_host_identifier(hostname)
        );
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            expected_identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    #[cfg(all(feature = "tunnel", feature = "mdns"))]
    fn remote_discovery_keys_use_only_existing_valid_records_and_match_saved_alt_irk() {
        use base64::Engine as _;

        let base_dir = temp_test_dir("remote_discovery_keys");
        let own_dir = base_dir.join("ios-rs");
        let compatible_dir = base_dir.join(".pymobiledevice3");
        let own_identity = HostIdentity::generate();
        let mut own_record = make_remote_pair_record(&own_identity);
        own_record.peer_alt_irk = Some(
            base64::engine::general_purpose::STANDARD
                .decode("Mgp6ZGPzXM2ku9br46vsiw==")
                .unwrap(),
        );
        own_record.save_for_identifier(&own_dir, "own").unwrap();
        PersistedCredentials {
            remote_identifier: Some("own".into()),
            host_identifier: own_identity.identifier.clone(),
            host_public_key_hex: hex::encode(own_identity.public_key_bytes()),
            host_private_key_hex: Some(hex::encode(own_identity.private_key_bytes())),
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&own_dir)
        .unwrap();

        // A good plist without the companion identity, a stale record, and a
        // corrupt file must not become network candidates.
        own_record.save_for_identifier(&own_dir, "orphan").unwrap();
        let hostname = "known-host";
        let compatible_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier(hostname),
            &[0x44; 32],
        )
        .unwrap();
        let mut compatible_record = make_remote_pair_record(&compatible_identity);
        compatible_record.peer_alt_irk = Some(vec![7; 16]);
        compatible_record
            .save_for_identifier(&compatible_dir, "compatible")
            .unwrap();
        let mut stale = make_remote_pair_record(&compatible_identity);
        stale.peer_alt_irk = None;
        stale.save_for_identifier(&compatible_dir, "stale").unwrap();
        std::fs::write(compatible_dir.join("remote_corrupt.plist"), b"not a plist").unwrap();

        let keys = load_remote_discovery_keys_from_dirs(&own_dir, &compatible_dir, hostname);
        assert_eq!(keys.len(), 2);
        assert_eq!(
            keys.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["compatible", "own"]
        );
        let service = BonjourService {
            instance: "opaque._remotepairing._tcp.local.".into(),
            port: 49152,
            addresses: vec!["192.0.2.1".into()],
            properties: HashMap::from([
                (
                    "identifier".into(),
                    "2BE6E510-0325-4365-923E-B14C6F57DB3A".into(),
                ),
                ("authTag".into(), "kXjlTr2l".into()),
            ]),
        };
        let matches = match_remote_pairing_targets(&[service], &keys, "", None).unwrap();
        assert_eq!(matches, vec![("own".into(), "192.0.2.1".into(), 49152)]);
        std::fs::remove_dir_all(base_dir).unwrap();
    }
}

#[cfg(all(test, feature = "mdns"))]
mod discovery_tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::collections::HashMap;

    #[test]
    #[cfg(feature = "tunnel")]
    fn remote_pairing_offline_matching_rejects_unmatched_and_ambiguous_keys() {
        let mut key = [0; 16];
        STANDARD
            .decode_slice("Mgp6ZGPzXM2ku9br46vsiw==", &mut key)
            .unwrap();
        let keys = vec![("device".into(), zeroize::Zeroizing::new(key))];
        let service = BonjourService {
            instance: "opaque._remotepairing._tcp.local.".into(),
            port: 49152,
            addresses: vec!["192.0.2.1".into(), "192.0.2.2".into()],
            properties: HashMap::from([
                (
                    "identifier".into(),
                    "2BE6E510-0325-4365-923E-B14C6F57DB3A".into(),
                ),
                ("authTag".into(), "kXjlTr2l".into()),
            ]),
        };
        let targets =
            match_remote_pairing_targets(std::slice::from_ref(&service), &keys, "", None).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].0, "device");
        assert!(
            match_remote_pairing_targets(std::slice::from_ref(&service), &keys, "other", None)
                .unwrap()
                .is_empty()
        );
        assert!(match_remote_pairing_targets(
            std::slice::from_ref(&service),
            &keys,
            "device",
            Some("192.0.2.3")
        )
        .unwrap()
        .is_empty());
        let mut duplicate = keys;
        duplicate.push(("other".into(), zeroize::Zeroizing::new(key)));
        assert!(match_remote_pairing_targets(
            std::slice::from_ref(&service),
            &duplicate,
            "device",
            None
        )
        .is_err());
        let mut missing = service;
        missing.properties.clear();
        assert!(
            match_remote_pairing_targets(&[missing], &duplicate, "", None)
                .unwrap()
                .is_empty()
        );
    }
}

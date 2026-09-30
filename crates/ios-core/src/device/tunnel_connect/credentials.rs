//! Remote-pairing credential discovery and compatibility validation.

use std::path::Path;

use crate::credentials::{PersistedCredentials, RemotePairingRecord};
use crate::error::CoreError;
use crate::lockdown::pairing::HostIdentity;

pub(super) struct LoadedRemotePairingCredentials {
    pub(super) host_identity: HostIdentity,
    #[cfg(feature = "mdns")]
    pub(super) peer_alt_irk: Option<zeroize::Zeroizing<[u8; 16]>>,
}

#[cfg(feature = "mdns")]
pub(super) type RemoteDiscoveryKeys = Vec<(String, zeroize::Zeroizing<[u8; 16]>)>;

#[cfg(feature = "mdns")]
pub(super) fn load_remote_discovery_keys() -> Result<RemoteDiscoveryKeys, CoreError> {
    let own_dir = PersistedCredentials::default_dir();
    let compatible_dir = PersistedCredentials::pymobiledevice3_dir();
    let hostname = current_hostname()?;
    Ok(load_remote_discovery_keys_from_dirs(
        &own_dir,
        &compatible_dir,
        &hostname,
    ))
}

#[cfg(feature = "mdns")]
pub(super) fn load_remote_discovery_keys_from_dirs(
    own_dir: &Path,
    compatible_dir: &Path,
    hostname: &str,
) -> RemoteDiscoveryKeys {
    let mut identifiers = std::collections::BTreeSet::new();
    for dir in [&own_dir, &compatible_dir] {
        identifiers.extend(RemotePairingRecord::list(dir).into_iter().map(|(id, _)| id));
    }
    identifiers
        .into_iter()
        .take(128)
        .filter_map(|identifier| {
            let loaded = load_remote_pairing_credentials_from_dirs(
                &identifier,
                own_dir,
                compatible_dir,
                hostname,
            )
            .ok()?;
            Some((identifier, loaded.peer_alt_irk?))
        })
        .collect()
}

pub(super) fn load_remote_pairing_credentials(
    remote_identifier: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let hostname = current_hostname()?;
    load_remote_pairing_credentials_from_dirs(
        remote_identifier,
        &PersistedCredentials::default_dir(),
        &PersistedCredentials::pymobiledevice3_dir(),
        &hostname,
    )
}

pub(super) fn load_remote_pairing_credentials_from_dirs(
    remote_identifier: &str,
    ios_rs_dir: &Path,
    pymobiledevice3_dir: &Path,
    hostname: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    if let Some(remote_pair_record) =
        RemotePairingRecord::load_for_identifier(ios_rs_dir, remote_identifier)
    {
        if let Some(persisted) = find_persisted_host_identity(ios_rs_dir, remote_identifier) {
            return load_ios_rs_remote_pairing_credentials(
                remote_identifier,
                remote_pair_record,
                persisted,
            );
        }
    }

    if let Some(remote_pair_record) =
        RemotePairingRecord::load_for_identifier(pymobiledevice3_dir, remote_identifier)
    {
        return load_pymobiledevice3_remote_pairing_credentials(
            remote_identifier,
            hostname,
            remote_pair_record,
            pymobiledevice3_dir,
        );
    }

    if RemotePairingRecord::load_for_identifier(ios_rs_dir, remote_identifier).is_some() {
        return Err(CoreError::Unsupported(
            "missing persisted host identity for remote pairing record".into(),
        ));
    }

    Err(CoreError::Unsupported(
        "missing remote pairing record in the configured credential directories".into(),
    ))
}

pub(super) fn find_persisted_host_identity(
    creds_dir: &Path,
    remote_identifier: &str,
) -> Option<PersistedCredentials> {
    PersistedCredentials::list(creds_dir)
        .into_iter()
        .find(|creds| creds.remote_identifier.as_deref() == Some(remote_identifier))
}

pub(super) fn load_ios_rs_remote_pairing_credentials(
    _remote_identifier: &str,
    remote_pair_record: RemotePairingRecord,
    persisted: PersistedCredentials,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_private_key = remote_pair_record.private_key.clone();
    let host_identity =
        HostIdentity::from_private_key_bytes(persisted.host_identifier, &host_private_key)
            .map_err(|e| CoreError::Other(format!("invalid persisted host identity: {e}")))?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Protocol(
            "persisted host key mismatch for remote pairing record".into(),
        ));
    }

    if let Some(host_private_key_hex) = persisted.host_private_key_hex {
        let persisted_private_key = hex::decode(host_private_key_hex)
            .map_err(|e| CoreError::Other(format!("invalid host private key hex: {e}")))?;
        if persisted_private_key != remote_pair_record.private_key {
            return Err(CoreError::Protocol(
                "persisted host private key mismatch for remote pairing record".into(),
            ));
        }
    }

    Ok(LoadedRemotePairingCredentials {
        host_identity,
        #[cfg(feature = "mdns")]
        peer_alt_irk: remote_pair_record
            .peer_alt_irk
            .as_deref()
            .and_then(|key| <[u8; 16]>::try_from(key).ok())
            .map(zeroize::Zeroizing::new),
    })
}

pub(super) fn load_pymobiledevice3_remote_pairing_credentials(
    _remote_identifier: &str,
    hostname: &str,
    remote_pair_record: RemotePairingRecord,
    _creds_dir: &Path,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_identifier = pymobiledevice3_host_identifier(hostname);
    let host_identity =
        HostIdentity::from_private_key_bytes(host_identifier, &remote_pair_record.private_key)
            .map_err(|e| {
                CoreError::Other(format!(
                    "invalid pymobiledevice3 remote pairing identity: {e}"
                ))
            })?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Protocol(
            "pymobiledevice3 host key mismatch for remote pairing record".into(),
        ));
    }

    Ok(LoadedRemotePairingCredentials {
        host_identity,
        #[cfg(feature = "mdns")]
        peer_alt_irk: remote_pair_record
            .peer_alt_irk
            .as_deref()
            .and_then(|key| <[u8; 16]>::try_from(key).ok())
            .map(zeroize::Zeroizing::new),
    })
}

pub(super) fn current_hostname() -> Result<String, CoreError> {
    Ok(crate::xpc::identity::current_hostname()?)
}

pub(super) fn pymobiledevice3_host_identifier(hostname: &str) -> String {
    crate::xpc::identity::hostname_uuid(hostname)
        .to_string()
        .to_uppercase()
}

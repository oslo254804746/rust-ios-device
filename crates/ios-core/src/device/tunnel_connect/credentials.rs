//! Remote-pairing credential discovery and compatibility validation.

use std::path::Path;

use crate::credentials::{PersistedCredentials, RemotePairingRecord};
use crate::error::CoreError;
use crate::lockdown::pairing::HostIdentity;

pub(super) struct LoadedRemotePairingCredentials {
    pub(super) host_identity: HostIdentity,
}

pub(super) fn load_remote_pairing_credentials(
    remote_identifier: &str,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    load_remote_pairing_credentials_from_dirs(
        remote_identifier,
        &PersistedCredentials::default_dir(),
        &PersistedCredentials::pymobiledevice3_dir(),
        &current_hostname(),
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
        return Err(CoreError::Unsupported(format!(
            "missing persisted host identity for remote identifier {remote_identifier}"
        )));
    }

    Err(CoreError::Unsupported(format!(
        "missing remote pairing record for {remote_identifier} in {} or {}",
        ios_rs_dir.display(),
        pymobiledevice3_dir.display()
    )))
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
    remote_identifier: &str,
    remote_pair_record: RemotePairingRecord,
    persisted: PersistedCredentials,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_private_key = remote_pair_record.private_key.clone();
    let host_identity =
        HostIdentity::from_private_key_bytes(persisted.host_identifier, &host_private_key)
            .map_err(|e| CoreError::Other(format!("invalid persisted host identity: {e}")))?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Protocol(format!(
            "persisted host key mismatch for remote identifier {remote_identifier}"
        )));
    }

    if let Some(host_private_key_hex) = persisted.host_private_key_hex {
        let persisted_private_key = hex::decode(host_private_key_hex)
            .map_err(|e| CoreError::Other(format!("invalid host private key hex: {e}")))?;
        if persisted_private_key != remote_pair_record.private_key {
            return Err(CoreError::Protocol(format!(
                "persisted host private key mismatch for remote identifier {remote_identifier}"
            )));
        }
    }

    Ok(LoadedRemotePairingCredentials { host_identity })
}

pub(super) fn load_pymobiledevice3_remote_pairing_credentials(
    remote_identifier: &str,
    hostname: &str,
    remote_pair_record: RemotePairingRecord,
    creds_dir: &Path,
) -> Result<LoadedRemotePairingCredentials, CoreError> {
    let host_identifier = pymobiledevice3_host_identifier(hostname);
    let host_identity =
        HostIdentity::from_private_key_bytes(host_identifier, &remote_pair_record.private_key)
            .map_err(|e| {
                CoreError::Other(format!(
                    "invalid pymobiledevice3 remote pairing identity for {remote_identifier}: {e}"
                ))
            })?;

    if host_identity.public_key_bytes() != remote_pair_record.public_key {
        return Err(CoreError::Protocol(format!(
            "pymobiledevice3 host key mismatch for remote identifier {remote_identifier} in {}",
            creds_dir.display()
        )));
    }

    Ok(LoadedRemotePairingCredentials { host_identity })
}

pub(super) fn current_hostname() -> String {
    std::env::var_os("COMPUTERNAME")
        .or_else(|| std::env::var_os("HOSTNAME"))
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub(super) fn pymobiledevice3_host_identifier(hostname: &str) -> String {
    const NAMESPACE_DNS: [u8; 16] = [
        0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30,
        0xc8,
    ];

    let mut input = Vec::with_capacity(NAMESPACE_DNS.len() + hostname.len());
    input.extend_from_slice(&NAMESPACE_DNS);
    input.extend_from_slice(hostname.as_bytes());

    let mut bytes = md5::compute(&input).0.to_vec();
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
    .to_uppercase()
}

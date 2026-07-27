//! Pairing credential persistence.
//!
//! Saves/loads the host identity generated during SRP pairing.
//! Stored as JSON at a platform-specific path.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedCredentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_identifier: Option<String>,
    pub host_identifier: String,
    /// Ed25519 public key (hex-encoded)
    pub host_public_key_hex: String,
    /// Ed25519 private key seed (hex-encoded) used for future verifyManualPairing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_private_key_hex: Option<String>,
    /// Base64-encoded remote unlock host key returned by createRemoteUnlockKey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_unlock_host_key: Option<String>,
    /// IPv6 address of the paired device
    pub device_address: String,
    /// RSD port at time of pairing
    pub rsd_port: u16,
}

impl PersistedCredentials {
    /// Default directory for storing pair credentials.
    pub fn default_dir() -> PathBuf {
        if cfg!(target_os = "macos") {
            dirs_next::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".ios-rs")
        } else if cfg!(windows) {
            std::env::var("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("C:\\ProgramData"))
                .join("ios-rs")
        } else {
            dirs_next::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(".ios-rs")
        }
    }

    /// Compatibility directory used by pymobiledevice3 remote pairing records.
    pub fn pymobiledevice3_dir() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".pymobiledevice3")
    }

    /// Path to the credential file for a specific device.
    pub fn path_for(dir: &std::path::Path, device_addr: &str) -> PathBuf {
        let safe_addr = device_addr.replace([':', '%'], "_");
        dir.join(format!("{safe_addr}.json"))
    }

    /// Save to disk.
    ///
    /// Contains the Ed25519 host private-key seed, so it is written owner-only.
    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        let path = Self::path_for(dir, &self.device_address);
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::secret_file::write_secret(&path, json.as_bytes())
    }

    /// Load from disk by device address.
    pub fn load(dir: &std::path::Path, device_addr: &str) -> Option<Self> {
        Self::load_from_path(&Self::path_for(dir, device_addr))
    }

    /// Read a single credential file.
    ///
    /// Callers treat `None` as "not paired yet" and pair again from scratch, so
    /// a file that exists but cannot be read or parsed must not look the same as
    /// a missing one: the record is still on disk and the surprise re-pair needs
    /// a trace to explain it.
    fn load_from_path(path: &std::path::Path) -> Option<Self> {
        match std::fs::read_to_string(path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(creds) => Some(creds),
                Err(err) => {
                    tracing::warn!("ignoring corrupt credential file {}: {err}", path.display());
                    None
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("no credential file at {}", path.display());
                None
            }
            Err(err) => {
                tracing::warn!(
                    "ignoring unreadable credential file {}: {err}",
                    path.display()
                );
                None
            }
        }
    }

    /// List all saved credentials in a directory.
    pub fn list(dir: &std::path::Path) -> Vec<Self> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("no credential directory at {}", dir.display());
                return vec![];
            }
            Err(err) => {
                tracing::warn!("cannot list credentials in {}: {err}", dir.display());
                return vec![];
            }
        };
        entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|path| path.extension().map(|x| x == "json").unwrap_or(false))
            .filter_map(|path| Self::load_from_path(&path))
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemotePairingRecord {
    pub public_key: Vec<u8>,
    pub private_key: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_unlock_host_key: Option<String>,
}

impl RemotePairingRecord {
    pub fn path_for_identifier(dir: &std::path::Path, remote_identifier: &str) -> PathBuf {
        dir.join(format!("remote_{remote_identifier}.plist"))
    }

    /// Save to disk.
    ///
    /// Contains the remote pairing private key, so it is written owner-only.
    pub fn save_for_identifier(
        &self,
        dir: &std::path::Path,
        remote_identifier: &str,
    ) -> std::io::Result<()> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, self).map_err(std::io::Error::other)?;
        crate::secret_file::write_secret(&Self::path_for_identifier(dir, remote_identifier), &buf)
    }

    pub fn load_for_identifier(dir: &std::path::Path, remote_identifier: &str) -> Option<Self> {
        Self::load_from_path(&Self::path_for_identifier(dir, remote_identifier))
    }

    /// Read a single pairing record.
    ///
    /// Same reasoning as `PersistedCredentials::load_from_path`: a missing
    /// record is routine, an unusable one is not. The file is read separately
    /// from the plist parse so that the two cases stay distinguishable —
    /// `plist::from_file` folds the io error into its own error type.
    fn load_from_path(path: &std::path::Path) -> Option<Self> {
        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("no remote pairing record at {}", path.display());
                return None;
            }
            Err(err) => {
                tracing::warn!(
                    "ignoring unreadable remote pairing record {}: {err}",
                    path.display()
                );
                return None;
            }
        };

        plist::from_bytes(&data)
            .inspect_err(|err| {
                tracing::warn!(
                    "ignoring corrupt remote pairing record {}: {err}",
                    path.display()
                );
            })
            .ok()
    }

    pub fn list(dir: &std::path::Path) -> Vec<(String, Self)> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("no remote pairing record directory at {}", dir.display());
                return vec![];
            }
            Err(err) => {
                tracing::warn!(
                    "cannot list remote pairing records in {}: {err}",
                    dir.display()
                );
                return vec![];
            }
        };

        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                // Files that do not follow the naming scheme belong to someone
                // else and are skipped without a trace; only records we claim
                // and then fail to read are worth warning about.
                let path = entry.path();
                let file_name = path.file_name()?.to_str()?;
                let remote_identifier = file_name
                    .strip_prefix("remote_")?
                    .strip_suffix(".plist")?
                    .to_string();
                Some((remote_identifier, Self::load_from_path(&path)?))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let dir = std::env::temp_dir().join("ios_rs_test_creds");
        let cred = PersistedCredentials {
            remote_identifier: Some("test-remote".into()),
            host_identifier: "test-id".into(),
            host_public_key_hex: "deadbeef".into(),
            host_private_key_hex: Some("cafebabe".into()),
            remote_unlock_host_key: Some("host-key".into()),
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        };
        cred.save(&dir).unwrap();
        let loaded = PersistedCredentials::load(&dir, "fd00::1").unwrap();
        assert_eq!(loaded.remote_identifier.as_deref(), Some("test-remote"));
        assert_eq!(loaded.host_identifier, "test-id");
        assert_eq!(loaded.rsd_port, 58783);
        assert_eq!(loaded.host_private_key_hex.as_deref(), Some("cafebabe"));
        assert_eq!(loaded.remote_unlock_host_key.as_deref(), Some("host-key"));
        // cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_backward_compatible_load_without_private_key() {
        let dir = std::env::temp_dir().join("ios_rs_test_creds_legacy");
        std::fs::create_dir_all(&dir).unwrap();
        let path = PersistedCredentials::path_for(&dir, "fd00::2");
        std::fs::write(
            &path,
            r#"{
  "host_identifier": "legacy-id",
  "host_public_key_hex": "deadbeef",
  "device_address": "fd00::2",
  "rsd_port": 58783
}"#,
        )
        .unwrap();

        let loaded = PersistedCredentials::load(&dir, "fd00::2").unwrap();
        assert_eq!(loaded.host_identifier, "legacy-id");
        assert!(loaded.host_private_key_hex.is_none());
        assert!(loaded.remote_identifier.is_none());
        assert!(loaded.remote_unlock_host_key.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_corrupt_credential_file_is_skipped_not_treated_as_valid() {
        let dir = std::env::temp_dir().join("ios_rs_test_creds_corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            PersistedCredentials::path_for(&dir, "fd00::3"),
            b"{ this is not json",
        )
        .unwrap();

        let good = PersistedCredentials {
            remote_identifier: None,
            host_identifier: "good-id".into(),
            host_public_key_hex: "deadbeef".into(),
            host_private_key_hex: None,
            remote_unlock_host_key: None,
            device_address: "fd00::4".into(),
            rsd_port: 58783,
        };
        good.save(&dir).unwrap();

        assert!(PersistedCredentials::load(&dir, "fd00::3").is_none());
        assert!(PersistedCredentials::load(&dir, "fd00::5").is_none());

        let listed = PersistedCredentials::list(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].host_identifier, "good-id");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_corrupt_remote_pairing_record_is_skipped() {
        let dir = std::env::temp_dir().join("ios_rs_test_remote_pair_record_corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            RemotePairingRecord::path_for_identifier(&dir, "BROKEN"),
            b"not a plist",
        )
        .unwrap();

        assert!(RemotePairingRecord::load_for_identifier(&dir, "BROKEN").is_none());
        assert!(RemotePairingRecord::load_for_identifier(&dir, "MISSING").is_none());
        assert!(RemotePairingRecord::list(&dir).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_remote_pairing_record_roundtrip() {
        let dir = std::env::temp_dir().join("ios_rs_test_remote_pair_record");
        let record = RemotePairingRecord {
            public_key: vec![0x01, 0x02, 0x03],
            private_key: vec![0x04, 0x05, 0x06],
            remote_unlock_host_key: Some("PcV5xhyuJBL7Qq9HOGeGVwtU4sJLe1jtl/vRy1tRKcI=".into()),
        };

        record
            .save_for_identifier(&dir, "00008150-000D6D6A1122401C")
            .unwrap();

        let loaded =
            RemotePairingRecord::load_for_identifier(&dir, "00008150-000D6D6A1122401C").unwrap();
        assert_eq!(loaded, record);

        let listed = RemotePairingRecord::list(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "00008150-000D6D6A1122401C");
        assert_eq!(listed[0].1, record);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

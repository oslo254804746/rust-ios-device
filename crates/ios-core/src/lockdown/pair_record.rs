use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum PairRecordError {
    #[error("pair record not found for UDID: {0}")]
    NotFound(String),
    #[error("failed to parse pair record: {0}")]
    Parse(String),
}

/// iOS device pair record, loaded from the platform-specific lockdown directory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PairRecord {
    /// DER/PEM-encoded device certificate
    #[serde(with = "serde_bytes")]
    pub device_certificate: Vec<u8>,
    /// DER/PEM-encoded host certificate
    #[serde(with = "serde_bytes")]
    pub host_certificate: Vec<u8>,
    /// DER/PEM-encoded host private key
    #[serde(with = "serde_bytes")]
    pub host_private_key: Vec<u8>,
    /// DER/PEM-encoded root certificate
    #[serde(with = "serde_bytes")]
    pub root_certificate: Vec<u8>,
    /// Host identifier (UUID string)
    #[serde(rename = "HostID")]
    pub host_id: String,
    /// System BUID
    #[serde(rename = "SystemBUID")]
    pub system_buid: String,
    /// Wi-Fi MAC address recorded by lockdown pairing, used for mobdev2 discovery matching.
    pub wifi_mac_address: Option<String>,
}

impl PairRecord {
    /// Load from the platform default path.
    pub fn load(udid: &str) -> Result<Self, PairRecordError> {
        let path = default_pair_record_path(udid);
        Self::load_from_path(&path, udid)
    }

    /// Load from an explicit path.
    pub fn load_from_path(path: &std::path::Path, udid: &str) -> Result<Self, PairRecordError> {
        let data = std::fs::read(path).map_err(|_| PairRecordError::NotFound(udid.to_string()))?;
        plist::from_bytes(&data).map_err(|e| PairRecordError::Parse(e.to_string()))
    }
}

pub fn default_pair_record_path(udid: &str) -> PathBuf {
    default_pair_record_dir().join(format!("{udid}.plist"))
}

pub fn default_pair_record_dir() -> PathBuf {
    pair_record_dir_for_platform(
        cfg!(target_os = "macos"),
        cfg!(windows),
        &std::env::var("ALLUSERSPROFILE").unwrap_or_default(),
    )
}

#[cfg(test)]
pub(crate) fn pair_record_path_for_platform(
    udid: &str,
    is_macos: bool,
    is_windows: bool,
    all_users_profile: &str,
) -> PathBuf {
    pair_record_dir_for_platform(is_macos, is_windows, all_users_profile)
        .join(format!("{udid}.plist"))
}

fn pair_record_dir_for_platform(
    is_macos: bool,
    is_windows: bool,
    all_users_profile: &str,
) -> PathBuf {
    if is_windows {
        PathBuf::from(all_users_profile)
            .join("Apple")
            .join("Lockdown")
    } else if is_macos {
        PathBuf::from("/var/db/lockdown")
    } else {
        PathBuf::from("/var/lib/lockdown")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pair_record_path_macos() {
        let path = pair_record_path_for_platform("ABC123DEF", true, false, "");
        assert_eq!(path, PathBuf::from("/var/db/lockdown/ABC123DEF.plist"));
    }

    #[test]
    fn test_pair_record_path_windows() {
        let path = pair_record_path_for_platform("ABC123DEF", false, true, "C:\\ProgramData");
        let s = path.to_string_lossy();
        assert!(s.contains("ABC123DEF"));
        assert!(s.contains("Apple"));
        assert!(s.contains("Lockdown"));
    }

    #[test]
    fn test_pair_record_path_linux() {
        let path = pair_record_path_for_platform("ABC123DEF", false, false, "");
        assert_eq!(path, PathBuf::from("/var/lib/lockdown/ABC123DEF.plist"));
    }

    #[test]
    fn test_pair_record_dir_windows() {
        let path = pair_record_dir_for_platform(false, true, "C:\\ProgramData");
        assert!(path.starts_with("C:\\ProgramData"));
        assert!(path.ends_with(PathBuf::from("Apple").join("Lockdown")));
    }
}

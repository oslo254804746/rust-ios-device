use std::path::PathBuf;

use serde::Deserialize;
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum PairRecordError {
    #[error("pair record not found for UDID: {0}")]
    NotFound(String),
    #[error("failed to read pair record {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
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
    /// DER/PEM-encoded host private key.
    ///
    /// Anyone holding these bytes can impersonate this host to the device, and
    /// the record stays alive behind an `Arc` for the whole session, so it is
    /// wiped once the last reference goes away. Derefs to `Vec<u8>`, so readers
    /// are unaffected.
    #[serde(deserialize_with = "deserialize_secret_bytes")]
    pub host_private_key: Zeroizing<Vec<u8>>,
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
    ///
    /// `rename_all = "PascalCase"` would derive `WifiMacAddress`, which never
    /// matches the key on disk — go-ios, pymobiledevice3 and this crate's own
    /// writer all use `WiFiMACAddress`, so the field silently stayed `None` and
    /// disabled Wi-Fi matching in discovery.
    #[serde(rename = "WiFiMACAddress")]
    pub wifi_mac_address: Option<String>,
}

/// Plists store key material as `<data>`, which only `serde_bytes` decodes into
/// a byte vector; wrapping happens here so the bytes are never held anywhere but
/// inside the guard.
fn deserialize_secret_bytes<'de, D>(deserializer: D) -> Result<Zeroizing<Vec<u8>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Zeroizing::new(serde_bytes::deserialize(deserializer)?))
}

impl PairRecord {
    /// Load from the platform default path.
    pub fn load(udid: &str) -> Result<Self, PairRecordError> {
        let path = default_pair_record_path(udid);
        Self::load_from_path(&path, udid)
    }

    /// Load from an explicit path.
    pub fn load_from_path(path: &std::path::Path, udid: &str) -> Result<Self, PairRecordError> {
        // The raw plist carries the private key too, so the read buffer gets the
        // same protection as the parsed field.
        let data = match std::fs::read(path) {
            Ok(data) => Zeroizing::new(data),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(PairRecordError::NotFound(udid.to_string()));
            }
            Err(source) => {
                return Err(PairRecordError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        plist::from_bytes(data.as_slice()).map_err(|e| PairRecordError::Parse(e.to_string()))
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

    const PAIR_RECORD_PLIST: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>DeviceCertificate</key><data>AQID</data>
    <key>HostCertificate</key><data>BAUG</data>
    <key>HostPrivateKey</key><data>BwgJ</data>
    <key>RootCertificate</key><data>CgsM</data>
    <key>HostID</key><string>HOST-ID</string>
    <key>SystemBUID</key><string>BUID</string>
</dict>
</plist>"#;

    #[test]
    fn load_from_path_decodes_plist_data_into_the_secret_wrapper() {
        // `HostPrivateKey` no longer goes through `#[serde(with = "serde_bytes")]`
        // now that it is wrapped, so pin the `<data>` decoding it replaced.
        let dir =
            std::env::temp_dir().join(format!("ios-rs-pair-record-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("UDID.plist");
        std::fs::write(&path, PAIR_RECORD_PLIST).unwrap();

        let record = PairRecord::load_from_path(&path, "UDID").unwrap();

        assert_eq!(record.host_private_key.as_slice(), &[7u8, 8, 9]);
        assert_eq!(record.device_certificate, vec![1u8, 2, 3]);
        assert_eq!(record.host_certificate, vec![4u8, 5, 6]);
        assert_eq!(record.root_certificate, vec![10u8, 11, 12]);
        assert_eq!(record.host_id, "HOST-ID");
        assert_eq!(record.system_buid, "BUID");
        assert!(record.wifi_mac_address.is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_path_preserves_non_missing_read_errors() {
        let dir =
            std::env::temp_dir().join(format!("ios-rs-pair-record-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let err = PairRecord::load_from_path(&dir, "UDID").unwrap_err();

        assert!(matches!(err, PairRecordError::Read { path, .. } if path == dir));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

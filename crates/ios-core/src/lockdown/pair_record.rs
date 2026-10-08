use std::path::PathBuf;

use serde::Deserialize;
use zeroize::Zeroizing;

pub(crate) const MAX_PAIR_RECORD_BYTES: u64 = 1024 * 1024;
#[cfg(feature = "mdns")]
pub(crate) const MAX_DISCOVERY_PAIR_RECORDS: usize = 128;

/// Shared bounded reader for local pairing stores. The original serialized
/// private keys are wiped when parsing finishes.
///
/// Only regular files are read; existing links to regular files remain valid.
/// Unix nonblocking open also protects the interval between the path and handle
/// checks from a FIFO replacement. This does not make filesystem metadata or
/// regular-file reads cancellable on a stalled filesystem.
pub(crate) fn read_pair_record_bytes(
    path: &std::path::Path,
) -> std::io::Result<Zeroizing<Vec<u8>>> {
    use std::io::Read;
    if !std::fs::metadata(path)?.is_file() {
        return Err(non_regular_pair_record());
    }
    let file = open_pair_record_file(path)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_PAIR_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PAIR_RECORD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "pair record exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn non_regular_pair_record() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "pair record path must be a regular file",
    )
}

fn open_pair_record_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(non_regular_pair_record());
    }
    Ok(file)
}

#[cfg(feature = "mdns")]
pub(crate) fn discovery_pair_record_paths(dir: &std::path::Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    // Bound directory work as well as parsed records. Skip links so enumeration
    // never expands the configured credential directory into other locations.
    let mut paths: Vec<_> = entries
        .take(4096)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "plist"))
        .collect();
    paths.sort();
    paths.truncate(MAX_DISCOVERY_PAIR_RECORDS);
    paths
}

#[derive(thiserror::Error)]
pub enum PairRecordError {
    #[error("pair record not found for device: {}", crate::redaction::diagnostic_id(.0))]
    NotFound(String),
    #[error("failed to read pair record ({:?})", .source.kind())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse pair record (invalid credential file)")]
    Parse(String),
}

impl std::fmt::Debug for PairRecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(identifier) => f
                .debug_tuple("PairRecordError::NotFound")
                .field(&crate::redaction::diagnostic_id(identifier))
                .finish(),
            Self::Read { source, .. } => f
                .debug_struct("PairRecordError::Read")
                .field("kind", &source.kind())
                .field("os_code", &source.raw_os_error())
                .finish_non_exhaustive(),
            Self::Parse(_) => f
                .debug_struct("PairRecordError::Parse")
                .finish_non_exhaustive(),
        }
    }
}

/// iOS device pair record, loaded from the platform-specific lockdown directory.
#[derive(Deserialize)]
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

impl std::fmt::Debug for PairRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairRecord").finish_non_exhaustive()
    }
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
    ///
    /// Read/parse failures retain their native source in `PairRecordError::Read`.
    /// The identifier argument remains for compatibility; the explicit path
    /// determines which record is loaded. Diagnostics omit both path and data.
    pub fn load_from_path(path: &std::path::Path, _udid: &str) -> Result<Self, PairRecordError> {
        // The raw plist carries the private key too, so the read buffer gets the
        // same protection as the parsed field.
        let data = read_pair_record_bytes(path).map_err(|source| PairRecordError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        plist::from_bytes(data.as_slice()).map_err(|error| PairRecordError::Read {
            path: path.to_path_buf(),
            source: crate::error::context::retained_io(
                std::io::ErrorKind::InvalidInput,
                "parse local pairing record plist",
                error,
            ),
        })
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
mod read_admission_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_record_error_diagnostics_hide_paths_identifiers_and_parser_data() {
        let secret = "synthetic-pair-record-private-material";
        let errors = [
            PairRecordError::NotFound(secret.into()),
            PairRecordError::Read {
                path: PathBuf::from(format!("/synthetic/{secret}.plist")),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, secret),
            },
            PairRecordError::Parse(secret.into()),
        ];
        for error in errors {
            let output = format!("{error} {error:?} {error:#?}");
            assert!(!output.contains(secret));
            assert!(output.contains("pair record"));
        }
        let read = PairRecordError::Read {
            path: PathBuf::from("synthetic-path"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        assert!(read.to_string().contains("PermissionDenied"));
        assert!(std::error::Error::source(&read).is_some());
    }

    #[test]
    fn pair_record_debug_hides_certificates_keys_and_identifiers() {
        let record = PairRecord {
            device_certificate: b"synthetic-device-certificate".to_vec(),
            host_certificate: b"synthetic-host-certificate".to_vec(),
            host_private_key: Zeroizing::new(b"synthetic-host-private-key".to_vec()),
            root_certificate: b"synthetic-root-certificate".to_vec(),
            host_id: "synthetic-host-identifier".into(),
            system_buid: "synthetic-system-identifier".into(),
            wifi_mac_address: Some("synthetic-mac-address".into()),
        };
        let output = format!("{record:?} {record:#?}");
        for secret in [
            "synthetic-host-identifier",
            "synthetic-system-identifier",
            "synthetic-mac-address",
        ] {
            assert!(!output.contains(secret));
        }
        for secret in [
            &record.device_certificate,
            &record.host_certificate,
            &*record.host_private_key,
            &record.root_certificate,
        ] {
            assert!(!output.contains(&format!("{secret:?}")));
            assert!(!output.contains(std::str::from_utf8(secret).unwrap()));
        }
        assert!(output.contains("PairRecord"));
    }

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

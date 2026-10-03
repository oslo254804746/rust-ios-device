use std::fmt;
use std::path::PathBuf;

use crate::services::device_link::{DeviceLinkClient, DeviceLinkError};

mod deadline;
mod host_fs;
mod protocol;
mod session;
pub use deadline::{DeviceOperation, DeviceOperationTimeout, DeviceOperationTimeoutPhase};
pub use host_fs::{
    backup_is_encrypted, backup_status_is_full, has_incremental_backup_metadata,
    initialize_backup_directory, load_backup_applications, should_do_full_backup,
    validate_backup_identifier,
};

mod filter;
pub use filter::{build_backup_filter, BackupFilter, BackupSelection, BackupSelectionRule};

#[cfg(feature = "backup2-manifest")]
mod crypto;
#[cfg(feature = "backup2-manifest")]
mod manifest;
#[cfg(feature = "backup2-manifest")]
mod mbdb;

#[cfg(feature = "backup2-manifest")]
pub use manifest::{
    extract_backup_file, list_backup_entries, patch_backup_directory, unback_backup,
    BackupManifestEntry, ExtractionResult, ManifestPatchResult,
};

pub const SERVICE_NAME: &str = "com.apple.mobilebackup2";
pub const RSD_SERVICE_NAME: &str = "com.apple.mobilebackup2.shim.remote";
pub const SUPPORTED_PROTOCOL_VERSIONS: [f64; 2] = [2.0, 2.1];
#[derive(Debug, Clone, PartialEq)]
pub struct VersionExchange {
    pub device_link_version: u64,
    pub protocol_version: f64,
    pub local_versions: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupDirectoryLayout {
    pub root: PathBuf,
    pub device_directory: PathBuf,
    pub target_identifier: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackupResult {
    pub layout: BackupDirectoryLayout,
    pub device_link_version: u64,
    pub protocol_version: f64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RestoreOptions<'a> {
    pub system: bool,
    pub reboot: bool,
    pub copy: bool,
    pub settings: bool,
    pub remove: bool,
    pub password: Option<&'a str>,
    pub source_identifier: Option<&'a str>,
}

/// Optional host-side policy applied while a device sends backup files.
///
/// `patch_manifest` and local extraction are enabled by the `backup2-manifest` feature. The
/// DeviceLink transfer filter itself is available in every build and never changes the wire
/// `MessageName` used by MobileBackup2.
#[derive(Clone, Default)]
pub struct BackupOptions {
    pub filter: Option<BackupFilter>,
    pub patch_manifest: bool,
    pub password: Option<String>,
}

impl fmt::Debug for RestoreOptions<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RestoreOptions")
            .field("system", &self.system)
            .field("reboot", &self.reboot)
            .field("copy", &self.copy)
            .field("settings", &self.settings)
            .field("remove", &self.remove)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("source_identifier", &self.source_identifier)
            .finish()
    }
}

impl fmt::Debug for BackupOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackupOptions")
            .field("filter", &self.filter)
            .field("patch_manifest", &self.patch_manifest)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Default for RestoreOptions<'_> {
    fn default() -> Self {
        Self {
            system: false,
            reboot: true,
            copy: false,
            settings: true,
            remove: false,
            password: None,
            source_identifier: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestoreResult {
    pub layout: BackupDirectoryLayout,
    pub device_link_version: u64,
    pub protocol_version: f64,
}

service_error!(
    Mobilebackup2Error,
    before {
    #[error("device link error: {0}")]
    DeviceLink(#[from] DeviceLinkError),
    },
    after {},
);

pub struct Mobilebackup2Client<S> {
    device_link: DeviceLinkClient<S>,
    // DeviceLink reports free space before asking the host to purge. Keeping the last pair lets
    // us turn the device's follow-up MBErrorDomain/105 into a useful diagnostic.
    reported_free_space: Option<u64>,
    required_free_space: Option<u64>,
    backup_filter: Option<BackupFilter>,
    discarded_files: Vec<PathBuf>,
    transfer_files: u64,
    transfer_bytes: u64,
}

impl<S> Mobilebackup2Client<S> {
    pub fn new(stream: S) -> Self {
        Self {
            device_link: DeviceLinkClient::new(stream),
            reported_free_space: None,
            required_free_space: None,
            backup_filter: None,
            discarded_files: Vec::new(),
            transfer_files: 0,
            transfer_bytes: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_option_debug_redacts_passwords() {
        let backup = BackupOptions {
            password: Some("backup-secret".into()),
            ..BackupOptions::default()
        };
        let restore = RestoreOptions {
            password: Some("restore-secret"),
            ..RestoreOptions::default()
        };
        let backup_debug = format!("{backup:?}");
        let restore_debug = format!("{restore:?}");
        assert!(!backup_debug.contains("backup-secret"));
        assert!(!restore_debug.contains("restore-secret"));
        assert!(backup_debug.contains("<redacted>"));
        assert!(restore_debug.contains("<redacted>"));
    }
}

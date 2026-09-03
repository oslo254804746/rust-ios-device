use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use std::time::SystemTime;

use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::warn;
#[cfg(feature = "backup2-manifest")]
use zeroize::Zeroizing;

use crate::services::device_link::{DeviceLinkClient, DeviceLinkError};

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
// Metadata plists are small control-plane documents. Bound them before invoking the plist
// decoder so a sparse or hostile backup cannot make parsing allocate without limit. Payload and
// Manifest.db data have separate, substantially larger limits in backup2::manifest.
const MAX_BACKUP_METADATA_BYTES: u64 = 16 * 1024 * 1024;

/// Built-in host-side file selections accepted by `backup --only`.
///
/// These names deliberately mirror pymobiledevice3. They are predicates used while
/// receiving a DeviceLink upload; they are not additional MobileBackup2 message names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupSelection {
    Bookmarks,
    CallHistory,
    Contacts,
    Messages,
    Sms,
    Whatsapp,
}

impl BackupSelection {
    pub const NAMES: [&'static str; 6] = [
        "bookmarks",
        "call_history",
        "contacts",
        "messages",
        "sms",
        "whatsapp",
    ];

    pub fn rules(self) -> &'static [BackupSelectionRule] {
        match self {
            Self::Bookmarks => &BOOKMARK_RULES,
            Self::CallHistory => &CALL_HISTORY_RULES,
            Self::Contacts => &CONTACT_RULES,
            Self::Messages => &MESSAGE_RULES,
            Self::Sms => &SMS_RULES,
            Self::Whatsapp => &WHATSAPP_RULES,
        }
    }
}

impl FromStr for BackupSelection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "bookmarks" => Ok(Self::Bookmarks),
            "call_history" | "call-history" => Ok(Self::CallHistory),
            "contacts" => Ok(Self::Contacts),
            "messages" => Ok(Self::Messages),
            "sms" => Ok(Self::Sms),
            "whatsapp" | "whats_app" | "whats-app" => Ok(Self::Whatsapp),
            _ => Err(format!(
                "unknown backup selection {value:?}; expected one of {}",
                Self::NAMES.join(", ")
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupSelectionRule {
    pub domain: &'static str,
    pub relative_path: &'static str,
    pub recursive: bool,
}

impl BackupSelectionRule {
    const fn new(domain: &'static str, relative_path: &'static str, recursive: bool) -> Self {
        Self {
            domain,
            relative_path,
            recursive,
        }
    }

    fn matches(&self, domain: &str, relative_path: &str) -> bool {
        self.domain == domain
            && (self.relative_path == relative_path
                || (self.recursive
                    && relative_path
                        .strip_prefix(self.relative_path)
                        .is_some_and(|suffix| suffix.starts_with('/'))))
    }

    fn matches_device_name(&self, device_name: &str) -> bool {
        let expected = self.relative_path.trim_end_matches('/');
        let exact = [
            format!("{}/{}", self.domain, expected),
            format!("{}-{}", self.domain, expected),
            expected.to_string(),
        ];
        exact.iter().any(|candidate| {
            device_name == candidate
                || (self.recursive && device_name.starts_with(&format!("{candidate}/")))
        }) || device_name.ends_with(&format!("/{expected}"))
            || (self.recursive && device_name.contains(&format!("/{expected}/")))
    }
}

static BOOKMARK_RULES: [BackupSelectionRule; 3] = [
    BackupSelectionRule::new("HomeDomain", "Library/Safari/Bookmarks.db", false),
    BackupSelectionRule::new("HomeDomain", "Library/Safari/Bookmarks.db-shm", false),
    BackupSelectionRule::new("HomeDomain", "Library/Safari/Bookmarks.db-wal", false),
];
static CALL_HISTORY_RULES: [BackupSelectionRule; 3] = [
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/CallHistoryDB/CallHistory.storedata",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/CallHistoryDB/CallHistory.storedata-shm",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/CallHistoryDB/CallHistory.storedata-wal",
        false,
    ),
];
static CONTACT_RULES: [BackupSelectionRule; 3] = [
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/AddressBook/AddressBook.sqlitedb",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/AddressBook/AddressBook.sqlitedb-shm",
        false,
    ),
    BackupSelectionRule::new(
        "HomeDomain",
        "Library/AddressBook/AddressBook.sqlitedb-wal",
        false,
    ),
];
static MESSAGE_RULES: [BackupSelectionRule; 2] = [
    BackupSelectionRule::new("HomeDomain", "Library/SMS", true),
    BackupSelectionRule::new("MediaDomain", "Library/SMS", true),
];
static SMS_RULES: [BackupSelectionRule; 1] = [BackupSelectionRule::new(
    "HomeDomain",
    "Library/SMS/sms.db",
    false,
)];
static WHATSAPP_RULES: [BackupSelectionRule; 6] = [
    BackupSelectionRule::new(
        "AppDomain-net.whatsapp.WhatsApp",
        "Documents/ChatStorage.sqlite",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomain-net.whatsapp.WhatsApp",
        "Documents/ChatStorage.sqlite-shm",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomain-net.whatsapp.WhatsApp",
        "Documents/ChatStorage.sqlite-wal",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite-shm",
        false,
    ),
    BackupSelectionRule::new(
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite-wal",
        false,
    ),
];

/// A bounded host-side filter applied to received backup entries.
#[derive(Debug, Clone)]
pub struct BackupFilter {
    selections: Vec<BackupSelection>,
    regexes: Vec<regex::Regex>,
}

impl BackupFilter {
    pub fn matches_device_name(&self, device_name: &str) -> bool {
        self.selections
            .iter()
            .flat_map(|selection| selection.rules())
            .any(|rule| rule.matches_device_name(device_name))
            || self.regexes.iter().any(|regex| regex.is_match(device_name))
    }

    pub fn matches_manifest_entry(&self, domain: &str, relative_path: &str) -> bool {
        self.selections
            .iter()
            .flat_map(|selection| selection.rules())
            .any(|rule| rule.matches(domain, relative_path))
            || self.regexes.iter().any(|regex| {
                candidate_names(domain, relative_path)
                    .iter()
                    .any(|name| regex.is_match(name))
            })
    }

    pub fn is_empty(&self) -> bool {
        self.selections.is_empty() && self.regexes.is_empty()
    }
}

/// Parse and validate the `--only` and `--only-regex` forms before a device session starts.
pub fn build_backup_filter(
    selections: &[String],
    regex_patterns: &[String],
) -> Result<Option<BackupFilter>, Mobilebackup2Error> {
    if selections.is_empty() && regex_patterns.is_empty() {
        return Ok(None);
    }
    if selections.len() + regex_patterns.len() > MAX_FILTER_TERMS {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup filter has too many terms ({}; max {MAX_FILTER_TERMS})",
            selections.len() + regex_patterns.len()
        )));
    }
    let selections = selections
        .iter()
        .map(|selection| BackupSelection::from_str(selection).map_err(Mobilebackup2Error::Protocol))
        .collect::<Result<Vec<_>, _>>()?;
    let regexes = regex_patterns
        .iter()
        .map(|pattern| {
            if pattern.len() > MAX_FILTER_PATTERN_BYTES {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup --only-regex pattern is too large ({} bytes; max {MAX_FILTER_PATTERN_BYTES})",
                    pattern.len()
                )));
            }
            regex::Regex::new(pattern).map_err(|error| {
                Mobilebackup2Error::Protocol(format!(
                    "invalid backup --only-regex pattern {pattern:?}: {error}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(BackupFilter {
        selections,
        regexes,
    }))
}

fn candidate_names<'a>(domain: &'a str, relative_path: &'a str) -> [String; 3] {
    [
        format!("{domain}/{relative_path}"),
        format!("{domain}-{relative_path}"),
        relative_path.to_string(),
    ]
}

const MAX_FILTER_TERMS: usize = 128;
const MAX_FILTER_PATTERN_BYTES: usize = 16 * 1024;

const FILE_TRANSFER_CODE_SUCCESS: u8 = 0x00; // Transfer completed successfully
const FILE_TRANSFER_CODE_LOCAL_ERROR: u8 = 0x06; // Local (host) file I/O error
const FILE_TRANSFER_CODE_FILE_DATA: u8 = 0x0c; // Payload contains file data chunk
const FILE_TRANSFER_CODE_REMOTE_ERROR: u8 = 0x0b; // Remote (device) reported an error
const BULK_OPERATION_ERROR: i64 = -13;
const PURGE_DISK_SPACE_ERROR: i64 = -1;
const PURGE_DISK_SPACE_ERROR_STRING: &str = "DLPurgeDiskSpace failed to purge";
// BackupAgent2 adds this amount to every purge request beyond the actual free-space threshold.
const PURGE_REQUEST_OVERSHOOT: u64 = 0x8000_0000;
const MB_ERROR_INSUFFICIENT_DISK_SPACE: u64 = 105;
const EMPTY_PARAMETER_STRING: &str = "___EmptyParameterString___";
// BackupAgent2 buffers one complete transfer frame before writing it to disk. Keep frames small
// enough for large incremental Manifest.db files to avoid exhausting the device process.
const DOWNLOAD_CHUNK_SIZE: usize = 32 * 1024;
const MAX_TRANSFER_ERROR_PREVIEW_SIZE: usize = 64 * 1024;
const MAX_DEVICE_TRANSFER_FILES: u64 = 1_000_000;
const MAX_DEVICE_TRANSFER_BYTES: u64 = 512 * 1024 * 1024 * 1024;
// Device-side Unback/Extract are long-running DeviceLink operations. Keep one
// bounded lifetime for the complete exchange, including version negotiation,
// so a peer that stops sending cannot leave a caller waiting forever.
const DEVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const INCREMENTAL_BACKUP_REQUIRED_FILES: &[&str] = &["Manifest.plist", "Status.plist"];
// 978_307_200 seconds = 2001-01-01T00:00:00Z Unix timestamp
// This is the Apple Core Data / NSDate epoch offset (seconds between Unix epoch and Apple epoch)
const APPLE_EPOCH_OFFSET: Duration = Duration::from_secs(978_307_200);

/// DeviceLink's plist status integers use the unsigned 64-bit representation used by
/// pymobiledevice3's `ctypes.c_uint64` conversion, including for negative protocol codes.
fn protocol_status_value(status_code: i64) -> plist::Value {
    plist::Value::Integer((status_code as u64).into())
}

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

impl<S> Mobilebackup2Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn version_exchange(&mut self) -> Result<VersionExchange, Mobilebackup2Error> {
        // A client can be reused for several operations in tests or by higher-level callers;
        // diagnostics from a previous DeviceLink session must not leak into the next one.
        self.reported_free_space = None;
        self.required_free_space = None;
        self.transfer_files = 0;
        self.transfer_bytes = 0;

        let device_link_version = self.device_link.version_exchange().await?;
        let local_versions = SUPPORTED_PROTOCOL_VERSIONS.to_vec();

        self.device_link
            .send_process_message(&HelloRequest {
                message_name: "Hello",
                supported_protocol_versions: local_versions.clone(),
            })
            .await?;

        let response = self.device_link.recv_process_message().await?;
        let error_code = response
            .get("ErrorCode")
            .and_then(plist_number_to_u64)
            .ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "backup2 hello response missing ErrorCode: {:?}",
                    redacted_protocol_dictionary(&response)
                ))
            })?;
        if error_code != 0 {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup2 hello returned ErrorCode={error_code}: {:?}",
                redacted_protocol_dictionary(&response)
            )));
        }

        let protocol_version = response
            .get("ProtocolVersion")
            .and_then(plist_number_to_f64)
            .ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "backup2 hello response missing ProtocolVersion: {:?}",
                    redacted_protocol_dictionary(&response)
                ))
            })?;
        if !local_versions.contains(&protocol_version) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup2 negotiated unsupported protocol version {protocol_version}"
            )));
        }

        Ok(VersionExchange {
            device_link_version,
            protocol_version,
            local_versions,
        })
    }

    pub async fn backup(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        full: bool,
        info_plist: &plist::Dictionary,
    ) -> Result<BackupResult, Mobilebackup2Error> {
        self.backup_with_options(
            backup_root,
            target_identifier,
            full,
            info_plist,
            BackupOptions::default(),
        )
        .await
    }

    /// Run the real MobileBackup2 `Backup` request with optional host-side selection policy.
    pub async fn backup_with_options(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        full: bool,
        info_plist: &plist::Dictionary,
        options: BackupOptions,
    ) -> Result<BackupResult, Mobilebackup2Error> {
        #[cfg(feature = "backup2-manifest")]
        let mut options = options;
        #[cfg(feature = "backup2-manifest")]
        let password = options.password.take().map(Zeroizing::new);
        validate_backup_identifier(target_identifier)?;
        if options.patch_manifest && options.filter.is_none() {
            return Err(Mobilebackup2Error::Protocol(
                "patching a backup manifest requires a file selection filter".into(),
            ));
        }
        #[cfg(not(feature = "backup2-manifest"))]
        if options.patch_manifest {
            return Err(Mobilebackup2Error::Protocol(
                "manifest patching requires the ios-core backup2-manifest feature".into(),
            ));
        }
        let version = self.version_exchange().await?;
        self.backup_filter = options.filter.clone();
        self.discarded_files.clear();
        let layout = {
            let root = backup_root.to_path_buf();
            let id = target_identifier.to_owned();
            let info = info_plist.clone();
            tokio::task::spawn_blocking(move || {
                initialize_backup_directory(&root, &id, &info, full || options.patch_manifest)
            })
            .await
            .map_err(|e| Mobilebackup2Error::Io(std::io::Error::other(e.to_string())))?
        }?;

        self.device_link
            .send_process_message(&BackupRequest {
                message_name: "Backup",
                target_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let session_result = self.finish_session(run_result).await;
        self.backup_filter = None;
        let discarded_files = std::mem::take(&mut self.discarded_files);
        let cleanup_layout = layout.clone();
        let cleanup_result =
            run_blocking(move || cleanup_discarded_files(&cleanup_layout, &discarded_files)).await;
        let _content = session_result?;
        cleanup_result?;

        #[cfg(feature = "backup2-manifest")]
        if options.patch_manifest {
            let root = layout.root.clone();
            let id = layout.target_identifier.clone();
            let filter = options.filter.clone().ok_or_else(|| {
                Mobilebackup2Error::Protocol("manifest patch filter unexpectedly missing".into())
            })?;
            run_blocking(move || {
                patch_backup_directory(&root, &id, &filter, password.as_deref().map(String::as_str))
            })
            .await?;
        }

        Ok(BackupResult {
            layout,
            device_link_version: version.device_link_version,
            protocol_version: version.protocol_version,
        })
    }

    pub async fn change_password(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        old_password: Option<&str>,
        new_password: Option<&str>,
    ) -> Result<(), Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        let _ = self.version_exchange().await?;
        let layout = {
            let root = backup_root.to_path_buf();
            let id = target_identifier.to_owned();
            run_blocking(move || create_runtime_layout(&root, &id)).await?
        };

        self.device_link
            .send_process_message(&ChangePasswordRequest {
                message_name: "ChangePassword",
                target_identifier,
                old_password,
                new_password,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let _ = self.finish_session(run_result).await?;
        Ok(())
    }

    pub async fn restore(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        options: RestoreOptions<'_>,
    ) -> Result<RestoreResult, Mobilebackup2Error> {
        let source_identifier = options.source_identifier.unwrap_or(target_identifier);
        validate_backup_identifier(target_identifier)?;
        validate_backup_identifier(source_identifier)?;
        let (layout, manifest) = {
            let root = backup_root.to_path_buf();
            let id = source_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                let layout = create_runtime_layout(&root, &id)?;
                let manifest = read_backup_dictionary(&metadata_file_path(
                    &layout.root,
                    &layout.target_identifier,
                    "Manifest.plist",
                )?)?;
                Ok((layout, manifest))
            })
            .await?
        };
        let password = if manifest
            .get("IsEncrypted")
            .and_then(plist_value_to_bool)
            .unwrap_or(false)
        {
            Some(options.password.ok_or_else(|| {
                Mobilebackup2Error::Protocol(
                    "backup is encrypted; restore requires a password".into(),
                )
            })?)
        } else {
            None
        };
        let version = self.version_exchange().await?;

        self.device_link
            .send_process_message(&RestoreRequest {
                message_name: "Restore",
                target_identifier,
                source_identifier,
                password,
                options: RestoreRequestOptions {
                    restore_should_reboot: options.reboot,
                    restore_dont_copy_backup: !options.copy,
                    restore_preserve_settings: options.settings,
                    restore_system_files: options.system,
                    remove_items_not_restored: options.remove,
                },
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let _ = self.finish_session(run_result).await?;
        Ok(RestoreResult {
            layout,
            device_link_version: version.device_link_version,
            protocol_version: version.protocol_version,
        })
    }

    pub async fn info(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
        let _ = self.version_exchange().await?;
        let layout_identifier = source_identifier.unwrap_or(target_identifier);
        let layout = {
            let root = backup_root.to_path_buf();
            let id = layout_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                create_runtime_layout(&root, &id)
            })
            .await?
        };

        self.device_link
            .send_process_message(&InfoRequest {
                message_name: "Info",
                target_identifier,
                source_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        self.finish_session(run_result).await
    }

    pub async fn list(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
        let _ = self.version_exchange().await?;
        let source_identifier = source_identifier.unwrap_or(target_identifier);
        let layout = {
            let root = backup_root.to_path_buf();
            let id = source_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                create_runtime_layout(&root, &id)
            })
            .await?
        };

        self.device_link
            .send_process_message(&ListRequest {
                message_name: "List",
                target_identifier,
                source_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        self.finish_session(run_result).await
    }

    /// Ask the device to expand a completed backup with MobileBackup2's real
    /// `Unback` operation. This is distinct from the host-side Manifest.db
    /// helper exposed behind the `backup2-manifest` feature.
    pub async fn unback(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        let deadline = tokio::time::Instant::now() + DEVICE_OPERATION_TIMEOUT;
        let result = match tokio::time::timeout_at(
            deadline,
            self.run_device_unback(backup_root, target_identifier, source_identifier, password),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Unback exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        };
        match tokio::time::timeout_at(deadline, self.finish_session(result)).await {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Unback exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn run_device_unback(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        let source_identifier = non_empty_protocol_string(source_identifier);
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
        let layout_identifier = source_identifier.unwrap_or(target_identifier);
        let layout = {
            let root = backup_root.to_path_buf();
            let id = layout_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                create_runtime_layout(&root, &id)
            })
            .await?
        };

        self.version_exchange().await?;
        self.device_link
            .send_process_message(&UnbackRequest {
                message_name: "Unback",
                target_identifier,
                source_identifier,
                password: non_empty_protocol_string(password),
            })
            .await?;
        self.run_loop(&layout).await
    }

    /// Ask the device to extract one domain/path from a completed backup with
    /// MobileBackup2's real `Extract` operation. `backup_root` is the local
    /// directory used by the DeviceLink file-transfer loop, not an output path.
    pub async fn extract(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        domain_name: &str,
        relative_path: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        let deadline = tokio::time::Instant::now() + DEVICE_OPERATION_TIMEOUT;
        let result = match tokio::time::timeout_at(
            deadline,
            self.run_device_extract(
                backup_root,
                target_identifier,
                domain_name,
                relative_path,
                source_identifier,
                password,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Extract exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        };
        match tokio::time::timeout_at(deadline, self.finish_session(result)).await {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Extract exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn run_device_extract(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        domain_name: &str,
        relative_path: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        // Domain names are single backup-domain components (HomeDomain,
        // AppDomain-* and so on). The relative path uses the same traversal
        // and platform-separator checks as DeviceLink file transfers.
        validate_backup_identifier(domain_name)?;
        let relative_path = sanitize_relative_path(relative_path)?;
        let relative_path = manifest_relative_path_string(&relative_path);
        let source_identifier = non_empty_protocol_string(source_identifier);
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
        let layout_identifier = source_identifier.unwrap_or(target_identifier);
        let layout = {
            let root = backup_root.to_path_buf();
            let id = layout_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                create_runtime_layout(&root, &id)
            })
            .await?
        };

        self.version_exchange().await?;
        self.device_link
            .send_process_message(&ExtractRequest {
                message_name: "Extract",
                target_identifier,
                domain_name,
                relative_path: &relative_path,
                source_identifier,
                password: non_empty_protocol_string(password),
            })
            .await?;
        self.run_loop(&layout).await
    }

    async fn disconnect_best_effort(&mut self) {
        if let Err(err) = self.device_link.disconnect().await {
            if !should_suppress_disconnect_error(&err) {
                warn!("backup2 disconnect failed: {err}");
            }
        }
    }

    async fn finish_session<T>(
        &mut self,
        result: Result<T, Mobilebackup2Error>,
    ) -> Result<T, Mobilebackup2Error> {
        self.disconnect_best_effort().await;
        result
    }

    async fn run_loop(
        &mut self,
        layout: &BackupDirectoryLayout,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        loop {
            let message = self.device_link.recv_message().await?;
            let parts = message.as_array().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "device link loop expected array message, got {:?}",
                    redacted_protocol_value(&message)
                ))
            })?;

            let command = parts
                .first()
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "device link message missing command: {:?}",
                        redacted_protocol_value(&message)
                    ))
                })?;
            match command {
                "DLMessageProcessMessage" => {
                    let payload = parts
                        .get(1)
                        .and_then(plist::Value::as_dictionary)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "process message missing dictionary payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let error_code = payload
                        .get("ErrorCode")
                        .and_then(plist_number_to_u64)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "backup process response missing numeric ErrorCode: {:?}",
                                redacted_protocol_dictionary(payload)
                            ))
                        })?;
                    if error_code != 0 {
                        let safe_payload = redacted_protocol_dictionary(payload);
                        if error_code == MB_ERROR_INSUFFICIENT_DISK_SPACE {
                            return Err(Mobilebackup2Error::Protocol(
                                insufficient_disk_space_message(
                                    &safe_payload,
                                    self.reported_free_space,
                                    self.required_free_space,
                                ),
                            ));
                        }
                        return Err(Mobilebackup2Error::Protocol(format!(
                            "backup process returned ErrorCode={error_code}: {safe_payload:?}"
                        )));
                    }
                    return Ok(payload.get("Content").cloned());
                }
                "DLMessageCreateDirectory" => {
                    let path = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "create directory missing path: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let directory = resolve_relative_path(layout, path)?;
                    create_layout_directory(layout, &directory)?;
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessageUploadFiles" => {
                    self.receive_uploaded_files(layout).await?;
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessageDownloadFiles" => {
                    let files = parts
                        .get(1)
                        .and_then(plist::Value::as_array)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "download files missing array payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let (status_code, status_message, status_payload) =
                        self.send_requested_files(layout, files).await?;
                    self.send_status_response(status_code, &status_message, status_payload)
                        .await?;
                }
                "DLMessageGetFreeDiskSpace" => {
                    let device_directory = layout.device_directory.clone();
                    let free_bytes =
                        run_blocking(move || available_space(&device_directory)).await?;
                    tracing::debug!(
                        free_bytes,
                        path = %layout.device_directory.display(),
                        "reporting available backup disk space"
                    );
                    self.reported_free_space = Some(free_bytes);
                    self.send_status_response(0, "", plist::Value::Integer(free_bytes.into()))
                        .await?;
                }
                "DLMessageMoveItems" | "DLMessageMoveFiles" => {
                    let items = parts
                        .get(1)
                        .and_then(plist::Value::as_dictionary)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "move items missing mapping payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    for (src, dst_value) in items {
                        let dst = dst_value.as_string().ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "move target for {src} was not a string: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                        let src_path = resolve_relative_path(layout, src)?;
                        let dst_path = resolve_relative_path(layout, dst)?;
                        if self.backup_filter.is_some()
                            && matches!(
                                fs::symlink_metadata(&src_path),
                                Err(error) if error.kind() == ErrorKind::NotFound
                            )
                        {
                            continue;
                        }
                        create_layout_parent_directory(layout, &dst_path)?;
                        let source_is_directory = fs::symlink_metadata(&src_path)?.is_dir();
                        rename_layout_path(layout, &src_path, &dst_path).await?;
                        self.relocate_discarded_files(&src_path, &dst_path, source_is_directory);
                    }
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessageRemoveItems" | "DLMessageRemoveFiles" => {
                    let items = parts
                        .get(1)
                        .and_then(plist::Value::as_array)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "remove items missing array payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    for item in items {
                        let rel = item.as_string().ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "remove item path was not a string: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                        let target = resolve_relative_path(layout, rel)?;
                        remove_layout_path(layout, &target).await?;
                        self.forget_discarded_files(&target);
                    }
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLContentsOfDirectory" => {
                    let rel = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "contents-of-directory missing path: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let path = resolve_relative_path(layout, rel)?;
                    let root = layout.root.clone();
                    let listing =
                        tokio::task::spawn_blocking(move || contents_of_directory(&root, &path))
                            .await
                            .map_err(|e| {
                                Mobilebackup2Error::Io(std::io::Error::other(e.to_string()))
                            })??;
                    self.send_status_response(0, "", plist::Value::Dictionary(listing))
                        .await?;
                }
                "DLMessageCopyItem" => {
                    let src = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "copy item missing source: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let dst = parts
                        .get(2)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "copy item missing destination: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let src_path = resolve_relative_path(layout, src)?;
                    let dst_path = resolve_relative_path(layout, dst)?;
                    if self.backup_filter.is_some()
                        && matches!(
                            fs::symlink_metadata(&src_path),
                            Err(error) if error.kind() == ErrorKind::NotFound
                        )
                    {
                        self.send_status_response(
                            0,
                            "",
                            plist::Value::Dictionary(plist::Dictionary::new()),
                        )
                        .await?;
                        continue;
                    }
                    let source_is_directory = fs::symlink_metadata(&src_path)?.is_dir();
                    let root = layout.root.clone();
                    let copy_source = src_path.clone();
                    let copy_destination = dst_path.clone();
                    tokio::task::spawn_blocking(move || {
                        copy_item(&root, &copy_source, &copy_destination)
                    })
                    .await
                    .map_err(|e| Mobilebackup2Error::Io(std::io::Error::other(e.to_string())))??;
                    self.copy_discarded_files(&src_path, &dst_path, source_is_directory);
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessagePurgeDiskSpace" => {
                    let requested = parts.get(1).and_then(plist_number_to_u64);
                    let urgency = parts.get(2).and_then(plist_number_to_u64);
                    self.required_free_space =
                        derive_required_free_space(self.reported_free_space, requested);
                    if let Some(required) = self.required_free_space {
                        let reported = self
                            .reported_free_space
                            .expect("derived free-space requirement has a report");
                        let additional = required.saturating_add(1).saturating_sub(reported);
                        tracing::warn!(
                            required_free_space = required,
                            reported_free_space = reported,
                            additional_free_space = additional,
                            requested,
                            urgency,
                            "device requested host disk-space purge; no purge backend is available"
                        );
                    } else {
                        tracing::warn!(
                            requested,
                            urgency,
                            "device requested host disk-space purge; no purge backend is available"
                        );
                    }
                    // Purging is a request, not a terminal protocol error. Apple's host replies
                    // with the purge-failed status and lets the device report its own diagnosis.
                    self.send_status_response_value(
                        protocol_status_value(PURGE_DISK_SPACE_ERROR),
                        PURGE_DISK_SPACE_ERROR_STRING,
                        plist::Value::Integer(0u64.into()),
                    )
                    .await?;
                }
                other => {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "unsupported backup device-link command {other}: {:?}",
                        redacted_protocol_value(&message)
                    )));
                }
            }
        }
    }

    fn account_transfer_file(&mut self) -> Result<(), Mobilebackup2Error> {
        self.transfer_files = self.transfer_files.checked_add(1).ok_or_else(|| {
            Mobilebackup2Error::Protocol("backup transfer file count overflow".into())
        })?;
        if self.transfer_files > MAX_DEVICE_TRANSFER_FILES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup transfer has too many files (max {MAX_DEVICE_TRANSFER_FILES})"
            )));
        }
        Ok(())
    }

    fn account_transfer_bytes(&mut self, bytes: usize) -> Result<(), Mobilebackup2Error> {
        let bytes = u64::try_from(bytes).map_err(|_| {
            Mobilebackup2Error::Protocol("backup transfer byte count does not fit in u64".into())
        })?;
        self.transfer_bytes = self.transfer_bytes.checked_add(bytes).ok_or_else(|| {
            Mobilebackup2Error::Protocol("backup transfer byte count overflow".into())
        })?;
        if self.transfer_bytes > MAX_DEVICE_TRANSFER_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup transfer exceeds byte budget (max {MAX_DEVICE_TRANSFER_BYTES} bytes)"
            )));
        }
        Ok(())
    }

    async fn receive_uploaded_files(
        &mut self,
        layout: &BackupDirectoryLayout,
    ) -> Result<(), Mobilebackup2Error> {
        loop {
            let device_name = read_prefixed_string(self.device_link.stream_mut()).await?;
            if device_name.is_empty() {
                break;
            }

            self.account_transfer_file()?;

            let file_name = read_prefixed_string(self.device_link.stream_mut()).await?;
            let output_path = resolve_relative_path(layout, &file_name)?;
            create_layout_parent_directory(layout, &output_path)?;
            let preserve = self.backup_filter.as_ref().map_or(true, |filter| {
                should_preserve_backup_file(&file_name, &device_name, filter)
            });
            let mut file = if preserve {
                Some(tokio::fs::File::from_std(open_layout_file_for_write(
                    layout,
                    &output_path,
                )?))
            } else {
                // BackupAgent2 may refer to a rejected file in a subsequent Move/Copy command.
                // A zero-byte placeholder keeps that protocol exchange valid; it is removed
                // only after the DeviceLink loop completes.
                let _ = open_layout_file_for_write(layout, &output_path)?;
                self.discarded_files.push(output_path.clone());
                None
            };

            loop {
                let frame_size = read_u32_be(self.device_link.stream_mut()).await?;
                let mut code = [0u8; 1];
                self.device_link.stream_mut().read_exact(&mut code).await?;
                let payload_len = frame_size.checked_sub(1).ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "backup file transfer frame too short for {file_name}"
                    ))
                })? as usize;

                match code[0] {
                    FILE_TRANSFER_CODE_FILE_DATA => {
                        self.account_transfer_bytes(payload_len)?;
                        // A device can advertise a very large frame size. Stream its payload in
                        // bounded pieces instead of allocating the complete frame up front.
                        if let Some(file) = file.as_mut() {
                            copy_transfer_payload(self.device_link.stream_mut(), file, payload_len)
                                .await?;
                        } else {
                            discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                                .await?;
                        }
                    }
                    FILE_TRANSFER_CODE_SUCCESS => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        break;
                    }
                    FILE_TRANSFER_CODE_REMOTE_ERROR => {
                        let message =
                            read_transfer_error_preview(self.device_link.stream_mut(), payload_len)
                                .await?;
                        warn!(
                            "backup upload for device path '{}' to local file '{}' reported remote error: {}",
                            device_name,
                            file_name,
                            message
                        );
                        break;
                    }
                    other => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        return Err(Mobilebackup2Error::Protocol(format!(
                            "unknown backup file transfer code 0x{other:02x} for {file_name}"
                        )));
                    }
                }
            }
            if let Some(file) = file.as_mut() {
                file.flush().await?;
            }
        }

        Ok(())
    }

    fn relocate_discarded_files(&mut self, source: &Path, destination: &Path, is_directory: bool) {
        let mut relocated = Vec::with_capacity(self.discarded_files.len());
        for path in self.discarded_files.drain(..) {
            let should_relocate = path == source || (is_directory && path.starts_with(source));
            if should_relocate {
                let suffix = path.strip_prefix(source).unwrap_or(Path::new(""));
                relocated.push(destination.join(suffix));
            } else {
                relocated.push(path);
            }
        }
        self.discarded_files = relocated;
    }

    fn copy_discarded_files(&mut self, source: &Path, destination: &Path, is_directory: bool) {
        let copied = self
            .discarded_files
            .iter()
            .filter(|path| *path == source || (is_directory && path.starts_with(source)))
            .map(|path| {
                let suffix = path.strip_prefix(source).unwrap_or(Path::new(""));
                destination.join(suffix)
            })
            .collect::<Vec<_>>();
        self.discarded_files.extend(copied);
    }

    fn forget_discarded_files(&mut self, target: &Path) {
        self.discarded_files
            .retain(|path| path != target && !path.starts_with(target));
    }

    async fn send_status_response(
        &mut self,
        status_code: i64,
        status_message: &str,
        status_payload: plist::Value,
    ) -> Result<(), Mobilebackup2Error> {
        self.send_status_response_value(
            protocol_status_value(status_code),
            status_message,
            status_payload,
        )
        .await
    }

    async fn send_status_response_value(
        &mut self,
        status_code: plist::Value,
        status_message: &str,
        status_payload: plist::Value,
    ) -> Result<(), Mobilebackup2Error> {
        self.device_link
            .send_message(&vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                status_code,
                plist::Value::String(
                    if status_message.is_empty() {
                        EMPTY_PARAMETER_STRING
                    } else {
                        status_message
                    }
                    .into(),
                ),
                status_payload,
            ])
            .await?;
        Ok(())
    }

    async fn send_requested_files(
        &mut self,
        layout: &BackupDirectoryLayout,
        files: &[plist::Value],
    ) -> Result<(i64, String, plist::Value), Mobilebackup2Error> {
        let mut failures = plist::Dictionary::new();
        for file in files {
            let rel = file.as_string().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "download file path was not a string: {file:?}"
                ))
            })?;
            let local_path = resolve_relative_path(layout, rel)?;
            self.account_transfer_file()?;
            write_prefixed_string(self.device_link.stream_mut(), rel).await?;

            match open_layout_file_for_read(layout, &local_path) {
                Ok(file) => {
                    let mut file = tokio::fs::File::from_std(file);
                    let mut buf = vec![0u8; DOWNLOAD_CHUNK_SIZE];
                    let mut read_failed = false;
                    loop {
                        let n = match file.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(err) => {
                                insert_file_failure(&mut failures, rel, &err);
                                write_transfer_frame(
                                    self.device_link.stream_mut(),
                                    FILE_TRANSFER_CODE_LOCAL_ERROR,
                                    err.to_string().as_bytes(),
                                )
                                .await?;
                                read_failed = true;
                                break;
                            }
                        };
                        self.account_transfer_bytes(n)?;
                        write_transfer_frame(
                            self.device_link.stream_mut(),
                            FILE_TRANSFER_CODE_FILE_DATA,
                            &buf[..n],
                        )
                        .await?;
                    }
                    if !read_failed {
                        write_transfer_frame(
                            self.device_link.stream_mut(),
                            FILE_TRANSFER_CODE_SUCCESS,
                            &[],
                        )
                        .await?;
                    }
                }
                Err(Mobilebackup2Error::Io(err)) => {
                    insert_file_failure(&mut failures, rel, &err);
                    write_transfer_frame(
                        self.device_link.stream_mut(),
                        FILE_TRANSFER_CODE_LOCAL_ERROR,
                        err.to_string().as_bytes(),
                    )
                    .await?;
                }
                Err(err) => return Err(err),
            }
        }

        self.device_link
            .stream_mut()
            .write_all(&0u32.to_be_bytes())
            .await?;
        self.device_link.stream_mut().flush().await?;
        if failures.is_empty() {
            Ok((
                0,
                String::new(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ))
        } else {
            Ok((
                BULK_OPERATION_ERROR,
                "Multi status".to_string(),
                plist::Value::Dictionary(failures),
            ))
        }
    }
}

fn should_preserve_backup_file(file_name: &str, device_name: &str, filter: &BackupFilter) -> bool {
    let metadata_name = file_name.rsplit('/').next().unwrap_or(file_name);
    matches!(
        metadata_name,
        "Info.plist"
            | "Manifest.plist"
            | "Manifest.db"
            | "Manifest.mbdb"
            | "Manifest.mbdx"
            | "Manifest.db-shm"
            | "Manifest.db-wal"
            | "Status.plist"
    ) || filter.matches_device_name(device_name)
}

fn cleanup_discarded_files(
    layout: &BackupDirectoryLayout,
    paths: &[PathBuf],
) -> Result<(), Mobilebackup2Error> {
    for path in paths {
        let relative = layout_relative_path(layout, path)?;
        ensure_no_symlink_components_at_root(&layout.root, &relative)?;
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(path));
        }
        if metadata.is_dir() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "filtered backup placeholder unexpectedly became a directory: {}",
                path.display()
            )));
        }
        fs::remove_file(path)?;

        // Remove only empty placeholder directories, stopping at the device directory. Existing
        // user/backup data is never recursively removed by this cleanup pass.
        let mut parent = path.parent().map(Path::to_path_buf);
        while let Some(directory) = parent {
            if directory == layout.device_directory
                || !directory.starts_with(&layout.device_directory)
            {
                break;
            }
            ensure_no_symlink_components_at_root(
                &layout.root,
                &layout_relative_path(layout, &directory)?,
            )?;
            let mut entries = fs::read_dir(&directory)?;
            if entries.next().is_some() {
                break;
            }
            fs::remove_dir(&directory)?;
            parent = directory.parent().map(Path::to_path_buf);
        }
    }
    Ok(())
}

pub fn initialize_backup_directory(
    backup_root: &Path,
    target_identifier: &str,
    info_plist: &plist::Dictionary,
    full: bool,
) -> Result<BackupDirectoryLayout, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    reject_symlink_components(backup_root)?;
    fs::create_dir_all(backup_root)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = create_dir_all_no_symlink(&root, Path::new(target_identifier))?;
    for file_name in ["Info.plist", "Status.plist", "Manifest.plist"] {
        validate_seed_file_path(&device_directory.join(file_name))?;
    }
    let full = should_do_full_backup(full, &device_directory)?;

    let mut info_file = open_file_for_write(&device_directory.join("Info.plist"))?;
    plist::to_writer_xml(
        &mut info_file,
        &plist::Value::Dictionary(info_plist.clone()),
    )?;

    let status_path = device_directory.join("Status.plist");
    let status_missing = matches!(
        fs::symlink_metadata(&status_path),
        Err(error) if error.kind() == ErrorKind::NotFound
    );
    // Apple keeps the previous Status.plist for an incremental backup. Replacing it here would
    // discard the device's incremental state (and differs from pymobiledevice3's
    // `if full or not status_path.exists()` behavior).
    if full || status_missing {
        let status = plist::Dictionary::from_iter([
            (
                "BackupState".to_string(),
                plist::Value::String("new".into()),
            ),
            (
                "Date".to_string(),
                plist::Value::Date(plist::Date::from(SystemTime::now())),
            ),
            ("IsFullBackup".to_string(), plist::Value::Boolean(full)),
            ("Version".to_string(), plist::Value::String("3.3".into())),
            (
                "SnapshotState".to_string(),
                plist::Value::String("finished".into()),
            ),
            (
                "UUID".to_string(),
                plist::Value::String(generate_backup_uuid()),
            ),
        ]);
        let mut status_file = open_file_for_write(&status_path)?;
        plist::to_writer_binary(&mut status_file, &plist::Value::Dictionary(status))?;
    }

    let manifest_path = device_directory.join("Manifest.plist");
    let create_manifest = match fs::symlink_metadata(&manifest_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(symlink_path_error(&manifest_path));
        }
        Ok(_) if full => {
            fs::remove_file(&manifest_path)?;
            true
        }
        Ok(_) => false,
        Err(error) if error.kind() == ErrorKind::NotFound => true,
        Err(error) => return Err(error.into()),
    };
    // `touch` the manifest only when it is new or a full backup removed it. In incremental mode
    // its existing plist is part of the backup state and must not be truncated.
    if create_manifest {
        let _ = open_file_for_write(&manifest_path)?;
    }

    Ok(BackupDirectoryLayout {
        root,
        device_directory,
        target_identifier: target_identifier.to_string(),
    })
}

fn validate_seed_file_path(path: &Path) -> Result<(), Mobilebackup2Error> {
    reject_symlink_components(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(symlink_path_error(path)),
        Ok(metadata) if !metadata.is_file() => Err(Mobilebackup2Error::Protocol(format!(
            "backup seed path is not a regular file: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn should_do_full_backup(
    full: bool,
    device_directory: &Path,
) -> Result<bool, Mobilebackup2Error> {
    Ok(full || !has_incremental_backup_metadata(device_directory)?)
}

pub fn has_incremental_backup_metadata(
    device_directory: &Path,
) -> Result<bool, Mobilebackup2Error> {
    for filename in INCREMENTAL_BACKUP_REQUIRED_FILES {
        let path = device_directory.join(filename);
        reject_symlink_components(&path)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&path));
        }
        if !metadata.is_file() || metadata.len() == 0 {
            return Ok(false);
        }
    }
    // iOS 10.2 and older backups use the flat Manifest.mbdb index.  A backup is
    // incrementally usable when exactly either manifest form is present; do not
    // require the modern SQLite name and accidentally turn every legacy backup
    // into a full backup.
    let mut manifest_found = false;
    for filename in ["Manifest.db", "Manifest.mbdb"] {
        let path = device_directory.join(filename);
        reject_symlink_components(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(symlink_path_error(&path));
            }
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => manifest_found = true,
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(manifest_found)
}

pub fn backup_status_is_full(device_directory: &Path) -> Result<bool, Mobilebackup2Error> {
    let status_path = safe_file_in_directory(device_directory, "Status.plist")?;
    let status = read_backup_dictionary(&status_path)?;
    Ok(status
        .get("IsFullBackup")
        .and_then(plist_value_to_bool)
        .unwrap_or(false))
}

/// Run a synchronous filesystem step off the runtime's worker threads.
///
/// The helpers below stat, create and parse files on disk; calling them
/// directly from an async fn blocks the reactor, which the `backup()` path
/// already avoids via `spawn_blocking`.
async fn run_blocking<T, F>(op: F) -> Result<T, Mobilebackup2Error>
where
    F: FnOnce() -> Result<T, Mobilebackup2Error> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| Mobilebackup2Error::Io(std::io::Error::other(err.to_string())))?
}

fn create_runtime_layout(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<BackupDirectoryLayout, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    reject_symlink_components(backup_root)?;
    fs::create_dir_all(backup_root)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = create_dir_all_no_symlink(&root, Path::new(target_identifier))?;
    Ok(BackupDirectoryLayout {
        root,
        device_directory,
        target_identifier: target_identifier.to_string(),
    })
}

fn ensure_backup_directory(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<(), Mobilebackup2Error> {
    let root = canonical_backup_root(backup_root)?;
    validate_backup_identifier(target_identifier)?;
    let device_directory = safe_path_from_root(&root, Path::new(target_identifier))?;
    for file_name in ["Info.plist", "Manifest.plist", "Status.plist"] {
        let path = device_directory.join(file_name);
        let relative = Path::new(target_identifier).join(file_name);
        ensure_no_symlink_components_at_root(&root, &relative)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup directory missing required file {}",
                    path.display()
                )));
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&path));
        }
        if !metadata.is_file() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup directory required path is not a file {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub fn load_backup_applications(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<Option<plist::Value>, Mobilebackup2Error> {
    ensure_backup_directory(backup_root, target_identifier)?;
    let info_path = metadata_file_path(backup_root, target_identifier, "Info.plist")?;
    let info = plist::Value::from_reader(open_backup_metadata_for_read(&info_path)?)?;
    Ok(info
        .as_dictionary()
        .and_then(|dict| dict.get("Applications"))
        .cloned())
}

pub fn backup_is_encrypted(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<bool, Mobilebackup2Error> {
    ensure_backup_directory(backup_root, target_identifier)?;
    let manifest_path = metadata_file_path(backup_root, target_identifier, "Manifest.plist")?;
    Ok(read_backup_dictionary(&manifest_path)?
        .get("IsEncrypted")
        .and_then(plist_value_to_bool)
        .unwrap_or(false))
}

fn read_backup_dictionary(path: &Path) -> Result<plist::Dictionary, Mobilebackup2Error> {
    plist::Value::from_reader(open_backup_metadata_for_read(path)?)?
        .into_dictionary()
        .ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "expected plist dictionary in backup metadata file {}",
                path.display()
            ))
        })
}

fn open_backup_metadata_for_read(path: &Path) -> Result<File, Mobilebackup2Error> {
    let file = open_file_for_read(path)?;
    let length = file.metadata()?.len();
    if length > MAX_BACKUP_METADATA_BYTES {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup metadata file is too large ({} bytes; max {MAX_BACKUP_METADATA_BYTES}): {}",
            length,
            path.display()
        )));
    }
    Ok(file)
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct HelloRequest {
    message_name: &'static str,
    supported_protocol_versions: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct BackupRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct RestoreRequestOptions {
    restore_should_reboot: bool,
    restore_dont_copy_backup: bool,
    restore_preserve_settings: bool,
    restore_system_files: bool,
    remove_items_not_restored: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct RestoreRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    source_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    options: RestoreRequestOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ChangePasswordRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_password: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct InfoRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_identifier: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ListRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    source_identifier: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct UnbackRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_identifier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ExtractRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    domain_name: &'a str,
    relative_path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_identifier: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
}

// A fresh random UUID is generated for each backup session.
// Backup UUIDs are not required to be deterministic across sessions.
fn generate_backup_uuid() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

/// Validate an identifier before it is ever used as a child of the backup root.
///
/// Device and source identifiers identify one backup directory, not a path.
/// Treating them as a single normal component keeps the contract explicit on
/// Unix and Windows alike; the explicit separator checks are needed because a
/// Windows-style path is a normal filename component on Unix.
pub fn validate_backup_identifier(identifier: &str) -> Result<(), Mobilebackup2Error> {
    if identifier.is_empty() {
        return Err(Mobilebackup2Error::Protocol(
            "backup identifier must not be empty".into(),
        ));
    }
    if identifier.as_bytes().contains(&0) {
        return Err(Mobilebackup2Error::Protocol(
            "backup identifier must not contain NUL".into(),
        ));
    }
    if identifier.contains('/') || identifier.contains('\\') {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup identifier must be one path component: {identifier:?}"
        )));
    }
    if has_windows_drive_prefix(identifier) {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup identifier must not contain a platform path prefix: {identifier:?}"
        )));
    }

    let mut components = Path::new(identifier).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(Mobilebackup2Error::Protocol(format!(
            "backup identifier must be one normal path component: {identifier:?}"
        ))),
    }
}

fn non_empty_protocol_string(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn sanitize_relative_path(path: &str) -> Result<PathBuf, Mobilebackup2Error> {
    if path.is_empty() {
        return Err(Mobilebackup2Error::Protocol(
            "backup relative path must not be empty".into(),
        ));
    }
    if path.as_bytes().contains(&0) {
        return Err(Mobilebackup2Error::Protocol(
            "backup relative path must not contain NUL".into(),
        ));
    }
    if path.contains('\\') || has_windows_drive_prefix(path) {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup relative path uses an unsupported platform separator or prefix: {path:?}"
        )));
    }

    let mut clean = PathBuf::new();
    let mut saw_component = false;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                clean.push(part);
                saw_component = true;
            }
            Component::CurDir => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup relative path contains a current-directory component: {path:?}"
                )));
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path escapes backup root: {path}"
                )));
            }
        }
    }

    if !saw_component {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup relative path must contain a normal component: {path:?}"
        )));
    }

    Ok(clean)
}

/// Render a sanitized relative path with the device's `/` separators.
///
/// `PathBuf` displays with `\` on Windows, which would corrupt comparisons
/// against manifest paths, backup filters, and DeviceLink transfer names:
/// those always use `/` on the wire.
pub(crate) fn manifest_relative_path_string(path: &Path) -> String {
    let mut rendered = String::new();
    for component in path.components() {
        if let Component::Normal(part) = component {
            if !rendered.is_empty() {
                rendered.push('/');
            }
            rendered.push_str(&part.to_string_lossy());
        }
    }
    rendered
}

fn resolve_relative_path(
    layout: &BackupDirectoryLayout,
    rel: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let clean = sanitize_relative_path(rel)?;
    validate_backup_identifier(&layout.target_identifier)?;
    let prefixed_with_target = clean
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        == Some(layout.target_identifier.as_str());

    let relative = if prefixed_with_target {
        clean
    } else {
        let mut relative = PathBuf::from(&layout.target_identifier);
        relative.push(clean);
        relative
    };
    let root = canonical_backup_root(&layout.root)?;
    ensure_no_symlink_components_at_root(&root, &relative)?;
    Ok(root.join(&relative))
}

fn canonical_backup_root(root: &Path) -> Result<PathBuf, Mobilebackup2Error> {
    reject_symlink_components(root)?;
    canonicalize_simplified(root).map_err(Mobilebackup2Error::Io)
}

/// Canonicalize a path and drop Windows' verbatim prefix when the plain form
/// still resolves.
///
/// `fs::canonicalize` returns a verbatim `\\?\C:\...` path on Windows.  The
/// extended-length form is functionally equivalent, but leaking it into
/// user-visible output (unback directories, resolved entries) and textual
/// comparisons is surprising, so the prefix is stripped only when the plain
/// path canonicalizes to exactly the same object. Paths that only exist in
/// extended-length form (very long paths) keep the prefix and keep working.
pub(crate) fn canonicalize_simplified(path: &Path) -> std::io::Result<PathBuf> {
    let canonical = fs::canonicalize(path)?;
    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let canonical_wide: Vec<u16> = canonical.as_os_str().encode_wide().collect();
        let verbatim_unc_prefix: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
        let verbatim_prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
        let simplified_wide = if canonical_wide.starts_with(&verbatim_unc_prefix) {
            let mut value = vec![b'\\' as u16, b'\\' as u16];
            value.extend_from_slice(&canonical_wide[verbatim_unc_prefix.len()..]);
            Some(value)
        } else if canonical_wide.starts_with(&verbatim_prefix) {
            Some(canonical_wide[verbatim_prefix.len()..].to_vec())
        } else {
            None
        };
        if let Some(simplified_wide) = simplified_wide {
            let simplified = PathBuf::from(OsString::from_wide(&simplified_wide));
            if let Ok(simplified_canonical) = fs::canonicalize(&simplified) {
                if simplified_canonical == canonical {
                    return Ok(simplified);
                }
            }
        }
    }
    Ok(canonical)
}

fn symlink_path_error(path: &Path) -> Mobilebackup2Error {
    Mobilebackup2Error::Protocol(format!(
        "backup path contains a symlink component: {}",
        path.display()
    ))
}

/// macOS keeps a few historical top-level paths as root-owned aliases into
/// `/private`.  They are part of the host's normal spelling for temporary and
/// system directories (including the value returned by `temp_dir`), rather
/// than user-controlled redirects.  Accept only the exact aliases and exact
/// relative targets; all other symlink components remain rejected.
fn is_approved_system_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let target = match fs::read_link(path) {
            Ok(target) => target,
            Err(_) => return false,
        };
        return [
            (Path::new("/var"), Path::new("private/var")),
            (Path::new("/tmp"), Path::new("private/tmp")),
            (Path::new("/etc"), Path::new("private/etc")),
        ]
        .into_iter()
        .any(|(alias, expected)| path == alias && target == expected);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

fn ensure_no_symlink_components_at_root(
    root: &Path,
    relative: &Path,
) -> Result<(), Mobilebackup2Error> {
    let mut current = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let part = match component {
            Component::Normal(part) => part,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path is not a safe relative path: {}",
                    relative.display()
                )));
            }
        };
        current.push(part);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&current));
        }
        if index + 1 < component_count && !metadata.is_dir() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup path component is not a directory: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn safe_path_from_root(root: &Path, relative: &Path) -> Result<PathBuf, Mobilebackup2Error> {
    ensure_no_symlink_components_at_root(root, relative)?;
    Ok(root.join(relative))
}

fn create_dir_all_no_symlink(root: &Path, relative: &Path) -> Result<PathBuf, Mobilebackup2Error> {
    let root = canonical_backup_root(root)?;
    let mut current = root.clone();
    for component in relative.components() {
        let part = match component {
            Component::Normal(part) => part,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup directory path is not safe: {}",
                    relative.display()
                )));
            }
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(symlink_path_error(&current));
                }
                if !metadata.is_dir() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "backup directory path is not a directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = fs::symlink_metadata(&current)?;
                if metadata.file_type().is_symlink() {
                    return Err(symlink_path_error(&current));
                }
                if !metadata.is_dir() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "backup directory path is not a directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) => return Err(error.into()),
        }

        let canonical = canonicalize_simplified(&current)?;
        if !canonical.starts_with(&root) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup directory path escapes backup root: {}",
                current.display()
            )));
        }
    }
    Ok(current)
}

fn reject_symlink_components(path: &Path) -> Result<(), Mobilebackup2Error> {
    let mut current = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    // A Windows prefix alone (for example `\\?\C:` from a canonicalized
    // verbatim path or `C:` from a drive-relative path) is not a complete root:
    // Win32 rejects metadata queries on it with ERROR_INVALID_FUNCTION.  The
    // prefix can never be a symlink, so it is accumulated without a metadata
    // check and the walk resumes once the full root or a real component has
    // been appended.
    for component in path.components() {
        match component {
            Component::Normal(part) => current.push(part),
            Component::Prefix(_) => {
                current.push(component.as_os_str());
                continue;
            }
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path is not safe: {}",
                    path.display()
                )));
            }
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                if !is_approved_system_alias(&current) {
                    return Err(symlink_path_error(&current));
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Open a backup file without following a final symlink. Unix uses the
/// kernel's `O_NOFOLLOW`; other platforms retain the component metadata check
/// above as the portable fallback.
fn open_file_for_read(path: &Path) -> Result<File, Mobilebackup2Error> {
    reject_symlink_components(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(Mobilebackup2Error::Io)
}

fn open_file_for_write(path: &Path) -> Result<File, Mobilebackup2Error> {
    reject_symlink_components(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Backup metadata and incoming payloads can contain personal data. Keep newly created
        // files private; extraction widens permissions only after an atomic replacement.
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(Mobilebackup2Error::Io)
}

fn metadata_file_path(
    backup_root: &Path,
    target_identifier: &str,
    file_name: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    let root = canonical_backup_root(backup_root)?;
    let relative = Path::new(target_identifier).join(file_name);
    let path = safe_path_from_root(&root, &relative)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_file() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup metadata path is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn layout_relative_path(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<PathBuf, Mobilebackup2Error> {
    path.strip_prefix(&layout.root)
        .map(PathBuf::from)
        .map_err(|_| {
            Mobilebackup2Error::Protocol(format!(
                "backup path is outside backup root: {}",
                path.display()
            ))
        })
}

fn create_layout_directory(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    create_dir_all_no_symlink(&layout.root, &relative)?;
    Ok(())
}

fn create_layout_parent_directory(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let parent = path.parent().ok_or_else(|| {
        Mobilebackup2Error::Protocol(format!(
            "backup path has no parent directory: {}",
            path.display()
        ))
    })?;
    let relative = layout_relative_path(layout, parent)?;
    create_dir_all_no_symlink(&layout.root, &relative)?;
    Ok(())
}

fn open_layout_file_for_write(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<File, Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    open_file_for_write(path)
}

fn open_layout_file_for_read(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<File, Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    open_file_for_read(path)
}

async fn rename_layout_path(
    layout: &BackupDirectoryLayout,
    src: &Path,
    dst: &Path,
) -> Result<(), Mobilebackup2Error> {
    let src_relative = layout_relative_path(layout, src)?;
    let dst_relative = layout_relative_path(layout, dst)?;
    ensure_no_symlink_components_at_root(&layout.root, &src_relative)?;
    ensure_no_symlink_components_at_root(&layout.root, &dst_relative)?;
    tokio::fs::rename(src, dst).await?;
    Ok(())
}

fn ensure_tree_has_no_symlinks(path: &Path) -> Result<(), Mobilebackup2Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(path));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            ensure_tree_has_no_symlinks(&entry?.path())?;
        }
    }
    Ok(())
}

async fn remove_layout_path(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(path));
    }
    if metadata.is_dir() {
        ensure_tree_has_no_symlinks(path)?;
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

fn copy_item(root: &Path, src: &Path, dst: &Path) -> Result<(), Mobilebackup2Error> {
    let src_relative = src.strip_prefix(root).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "backup copy source is outside backup root: {}",
            src.display()
        ))
    })?;
    let dst_relative = dst.strip_prefix(root).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "backup copy destination is outside backup root: {}",
            dst.display()
        ))
    })?;
    ensure_no_symlink_components_at_root(root, src_relative)?;
    ensure_no_symlink_components_at_root(root, dst_relative)?;
    if src == dst || dst.starts_with(src) {
        return Err(Mobilebackup2Error::Protocol(
            "cannot copy a directory into itself".into(),
        ));
    }

    let source_metadata = fs::symlink_metadata(src)?;
    if source_metadata.file_type().is_symlink() {
        return Err(symlink_path_error(src));
    }
    if source_metadata.is_dir() {
        create_dir_all_no_symlink(root, dst_relative)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_item(root, &entry.path(), &dst.join(entry.file_name()))?;
        }
    } else if source_metadata.is_file() {
        if let Some(parent) = dst.parent() {
            let parent_relative = parent.strip_prefix(root).map_err(|_| {
                Mobilebackup2Error::Protocol(format!(
                    "backup copy destination parent is outside backup root: {}",
                    parent.display()
                ))
            })?;
            create_dir_all_no_symlink(root, parent_relative)?;
        }
        let mut source = open_file_for_read(src)?;
        let mut destination = open_file_for_write(dst)?;
        std::io::copy(&mut source, &mut destination)?;
    } else {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup copy source is neither a file nor a directory: {}",
            src.display()
        )));
    }

    Ok(())
}

fn contents_of_directory(
    root: &Path,
    path: &Path,
) -> Result<plist::Dictionary, Mobilebackup2Error> {
    let relative = path.strip_prefix(root).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "backup directory is outside backup root: {}",
            path.display()
        ))
    })?;
    ensure_no_symlink_components_at_root(root, relative)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(path));
    }
    if !metadata.is_dir() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup directory path is not a directory: {}",
            path.display()
        )));
    }

    let mut entries = plist::Dictionary::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path)?;
        let file_type = if metadata.file_type().is_symlink() {
            "DLFileTypeUnknown"
        } else if metadata.is_dir() {
            "DLFileTypeDirectory"
        } else if metadata.is_file() {
            "DLFileTypeRegular"
        } else {
            "DLFileTypeUnknown"
        };
        let modified = metadata.modified().unwrap_or_else(|err| {
            tracing::debug!("cannot read mtime for {}: {err}", entry_path.display());
            SystemTime::UNIX_EPOCH
        });
        entries.insert(
            entry.file_name().to_string_lossy().into_owned(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "DLFileType".to_string(),
                    plist::Value::String(file_type.into()),
                ),
                (
                    "DLFileSize".to_string(),
                    plist::Value::Integer(metadata.len().into()),
                ),
                (
                    "DLFileModificationDate".to_string(),
                    plist::Value::Date(device_link_modification_date(modified)),
                ),
            ])),
        );
    }

    Ok(entries)
}

/*
 * Keep this helper adjacent to the path policy so every file operation above
 * goes through the same root and symlink checks. The protocol permits nested
 * relative paths, but never permits them to become an alternate root.
 */
fn safe_file_in_directory(
    directory: &Path,
    file_name: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let path = directory.join(file_name);
    reject_symlink_components(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_file() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup metadata path is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn device_link_modification_date(modified: SystemTime) -> plist::Date {
    // pymobiledevice3 encodes directory mtimes as local wall-clock time relative to Apple's
    // 2001 epoch, then serializes that wall-clock timestamp as if it were UTC.  plistlib's XML
    // serializer drops subsecond precision, so truncate before constructing the Date to keep
    // the DeviceLink wire representation identical (iOS 15 rejects fractional listing dates).
    let modified = device_link_local_wall_clock(modified);
    let shifted = modified
        .checked_sub(APPLE_EPOCH_OFFSET)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let shifted = truncate_system_time_to_seconds(shifted);
    plist::Date::from(shifted)
}

fn truncate_system_time_to_seconds(time: SystemTime) -> SystemTime {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => SystemTime::UNIX_EPOCH + Duration::from_secs(duration.as_secs()),
        Err(error) => {
            let duration = error.duration();
            let seconds = duration
                .as_secs()
                .saturating_add(if duration.subsec_nanos() == 0 { 0 } else { 1 });
            SystemTime::UNIX_EPOCH
                .checked_sub(Duration::from_secs(seconds))
                .unwrap_or(SystemTime::UNIX_EPOCH)
        }
    }
}

fn device_link_local_wall_clock(modified: SystemTime) -> SystemTime {
    let utc = OffsetDateTime::from(modified);
    let local_offset = UtcOffset::local_offset_at(utc).unwrap_or(UtcOffset::UTC);
    let local_wall_clock = utc.to_offset(local_offset).replace_offset(UtcOffset::UTC);
    local_wall_clock.into()
}

/// Maximum size for a length-prefixed string (64 KiB). Device names and file
/// paths are never anywhere near this limit; the guard protects against
/// corrupted or malicious size fields causing unbounded allocation.
const MAX_PREFIXED_STRING_SIZE: usize = 64 * 1024;
async fn read_prefixed_string<S>(stream: &mut S) -> Result<String, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let size = read_u32_be(stream).await? as usize;
    if size == 0 {
        return Ok(String::new());
    }
    if size > MAX_PREFIXED_STRING_SIZE {
        return Err(Mobilebackup2Error::Protocol(format!(
            "prefixed string too large: {size} bytes (max {MAX_PREFIXED_STRING_SIZE})"
        )));
    }

    let mut buf = vec![0u8; size];
    stream.read_exact(&mut buf).await?;
    String::from_utf8(buf)
        .map_err(|err| Mobilebackup2Error::Protocol(format!("backup path was not utf-8: {err}")))
}

async fn read_u32_be<S>(stream: &mut S) -> Result<u32, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}

async fn copy_transfer_payload<S, W>(
    stream: &mut S,
    writer: &mut W,
    mut remaining: usize,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0u8; DOWNLOAD_CHUNK_SIZE];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len());
        stream.read_exact(&mut buffer[..chunk_len]).await?;
        writer.write_all(&buffer[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

async fn discard_transfer_payload<S>(
    stream: &mut S,
    mut remaining: usize,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let mut buffer = [0u8; DOWNLOAD_CHUNK_SIZE];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len());
        stream.read_exact(&mut buffer[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

async fn read_transfer_error_preview<S>(
    stream: &mut S,
    payload_len: usize,
) -> Result<String, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let preview_len = payload_len.min(MAX_TRANSFER_ERROR_PREVIEW_SIZE);
    let mut preview = vec![0u8; preview_len];
    stream.read_exact(&mut preview).await?;
    discard_transfer_payload(stream, payload_len - preview_len).await?;
    Ok(String::from_utf8_lossy(&preview).into_owned())
}

async fn write_prefixed_string<S>(stream: &mut S, value: &str) -> Result<(), Mobilebackup2Error>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&(value.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(value.as_bytes()).await?;
    Ok(())
}

async fn write_transfer_frame<S>(
    stream: &mut S,
    code: u8,
    payload: &[u8],
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&((payload.len() as u32) + 1).to_be_bytes())
        .await?;
    stream.write_all(&[code]).await?;
    if !payload.is_empty() {
        stream.write_all(payload).await?;
    }
    Ok(())
}

#[cfg(windows)]
fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let probe = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    let wide: Vec<u16> = OsStr::new(probe.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(Mobilebackup2Error::Io(std::io::Error::last_os_error()));
    }
    Ok(available)
}

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let probe = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    let c_path = CString::new(probe.as_os_str().as_bytes()).map_err(|e| {
        Mobilebackup2Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    })?;

    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return Err(Mobilebackup2Error::Io(std::io::Error::last_os_error()));
        }
        // Available space = available blocks * fragment size
        Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
}

fn plist_number_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned(),
        plist::Value::Real(value) if value.is_finite() && *value >= 0.0 => {
            // Rust's float-to-integer cast saturates, so an unchecked cast
            // would silently turn negative, fractional, NaN, or oversized
            // protocol values into a different error/space value.
            let upper_bound = (u64::MAX as f64) + 1.0;
            if *value < upper_bound && value.fract() == 0.0 {
                Some(*value as u64)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn plist_number_to_f64(value: &plist::Value) -> Option<f64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned().map(|value| value as f64),
        plist::Value::Real(value) => Some(*value),
        _ => None,
    }
}

fn redacted_protocol_dictionary(dict: &plist::Dictionary) -> plist::Dictionary {
    dict.iter()
        .map(|(key, value)| {
            let value = if is_sensitive_protocol_key(key) {
                plist::Value::String("<redacted>".into())
            } else {
                redacted_protocol_value(value)
            };
            (key.clone(), value)
        })
        .collect()
}

fn redacted_protocol_value(value: &plist::Value) -> plist::Value {
    match value {
        plist::Value::Array(values) => {
            plist::Value::Array(values.iter().map(redacted_protocol_value).collect())
        }
        plist::Value::Dictionary(dict) => {
            plist::Value::Dictionary(redacted_protocol_dictionary(dict))
        }
        _ => value.clone(),
    }
}

fn is_sensitive_protocol_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "password"
            | "oldpassword"
            | "newpassword"
            | "passcode"
            | "unlocktoken"
            | "p12"
            | "privatekey"
    )
}

fn derive_required_free_space(reported: Option<u64>, requested: Option<u64>) -> Option<u64> {
    let reported = reported?;
    let requested = requested?;
    if requested < PURGE_REQUEST_OVERSHOOT {
        return None;
    }

    requested
        .checked_add(reported)
        .and_then(|value| value.checked_sub(PURGE_REQUEST_OVERSHOOT))
}

fn insufficient_disk_space_message(
    response: &plist::Dictionary,
    reported: Option<u64>,
    required: Option<u64>,
) -> String {
    let detail = "the device counts hardlink and clone references at full size because each uploaded path is stored separately";
    match (reported, required) {
        (Some(reported), Some(required)) => {
            let additional = required.saturating_add(1).saturating_sub(reported);
            format!(
                "backup process returned ErrorCode={MB_ERROR_INSUFFICIENT_DISK_SPACE}: device needs more than {required} bytes free, host reported {reported} bytes ({additional} more needed) (MBErrorDomain/{MB_ERROR_INSUFFICIENT_DISK_SPACE}); {detail}; response: {response:?}"
            )
        }
        _ => format!(
            "backup process returned ErrorCode={MB_ERROR_INSUFFICIENT_DISK_SPACE}: {response:?}; {detail}"
        ),
    }
}

fn plist_value_to_bool(value: &plist::Value) -> Option<bool> {
    match value {
        plist::Value::Boolean(value) => Some(*value),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(|value| value != 0)
            .or_else(|| value.as_unsigned().map(|value| value != 0)),
        _ => None,
    }
}

fn insert_file_failure(failures: &mut plist::Dictionary, rel: &str, err: &std::io::Error) {
    let mut failure = plist::Dictionary::from_iter([(
        "DLFileErrorString".to_string(),
        plist::Value::String(err.to_string()),
    )]);
    if let Some(code) = file_error_code_from_os_error(err) {
        failure.insert("DLFileErrorCode".to_string(), protocol_status_value(code));
    }
    failures.insert(rel.to_string(), plist::Value::Dictionary(failure));
}

fn file_error_code_from_os_error(error: &std::io::Error) -> Option<i64> {
    match error.raw_os_error()? {
        2 => Some(-6),
        17 => Some(-7),
        20 => Some(-8),
        21 => Some(-9),
        62 => Some(-10),
        5 => Some(-11),
        28 => Some(-15),
        _ => None,
    }
}

fn should_suppress_disconnect_error(error: &DeviceLinkError) -> bool {
    matches!(
        error,
        DeviceLinkError::Io(io_error)
            if matches!(
                io_error.kind(),
                ErrorKind::BrokenPipe
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionReset
                    | ErrorKind::NotConnected
                    | ErrorKind::UnexpectedEof
            )
    )
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn encode_test_frame(value: &plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, value).expect("plist serialization");
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    async fn read_test_frame_bytes(stream: &mut tokio::io::DuplexStream) -> Vec<u8> {
        let mut length = [0u8; 4];
        stream.read_exact(&mut length).await.expect("frame length");
        let length = u32::from_be_bytes(length) as usize;
        let mut payload = vec![0u8; length];
        stream
            .read_exact(&mut payload)
            .await
            .expect("frame payload");
        payload
    }

    async fn read_test_frame(stream: &mut tokio::io::DuplexStream) -> plist::Value {
        plist::from_bytes(&read_test_frame_bytes(stream).await).expect("plist frame")
    }

    async fn assert_status_response_ok(stream: &mut tokio::io::DuplexStream) {
        let response = read_test_frame(stream).await;
        let parts = response.as_array().expect("status response array");
        assert_eq!(parts[0].as_string(), Some("DLMessageStatusResponse"));
        assert_eq!(parts[1], plist::Value::Integer(0u64.into()));
    }

    #[test]
    fn negative_status_and_transfer_codes_use_exact_unsigned_plist_wire() {
        for status_code in [BULK_OPERATION_ERROR, -6, PURGE_DISK_SPACE_ERROR] {
            let frame = encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                protocol_status_value(status_code),
                plist::Value::String(EMPTY_PARAMETER_STRING.into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ]));
            let payload_len = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
            assert_eq!(payload_len, frame.len() - 4);
            let payload = String::from_utf8(frame[4..].to_vec()).unwrap();
            let expected = status_code as u64;
            assert!(payload.contains(&format!("<integer>{expected}</integer>")));
            assert!(!payload.contains(&format!("<integer>{status_code}</integer>")));
            let decoded: plist::Value = plist::from_bytes(&frame[4..]).unwrap();
            assert_eq!(
                decoded.as_array().and_then(|values| values.get(1)),
                Some(&plist::Value::Integer(expected.into()))
            );
        }

        let mut failure = plist::Dictionary::new();
        failure.insert("DLFileErrorCode".into(), protocol_status_value(-6));
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, &plist::Value::Dictionary(failure)).unwrap();
        let payload = String::from_utf8(payload).unwrap();
        assert!(payload.contains("<integer>18446744073709551610</integer>"));
    }

    // The path policy rejects symlink components deliberately.  macOS exposes
    // its temporary directory through `/var`, which is a system symlink, and
    // Windows may return a verbatim or short-name spelling.  Use the same
    // canonical spelling as production code for fixtures so these tests cover
    // the path policy itself rather than host-specific aliases.
    fn test_temp_dir() -> PathBuf {
        super::canonicalize_simplified(&std::env::temp_dir()).expect("canonical temp directory")
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_system_aliases_are_accepted() {
        assert!(is_approved_system_alias(Path::new("/var")));
        reject_symlink_components(&std::env::temp_dir()).expect("macOS temp alias");

        let root = std::env::temp_dir().join(format!(
            "ios-core-backup2-macos-alias-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let layout =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), true)
                .expect("backup root below the macOS temp alias");
        assert!(layout.device_directory.join("Info.plist").is_file());
        std::fs::remove_dir_all(root).expect("remove macOS alias fixture");
    }

    #[cfg(windows)]
    #[test]
    fn windows_temp_directory_spelling_is_normalized_in_layout() {
        let raw_root = std::env::temp_dir().join(format!(
            "ios-core-backup2-windows-temp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&raw_root).unwrap();

        let layout =
            initialize_backup_directory(&raw_root, "device-id", &plist::Dictionary::new(), true)
                .unwrap();
        assert_eq!(
            layout.root,
            canonicalize_simplified(&raw_root).unwrap(),
            "returned layout must use the production canonical spelling"
        );
        assert_eq!(
            layout.device_directory,
            layout.root.join("device-id"),
            "device directory must share the canonical root spelling"
        );

        std::fs::remove_dir_all(&raw_root).unwrap();
    }

    fn test_backup_root(label: &str, identifier: &str) -> PathBuf {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-device-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let directory = root.join(identifier);
        std::fs::create_dir_all(&directory).expect("backup fixture directory");
        for name in ["Info.plist", "Manifest.plist", "Status.plist"] {
            std::fs::write(directory.join(name), b"fixture").expect("backup fixture metadata");
        }
        root
    }

    async fn complete_device_link_handshake(stream: &mut tokio::io::DuplexStream) {
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300u64.into()),
            ])))
            .await
            .expect("write version exchange");
        assert_eq!(
            read_test_frame(stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::String("DLVersionsOk".into()),
                plist::Value::Integer(300u64.into()),
            ])
        );
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");
        let hello = read_test_frame(stream).await;
        let hello = hello.as_array().expect("hello frame");
        assert_eq!(hello[0].as_string(), Some("DLMessageProcessMessage"));
        let hello = hello[1].as_dictionary().expect("hello dictionary");
        assert_eq!(
            hello.get("MessageName").and_then(plist::Value::as_string),
            Some("Hello")
        );
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("ErrorCode".to_string(), plist::Value::Integer(0u64.into())),
                    ("ProtocolVersion".to_string(), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");
    }

    async fn complete_process_message(
        stream: &mut tokio::io::DuplexStream,
        error_code: u64,
        content: Option<plist::Value>,
    ) {
        let mut response = plist::Dictionary::from_iter([(
            "ErrorCode".to_string(),
            plist::Value::Integer(error_code.into()),
        )]);
        if let Some(content) = content {
            response.insert("Content".into(), content);
        }
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(response),
            ])))
            .await
            .expect("write process response");
    }

    #[tokio::test]
    async fn device_unback_sends_exact_request_and_returns_content() {
        let root = test_backup_root("unback", "source-id");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .unback(
                    &client_root,
                    "target-device",
                    Some("source-id"),
                    Some("秘密🔐"),
                )
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let request = read_test_frame(&mut server_stream).await;
        let request = request.as_array().expect("Unback process frame");
        assert_eq!(request[0].as_string(), Some("DLMessageProcessMessage"));
        assert_eq!(
            request[1]
                .as_dictionary()
                .expect("Unback request dictionary"),
            &plist::Dictionary::from_iter([
                (
                    "MessageName".to_string(),
                    plist::Value::String("Unback".into()),
                ),
                (
                    "TargetIdentifier".to_string(),
                    plist::Value::String("target-device".into()),
                ),
                (
                    "SourceIdentifier".to_string(),
                    plist::Value::String("source-id".into()),
                ),
                (
                    "Password".to_string(),
                    plist::Value::String("秘密🔐".into()),
                ),
            ])
        );

        complete_process_message(
            &mut server_stream,
            0,
            Some(plist::Value::String("expanded".into())),
        )
        .await;
        assert_eq!(
            task.await
                .expect("Unback client task")
                .expect("Unback result"),
            Some(plist::Value::String("expanded".into()))
        );
        assert_eq!(
            read_test_frame(&mut server_stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageDisconnect".into()),
                plist::Value::String("___EmptyParameterString___".into()),
            ])
        );
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_extract_preserves_unicode_and_omits_empty_optional_fields() {
        let root = test_backup_root("extract", "target-device");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .extract(
                    &client_root,
                    "target-device",
                    "AppDomain-com.example.测试",
                    "Library/数据/файл.txt",
                    Some(""),
                    Some(""),
                )
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let request = read_test_frame(&mut server_stream).await;
        let request = request.as_array().expect("Extract process frame");
        let request = request[1].as_dictionary().expect("Extract dictionary");
        assert_eq!(
            request.get("MessageName").and_then(plist::Value::as_string),
            Some("Extract")
        );
        assert_eq!(
            request
                .get("TargetIdentifier")
                .and_then(plist::Value::as_string),
            Some("target-device")
        );
        assert_eq!(
            request.get("DomainName").and_then(plist::Value::as_string),
            Some("AppDomain-com.example.测试")
        );
        assert_eq!(
            request
                .get("RelativePath")
                .and_then(plist::Value::as_string),
            Some("Library/数据/файл.txt")
        );
        assert!(!request.contains_key("SourceIdentifier"));
        assert!(!request.contains_key("Password"));

        complete_process_message(&mut server_stream, 0, None).await;
        assert_eq!(
            task.await
                .expect("Extract client task")
                .expect("Extract result"),
            None
        );
        let _ = read_test_frame(&mut server_stream).await;
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_unback_error_is_redacted_and_disconnects() {
        let root = test_backup_root("error", "target-device");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .unback(
                    &client_root,
                    "target-device",
                    None,
                    Some("do-not-leak-this-password"),
                )
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let _ = read_test_frame(&mut server_stream).await;
        let response = plist::Value::Array(vec![
            plist::Value::String("DLMessageProcessMessage".into()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("ErrorCode".to_string(), plist::Value::Integer(77u64.into())),
                (
                    "Password".to_string(),
                    plist::Value::String("do-not-leak-this-password".into()),
                ),
            ])),
        ]);
        server_stream
            .write_all(&encode_test_frame(&response))
            .await
            .expect("write Unback error");
        let error = task
            .await
            .expect("Unback client task")
            .expect_err("device error should fail Unback");
        assert!(!error.to_string().contains("do-not-leak-this-password"));
        let _ = read_test_frame(&mut server_stream).await;
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_process_response_requires_error_code() {
        let root = test_backup_root("missing-error-code", "target-device");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .unback(&client_root, "target-device", None, None)
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let _ = read_test_frame(&mut server_stream).await;
        let response = plist::Value::Array(vec![
            plist::Value::String("DLMessageProcessMessage".into()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Content".to_string(),
                    plist::Value::String("unexpected-success".into()),
                ),
                (
                    "Password".to_string(),
                    plist::Value::String("secret-not-for-errors".into()),
                ),
            ])),
        ]);
        server_stream
            .write_all(&encode_test_frame(&response))
            .await
            .expect("write malformed process response");
        let error = task
            .await
            .expect("Unback client task")
            .expect_err("missing ErrorCode must fail the operation");
        assert!(error.to_string().contains("missing numeric ErrorCode"));
        assert!(!error.to_string().contains("secret-not-for-errors"));
        let _ = read_test_frame(&mut server_stream).await;
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_extract_rejects_path_escape_before_connecting() {
        let (client_stream, _server_stream) = duplex(1024);
        let mut client = Mobilebackup2Client::new(client_stream);
        let error = client
            .extract(
                Path::new("does-not-exist"),
                "target-device",
                "HomeDomain",
                "../outside",
                None,
                None,
            )
            .await
            .expect_err("path traversal must be rejected");
        assert!(error.to_string().contains("escapes"));
    }

    #[tokio::test]
    async fn device_link_upload_frame_is_written_with_bounded_payload_handling() {
        let root = test_backup_root("upload", "source-id");
        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: root.join("source-id"),
            target_identifier: "source-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_layout = layout.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.run_loop(&client_layout).await
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageUploadFiles".into()),
            ])))
            .await
            .expect("write upload command");
        write_prefixed_string(&mut server_stream, "HomeDomain/数据.txt")
            .await
            .expect("write device path");
        write_prefixed_string(&mut server_stream, "HomeDomain/数据.txt")
            .await
            .expect("write local path");
        write_transfer_frame(
            &mut server_stream,
            FILE_TRANSFER_CODE_FILE_DATA,
            "内容".as_bytes(),
        )
        .await
        .expect("write upload data");
        write_transfer_frame(&mut server_stream, FILE_TRANSFER_CODE_SUCCESS, &[])
            .await
            .expect("write upload completion");
        write_prefixed_string(&mut server_stream, "")
            .await
            .expect("write upload terminator");

        assert_eq!(
            read_test_frame(&mut server_stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(0i64.into()),
                plist::Value::String(EMPTY_PARAMETER_STRING.into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ])
        );
        complete_process_message(&mut server_stream, 0, None).await;
        assert_eq!(task.await.expect("upload client task").unwrap(), None);
        assert_eq!(
            std::fs::read(root.join("source-id/HomeDomain/数据.txt")).unwrap(),
            "内容".as_bytes()
        );
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn filtered_placeholder_replay_drains_moves_copies_and_finishes_after_listing() {
        let root = test_backup_root("placeholder-replay", "device-id");
        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: root.join("device-id"),
            target_identifier: "device-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(64 * 1024);
        let client_layout = layout.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.backup_filter =
                Some(build_backup_filter(&["sms".into()], &[]).unwrap().unwrap());
            let result = client.run_loop(&client_layout).await;
            (result, client.discarded_files.len())
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageUploadFiles".into()),
            ])))
            .await
            .unwrap();
        write_prefixed_string(&mut server_stream, "HomeDomain/Library/Notes/rejected.bin")
            .await
            .unwrap();
        write_prefixed_string(&mut server_stream, "device-id/aa/rejected.bin")
            .await
            .unwrap();
        write_transfer_frame(
            &mut server_stream,
            FILE_TRANSFER_CODE_FILE_DATA,
            b"discarded payload",
        )
        .await
        .unwrap();
        write_transfer_frame(&mut server_stream, FILE_TRANSFER_CODE_SUCCESS, &[])
            .await
            .unwrap();
        write_prefixed_string(&mut server_stream, "").await.unwrap();
        assert_status_response_ok(&mut server_stream).await;

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageMoveItems".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "device-id/aa/rejected.bin".to_string(),
                    plist::Value::String("device-id/bb/rejected.bin".into()),
                )])),
                plist::Value::Dictionary(plist::Dictionary::new()),
                plist::Value::Real(0.0),
            ])))
            .await
            .unwrap();
        assert_status_response_ok(&mut server_stream).await;

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageCopyItem".into()),
                plist::Value::String("device-id/bb/rejected.bin".into()),
                plist::Value::String("device-id/cc/rejected.bin".into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
                plist::Value::Real(0.0),
            ])))
            .await
            .unwrap();
        assert_status_response_ok(&mut server_stream).await;

        for directory in ["device-id", "device-id/bb", "device-id/cc"] {
            server_stream
                .write_all(&encode_test_frame(&plist::Value::Array(vec![
                    plist::Value::String("DLContentsOfDirectory".into()),
                    plist::Value::String(directory.into()),
                    plist::Value::Dictionary(plist::Dictionary::new()),
                    plist::Value::Real(0.0),
                ])))
                .await
                .unwrap();
            let raw_response = read_test_frame_bytes(&mut server_stream).await;
            let raw_response_text = String::from_utf8(raw_response.clone()).unwrap();
            assert!(
                !raw_response_text
                    .split("<date>")
                    .skip(1)
                    .filter_map(|part| part.split("</date>").next())
                    .any(|date| date.contains('.')),
                "directory listing XML dates must use second precision"
            );
            let response: plist::Value = plist::from_bytes(&raw_response).unwrap();
            let parts = response.as_array().expect("directory status response");
            assert_eq!(parts[0].as_string(), Some("DLMessageStatusResponse"));
            assert_eq!(parts[1], plist::Value::Integer(0u64.into()));
            let listing = parts[3]
                .as_dictionary()
                .expect("directory listing dictionary");
            assert!(!listing.is_empty());
            for value in listing.values() {
                let entry = value.as_dictionary().expect("directory entry dictionary");
                let date = entry
                    .get("DLFileModificationDate")
                    .and_then(plist::Value::as_date)
                    .expect("directory entry date");
                assert!(
                    !date.to_xml_format().contains('.'),
                    "directory listing dates must use pmd3's second precision"
                );
            }
        }

        complete_process_message(
            &mut server_stream,
            0,
            Some(plist::Value::String("finished".into())),
        )
        .await;
        let (result, placeholder_count) = task.await.unwrap();
        assert_eq!(
            result.unwrap(),
            Some(plist::Value::String("finished".into()))
        );
        assert_eq!(
            placeholder_count, 2,
            "move and copy must retain both placeholders"
        );
        for path in [
            root.join("device-id/bb/rejected.bin"),
            root.join("device-id/cc/rejected.bin"),
        ] {
            assert_eq!(
                std::fs::metadata(&path).unwrap().len(),
                0,
                "discarded upload payload must be drained; placeholders remain zero-byte"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transfer_budgets_reject_excess_files_and_bytes() {
        let (stream, _) = duplex(16);
        let mut client = Mobilebackup2Client::new(stream);
        client.transfer_files = MAX_DEVICE_TRANSFER_FILES;
        assert!(client.account_transfer_file().is_err());
        client.transfer_bytes = MAX_DEVICE_TRANSFER_BYTES;
        assert!(client.account_transfer_bytes(1).is_err());
    }

    #[tokio::test]
    async fn purge_disk_space_is_answered_and_the_loop_continues() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.run_loop(&layout).await
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessagePurgeDiskSpace".into()),
                plist::Value::Integer(42u64.into()),
                plist::Value::Integer(4u64.into()),
            ])))
            .await
            .expect("write purge request");

        assert_eq!(
            read_test_frame(&mut server_stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                protocol_status_value(PURGE_DISK_SPACE_ERROR),
                plist::Value::String(PURGE_DISK_SPACE_ERROR_STRING.into()),
                plist::Value::Integer(0u64.into()),
            ])
        );

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("ErrorCode".to_string(), plist::Value::Integer(0u64.into())),
                    (
                        "Content".to_string(),
                        plist::Value::String("backup finished".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write completion");

        assert_eq!(
            task.await.expect("run loop task").expect("backup loop"),
            Some(plist::Value::String("backup finished".into()))
        );
    }

    #[tokio::test]
    async fn insufficient_space_process_error_uses_the_last_purge_diagnostics() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.reported_free_space = Some(1_000_000);
            client.run_loop(&layout).await
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessagePurgeDiskSpace".into()),
                plist::Value::Integer(2_335_105_975u64.into()),
                plist::Value::Integer(4u64.into()),
            ])))
            .await
            .expect("write purge request");
        let _ = read_test_frame(&mut server_stream).await;

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        "ErrorCode".to_string(),
                        plist::Value::Integer(105u64.into()),
                    ),
                    (
                        "ErrorDescription".to_string(),
                        plist::Value::String("Insufficient free disk space".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write insufficient-space response");

        let error = task
            .await
            .expect("run loop task")
            .expect_err("insufficient space should fail the operation");
        let message = error.to_string();
        assert!(message.contains("needs more than 188622327 bytes free"));
        assert!(message.contains("host reported 1000000 bytes"));
        assert!(message.contains("MBErrorDomain/105"));
    }

    #[test]
    fn derives_required_free_space_from_purge_request() {
        let required = derive_required_free_space(Some(1_000_000), Some(2_335_105_975));
        assert_eq!(required, Some(188_622_327));
        assert_eq!(
            derive_required_free_space(Some(1_000_000), Some(PURGE_REQUEST_OVERSHOOT - 1)),
            None
        );
        assert_eq!(
            derive_required_free_space(None, Some(PURGE_REQUEST_OVERSHOOT)),
            None
        );
    }

    #[test]
    fn plist_u64_parser_rejects_non_integral_and_out_of_range_reals() {
        assert_eq!(plist_number_to_u64(&plist::Value::Real(42.0)), Some(42));
        for value in [-1.0, 42.5, f64::NAN, f64::INFINITY, u64::MAX as f64] {
            assert_eq!(
                plist_number_to_u64(&plist::Value::Real(value)),
                None,
                "real protocol value must be a finite non-negative integer: {value:?}"
            );
        }
    }

    #[test]
    fn insufficient_space_message_includes_requirement_and_host_report() {
        let response = plist::Dictionary::from_iter([(
            "ErrorDescription".to_string(),
            plist::Value::String("Insufficient free disk space".into()),
        )]);
        let message =
            insufficient_disk_space_message(&response, Some(1_000_000), Some(188_622_327));
        assert!(message.contains("needs more than 188622327 bytes free"));
        assert!(message.contains("host reported 1000000 bytes"));
        assert!(message.contains("187622328 more needed"));
        assert!(message.contains("MBErrorDomain/105"));
    }

    #[test]
    fn initialize_backup_directory_creates_expected_seed_files() {
        let root = test_temp_dir().join(format!("ios-core-backup2-layout-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();

        let info = plist::Dictionary::from_iter([(
            "Device Name".to_string(),
            plist::Value::String("Example".into()),
        )]);
        let layout = initialize_backup_directory(&root, "device-id", &info, true).unwrap();

        assert_eq!(layout.device_directory, root.join("device-id"));
        assert!(layout.device_directory.join("Info.plist").exists());
        assert!(layout.device_directory.join("Status.plist").exists());
        assert!(layout.device_directory.join("Manifest.plist").exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initial_backup_becomes_full_when_incremental_metadata_is_missing() {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-initial-full-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();

        let layout =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), false)
                .unwrap();

        let status = plist::Value::from_file(layout.device_directory.join("Status.plist"))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(status["IsFullBackup"].as_boolean(), Some(true));
        assert!(backup_status_is_full(&layout.device_directory).unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incremental_backup_is_preserved_when_required_metadata_exists() {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-incremental-{}",
            std::process::id()
        ));
        let device_dir = root.join("device-id");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&device_dir).unwrap();
        plist::to_file_xml(
            device_dir.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::new()),
        )
        .unwrap();
        plist::to_file_xml(
            device_dir.join("Status.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "IsFullBackup".to_string(),
                plist::Value::Boolean(false),
            )])),
        )
        .unwrap();
        std::fs::write(device_dir.join("Manifest.db"), b"data").unwrap();
        let old_status = std::fs::read(device_dir.join("Status.plist")).unwrap();
        let old_manifest = std::fs::read(device_dir.join("Manifest.plist")).unwrap();

        let layout =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), false)
                .unwrap();
        assert!(!backup_status_is_full(&layout.device_directory).unwrap());
        assert_eq!(
            std::fs::read(layout.device_directory.join("Status.plist")).unwrap(),
            old_status
        );
        assert_eq!(
            std::fs::read(layout.device_directory.join("Manifest.plist")).unwrap(),
            old_manifest
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_incremental_metadata_forces_full_backup() {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-empty-metadata-{}",
            std::process::id()
        ));
        let device_dir = root.join("device-id");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&device_dir).unwrap();
        std::fs::write(device_dir.join("Manifest.plist"), b"").unwrap();
        std::fs::write(device_dir.join("Manifest.db"), b"data").unwrap();
        std::fs::write(device_dir.join("Status.plist"), b"data").unwrap();

        assert!(should_do_full_backup(false, &device_dir).unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_relative_path_accepts_plain_and_prefixed_paths() {
        let root = test_temp_dir().join(format!("ios-core-backup2-resolve-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(root.join("device-id")).unwrap();
        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: root.join("device-id"),
            target_identifier: "device-id".into(),
        };

        assert_eq!(
            resolve_relative_path(&layout, "Manifest.db").unwrap(),
            root.join("device-id/Manifest.db")
        );
        assert_eq!(
            resolve_relative_path(&layout, "device-id/Manifest.db").unwrap(),
            root.join("device-id/Manifest.db")
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_relative_path_rejects_parent_escapes() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };

        let err = resolve_relative_path(&layout, "../outside").unwrap_err();
        assert!(err.to_string().contains("escapes"));
    }

    /// R3 regression matrix: the component walk must defer metadata queries
    /// until a complete root exists.  Canonicalized Windows paths start with a
    /// verbatim prefix (`\\?\C:`) that rejects `symlink_metadata` with
    /// ERROR_INVALID_FUNCTION until the root separator is appended.
    #[test]
    fn reject_symlink_components_accepts_verbatim_relative_and_unicode_paths() {
        let base = test_temp_dir().join(format!(
            "ios-core-backup2-r3-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let unicode_leaf = base.join("备份-テスト-ümlaut");
        std::fs::create_dir_all(&unicode_leaf).unwrap();
        std::fs::write(unicode_leaf.join("present.txt"), b"data").unwrap();

        // Plain absolute path with existing and missing leaves.
        reject_symlink_components(&unicode_leaf).unwrap();
        let missing = unicode_leaf.join("missing-file.bin");
        reject_symlink_components(&missing).unwrap();

        // Canonical verbatim form (`\\?\C:\...`): the exact R3 failure shape.
        let canonical = unicode_leaf.canonicalize().unwrap();
        reject_symlink_components(&canonical).unwrap();
        reject_symlink_components(&canonical.join("missing-file.bin")).unwrap();

        // The bare drive prefixes that must not be queried at all.
        reject_symlink_components(Path::new("C:")).unwrap();
        reject_symlink_components(Path::new(r"\\?\C:")).unwrap();

        // A complete verbatim drive root is queryable.
        reject_symlink_components(Path::new(r"\\?\C:\")).unwrap();

        // Relative paths resolve against the current directory.
        reject_symlink_components(Path::new("relative/probe/missing.bin")).unwrap();

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn canonicalize_simplified_does_not_alias_distinct_verbatim_tail_directory() {
        let base = test_temp_dir().join(format!(
            "ios-core-backup2-canonical-alias-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let ordinary = base.join("backup");
        let verbatim = PathBuf::from(format!(r"\\?\{}\backup.", base.display()));
        std::fs::create_dir_all(&ordinary).unwrap();
        std::fs::create_dir_all(&verbatim).unwrap();
        std::fs::write(ordinary.join("marker.txt"), b"ordinary-directory").unwrap();
        std::fs::write(verbatim.join("marker.txt"), b"verbatim-directory").unwrap();

        let canonical = std::fs::canonicalize(&verbatim).unwrap();
        let simplified = canonicalize_simplified(&verbatim).unwrap();
        assert_eq!(
            std::fs::canonicalize(&simplified).unwrap(),
            canonical,
            "simplifying a verbatim path must preserve its resolved directory"
        );
        assert_eq!(
            std::fs::read(simplified.join("marker.txt")).unwrap(),
            b"verbatim-directory",
            "the candidate must not alias the ordinary directory"
        );

        std::fs::remove_dir_all(&verbatim).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn canonicalize_simplified_preserves_invalid_wtf16_components() {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let base = test_temp_dir().join(format!(
            "ios-core-backup2-canonical-wtf16-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let mut invalid = PathBuf::from(format!(r"\\?\{}", base.display()));
        let invalid_component = OsString::from_wide(&[0xD800]);
        invalid.push(&invalid_component);
        let mut replacement = PathBuf::from(format!(r"\\?\{}", base.display()));
        let replacement_component = OsString::from_wide(&[0xFFFD]);
        replacement.push(&replacement_component);
        std::fs::create_dir_all(&invalid).unwrap();
        std::fs::create_dir_all(&replacement).unwrap();
        std::fs::write(invalid.join("marker.txt"), b"invalid-wtf16").unwrap();
        std::fs::write(replacement.join("marker.txt"), b"replacement").unwrap();

        let canonical = std::fs::canonicalize(&invalid).unwrap();
        let simplified = canonicalize_simplified(&invalid).unwrap();
        assert_eq!(
            std::fs::canonicalize(&simplified).unwrap(),
            canonical,
            "simplifying an invalid WTF-16 path must preserve its resolved directory"
        );
        assert_eq!(
            std::fs::read(simplified.join("marker.txt")).unwrap(),
            b"invalid-wtf16",
            "the invalid component must not be replaced with U+FFFD"
        );
        assert!(
            canonical
                .as_os_str()
                .encode_wide()
                .any(|unit| unit == 0xD800),
            "the canonical path should retain the isolated surrogate"
        );

        std::fs::remove_dir_all(&invalid).unwrap();
        std::fs::remove_dir_all(&replacement).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }

    /// Symlink rejection still applies on Windows when the test environment
    /// is allowed to create symlinks (Developer Mode or admin privilege).
    /// Without that privilege the test reports the exact blocker and stops:
    /// the ordinary-path assertions above must not be mistaken for symlink
    /// coverage.
    #[cfg(windows)]
    #[test]
    fn reject_symlink_components_rejects_windows_directory_symlinks() {
        use std::os::windows::fs::symlink_dir;

        let base = test_temp_dir().join(format!(
            "ios-core-backup2-r3-symlink-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let link = root.join("escape");
        if let Err(error) = symlink_dir(&outside, &link) {
            assert_eq!(
                error.raw_os_error(),
                Some(1314),
                "unexpected symlink creation failure: {error}"
            );
            eprintln!(
                "SKIP symlink assertion: creating a directory symlink requires \
                 Developer Mode or administrator privilege (os error 1314)"
            );
            std::fs::remove_dir_all(&base).unwrap();
            return;
        }

        let error = reject_symlink_components(&link).unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
        // A missing leaf beyond the symlink stays guarded by the same walk.
        let error = reject_symlink_components(&link.join("deeper")).unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn backup_identifiers_are_single_normal_components() {
        for identifier in [
            "",
            ".",
            "..",
            "../outside",
            "/absolute",
            "nested/id",
            r"nested\id",
            "C:backup",
            r"C:\backup",
        ] {
            assert!(
                validate_backup_identifier(identifier).is_err(),
                "identifier should be rejected: {identifier:?}"
            );
        }
        for identifier in ["device-id", "设备-✅"] {
            assert!(
                validate_backup_identifier(identifier).is_ok(),
                "identifier should be accepted: {identifier:?}"
            );
        }
    }

    #[test]
    fn relative_paths_reject_absolute_parent_and_platform_forms() {
        for path in [
            "",
            ".",
            "..",
            "../outside",
            "/absolute",
            r"..\outside",
            r"nested\file",
            "C:relative",
            r"C:\absolute",
        ] {
            assert!(
                sanitize_relative_path(path).is_err(),
                "relative path should be rejected: {path:?}"
            );
        }

        assert_eq!(
            sanitize_relative_path("nested/目录/файл").unwrap(),
            PathBuf::from("nested/目录/файл")
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_components_are_rejected_without_writing_outside_root() {
        use std::os::unix::fs::symlink;

        let base = test_temp_dir().join(format!("ios-core-backup2-symlink-{}", std::process::id()));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        let root = base.join("root");
        let device = root.join("device-id");
        let outside = base.join("outside");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside").unwrap();

        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: device.clone(),
            target_identifier: "device-id".into(),
        };

        symlink(&outside, device.join("middle")).unwrap();
        let err = resolve_relative_path(&layout, "middle/created").unwrap_err();
        assert!(err.to_string().contains("symlink"));

        symlink(outside.join("sentinel"), device.join("final")).unwrap();
        let err = resolve_relative_path(&layout, "final").unwrap_err();
        assert!(err.to_string().contains("symlink"));

        let err = open_layout_file_for_write(&layout, &device.join("final")).unwrap_err();
        assert!(err.to_string().contains("symlink"));
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"outside");

        std::fs::write(device.join("source"), b"inside").unwrap();
        let copy_target = device.join("copy");
        symlink(outside.join("sentinel"), copy_target.clone()).unwrap();
        let err = copy_item(&root, &device.join("source"), &copy_target).unwrap_err();
        assert!(err.to_string().contains("symlink"));
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"outside");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn seed_symlink_is_rejected_before_existing_metadata_is_overwritten() {
        use std::os::unix::fs::symlink;

        let base = test_temp_dir().join(format!(
            "ios-core-backup2-seed-symlink-{}",
            std::process::id()
        ));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        let root = base.join("root");
        let device = root.join("device-id");
        let outside = base.join("outside");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(device.join("Info.plist"), b"old-info").unwrap();
        std::fs::write(device.join("Status.plist"), b"old-status").unwrap();
        std::fs::write(outside.join("sentinel"), b"outside").unwrap();
        symlink(outside.join("sentinel"), device.join("Manifest.plist")).unwrap();

        let result =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), true);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(device.join("Info.plist")).unwrap(),
            b"old-info"
        );
        assert_eq!(
            std::fs::read(device.join("Status.plist")).unwrap(),
            b"old-status"
        );
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"outside");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_root_symlink_is_rejected_before_creating_outside_directory() {
        use std::os::unix::fs::symlink;

        let base = test_temp_dir().join(format!(
            "ios-core-backup2-root-symlink-{}",
            std::process::id()
        ));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        std::fs::create_dir_all(&base).unwrap();
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let linked_root = base.join("linked-root");
        symlink(&outside, &linked_root).unwrap();

        let result = initialize_backup_directory(
            &linked_root.join("new-device"),
            "device-id",
            &plist::Dictionary::new(),
            true,
        );
        assert!(result.is_err());
        assert!(!outside.join("new-device").exists());

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn generated_backup_uuid_is_uppercase_v4() {
        let generated = generate_backup_uuid();
        let parsed = uuid::Uuid::parse_str(&generated).expect("status UUID should be parseable");

        assert_eq!(generated, generated.to_uppercase());
        assert_eq!(parsed.get_version_num(), 4);
    }

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

    #[test]
    fn backup_is_encrypted_reads_manifest_flag() {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-encryption-{}",
            std::process::id()
        ));
        let device_dir = root.join("device-id");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&device_dir).unwrap();
        std::fs::write(device_dir.join("Info.plist"), b"info").unwrap();
        plist::to_file_xml(
            device_dir.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "IsEncrypted".to_string(),
                plist::Value::Boolean(true),
            )])),
        )
        .unwrap();
        std::fs::write(device_dir.join("Status.plist"), b"status").unwrap();

        assert!(backup_is_encrypted(&root, "device-id").unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn metadata_plist_size_is_checked_before_decoding() {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-metadata-limit-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("Manifest.plist");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_BACKUP_METADATA_BYTES + 1).unwrap();
        let error = read_backup_dictionary(&path).expect_err("sparse metadata must be rejected");
        assert!(error.to_string().contains("too large"));
        assert!(error.to_string().contains("16777217"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn device_link_modification_date_matches_pmd3_second_precision_wire() {
        let modified = SystemTime::UNIX_EPOCH
            + APPLE_EPOCH_OFFSET
            + Duration::from_secs(123)
            + Duration::from_millis(900);
        let encoded = device_link_modification_date(modified);
        let shifted: SystemTime = encoded.into();
        let expected = device_link_local_wall_clock(modified)
            .checked_sub(APPLE_EPOCH_OFFSET)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let expected = truncate_system_time_to_seconds(expected);

        assert_eq!(shifted, expected);
        assert_eq!(
            shifted
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("fixture date is after the Unix epoch")
                .subsec_nanos(),
            0
        );
        assert!(
            !encoded.to_xml_format().contains('.'),
            "pmd3 plistlib wire dates do not contain fractional seconds"
        );
    }

    #[test]
    fn suppresses_expected_disconnect_transport_errors() {
        for kind in [
            ErrorKind::BrokenPipe,
            ErrorKind::ConnectionAborted,
            ErrorKind::ConnectionReset,
            ErrorKind::NotConnected,
            ErrorKind::UnexpectedEof,
        ] {
            assert!(should_suppress_disconnect_error(&DeviceLinkError::Io(
                std::io::Error::from(kind),
            )));
        }
    }

    #[test]
    fn keeps_unexpected_disconnect_errors_visible() {
        assert!(!should_suppress_disconnect_error(&DeviceLinkError::Io(
            std::io::Error::from(ErrorKind::Other),
        )));
        assert!(!should_suppress_disconnect_error(
            &DeviceLinkError::Protocol("disconnect protocol mismatch".into(),)
        ));
    }

    #[tokio::test]
    async fn read_prefixed_string_rejects_oversized_allocation() {
        // Craft a size field that exceeds MAX_PREFIXED_STRING_SIZE
        let size = (MAX_PREFIXED_STRING_SIZE as u32) + 1;
        let data = size.to_be_bytes();
        let mut cursor = std::io::Cursor::new(data.to_vec());
        let err = read_prefixed_string(&mut cursor).await.unwrap_err();
        assert!(
            err.to_string().contains("too large"),
            "expected size guard error, got: {err}"
        );
    }

    #[tokio::test]
    async fn read_prefixed_string_accepts_normal_size() {
        let payload = b"hello";
        let size = (payload.len() as u32).to_be_bytes();
        let mut data = size.to_vec();
        data.extend_from_slice(payload);
        let mut cursor = std::io::Cursor::new(data);
        let result = read_prefixed_string(&mut cursor).await.unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn backup_filter_matches_upstream_presets_and_manifest_candidates() {
        let filter = build_backup_filter(&["sms".into()], &[])
            .expect("selection should parse")
            .expect("selection should create a filter");
        assert!(filter.matches_device_name("HomeDomain/Library/SMS/sms.db"));
        assert!(filter.matches_manifest_entry("HomeDomain", "Library/SMS/sms.db"));
        assert!(!filter.matches_manifest_entry("HomeDomain", "Library/Notes/notes.db"));

        let recursive = build_backup_filter(&["messages".into()], &[])
            .expect("selection should parse")
            .expect("selection should create a filter");
        assert!(recursive.matches_device_name("MediaDomain/Library/SMS/Attachments/a.bin"));
    }

    #[test]
    fn backup_filter_regex_is_or_combined_and_bounded() {
        let filter = build_backup_filter(&["sms".into()], &["Notes/.*\\.db".into()])
            .expect("regex should parse")
            .expect("filter should exist");
        assert!(filter.matches_manifest_entry("HomeDomain", "Library/SMS/sms.db"));
        assert!(filter.matches_manifest_entry("HomeDomain", "Library/Notes/notes.db"));
        assert!(!filter.matches_manifest_entry("HomeDomain", "Library/Notes/notes.plist"));

        let too_long = "x".repeat(MAX_FILTER_PATTERN_BYTES + 1);
        assert!(build_backup_filter(&[], &[too_long]).is_err());
        assert!(build_backup_filter(&[], &["[".into()]).is_err());
    }

    #[test]
    fn backup_filter_keeps_metadata_and_discards_unmatched_device_paths() {
        let filter = build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        assert!(should_preserve_backup_file(
            "aa/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "HomeDomain/Library/SMS/sms.db",
            &filter
        ));
        assert!(!should_preserve_backup_file(
            "bb/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "HomeDomain/Library/Notes/notes.db",
            &filter
        ));
        for metadata in [
            "Info.plist",
            "Manifest.plist",
            "Manifest.db",
            "Manifest.mbdb",
            "Manifest.mbdx",
            "Manifest.db-shm",
            "Manifest.db-wal",
            "Status.plist",
        ] {
            assert!(should_preserve_backup_file(metadata, "", &filter));
        }
    }
}

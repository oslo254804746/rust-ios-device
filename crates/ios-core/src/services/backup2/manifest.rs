//! Local MobileBackup2 Manifest.db/Manifest.mbdb operations.
//!
//! The device protocol does not have a separate "extract" or "patch" message.  These helpers
//! operate on a completed backup on the host.  Modern encrypted backups (iOS 10.3 and newer)
//! and legacy encrypted payloads (iOS 10.2 and older) are handled with the audited
//! BackupKeyBag/AES implementation in [`super::crypto`].  Legacy Manifest.mbdb itself is
//! plaintext and is rewritten directly; the unsupported MBDX sidecar is not needed by the
//! upstream pyiosbackup layout.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::proto::nskeyedarchiver::{unarchive, ArchiveValue};

use super::crypto::{expected_payload_ciphertext_len, BackupCrypto, CryptoError};
#[cfg(test)]
use super::mbdb::MbdbRecord;
use super::mbdb::{MbdbError, MbdbManifest};
use super::{
    canonical_backup_root, create_dir_all_no_symlink, ensure_backup_directory,
    ensure_no_symlink_components_at_root, open_file_for_read, open_file_for_write,
    read_backup_dictionary, sanitize_relative_path, symlink_path_error, validate_backup_identifier,
    BackupFilter, Mobilebackup2Error,
};

const MAX_MANIFEST_ROWS: usize = 1_000_000;
const MAX_MANIFEST_DB_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_MANIFEST_TEXT_BYTES: u64 = 1024 * 1024;
const MAX_ARCHIVE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TOTAL_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_SINGLE_ENTRY_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_TOTAL_EXTRACT_BYTES: u64 = 512 * 1024 * 1024 * 1024;
const MAX_SYMLINK_TARGET_BYTES: usize = 64 * 1024;
const MODE_TYPE_MASK: u32 = 0xe000;
const MODE_TYPE_SYMLINK: u32 = 0xa000;
const MODE_TYPE_FILE: u32 = 0x8000;
const MODE_TYPE_DIRECTORY: u32 = 0x4000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestPatchResult {
    pub entries_seen: u64,
    pub entries_kept: u64,
    pub entries_removed: u64,
    pub payloads_removed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtractionResult {
    pub output_directory: PathBuf,
    pub entries_seen: u64,
    pub files_extracted: u64,
    pub bytes_extracted: u64,
}

/// A redacted, host-side view of one entry in a completed backup manifest.
///
/// Encryption keys are intentionally not exposed. `link_target` is present only for legacy
/// MBDB records because modern manifests carry symlink contents in the payload blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackupManifestEntry {
    pub file_id: String,
    pub domain: String,
    pub relative_path: String,
    pub mode: u32,
    pub size: u64,
    pub link_target: Option<String>,
}

struct ManifestEntry {
    file_id: String,
    domain: String,
    relative_path: String,
    mode: u32,
    size: u64,
    encryption_key: Option<Zeroizing<Vec<u8>>>,
    link_target: Option<String>,
}

struct ManifestWorkspace {
    source_path: PathBuf,
    original_path: PathBuf,
    crypto: Option<BackupCrypto>,
    modern: bool,
    mbdb: Option<MbdbManifest>,
    _temporary: Option<TemporaryFile>,
}

struct TemporaryFile {
    path: PathBuf,
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        for suffix in ["-shm", "-wal", "-journal"] {
            let sidecar = self.path.with_file_name(format!(
                "{}{}",
                self.path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("temporary"),
                suffix
            ));
            let _ = fs::remove_file(sidecar);
        }
    }
}

type ManifestFilterRows = (Vec<(i64, String)>, Vec<i64>, HashSet<String>);

/// Remove entries not selected by `filter` from a completed backup's Manifest.db and payload
/// layout. Metadata plist files are always retained. This is a local operation; no device-link
/// message is sent.
pub fn patch_backup_directory(
    backup_root: &Path,
    target_identifier: &str,
    filter: &BackupFilter,
    password: Option<&str>,
) -> Result<ManifestPatchResult, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    ensure_backup_directory(backup_root, target_identifier)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = safe_device_directory(&root, target_identifier)?;
    let manifest = open_manifest_workspace(&device_directory, password)?;
    if manifest.mbdb.is_some() {
        return patch_mbdb_backup_directory(&device_directory, &manifest, filter);
    }
    let (rows, row_ids, allowed_ids) = collect_manifest_rows(&manifest.source_path, filter)?;
    let entries_seen = u64::try_from(rows.len()).map_err(|_| {
        Mobilebackup2Error::Protocol("Manifest.db row count does not fit in u64".into())
    })?;
    let entries_kept = u64::try_from(allowed_ids.len()).map_err(|_| {
        Mobilebackup2Error::Protocol("Manifest.db row count does not fit in u64".into())
    })?;
    let entries_removed = entries_seen.saturating_sub(entries_kept);

    // Validate the complete payload tree before creating any replacement. In particular, a
    // rejected symlink or special payload must not leave the manifest half-patched just because
    // pruning discovered it after the SQL transaction committed.
    validate_payload_tree(&device_directory)?;

    // Always make a replacement database. SQL is confined to this private file until all
    // payload moves have succeeded, so every failure before the final rename leaves the backup
    // usable. This also avoids changing an unencrypted Manifest.db in place.
    let plain_temporary = copy_manifest_to_temporary(&manifest.source_path, &device_directory)?;
    patch_manifest_file(&plain_temporary.path, &row_ids)?;
    let encrypted_temporary = if let Some(crypto) = manifest.crypto.as_ref() {
        let temporary = create_temporary_file(&device_directory, "manifest-encrypted")?;
        crypto
            .encrypt_manifest_file(&plain_temporary.path, &temporary.path)
            .map_err(crypto_error)?;
        Some(temporary)
    } else {
        None
    };
    let replacement = encrypted_temporary
        .as_ref()
        .map(|file| file.path.as_path())
        .unwrap_or(plain_temporary.path.as_path());

    let mut staged = stage_payloads(&device_directory, &allowed_ids)?;
    if let Err(error) = replace_file_atomically(replacement, &manifest.original_path) {
        staged.rollback();
        return Err(error);
    }
    let payloads_removed = staged.finish();
    Ok(ManifestPatchResult {
        entries_seen,
        entries_kept,
        entries_removed,
        payloads_removed,
    })
}

fn patch_mbdb_backup_directory(
    device_directory: &Path,
    manifest: &ManifestWorkspace,
    filter: &BackupFilter,
) -> Result<ManifestPatchResult, Mobilebackup2Error> {
    let mbdb = manifest.mbdb.as_ref().ok_or_else(|| {
        Mobilebackup2Error::Protocol("legacy manifest workspace has no parsed Manifest.mbdb".into())
    })?;
    let entries = load_mbdb_entries(mbdb, manifest.crypto.is_some())?;
    let entries_seen = u64::try_from(entries.len()).map_err(|_| {
        Mobilebackup2Error::Protocol("Manifest.mbdb record count does not fit in u64".into())
    })?;
    let allowed_ids: HashSet<String> = entries
        .iter()
        .filter(|entry| filter.matches_manifest_entry(&entry.domain, &entry.relative_path))
        .map(|entry| entry.file_id.clone())
        .collect();
    let entries_kept = u64::try_from(allowed_ids.len()).map_err(|_| {
        Mobilebackup2Error::Protocol("Manifest.mbdb record count does not fit in u64".into())
    })?;
    let entries_removed = entries_seen.saturating_sub(entries_kept);

    validate_payload_tree(device_directory)?;
    let temporary = write_mbdb_to_temporary(mbdb, &allowed_ids, device_directory)?;
    let mut staged = stage_payloads(device_directory, &allowed_ids)?;
    if let Err(error) = replace_file_atomically(&temporary.path, &manifest.original_path) {
        staged.rollback();
        return Err(error);
    }
    let payloads_removed = staged.finish();
    Ok(ManifestPatchResult {
        entries_seen,
        entries_kept,
        entries_removed,
        payloads_removed,
    })
}

fn write_mbdb_to_temporary(
    manifest: &MbdbManifest,
    allowed_ids: &HashSet<String>,
    directory: &Path,
) -> Result<TemporaryFile, Mobilebackup2Error> {
    let bytes = manifest.serialize(Some(allowed_ids)).map_err(mbdb_error)?;
    let temporary = create_temporary_file(directory, "manifest-patch")?;
    let result = (|| {
        let mut file = open_file_for_write(&temporary.path)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        Ok::<(), Mobilebackup2Error>(())
    })();
    if let Err(error) = result {
        drop(temporary);
        return Err(error);
    }
    Ok(temporary)
}

/// Expand all regular files (and safe relative symlinks on Unix) from a completed backup into a
/// domain/relative-path directory tree. The source backup itself is never modified.
pub fn unback_backup(
    backup_root: &Path,
    target_identifier: &str,
    output: Option<&Path>,
    password: Option<&str>,
) -> Result<ExtractionResult, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    ensure_backup_directory(backup_root, target_identifier)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = safe_device_directory(&root, target_identifier)?;
    let manifest = open_manifest_workspace(&device_directory, password)?;
    let entries = load_manifest_entries(&manifest)?;
    let output_directory = prepare_output_directory(
        &root,
        &device_directory,
        target_identifier,
        output,
        "unback",
    )?;
    extract_entries(
        &device_directory,
        &output_directory,
        &entries,
        manifest.crypto.as_ref(),
        manifest.modern,
    )
}

/// Expand one file or directory selected by its Manifest.db domain and relative path. If `output`
/// names an existing directory, the entry's final component is appended, matching pyiosbackup.
pub fn extract_backup_file(
    backup_root: &Path,
    target_identifier: &str,
    domain: &str,
    relative_path: &str,
    output: Option<&Path>,
    password: Option<&str>,
) -> Result<ExtractionResult, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    ensure_backup_directory(backup_root, target_identifier)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = safe_device_directory(&root, target_identifier)?;
    let manifest = open_manifest_workspace(&device_directory, password)?;
    let entries = load_manifest_entries(&manifest)?;
    let domain = validate_domain(domain)?;
    let relative_path = sanitize_relative_path(relative_path)?
        .to_string_lossy()
        .into_owned();
    let entry = entries
        .into_iter()
        .find(|entry| entry.domain == domain && entry.relative_path == relative_path)
        .ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "backup entry not found: {domain}/{relative_path}"
            ))
        })?;

    let (output_root, destination) = match output {
        Some(path) => {
            reject_output_path(path)?;
            let requested_destination = if path.is_dir() {
                let name = Path::new(&entry.relative_path).file_name().ok_or_else(|| {
                    Mobilebackup2Error::Protocol("backup entry has no final path component".into())
                })?;
                path.join(name)
            } else {
                path.to_path_buf()
            };
            let parent = requested_destination.parent().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "output has no parent: {}",
                    requested_destination.display()
                ))
            })?;
            let output_root = prepare_explicit_parent(parent, &device_directory)?;
            let filename = requested_destination.file_name().ok_or_else(|| {
                Mobilebackup2Error::Protocol("output has no final component".into())
            })?;
            (output_root.clone(), output_root.join(filename))
        }
        None => {
            let output_root = prepare_output_directory(
                &root,
                &device_directory,
                target_identifier,
                None,
                "extract",
            )?;
            let destination = output_root.join(&entry.domain).join(&entry.relative_path);
            (output_root, destination)
        }
    };
    ensure_no_symlink_components_at_root(
        &output_root,
        destination
            .strip_prefix(&output_root)
            .unwrap_or(Path::new(".")),
    )?;

    let mut result = ExtractionResult {
        output_directory: output_root.clone(),
        entries_seen: 1,
        files_extracted: 0,
        bytes_extracted: 0,
    };
    extract_entry_to(
        &device_directory,
        &output_root,
        &entry,
        Some(&destination),
        manifest.crypto.as_ref(),
        manifest.modern,
        &mut result,
    )?;
    Ok(result)
}

/// List the entries in a completed host-side backup without contacting a device.
///
/// The manifest format is selected from `Manifest.plist`'s ProductVersion: modern backups use
/// SQLite `Manifest.db`, while iOS 10.2 and older backups use the flat `Manifest.mbdb` index.
/// Encrypted backups still require a password so the keybag is validated before entries are
/// returned, even though the legacy index itself is plaintext.
pub fn list_backup_entries(
    backup_root: &Path,
    target_identifier: &str,
    password: Option<&str>,
) -> Result<Vec<BackupManifestEntry>, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    ensure_backup_directory(backup_root, target_identifier)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = safe_device_directory(&root, target_identifier)?;
    let manifest = open_manifest_workspace(&device_directory, password)?;
    let entries = load_manifest_entries(&manifest)?;
    Ok(entries
        .into_iter()
        .map(|entry| BackupManifestEntry {
            file_id: entry.file_id,
            domain: entry.domain,
            relative_path: entry.relative_path,
            mode: entry.mode,
            size: entry.size,
            link_target: entry.link_target,
        })
        .collect())
}

fn open_manifest_workspace(
    device_directory: &Path,
    password: Option<&str>,
) -> Result<ManifestWorkspace, Mobilebackup2Error> {
    let mut manifest_plist =
        read_backup_dictionary(&safe_file(device_directory, "Manifest.plist")?)?;
    let encrypted = manifest_is_encrypted(&manifest_plist);
    // Move known key material out of the generic plist value before any later validation can
    // return.  plist::Value owns plain Vec<u8> buffers and does not zeroize them on drop; keeping
    // these fields in Zeroizing ensures malformed encrypted manifests do not retain keys simply
    // because ProductVersion or another control field is invalid.
    let (keybag, manifest_key) = if encrypted {
        let keybag = match manifest_plist.remove("BackupKeyBag") {
            Some(plist::Value::Data(value)) => Zeroizing::new(value),
            Some(_) => {
                return Err(Mobilebackup2Error::Protocol(
                    "encrypted Manifest.plist BackupKeyBag is not data".into(),
                ))
            }
            None => {
                return Err(Mobilebackup2Error::Protocol(
                    "encrypted Manifest.plist is missing BackupKeyBag".into(),
                ))
            }
        };
        let manifest_key = match manifest_plist.remove("ManifestKey") {
            Some(plist::Value::Data(value)) => Some(Zeroizing::new(value)),
            Some(_) => {
                return Err(Mobilebackup2Error::Protocol(
                    "encrypted Manifest.plist ManifestKey is not data".into(),
                ))
            }
            None => None,
        };
        (Some(keybag), manifest_key)
    } else {
        (None, None)
    };
    let product_version = manifest_plist
        .get("Lockdown")
        .and_then(plist::Value::as_dictionary)
        .and_then(|lockdown| lockdown.get("ProductVersion"))
        .and_then(plist::Value::as_string)
        .ok_or_else(|| {
            Mobilebackup2Error::Protocol(
                if encrypted {
                    "encrypted Manifest.plist is missing Lockdown.ProductVersion"
                } else {
                    "Manifest.plist is missing Lockdown.ProductVersion; legacy Manifest.mbdb cannot be handled safely"
                }
                    .into(),
            )
    })?;
    let modern = is_modern_storage(product_version)?;
    let manifest_db = optional_safe_file(device_directory, "Manifest.db")?;
    let manifest_mbdb = optional_safe_file(device_directory, "Manifest.mbdb")?;
    let (original_path, mbdb) = if modern {
        let path = manifest_db.ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "ProductVersion {product_version:?} requires Manifest.db; Manifest.mbdb is a legacy index"
            ))
        })?;
        validate_manifest_sidecars(&path)?;
        (path, None)
    } else {
        let path = manifest_mbdb.ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "ProductVersion {product_version:?} requires legacy Manifest.mbdb; Manifest.db is a modern index"
            ))
        })?;
        let bytes = read_bounded_file(&path, MAX_MANIFEST_DB_BYTES, "Manifest.mbdb")?;
        let parsed = MbdbManifest::parse(&bytes).map_err(mbdb_error)?;
        (path, Some(parsed))
    };
    if !encrypted {
        return Ok(ManifestWorkspace {
            source_path: original_path.clone(),
            original_path,
            crypto: None,
            modern,
            mbdb,
            _temporary: None,
        });
    }

    let password = password.ok_or_else(|| {
        Mobilebackup2Error::Protocol(
            "encrypted backup requires a password for local manifest operations".into(),
        )
    })?;
    let keybag = keybag.ok_or_else(|| {
        Mobilebackup2Error::Protocol("encrypted Manifest.plist is missing BackupKeyBag".into())
    })?;
    if !modern {
        let crypto = BackupCrypto::from_keybag(&keybag, password, false).map_err(crypto_error)?;
        return Ok(ManifestWorkspace {
            source_path: original_path.clone(),
            original_path,
            crypto: Some(crypto),
            modern,
            mbdb,
            _temporary: None,
        });
    }
    let manifest_key = manifest_key.ok_or_else(|| {
        Mobilebackup2Error::Protocol(
            "encrypted modern Manifest.plist is missing ManifestKey".into(),
        )
    })?;
    let crypto = BackupCrypto::from_manifest(&keybag, &manifest_key, password, true)
        .map_err(crypto_error)?;
    let temporary = create_temporary_file(device_directory, "manifest-plain")?;
    if let Err(error) = crypto.decrypt_manifest_file(&original_path, &temporary.path) {
        return Err(crypto_error(error));
    }
    Ok(ManifestWorkspace {
        source_path: temporary.path.clone(),
        original_path,
        crypto: Some(crypto),
        modern,
        mbdb,
        _temporary: Some(temporary),
    })
}

fn manifest_is_encrypted(manifest: &plist::Dictionary) -> bool {
    manifest
        .get("IsEncrypted")
        .and_then(|value| match value {
            plist::Value::Boolean(value) => Some(*value),
            plist::Value::Integer(value) => value
                .as_signed()
                .map(|value| value != 0)
                .or_else(|| value.as_unsigned().map(|value| value != 0)),
            _ => None,
        })
        .unwrap_or(false)
}

fn crypto_error(error: CryptoError) -> Mobilebackup2Error {
    Mobilebackup2Error::Protocol(error.to_string())
}

fn mbdb_error(error: MbdbError) -> Mobilebackup2Error {
    Mobilebackup2Error::Protocol(format!("Manifest.mbdb is malformed: {error}"))
}

fn safe_device_directory(
    root: &Path,
    target_identifier: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let relative = Path::new(target_identifier);
    ensure_no_symlink_components_at_root(root, relative)?;
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_dir() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup device path is not a directory: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn safe_file(directory: &Path, name: &str) -> Result<PathBuf, Mobilebackup2Error> {
    let path = directory.join(name);
    super::reject_symlink_components(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_file() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup metadata path is not a regular file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn optional_safe_file(directory: &Path, name: &str) -> Result<Option<PathBuf>, Mobilebackup2Error> {
    match safe_file(directory, name) {
        Ok(path) => Ok(Some(path)),
        Err(Mobilebackup2Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn read_bounded_file(
    path: &Path,
    maximum: u64,
    description: &str,
) -> Result<Vec<u8>, Mobilebackup2Error> {
    let mut file = open_file_for_read(path)?;
    let length = file.metadata()?.len();
    if length > maximum {
        return Err(Mobilebackup2Error::Protocol(format!(
            "{description} is too large ({} bytes; max {maximum})",
            length
        )));
    }
    let capacity = usize::try_from(length).map_err(|_| {
        Mobilebackup2Error::Protocol(format!("{description} length does not fit in host memory"))
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)?;
    if bytes.len() as u64 != length {
        return Err(Mobilebackup2Error::Protocol(format!(
            "{description} changed while being read"
        )));
    }
    Ok(bytes)
}

fn create_temporary_file(
    directory: &Path,
    purpose: &str,
) -> Result<TemporaryFile, Mobilebackup2Error> {
    if purpose.is_empty()
        || purpose
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-' && byte != b'_')
    {
        return Err(Mobilebackup2Error::Protocol(
            "temporary file purpose is not safe".into(),
        ));
    }
    for _ in 0..16 {
        let name = format!(".{purpose}-{}.tmp", Uuid::new_v4());
        let path = directory.join(name);
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                if let Err(error) = file.sync_all() {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error.into());
                }
                return Ok(TemporaryFile { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Mobilebackup2Error::Protocol(
        "could not allocate a unique temporary backup file".into(),
    ))
}

fn replace_file_atomically(temporary: &Path, destination: &Path) -> Result<(), Mobilebackup2Error> {
    // The temporary file is created in the destination directory and has already been synced by
    // the crypto writer. Unix rename atomically replaces the old file. Windows' std::fs::rename
    // does not replace an existing destination, so use MoveFileEx(REPLACE_EXISTING) there rather
    // than deleting the original first.
    super::reject_symlink_components(destination)?;
    #[cfg(windows)]
    replace_existing_windows(temporary, destination)?;
    #[cfg(not(windows))]
    fs::rename(temporary, destination)?;
    // A directory fsync failure occurs after the atomic replacement and must not be reported as
    // a failed transaction: callers cannot safely roll the replacement back at that point.
    if let Err(error) = sync_directory(destination.parent()) {
        warn!(path = %destination.display(), error = %error, "could not sync backup replacement directory");
    }
    Ok(())
}

#[cfg(windows)]
fn replace_existing_windows(
    temporary: &Path,
    destination: &Path,
) -> Result<(), Mobilebackup2Error> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x2;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(once(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect();
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let result = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

fn sync_directory(directory: Option<&Path>) -> Result<(), Mobilebackup2Error> {
    #[cfg(unix)]
    if let Some(directory) = directory {
        File::open(directory)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

fn validate_manifest_sidecars(path: &Path) -> Result<(), Mobilebackup2Error> {
    for suffix in ["-shm", "-wal", "-journal"] {
        let sidecar = path.with_file_name(format!(
            "{}{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Manifest.db"),
            suffix
        ));
        let metadata = match fs::symlink_metadata(&sidecar) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&sidecar));
        }
        if !metadata.is_file() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db sidecar is not a regular file: {}",
                sidecar.display()
            )));
        }
        if metadata.len() > MAX_MANIFEST_DB_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db sidecar is too large ({} bytes; max {MAX_MANIFEST_DB_BYTES}): {}",
                metadata.len(),
                sidecar.display()
            )));
        }
        if metadata.len() != 0 {
            return Err(Mobilebackup2Error::Protocol(format!(
                "non-empty Manifest.db sidecar is unsupported: {}",
                sidecar.display()
            )));
        }
    }
    Ok(())
}

fn open_manifest(path: &Path, read_only: bool) -> Result<Connection, Mobilebackup2Error> {
    let size = fs::symlink_metadata(path)?.len();
    if size > MAX_MANIFEST_DB_BYTES {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db is too large ({} bytes; max {MAX_MANIFEST_DB_BYTES})",
            size
        )));
    }
    // SQLite may open the WAL/SHM/journal companions even when the main database is opened with
    // SQLITE_OPEN_NOFOLLOW.  Only empty companions are accepted; a non-empty sidecar belongs to
    // a live/incomplete SQLite state that cannot be safely decrypted or patched in isolation.
    validate_manifest_sidecars(path)?;
    let flags = if read_only {
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW
    } else {
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW
    };
    Connection::open_with_flags(path, flags).map_err(sqlite_error)
}

fn collect_manifest_rows(
    path: &Path,
    filter: &BackupFilter,
) -> Result<ManifestFilterRows, Mobilebackup2Error> {
    let connection = open_manifest(path, true)?;
    let mut statement = connection
        // Check text lengths before asking SQLite/rusqlite to materialize attacker-controlled
        // strings into Rust allocations.
        .prepare(
            "SELECT rowid, length(fileID), length(domain), length(relativePath), \
                    fileID, domain, relativePath FROM Files",
        )
        .map_err(sqlite_error)?;
    let mut rows = statement.query([]).map_err(sqlite_error)?;
    let mut all = Vec::new();
    let mut remove = Vec::new();
    let mut allowed_ids = HashSet::new();
    let mut seen_ids = HashSet::new();
    while let Some(row) = rows.next().map_err(sqlite_error)? {
        if all.len() >= MAX_MANIFEST_ROWS {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db exceeds the {MAX_MANIFEST_ROWS} row safety limit"
            )));
        }
        let rowid: i64 = row.get(0).map_err(sqlite_error)?;
        check_manifest_text_length(row.get(1).map_err(sqlite_error)?, "fileID")?;
        check_manifest_text_length(row.get(2).map_err(sqlite_error)?, "domain")?;
        check_manifest_text_length(row.get(3).map_err(sqlite_error)?, "relativePath")?;
        let file_id: String = row.get(4).map_err(sqlite_error)?;
        let domain: String = row.get(5).map_err(sqlite_error)?;
        let relative_path: String = row.get(6).map_err(sqlite_error)?;
        check_manifest_text_bytes(&file_id, "fileID")?;
        check_manifest_text_bytes(&domain, "domain")?;
        check_manifest_text_bytes(&relative_path, "relativePath")?;
        validate_file_id(&file_id)?;
        let domain = validate_domain(&domain)?;
        let relative_path = sanitize_relative_path(&relative_path)?
            .to_string_lossy()
            .into_owned();
        if !seen_ids.insert(file_id.clone()) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db contains duplicate fileID {file_id:?}"
            )));
        }
        let keep = filter.matches_manifest_entry(&domain, &relative_path);
        all.push((rowid, file_id.clone()));
        if keep {
            allowed_ids.insert(file_id);
        } else {
            remove.push(rowid);
        }
    }
    Ok((all, remove, allowed_ids))
}

fn load_manifest_entries(
    manifest: &ManifestWorkspace,
) -> Result<Vec<ManifestEntry>, Mobilebackup2Error> {
    if let Some(mbdb) = manifest.mbdb.as_ref() {
        return load_mbdb_entries(mbdb, manifest.crypto.is_some());
    }
    let connection = open_manifest(&manifest.source_path, true)?;
    let mut statement = connection
        // Read the blob length first. Calling row.get::<_, Vec<u8>>() before checking it would
        // allocate an attacker-controlled SQLite BLOB in full, defeating the archive budget.
        .prepare(
            "SELECT length(fileID), length(domain), length(relativePath), length(file), \
                    fileID, domain, relativePath, file FROM Files",
        )
        .map_err(sqlite_error)?;
    let mut rows = statement.query([]).map_err(sqlite_error)?;
    let mut entries = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut total_size = 0u64;
    let mut total_archive_bytes = 0u64;
    while let Some(row) = rows.next().map_err(sqlite_error)? {
        if entries.len() >= MAX_MANIFEST_ROWS {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db exceeds the {MAX_MANIFEST_ROWS} row safety limit"
            )));
        }
        check_manifest_text_length(row.get(0).map_err(sqlite_error)?, "fileID")?;
        check_manifest_text_length(row.get(1).map_err(sqlite_error)?, "domain")?;
        check_manifest_text_length(row.get(2).map_err(sqlite_error)?, "relativePath")?;
        let archive_len: i64 = row.get(3).map_err(sqlite_error)?;
        let file_id: String = row.get(4).map_err(sqlite_error)?;
        let domain: String = row.get(5).map_err(sqlite_error)?;
        let relative_path: String = row.get(6).map_err(sqlite_error)?;
        check_manifest_text_bytes(&file_id, "fileID")?;
        check_manifest_text_bytes(&domain, "domain")?;
        check_manifest_text_bytes(&relative_path, "relativePath")?;
        if archive_len < 0 || archive_len as u64 > MAX_ARCHIVE_BYTES as u64 {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata archive for {file_id} is too large ({} bytes; max {MAX_ARCHIVE_BYTES})",
                archive_len.max(0)
            )));
        }
        let archive: Vec<u8> = row.get(7).map_err(sqlite_error)?;
        if archive.len() != archive_len as usize {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata archive for {file_id} changed while being read"
            )));
        }
        validate_file_id(&file_id)?;
        let domain = validate_domain(&domain)?;
        let relative_path = sanitize_relative_path(&relative_path)?
            .to_string_lossy()
            .into_owned();
        if !seen_ids.insert(file_id.clone()) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db contains duplicate fileID {file_id:?}"
            )));
        }
        if !seen_paths.insert((domain.clone(), relative_path.clone())) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db contains duplicate entry {domain}/{relative_path}"
            )));
        }
        if archive.len() > MAX_ARCHIVE_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata archive for {file_id} is too large ({} bytes; max {MAX_ARCHIVE_BYTES})",
                archive.len()
            )));
        }
        total_archive_bytes = total_archive_bytes
            .checked_add(u64::try_from(archive.len()).map_err(|_| {
                Mobilebackup2Error::Protocol("Manifest.db archive length does not fit u64".into())
            })?)
            .ok_or_else(|| {
                Mobilebackup2Error::Protocol(
                    "Manifest.db metadata archive byte count overflow".into(),
                )
            })?;
        if total_archive_bytes > MAX_TOTAL_ARCHIVE_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata archives exceed {MAX_TOTAL_ARCHIVE_BYTES} bytes"
            )));
        }
        let value = unarchive(&archive).map_err(|error| {
            Mobilebackup2Error::Protocol(format!(
                "cannot decode Manifest.db metadata for {file_id}: {error}"
            ))
        })?;
        let dictionary = value.as_dict().ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata for {file_id} is not an MBFile dictionary"
            ))
        })?;
        let mode = archive_integer(dictionary, "Mode")?.ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!("Manifest.db metadata for {file_id} has no Mode"))
        })?;
        let mode = u32::try_from(mode).map_err(|_| {
            Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata Mode for {file_id} is outside u32"
            ))
        })?;
        let file_type = mode & MODE_TYPE_MASK;
        if !matches!(
            file_type,
            MODE_TYPE_FILE | MODE_TYPE_DIRECTORY | MODE_TYPE_SYMLINK
        ) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata for {file_id} has unsupported mode type 0x{file_type:04x}"
            )));
        }
        let size = archive_integer(dictionary, "Size")?.unwrap_or(0);
        let size = u64::try_from(size).map_err(|_| {
            Mobilebackup2Error::Protocol(format!(
                "Manifest.db metadata Size for {file_id} is negative"
            ))
        })?;
        if size > MAX_SINGLE_ENTRY_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db entry {file_id} is too large ({} bytes; max {MAX_SINGLE_ENTRY_BYTES})",
                size
            )));
        }
        total_size = total_size.checked_add(size).ok_or_else(|| {
            Mobilebackup2Error::Protocol("Manifest.db entry size total overflow".into())
        })?;
        if total_size > MAX_TOTAL_EXTRACT_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db total extraction size exceeds {MAX_TOTAL_EXTRACT_BYTES} bytes"
            )));
        }
        let encryption_key = archive_encryption_key(dictionary, &file_id)?;
        if manifest.crypto.is_some()
            && matches!(file_type, MODE_TYPE_FILE | MODE_TYPE_SYMLINK)
            && encryption_key.is_none()
        {
            return Err(Mobilebackup2Error::Protocol(format!(
                "encrypted Manifest.db entry {file_id} is missing EncryptionKey"
            )));
        }
        entries.push(ManifestEntry {
            file_id,
            domain,
            relative_path,
            mode,
            size,
            encryption_key,
            link_target: None,
        });
    }
    Ok(entries)
}

fn load_mbdb_entries(
    manifest: &MbdbManifest,
    encrypted: bool,
) -> Result<Vec<ManifestEntry>, Mobilebackup2Error> {
    if manifest.records.len() > MAX_MANIFEST_ROWS {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.mbdb exceeds the {MAX_MANIFEST_ROWS} record safety limit"
        )));
    }
    let mut entries = Vec::with_capacity(manifest.records.len());
    let mut seen_ids = HashSet::new();
    let mut seen_paths = HashSet::new();
    let mut total_size = 0u64;
    for record in &manifest.records {
        let file_id = record.file_id().map_err(mbdb_error)?;
        validate_file_id(&file_id)?;
        let domain = record.domain.as_deref().ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb record {file_id} is missing domain"
            ))
        })?;
        let domain = validate_domain(domain)?;
        let relative_path = record.relative_path.as_deref().ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb record {file_id} is missing relative path"
            ))
        })?;
        let relative_path = sanitize_relative_path(relative_path)?
            .to_string_lossy()
            .into_owned();
        if !seen_ids.insert(file_id.clone()) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb contains duplicate file id {file_id:?}"
            )));
        }
        if !seen_paths.insert((domain.clone(), relative_path.clone())) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb contains duplicate entry {domain}/{relative_path}"
            )));
        }
        let mode = u32::from(record.mode);
        let file_type = mode & MODE_TYPE_MASK;
        if !matches!(
            file_type,
            MODE_TYPE_FILE | MODE_TYPE_DIRECTORY | MODE_TYPE_SYMLINK
        ) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb record {file_id} has unsupported mode type 0x{file_type:04x}"
            )));
        }
        if file_type == MODE_TYPE_SYMLINK {
            if let Some(target) = record.link_target.as_deref() {
                if target.len() > MAX_SYMLINK_TARGET_BYTES {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "Manifest.mbdb symlink target for {file_id} is too large"
                    )));
                }
                normalize_symlink_target(target)?;
            }
        }
        if record.size > MAX_SINGLE_ENTRY_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb entry {file_id} is too large ({} bytes; max {MAX_SINGLE_ENTRY_BYTES})",
                record.size
            )));
        }
        total_size = total_size.checked_add(record.size).ok_or_else(|| {
            Mobilebackup2Error::Protocol("Manifest.mbdb entry size total overflow".into())
        })?;
        if total_size > MAX_TOTAL_EXTRACT_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.mbdb total extraction size exceeds {MAX_TOTAL_EXTRACT_BYTES} bytes"
            )));
        }
        let encryption_key = record
            .encryption_key
            .as_ref()
            .map(|key| {
                if key.len() == 44 {
                    Ok(Zeroizing::new(key.to_vec()))
                } else {
                    Err(Mobilebackup2Error::Protocol(format!(
                        "Manifest.mbdb EncryptionKey for {file_id} has invalid length {}",
                        key.len()
                    )))
                }
            })
            .transpose()?;
        if encrypted
            && matches!(file_type, MODE_TYPE_FILE | MODE_TYPE_SYMLINK)
            && encryption_key.is_none()
        {
            return Err(Mobilebackup2Error::Protocol(format!(
                "encrypted Manifest.mbdb entry {file_id} is missing EncryptionKey"
            )));
        }
        entries.push(ManifestEntry {
            file_id,
            domain,
            relative_path,
            mode,
            size: record.size,
            encryption_key,
            link_target: record.link_target.clone(),
        });
    }
    Ok(entries)
}

fn archive_encryption_key(
    dictionary: &HashMap<String, ArchiveValue>,
    file_id: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, Mobilebackup2Error> {
    let Some(value) = dictionary.get("EncryptionKey") else {
        return Ok(None);
    };
    let ArchiveValue::Data(data) = value else {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db EncryptionKey for {file_id} is not NSData"
        )));
    };
    if data.len() != 44 {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db EncryptionKey for {file_id} has invalid length {}",
            data.len()
        )));
    }
    Ok(Some(Zeroizing::new(data.to_vec())))
}

fn archive_integer(
    dictionary: &HashMap<String, ArchiveValue>,
    key: &str,
) -> Result<Option<i64>, Mobilebackup2Error> {
    dictionary
        .get(key)
        .map(|value| {
            value.as_int().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "Manifest.db metadata field {key} is not an integer"
                ))
            })
        })
        .transpose()
}

fn validate_file_id(file_id: &str) -> Result<(), Mobilebackup2Error> {
    validate_backup_identifier(file_id).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "Manifest.db fileID must be one safe path component: {file_id:?}"
        ))
    })
}

fn check_manifest_text_length(length: i64, field: &str) -> Result<(), Mobilebackup2Error> {
    if length < 0 || length as u64 > MAX_MANIFEST_TEXT_BYTES {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db {field} text is too large ({} bytes; max {MAX_MANIFEST_TEXT_BYTES})",
            length.max(0)
        )));
    }
    Ok(())
}

fn check_manifest_text_bytes(value: &str, field: &str) -> Result<(), Mobilebackup2Error> {
    if value.len() as u64 > MAX_MANIFEST_TEXT_BYTES {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db {field} text is too large ({} bytes; max {MAX_MANIFEST_TEXT_BYTES})",
            value.len()
        )));
    }
    Ok(())
}

fn validate_domain(domain: &str) -> Result<String, Mobilebackup2Error> {
    if domain.is_empty()
        || domain.as_bytes().contains(&0)
        || domain.contains('/')
        || domain.contains('\\')
    {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db domain must be one path component: {domain:?}"
        )));
    }
    if super::has_windows_drive_prefix(domain) {
        return Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db domain has a platform path prefix: {domain:?}"
        )));
    }
    match Path::new(domain).components().next() {
        Some(Component::Normal(_)) if Path::new(domain).components().count() == 1 => {
            Ok(domain.to_owned())
        }
        _ => Err(Mobilebackup2Error::Protocol(format!(
            "Manifest.db domain must be one normal path component: {domain:?}"
        ))),
    }
}

fn copy_manifest_to_temporary(
    source: &Path,
    directory: &Path,
) -> Result<TemporaryFile, Mobilebackup2Error> {
    let temporary = create_temporary_file(directory, "manifest-patch")?;
    let result = (|| {
        let mut input = open_file_for_read(source)?;
        let length = input.metadata()?.len();
        if length > MAX_MANIFEST_DB_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db is too large ({} bytes; max {MAX_MANIFEST_DB_BYTES})",
                length
            )));
        }
        let mut output = open_file_for_write(&temporary.path)?;
        std::io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(temporary);
        return Err(error);
    }
    Ok(temporary)
}

fn patch_manifest_file(path: &Path, row_ids: &[i64]) -> Result<(), Mobilebackup2Error> {
    if row_ids.is_empty() {
        return Ok(());
    }
    let mut connection = open_manifest(path, false)?;
    let transaction = connection.transaction().map_err(sqlite_error)?;
    for rowid in row_ids {
        transaction
            .execute("DELETE FROM Files WHERE rowid = ?1", params![rowid])
            .map_err(sqlite_error)?;
    }
    transaction.commit().map_err(sqlite_error)?;
    File::open(path)?.sync_all()?;
    Ok(())
}

struct PayloadStage {
    directory: PathBuf,
    staging: PathBuf,
    moved: Vec<(PathBuf, PathBuf)>,
    removed: u64,
}

impl PayloadStage {
    fn rollback(&mut self) {
        for (source, staged) in self.moved.iter().rev() {
            if fs::symlink_metadata(staged).is_ok() {
                if let Err(error) = fs::rename(staged, source) {
                    warn!(
                        source = %source.display(),
                        staged = %staged.display(),
                        error = %error,
                        "could not roll back staged backup payload"
                    );
                }
            }
        }
        let _ = fs::remove_dir(&self.staging);
    }

    fn finish(&mut self) -> u64 {
        // Staged payloads are the files selected for removal.  They must be unlinked before the
        // private staging directory can be removed; `remove_dir` alone always fails while those
        // files are still present and would leave the supposedly committed backup behind with
        // a second copy of every removed payload.  Use symlink_metadata and remove_file so an
        // unexpected replacement cannot make cleanup recurse outside the staging directory.
        let staging_is_directory = match fs::symlink_metadata(&self.staging) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => true,
            Ok(metadata) if metadata.file_type().is_symlink() => {
                warn!(path = %self.staging.display(), "refusing to clean symlinked backup staging directory");
                false
            }
            Ok(_) => {
                warn!(path = %self.staging.display(), "refusing to clean non-directory backup staging path");
                false
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                warn!(
                    path = %self.staging.display(),
                    error = %error,
                    "could not inspect backup payload staging directory"
                );
                false
            }
        };
        if staging_is_directory {
            match fs::read_dir(&self.staging) {
                Ok(entries) => {
                    for entry in entries {
                        let path = match entry {
                            Ok(entry) => entry.path(),
                            Err(error) => {
                                warn!(
                                    path = %self.staging.display(),
                                    error = %error,
                                    "could not enumerate backup payload staging directory"
                                );
                                continue;
                            }
                        };
                        match fs::symlink_metadata(&path) {
                            Ok(metadata)
                                if metadata.is_file() || metadata.file_type().is_symlink() =>
                            {
                                if let Err(error) = fs::remove_file(&path) {
                                    warn!(
                                        path = %path.display(),
                                        error = %error,
                                        "could not remove staged backup payload"
                                    );
                                }
                            }
                            Ok(metadata) if metadata.is_dir() => {
                                // Stage entries are flat regular files.  Never recursively delete an
                                // unexpected directory because the staging path shares a user-owned
                                // filesystem and cleanup must not follow an injected tree.
                                warn!(path = %path.display(), "refusing to recursively remove staged directory");
                            }
                            Ok(_) => {
                                warn!(path = %path.display(), "refusing to remove special staged payload");
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(error) => {
                                warn!(
                                    path = %path.display(),
                                    error = %error,
                                    "could not inspect staged backup payload"
                                );
                            }
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    warn!(
                        path = %self.staging.display(),
                        error = %error,
                        "could not inspect backup payload staging directory"
                    );
                }
            }
        }
        if let Err(error) = fs::remove_dir(&self.staging) {
            warn!(path = %self.staging.display(), error = %error, "could not remove backup payload staging directory");
        }
        // Prefix directories are only an index for hashed payload names. Remove only the direct
        // parents of files moved by this operation; scanning every empty directory here could
        // delete unrelated user-created directories. Do not turn a cleanup failure into a false
        // rollback after Manifest.db was already atomically replaced.
        let prefix_directories: HashSet<PathBuf> = self
            .moved
            .iter()
            .filter_map(|(source, _)| source.parent())
            .filter(|parent| *parent != self.directory)
            .map(Path::to_path_buf)
            .collect();
        for path in prefix_directories {
            if fs::symlink_metadata(&path)
                .map(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
                .unwrap_or(false)
                && fs::read_dir(&path)
                    .map(|mut children| children.next().is_none())
                    .unwrap_or(false)
            {
                let _ = fs::remove_dir(path);
            }
        }
        self.removed
    }
}

fn stage_payloads(
    device_directory: &Path,
    allowed_ids: &HashSet<String>,
) -> Result<PayloadStage, Mobilebackup2Error> {
    let mut staging = None;
    for _ in 0..16 {
        let candidate = device_directory.join(format!(".backup2-stage-{}", Uuid::new_v4()));
        match fs::create_dir(&candidate) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700))?;
                }
                staging = Some(candidate);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    let staging = staging.ok_or_else(|| {
        Mobilebackup2Error::Protocol("could not allocate backup payload staging directory".into())
    })?;
    let mut result = PayloadStage {
        directory: device_directory.to_owned(),
        staging,
        moved: Vec::new(),
        removed: 0,
    };
    let operation = (|| {
        for entry in fs::read_dir(device_directory)? {
            let entry = entry?;
            let path = entry.path();
            if path == result.staging {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // Replacement Manifest.db files are created as hidden siblings. They are control
            // artifacts, not payloads, and must remain available until the final atomic rename.
            if name.starts_with(".manifest-") {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                return Err(symlink_path_error(&path));
            }
            if metadata.is_file() {
                if !matches!(
                    name.as_str(),
                    "Info.plist"
                        | "Manifest.plist"
                        | "Manifest.db"
                        | "Manifest.mbdx"
                        | "Manifest.db-shm"
                        | "Manifest.db-wal"
                        | "Status.plist"
                ) && !allowed_ids.contains(&name)
                {
                    result.move_to_staging(&path, &name)?;
                }
                continue;
            }
            if !metadata.is_dir() {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup payload is neither a file nor directory: {}",
                    path.display()
                )));
            }
            for child in fs::read_dir(&path)? {
                let child = child?;
                let child_path = child.path();
                let child_name = child.file_name().to_string_lossy().into_owned();
                let child_metadata = fs::symlink_metadata(&child_path)?;
                if child_metadata.file_type().is_symlink() {
                    return Err(symlink_path_error(&child_path));
                }
                if !child_metadata.is_file() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "backup payload directory is nested or special: {}",
                        child_path.display()
                    )));
                }
                if !allowed_ids.contains(&child_name) {
                    result.move_to_staging(&child_path, &child_name)?;
                }
            }
        }
        Ok::<(), Mobilebackup2Error>(())
    })();
    if let Err(error) = operation {
        result.rollback();
        return Err(error);
    }
    Ok(result)
}

impl PayloadStage {
    fn move_to_staging(&mut self, source: &Path, name: &str) -> Result<(), Mobilebackup2Error> {
        let staged = self.staging.join(format!("{}-{}", self.moved.len(), name));
        fs::rename(source, &staged)?;
        self.moved.push((source.to_owned(), staged));
        self.removed = self.removed.saturating_add(1);
        Ok(())
    }
}

fn validate_payload_tree(device_directory: &Path) -> Result<(), Mobilebackup2Error> {
    let entries = fs::read_dir(device_directory)?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&path));
        }
        if metadata.is_dir() {
            for child in fs::read_dir(&path)? {
                let child = child?;
                let child_path = child.path();
                let child_metadata = fs::symlink_metadata(&child_path)?;
                if child_metadata.file_type().is_symlink() {
                    return Err(symlink_path_error(&child_path));
                }
                if !child_metadata.is_file() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "backup payload directory is nested or special: {}",
                        child_path.display()
                    )));
                }
            }
        } else if !metadata.is_file() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup payload is neither a file nor directory: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn prepare_output_directory(
    root: &Path,
    source_directory: &Path,
    target_identifier: &str,
    output: Option<&Path>,
    suffix: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let path = match output {
        Some(path) => path.to_path_buf(),
        None => root.join(format!("{target_identifier}.{suffix}")),
    };
    reject_output_path(&path)?;
    if output.is_none() {
        let relative = PathBuf::from(format!("{target_identifier}.{suffix}"));
        // pyiosbackup's host-side unback helper replaces its default sibling output.  Keep
        // that contract while refusing to recursively remove anything except an existing,
        // symlink-free directory wholly below the trusted backup root.
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.is_dir() {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "output path is not a directory: {}",
                    path.display()
                )));
            }
            super::ensure_tree_has_no_symlinks(&path)?;
            fs::remove_dir_all(&path)?;
        }
        let path = create_dir_all_no_symlink(root, &relative)?;
        reject_output_overlap(&path, source_directory)?;
        Ok(path)
    } else {
        fs::create_dir_all(&path)?;
        super::reject_symlink_components(&path)?;
        let path = fs::canonicalize(&path)?;
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "output path is not a directory: {}",
                path.display()
            )));
        }
        reject_output_overlap(&path, source_directory)?;
        Ok(path)
    }
}

fn prepare_explicit_parent(
    path: &Path,
    source_directory: &Path,
) -> Result<PathBuf, Mobilebackup2Error> {
    reject_output_path(path)?;
    fs::create_dir_all(path)?;
    super::reject_symlink_components(path)?;
    let canonical = fs::canonicalize(path)?;
    if !canonical.is_dir() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "output parent is not a directory: {}",
            canonical.display()
        )));
    }
    reject_output_overlap(&canonical, source_directory)?;
    Ok(canonical)
}

fn reject_output_path(path: &Path) -> Result<(), Mobilebackup2Error> {
    if path.as_os_str().is_empty() {
        return Err(Mobilebackup2Error::Protocol(
            "output path must not be empty".into(),
        ));
    }
    super::reject_symlink_components(path)
}

fn reject_output_overlap(path: &Path, source_directory: &Path) -> Result<(), Mobilebackup2Error> {
    if path == source_directory
        || path.starts_with(source_directory)
        || source_directory.starts_with(path)
    {
        return Err(Mobilebackup2Error::Protocol(format!(
            "output directory {} overlaps source backup {}",
            path.display(),
            source_directory.display()
        )));
    }
    Ok(())
}

fn extract_entries(
    source_directory: &Path,
    output_directory: &Path,
    entries: &[ManifestEntry],
    crypto: Option<&BackupCrypto>,
    modern: bool,
) -> Result<ExtractionResult, Mobilebackup2Error> {
    let mut result = ExtractionResult {
        output_directory: output_directory.to_path_buf(),
        entries_seen: u64::try_from(entries.len()).map_err(|_| {
            Mobilebackup2Error::Protocol("Manifest.db entry count does not fit in u64".into())
        })?,
        files_extracted: 0,
        bytes_extracted: 0,
    };
    let mut directories = Vec::new();
    for entry in entries {
        let destination = output_path(output_directory, entry)?;
        let is_directory = entry.mode & MODE_TYPE_MASK == MODE_TYPE_DIRECTORY;
        extract_entry_to(
            source_directory,
            output_directory,
            entry,
            Some(&destination),
            crypto,
            modern,
            &mut result,
        )?;
        if is_directory {
            directories.push((destination, entry.mode & 0o7777));
        }
    }
    for (directory, mode) in directories.into_iter().rev() {
        set_permissions(&directory, mode)?;
    }
    Ok(result)
}

fn output_path(
    output_directory: &Path,
    entry: &ManifestEntry,
) -> Result<PathBuf, Mobilebackup2Error> {
    let relative = Path::new(&entry.domain).join(&entry.relative_path);
    ensure_no_symlink_components_at_root(output_directory, &relative)?;
    Ok(output_directory.join(relative))
}

fn extract_entry_to(
    source_directory: &Path,
    output_directory: &Path,
    entry: &ManifestEntry,
    explicit_destination: Option<&Path>,
    crypto: Option<&BackupCrypto>,
    modern: bool,
    result: &mut ExtractionResult,
) -> Result<(), Mobilebackup2Error> {
    let destination = match explicit_destination {
        Some(path) => path.to_path_buf(),
        None => output_path(output_directory, entry)?,
    };
    let relative = destination.strip_prefix(output_directory).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "extraction destination is outside output directory: {}",
            destination.display()
        ))
    })?;
    ensure_no_symlink_components_at_root(output_directory, relative)?;
    let file_type = entry.mode & MODE_TYPE_MASK;
    match file_type {
        MODE_TYPE_DIRECTORY => {
            create_dir_all_no_symlink(output_directory, relative)?;
            if explicit_destination.is_some() {
                set_permissions(&destination, entry.mode & 0o7777)?;
            }
        }
        MODE_TYPE_FILE => {
            let parent = relative.parent().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "file destination has no parent: {}",
                    destination.display()
                ))
            })?;
            create_dir_all_no_symlink(output_directory, parent)?;
            let stored = stored_payload_path(source_directory, &entry.file_id, modern)?;
            let mut source = open_file_for_read(&stored)?;
            let stored_size = source.metadata()?.len();
            let temporary = create_temporary_file(
                destination.parent().ok_or_else(|| {
                    Mobilebackup2Error::Protocol("file destination has no parent".into())
                })?,
                "entry",
            )?;
            let copied = if let Some(crypto) = crypto {
                let encryption_key = entry.encryption_key.as_ref().ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "encrypted Manifest.db entry {} is missing EncryptionKey",
                        entry.file_id
                    ))
                })?;
                let expected_ciphertext =
                    expected_payload_ciphertext_len(entry.size).map_err(crypto_error)?;
                if stored_size != expected_ciphertext {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "encrypted payload size mismatch for {}: expected {}, stored {}",
                        entry.file_id, expected_ciphertext, stored_size
                    )));
                }
                drop(source);
                crypto
                    .decrypt_payload_file(
                        &stored,
                        &temporary.path,
                        encryption_key.as_ref(),
                        entry.size,
                    )
                    .map_err(crypto_error)?
            } else {
                if stored_size != entry.size {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "payload size mismatch for {}: Manifest.db says {}, stored {}",
                        entry.file_id, entry.size, stored_size
                    )));
                }
                let mut destination_file = open_file_for_write(&temporary.path)?;
                let copied =
                    copy_bounded_no_accounting(&mut source, &mut destination_file, entry.size)?;
                destination_file.flush()?;
                destination_file.sync_all()?;
                copied
            };
            if copied != entry.size {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "payload size mismatch for {}: Manifest.db says {}, copied {}",
                    entry.file_id, entry.size, copied
                )));
            }
            account_extracted_bytes(result, entry.size)?;
            replace_file_atomically(&temporary.path, &destination)?;
            set_permissions(&destination, entry.mode & 0o7777)?;
            result.files_extracted = result.files_extracted.saturating_add(1);
        }
        MODE_TYPE_SYMLINK => extract_symlink(
            source_directory,
            output_directory,
            entry,
            &destination,
            crypto,
            modern,
            result,
        )?,
        other => {
            return Err(Mobilebackup2Error::Protocol(format!(
                "unsupported Manifest.db mode type 0x{other:04x} for {}",
                entry.file_id
            )))
        }
    }
    Ok(())
}

fn copy_bounded_no_accounting<R: Read, W: Write>(
    source: &mut R,
    destination: &mut W,
    expected: u64,
) -> Result<u64, Mobilebackup2Error> {
    let mut remaining = expected;
    let mut buffer = [0u8; 128 * 1024];
    let mut copied = 0u64;
    while remaining > 0 {
        let request = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
            Mobilebackup2Error::Protocol("file transfer chunk does not fit usize".into())
        })?;
        let count = source.read(&mut buffer[..request])?;
        if count == 0 {
            break;
        }
        destination.write_all(&buffer[..count])?;
        let count = u64::try_from(count).map_err(|_| {
            Mobilebackup2Error::Protocol("file transfer count does not fit u64".into())
        })?;
        remaining -= count;
        copied = copied.checked_add(count).ok_or_else(|| {
            Mobilebackup2Error::Protocol("file transfer byte count overflow".into())
        })?;
    }
    Ok(copied)
}

fn account_extracted_bytes(
    result: &mut ExtractionResult,
    bytes: u64,
) -> Result<(), Mobilebackup2Error> {
    result.bytes_extracted = result.bytes_extracted.checked_add(bytes).ok_or_else(|| {
        Mobilebackup2Error::Protocol("total extracted byte count overflow".into())
    })?;
    if result.bytes_extracted > MAX_TOTAL_EXTRACT_BYTES {
        return Err(Mobilebackup2Error::Protocol(format!(
            "extraction exceeds {MAX_TOTAL_EXTRACT_BYTES} bytes"
        )));
    }
    Ok(())
}

fn stored_payload_path(
    source_directory: &Path,
    file_id: &str,
    modern: bool,
) -> Result<PathBuf, Mobilebackup2Error> {
    validate_file_id(file_id)?;
    let relative = if modern {
        let prefix: String = file_id.chars().take(2).collect();
        if prefix.chars().count() != 2 {
            return Err(Mobilebackup2Error::Protocol(format!(
                "Manifest.db fileID is too short for hashed storage: {file_id:?}"
            )));
        }
        PathBuf::from(prefix).join(file_id)
    } else {
        PathBuf::from(file_id)
    };
    ensure_no_symlink_components_at_root(source_directory, &relative)?;
    let path = source_directory.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_file() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup payload is not a regular file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn is_modern_storage(version: &str) -> Result<bool, Mobilebackup2Error> {
    // ProductVersion may carry an Apple prerelease suffix (for example 10.3rc1). Parse the
    // leading decimal portion of every component instead of treating that suffix as a parse
    // failure and accidentally selecting the legacy Manifest.mbdb layout.
    let numeric_component = |component: &str| -> Result<u64, Mobilebackup2Error> {
        let digits: String = component
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        if digits.is_empty() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "invalid ProductVersion component {component:?}"
            )));
        }
        digits.parse::<u64>().map_err(|_| {
            Mobilebackup2Error::Protocol(format!(
                "ProductVersion component {component:?} is too large"
            ))
        })
    };
    let mut components = version.split('.');
    let major = components
        .next()
        .ok_or_else(|| Mobilebackup2Error::Protocol("ProductVersion is empty".into()))
        .and_then(numeric_component)?;
    let minor = components
        .next()
        .map(numeric_component)
        .transpose()?
        .unwrap_or(0);
    let patch = components
        .next()
        .map(numeric_component)
        .transpose()?
        .unwrap_or(0);
    // pyiosbackup uses Version("10.2") as the cutoff, so 10.2.1 is modern too. Comparing only
    // major/minor would incorrectly look up a 10.2.x payload using the legacy flat layout.
    Ok(major > 10 || (major == 10 && (minor > 2 || (minor == 2 && patch > 0))))
}

#[cfg(not(unix))]
fn extract_symlink(
    _source_directory: &Path,
    _output_directory: &Path,
    _entry: &ManifestEntry,
    _destination: &Path,
    _crypto: Option<&BackupCrypto>,
    _modern: bool,
    _result: &mut ExtractionResult,
) -> Result<(), Mobilebackup2Error> {
    Err(Mobilebackup2Error::Protocol(
        "symlink extraction is unsupported on this platform".into(),
    ))
}

#[cfg(unix)]
fn extract_symlink(
    source_directory: &Path,
    output_directory: &Path,
    entry: &ManifestEntry,
    destination: &Path,
    crypto: Option<&BackupCrypto>,
    modern: bool,
    result: &mut ExtractionResult,
) -> Result<(), Mobilebackup2Error> {
    if entry.size > MAX_SYMLINK_TARGET_BYTES as u64 {
        return Err(Mobilebackup2Error::Protocol(format!(
            "symlink target for {} is too large",
            entry.file_id
        )));
    }
    let (target, accounted_size) = if let Some(link_target) = entry.link_target.as_deref() {
        let target = link_target.as_bytes().to_vec();
        if target.len() > MAX_SYMLINK_TARGET_BYTES {
            return Err(Mobilebackup2Error::Protocol(
                "symlink target exceeds safety limit".into(),
            ));
        }
        let size = u64::try_from(target.len()).map_err(|_| {
            Mobilebackup2Error::Protocol("symlink target length does not fit u64".into())
        })?;
        (target, size)
    } else {
        let stored = stored_payload_path(source_directory, &entry.file_id, modern)?;
        let mut target = Vec::new();
        if let Some(crypto) = crypto {
            let encryption_key = entry.encryption_key.as_ref().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "encrypted backup symlink {} is missing EncryptionKey",
                    entry.file_id
                ))
            })?;
            let expected_ciphertext =
                expected_payload_ciphertext_len(entry.size).map_err(crypto_error)?;
            let stored_size = fs::symlink_metadata(&stored)?.len();
            if stored_size != expected_ciphertext {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "encrypted symlink payload size mismatch for {}: expected {}, stored {}",
                    entry.file_id, expected_ciphertext, stored_size
                )));
            }
            let temporary = create_temporary_file(
                destination
                    .parent()
                    .ok_or_else(|| Mobilebackup2Error::Protocol("symlink has no parent".into()))?,
                "symlink",
            )?;
            crypto
                .decrypt_payload_file(
                    &stored,
                    &temporary.path,
                    encryption_key.as_ref(),
                    entry.size,
                )
                .map_err(crypto_error)?;
            let metadata = fs::metadata(&temporary.path)?;
            if metadata.len() > MAX_SYMLINK_TARGET_BYTES as u64 {
                return Err(Mobilebackup2Error::Protocol(
                    "symlink target exceeds safety limit".into(),
                ));
            }
            File::open(&temporary.path)?
                .take(MAX_SYMLINK_TARGET_BYTES as u64 + 1)
                .read_to_end(&mut target)?;
        } else {
            let source = open_file_for_read(&stored)?;
            source
                .take(MAX_SYMLINK_TARGET_BYTES as u64 + 1)
                .read_to_end(&mut target)?;
        }
        (target, entry.size)
    };
    if target.len() > MAX_SYMLINK_TARGET_BYTES {
        return Err(Mobilebackup2Error::Protocol(
            "symlink target exceeds safety limit".into(),
        ));
    }
    let target = String::from_utf8(target).map_err(|error| {
        Mobilebackup2Error::Protocol(format!("symlink target is not UTF-8: {error}"))
    })?;
    let clean_target = normalize_symlink_target(&target)?;
    let parent = destination
        .parent()
        .ok_or_else(|| Mobilebackup2Error::Protocol("symlink destination has no parent".into()))?;
    let target_path = parent.join(&clean_target);
    let target_relative = target_path.strip_prefix(output_directory).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "symlink target escapes output directory: {target:?}"
        ))
    })?;
    ensure_no_symlink_components_at_root(output_directory, target_relative)?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "refusing to overwrite extraction path: {}",
            destination.display()
        )));
    }
    if let Some(parent_relative) = destination
        .strip_prefix(output_directory)
        .ok()
        .and_then(Path::parent)
    {
        create_dir_all_no_symlink(output_directory, parent_relative)?;
    }
    std::os::unix::fs::symlink(&clean_target, destination)?;
    account_extracted_bytes(result, accounted_size)?;
    result.files_extracted = result.files_extracted.saturating_add(1);
    Ok(())
}

fn normalize_symlink_target(target: &str) -> Result<PathBuf, Mobilebackup2Error> {
    if target.is_empty() || target.as_bytes().contains(&0) || target.contains('\\') {
        return Err(Mobilebackup2Error::Protocol(format!(
            "symlink target is not a safe relative path: {target:?}"
        )));
    }
    let mut clean = PathBuf::new();
    for component in Path::new(target).components() {
        match component {
            Component::Normal(value) => clean.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !clean.pop() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "symlink target escapes output directory: {target:?}"
                    )));
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "symlink target must be relative: {target:?}"
                )))
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(Mobilebackup2Error::Protocol(
            "symlink target is empty".into(),
        ));
    }
    Ok(clean)
}

fn set_permissions(path: &Path, mode: u32) -> Result<(), Mobilebackup2Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

fn sqlite_error(error: rusqlite::Error) -> Mobilebackup2Error {
    Mobilebackup2Error::Protocol(format!("Manifest.db SQLite error: {error}"))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use rusqlite::Connection;
    use zeroize::Zeroize;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::proto::nskeyedarchiver_encode::archive_dict;

    fn fixture_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ios-core-backup2-manifest-{}-{label}-{nonce}",
            std::process::id()
        ))
    }

    fn metadata_archive(mode: i64, size: i64) -> Vec<u8> {
        archive_dict(vec![
            ("RelativePath".into(), plist::Value::String("unused".into())),
            ("Mode".into(), plist::Value::Integer(mode.into())),
            ("Size".into(), plist::Value::Integer(size.into())),
            ("GroupID".into(), plist::Value::Integer(0.into())),
            ("UserID".into(), plist::Value::Integer(0.into())),
        ])
    }

    fn write_fixture(label: &str) -> (PathBuf, PathBuf, String, String) {
        let root = fixture_root(label);
        let device = root.join("device-id");
        fs::create_dir_all(&device).expect("create fixture");
        fs::write(device.join("Info.plist"), b"info").expect("info");
        fs::write(device.join("Status.plist"), b"status").expect("status");
        plist::to_file_xml(
            device.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("IsEncrypted".to_string(), plist::Value::Boolean(false)),
                (
                    "Lockdown".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "ProductVersion".to_string(),
                        plist::Value::String("17.0".into()),
                    )])),
                ),
            ])),
        )
        .expect("manifest plist");

        let keep_id = "1111111111111111111111111111111111111111".to_string();
        let drop_id = "2222222222222222222222222222222222222222".to_string();
        let connection = Connection::open(device.join("Manifest.db")).expect("manifest db");
        connection
            .execute(
                "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT, file BLOB)",
                [],
            )
            .expect("create Files table");
        connection
            .execute(
                "INSERT INTO Files (fileID, domain, relativePath, file) VALUES (?1, ?2, ?3, ?4)",
                params![
                    &keep_id,
                    "HomeDomain",
                    "Library/SMS/sms.db",
                    metadata_archive(MODE_TYPE_FILE as i64 | 0o640, 4),
                ],
            )
            .expect("insert keep row");
        connection
            .execute(
                "INSERT INTO Files (fileID, domain, relativePath, file) VALUES (?1, ?2, ?3, ?4)",
                params![
                    &drop_id,
                    "HomeDomain",
                    "Library/Notes/note.db",
                    metadata_archive(MODE_TYPE_FILE as i64 | 0o600, 4),
                ],
            )
            .expect("insert drop row");
        drop(connection);

        let keep_path = device.join(&keep_id[..2]).join(&keep_id);
        let drop_path = device.join(&drop_id[..2]).join(&drop_id);
        fs::create_dir_all(keep_path.parent().expect("keep parent")).expect("keep directory");
        fs::create_dir_all(drop_path.parent().expect("drop parent")).expect("drop directory");
        fs::create_dir(device.join("unrelated-empty")).expect("unrelated directory");
        fs::write(&keep_path, b"keep").expect("keep payload");
        fs::write(&drop_path, b"drop").expect("drop payload");
        (root, device, keep_id, drop_id)
    }

    fn encrypted_metadata_archive(mode: i64, size: i64, encryption_key: &[u8]) -> Vec<u8> {
        archive_dict(vec![
            ("RelativePath".into(), plist::Value::String("unused".into())),
            ("Mode".into(), plist::Value::Integer(mode.into())),
            ("Size".into(), plist::Value::Integer(size.into())),
            ("GroupID".into(), plist::Value::Integer(501.into())),
            ("UserID".into(), plist::Value::Integer(501.into())),
            (
                "EncryptionKey".into(),
                plist::Value::Data(encryption_key.to_vec()),
            ),
        ])
    }

    fn append_tlv(output: &mut Vec<u8>, tag: &[u8; 4], value: &[u8]) {
        output.extend_from_slice(tag);
        output.extend_from_slice(
            &u32::try_from(value.len())
                .expect("test TLV length")
                .to_be_bytes(),
        );
        output.extend_from_slice(value);
    }

    fn append_tlv_integer(output: &mut Vec<u8>, tag: &[u8; 4], value: u32) {
        append_tlv(output, tag, &value.to_be_bytes());
    }

    fn test_encrypted_material(password: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        use aes_kw::KekAes256;
        use pbkdf2::pbkdf2_hmac;
        use sha1::Sha1;
        use sha2::Sha256;

        let salt = [0u8; 20];
        let password_salt = [0u8; 20];
        let mut intermediate = [0u8; 32];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &password_salt, 1, &mut intermediate);
        let mut password_key = [0u8; 32];
        pbkdf2_hmac::<Sha1>(&intermediate, &salt, 1, &mut password_key);
        intermediate.zeroize();

        let kek = KekAes256::try_from(&password_key[..]).expect("test KEK");
        password_key.zeroize();
        let mut wrapped_class_key = [0u8; 40];
        kek.wrap(&[0u8; 32], &mut wrapped_class_key)
            .expect("wrap class key");
        let class_kek = KekAes256::from([0u8; 32]);
        let mut wrapped_data_key = [0u8; 40];
        class_kek
            .wrap(&[0u8; 32], &mut wrapped_data_key)
            .expect("wrap data key");

        let mut keybag = Vec::new();
        append_tlv_integer(&mut keybag, b"VERS", 5);
        append_tlv_integer(&mut keybag, b"TYPE", 1);
        append_tlv(&mut keybag, b"UUID", &[0u8; 16]);
        append_tlv_integer(&mut keybag, b"WRAP", 0);
        append_tlv(&mut keybag, b"SALT", &salt);
        append_tlv_integer(&mut keybag, b"ITER", 1);
        append_tlv_integer(&mut keybag, b"DPWT", 1);
        append_tlv_integer(&mut keybag, b"DPIC", 1);
        append_tlv(&mut keybag, b"DPSL", &password_salt);
        append_tlv(&mut keybag, b"UUID", &[0u8; 16]);
        append_tlv_integer(&mut keybag, b"CLAS", 2);
        append_tlv_integer(&mut keybag, b"WRAP", 3);
        append_tlv_integer(&mut keybag, b"KTYP", 0);
        append_tlv(&mut keybag, b"WPKY", &wrapped_class_key);

        let mut encryption_key = vec![2, 0, 0, 0];
        encryption_key.extend_from_slice(&wrapped_data_key);
        (keybag, encryption_key.clone(), encryption_key)
    }

    fn legacy_encrypted_material(password: &str) -> (Vec<u8>, Vec<u8>) {
        use aes_kw::KekAes256;
        use pbkdf2::pbkdf2_hmac;
        use sha1::Sha1;

        let salt = [0u8; 20];
        let mut password_key = [0u8; 32];
        pbkdf2_hmac::<Sha1>(password.as_bytes(), &salt, 1, &mut password_key);
        let password_kek = KekAes256::try_from(&password_key[..]).expect("legacy password KEK");
        password_key.zeroize();
        let mut wrapped_class_key = [0u8; 40];
        password_kek
            .wrap(&[0u8; 32], &mut wrapped_class_key)
            .expect("legacy class key wrap");
        let class_kek = KekAes256::from([0u8; 32]);
        let mut wrapped_data_key = [0u8; 40];
        class_kek
            .wrap(&[0u8; 32], &mut wrapped_data_key)
            .expect("legacy data key wrap");

        let mut keybag = Vec::new();
        append_tlv_integer(&mut keybag, b"VERS", 5);
        append_tlv_integer(&mut keybag, b"TYPE", 1);
        append_tlv(&mut keybag, b"UUID", &[0u8; 16]);
        append_tlv_integer(&mut keybag, b"WRAP", 0);
        append_tlv(&mut keybag, b"SALT", &salt);
        append_tlv_integer(&mut keybag, b"ITER", 1);
        append_tlv(&mut keybag, b"UUID", &[0u8; 16]);
        append_tlv_integer(&mut keybag, b"CLAS", 2);
        append_tlv_integer(&mut keybag, b"WRAP", 3);
        append_tlv_integer(&mut keybag, b"KTYP", 0);
        append_tlv(&mut keybag, b"WPKY", &wrapped_class_key);

        let mut encryption_key = vec![2, 0, 0, 0];
        encryption_key.extend_from_slice(&wrapped_data_key);
        (keybag, encryption_key)
    }

    fn legacy_record(relative_path: &str, size: u64, encryption_key: Option<&[u8]>) -> MbdbRecord {
        MbdbRecord {
            domain: Some("MyTestDomain".into()),
            relative_path: Some(relative_path.into()),
            link_target: None,
            data_hash: None,
            encryption_key: encryption_key.map(|key| Zeroizing::new(key.to_vec())),
            mode: 0x81ed,
            unknown2: 0,
            unknown3: 0x12d8,
            user_id: 501,
            group_id: 501,
            mtime: 1_626_082_591,
            atime: 1_626_082_637,
            ctime: 1_626_011_940,
            size,
            flags: 4,
            properties: vec![(Some("Unknown".into()), Some("preserve".into()))],
        }
    }

    fn write_legacy_fixture(label: &str, encrypted: bool) -> (PathBuf, PathBuf, String, String) {
        let root = fixture_root(label);
        let device = root.join("device-id");
        fs::create_dir_all(&device).expect("create legacy fixture");
        fs::write(device.join("Info.plist"), b"info").expect("info");
        fs::write(device.join("Status.plist"), b"status").expect("status");

        let password = "0000";
        let (keybag, encryption_key) = if encrypted {
            let (keybag, key) = legacy_encrypted_material(password);
            (Some(keybag), Some(key))
        } else {
            (None, None)
        };
        let size = if encrypted { 9 } else { 4 };
        let keep_record = legacy_record("Media/Test.txt", size, encryption_key.as_deref());
        let drop_record = legacy_record("Media/Drop.txt", size, encryption_key.as_deref());
        let keep_id = keep_record.file_id().expect("keep id");
        let drop_id = drop_record.file_id().expect("drop id");
        let manifest = MbdbManifest {
            records: vec![keep_record, drop_record],
        };
        fs::write(
            device.join("Manifest.mbdb"),
            manifest.serialize(None).expect("legacy manifest"),
        )
        .expect("write legacy manifest");
        let mut manifest_values = vec![
            ("IsEncrypted".to_string(), plist::Value::Boolean(encrypted)),
            (
                "Lockdown".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ProductVersion".to_string(),
                    plist::Value::String("9.0.1".into()),
                )])),
            ),
        ];
        if let Some(keybag) = keybag {
            manifest_values.push(("BackupKeyBag".to_string(), plist::Value::Data(keybag)));
        }
        plist::to_file_xml(
            device.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter(manifest_values)),
        )
        .expect("legacy manifest plist");

        let payload = if encrypted {
            hex::decode("78b51ca5374c3ad575174288688cda49").expect("legacy payload vector")
        } else {
            b"keep".to_vec()
        };
        fs::write(device.join(&keep_id), &payload).expect("keep payload");
        let drop_payload = if encrypted { payload } else { b"drop".to_vec() };
        fs::write(device.join(&drop_id), drop_payload).expect("drop payload");
        fs::create_dir(device.join("unrelated-empty")).expect("unrelated directory");
        (root, device, keep_id, drop_id)
    }

    fn write_encrypted_fixture(label: &str) -> (PathBuf, PathBuf, String, String) {
        let root = fixture_root(label);
        let device = root.join("device-id");
        fs::create_dir_all(&device).expect("create encrypted fixture");
        fs::write(device.join("Info.plist"), b"info").expect("info");
        fs::write(device.join("Status.plist"), b"status").expect("status");

        let password = "0000";
        let (keybag, manifest_key, encryption_key) = test_encrypted_material(password);
        plist::to_file_xml(
            device.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("IsEncrypted".to_string(), plist::Value::Boolean(true)),
                (
                    "BackupKeyBag".to_string(),
                    plist::Value::Data(keybag.clone()),
                ),
                ("ManifestKey".to_string(), plist::Value::Data(manifest_key)),
                (
                    "Lockdown".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "ProductVersion".to_string(),
                        plist::Value::String("17.0".into()),
                    )])),
                ),
            ])),
        )
        .expect("encrypted manifest plist");

        let keep_id = "5757575757575757575757575757575757575757".to_string();
        let drop_id = "7878787878787878787878787878787878787878".to_string();
        let plain_manifest = device.join(".manifest-plain");
        let connection = Connection::open(&plain_manifest).expect("plain manifest");
        connection
            .execute(
                "CREATE TABLE Files (fileID TEXT, domain TEXT, relativePath TEXT, file BLOB)",
                [],
            )
            .expect("create Files");
        for (file_id, relative_path, mode) in [
            (
                &keep_id,
                "Library/SMS/sms.db",
                MODE_TYPE_FILE as i64 | 0o640,
            ),
            (
                &drop_id,
                "Library/Notes/note.db",
                MODE_TYPE_FILE as i64 | 0o600,
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO Files (fileID, domain, relativePath, file) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        file_id,
                        "HomeDomain",
                        relative_path,
                        encrypted_metadata_archive(mode, 9, &encryption_key),
                    ],
                )
                .expect("insert encrypted entry");
        }
        drop(connection);

        let crypto = BackupCrypto::from_manifest(&keybag, &encryption_key, password, true)
            .expect("fixture crypto");
        fs::File::create(device.join("Manifest.db")).expect("encrypted destination");
        crypto
            .encrypt_manifest_file(&plain_manifest, &device.join("Manifest.db"))
            .expect("encrypt manifest");
        fs::remove_file(plain_manifest).expect("remove plaintext manifest");

        for file_id in [&keep_id, &drop_id] {
            let payload = device.join(&file_id[..2]).join(file_id);
            fs::create_dir_all(payload.parent().expect("payload parent")).expect("payload dir");
            fs::write(
                payload,
                hex::decode("78b51ca5374c3ad575174288688cda49").expect("payload hex"),
            )
            .expect("payload");
        }
        (root, device, keep_id, drop_id)
    }

    #[test]
    fn patches_manifest_and_payloads_then_unbacks_selected_entry() {
        let (root, device, keep_id, drop_id) = write_fixture("selection");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let result = patch_backup_directory(&root, "device-id", &filter, None).expect("patch");
        assert_eq!(result.entries_seen, 2);
        assert_eq!(result.entries_kept, 1);
        assert_eq!(result.entries_removed, 1);
        assert!(device.join(&keep_id[..2]).join(&keep_id).exists());
        assert!(!device.join(&drop_id[..2]).join(&drop_id).exists());
        assert!(
            device.join("unrelated-empty").is_dir(),
            "successful patch must not remove unrelated empty directories"
        );
        assert!(
            fs::read_dir(&device)
                .expect("device directory")
                .flatten()
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".backup2-stage-")),
            "successful patch must remove its private staging directory"
        );

        let output = root.join("expanded");
        let extracted = unback_backup(&root, "device-id", Some(&output), None).expect("unback");
        assert_eq!(extracted.files_extracted, 1);
        assert_eq!(
            fs::read(output.join("HomeDomain/Library/SMS/sms.db")).expect("expanded bytes"),
            b"keep"
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(output.join("HomeDomain/Library/SMS/sms.db"))
                .expect("expanded metadata")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        let default_output = root.join("device-id.unback");
        fs::create_dir_all(&default_output).expect("default output");
        fs::write(default_output.join("stale"), b"stale").expect("stale output");
        let default_result = unback_backup(&root, "device-id", None, None).expect("default unback");
        assert_eq!(default_result.output_directory, default_output);
        assert!(!default_output.join("stale").exists());
        assert!(default_output
            .join("HomeDomain/Library/SMS/sms.db")
            .exists());
        let one = root.join("one");
        fs::create_dir_all(&one).expect("one output");
        let one_result = extract_backup_file(
            &root,
            "device-id",
            "HomeDomain",
            "Library/SMS/sms.db",
            Some(&one),
            None,
        )
        .expect("extract one");
        assert_eq!(one_result.files_extracted, 1);
        assert_eq!(fs::read(one.join("sms.db")).expect("one bytes"), b"keep");

        let connection = Connection::open(device.join("Manifest.db")).expect("manifest db");
        let rows: Vec<String> = connection
            .prepare("SELECT fileID FROM Files")
            .expect("query")
            .query_map([], |row| row.get(0))
            .expect("rows")
            .collect::<Result<_, _>>()
            .expect("collect rows");
        assert_eq!(rows, vec![keep_id]);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn supports_modern_encrypted_manifest_patch_and_unback_without_plaintext_leaks() {
        let (root, device, keep_id, drop_id) = write_encrypted_fixture("encrypted");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let result = patch_backup_directory(&root, "device-id", &filter, Some("0000"))
            .expect("encrypted patch");
        assert_eq!(result.entries_seen, 2);
        assert_eq!(result.entries_kept, 1);
        assert!(!device.join(&drop_id[..2]).join(&drop_id).exists());

        let output = root.join("expanded-encrypted");
        let extracted = unback_backup(&root, "device-id", Some(&output), Some("0000"))
            .expect("encrypted unback");
        assert_eq!(extracted.files_extracted, 1);
        assert_eq!(
            fs::read(output.join("HomeDomain/Library/SMS/sms.db")).expect("plaintext payload"),
            b"Test data"
        );
        assert!(!output.join(".manifest-plain").exists());

        let wrong_output = root.join("wrong-password-output");
        let error = unback_backup(
            &root,
            "device-id",
            Some(&wrong_output),
            Some("wrong-password"),
        )
        .expect_err("wrong password");
        assert!(error.to_string().contains("password") || error.to_string().contains("wrapped"));
        assert!(!wrong_output.join("HomeDomain/Library/SMS/sms.db").exists());
        assert!(device.join("Manifest.db").is_file());
        assert!(device.join(&keep_id[..2]).join(&keep_id).is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_manifest_path_traversal_and_encrypted_local_operations() {
        let (root, device, _, _) = write_fixture("malformed");
        let connection = Connection::open(device.join("Manifest.db")).expect("manifest db");
        connection
            .execute(
                "INSERT INTO Files (fileID, domain, relativePath, file) VALUES (?1, ?2, ?3, ?4)",
                params![
                    "3333333333333333333333333333333333333333",
                    "HomeDomain",
                    "../outside",
                    metadata_archive(MODE_TYPE_FILE as i64, 0),
                ],
            )
            .expect("insert malformed row");
        drop(connection);
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        assert!(patch_backup_directory(&root, "device-id", &filter, None).is_err());
        assert!(!root.parent().expect("parent").join("outside").exists());

        plist::to_file_xml(
            device.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("IsEncrypted".to_string(), plist::Value::Boolean(true)),
                (
                    "BackupKeyBag".to_string(),
                    plist::Value::Data(vec![0x11; 32]),
                ),
                (
                    "ManifestKey".to_string(),
                    plist::Value::Data(vec![0x22; 44]),
                ),
            ])),
        )
        .expect("encrypted manifest plist");
        let error = unback_backup(&root, "device-id", None, Some("password"))
            .expect_err("encrypted local extraction must be explicit");
        assert!(error.to_string().contains("ProductVersion"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn modern_payload_layout_matches_ios_version_cutoff() {
        assert!(!is_modern_storage("10.2").unwrap());
        assert!(is_modern_storage("10.2.1").unwrap());
        assert!(is_modern_storage("10.3rc1").unwrap());
        assert!(!is_modern_storage("10.2rc1").unwrap());
        assert!(is_modern_storage("17.0").unwrap());
        assert!(!is_modern_storage("10.1.9").unwrap());
        assert!(is_modern_storage("10").is_ok());
        assert!(is_modern_storage("10..3").is_err());
        assert!(is_modern_storage("not-a-version").is_err());
        assert!(is_modern_storage("999999999999999999999999.0").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn temporary_backup_files_are_private_and_cleaned_up() {
        let directory = fixture_root("temporary-permissions");
        fs::create_dir_all(&directory).expect("temporary directory");
        let temporary = create_temporary_file(&directory, "test").expect("temporary file");
        let mode = fs::metadata(&temporary.path)
            .expect("temporary metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let path = temporary.path.clone();
        drop(temporary);
        assert!(
            !path.exists(),
            "temporary file must be removed on all exits"
        );
        fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn failed_manifest_replacement_rolls_payloads_back() {
        let (root, device, keep_id, drop_id) = write_fixture("rollback");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let manifest = open_manifest_workspace(&device, None).expect("workspace");
        let (_, row_ids, allowed_ids) =
            collect_manifest_rows(&manifest.source_path, &filter).expect("rows");
        let temporary = copy_manifest_to_temporary(&manifest.source_path, &device).expect("copy");
        patch_manifest_file(&temporary.path, &row_ids).expect("patch temporary");
        let mut staged = stage_payloads(&device, &allowed_ids).expect("stage");
        fs::create_dir(device.join("Manifest.db-destination")).expect("replacement directory");
        let error =
            replace_file_atomically(&temporary.path, &device.join("Manifest.db-destination"))
                .expect_err("a file cannot replace a directory");
        assert!(error.to_string().contains("directory") || error.to_string().contains("Is a"));
        staged.rollback();
        drop(temporary);
        assert!(device.join(&keep_id[..2]).join(&keep_id).is_file());
        assert!(device.join(&drop_id[..2]).join(&drop_id).is_file());
        let rows = Connection::open(device.join("Manifest.db"))
            .expect("manifest db")
            .query_row("SELECT count(*) FROM Files", [], |row| row.get::<_, i64>(0))
            .expect("row count");
        assert_eq!(rows, 2, "original manifest must remain unchanged");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn staging_failure_restores_payloads_and_removes_private_stage() {
        let (root, device, keep_id, drop_id) = write_fixture("staging-failure");
        fs::create_dir(device.join(&drop_id[..2]).join("nested")).expect("nested invalid path");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let manifest = open_manifest_workspace(&device, None).expect("workspace");
        let (_, _, allowed_ids) =
            collect_manifest_rows(&manifest.source_path, &filter).expect("rows");
        assert!(stage_payloads(&device, &allowed_ids).is_err());
        assert!(device.join(&keep_id[..2]).join(&keep_id).is_file());
        assert!(device.join(&drop_id[..2]).join(&drop_id).is_file());
        assert!(fs::read_dir(&device)
            .expect("device directory")
            .flatten()
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .starts_with(".backup2-stage-")));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn nonempty_manifest_sidecar_is_rejected_but_empty_sidecar_is_safe() {
        let (root, device, _, _) = write_fixture("sidecar");
        fs::write(device.join("Manifest.db-wal"), b"live sqlite state").expect("sidecar");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let error = patch_backup_directory(&root, "device-id", &filter, None)
            .expect_err("non-empty sidecar must be rejected");
        assert!(error.to_string().contains("non-empty Manifest.db sidecar"));
        fs::write(device.join("Manifest.db-wal"), []).expect("empty sidecar");
        patch_backup_directory(&root, "device-id", &filter, None)
            .expect("empty sidecar is harmless");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn supports_legacy_mbdb_flat_payload_patch_and_extract() {
        let (root, device, keep_id, drop_id) = write_legacy_fixture("legacy", false);
        let listed = list_backup_entries(&root, "device-id", None).expect("list legacy manifest");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].file_id, keep_id);
        assert_eq!(listed[0].relative_path, "Media/Test.txt");
        assert_eq!(listed[0].link_target, None);
        let filter = super::super::build_backup_filter(&[], &["Media/Test\\.txt".into()])
            .expect("selection")
            .expect("filter");
        // The fixture is deliberately on the legacy side of the ProductVersion
        // cutoff and stores payloads as flat file-id names.
        let result = patch_backup_directory(&root, "device-id", &filter, None).expect("patch");
        assert_eq!(result.entries_seen, 2);
        assert_eq!(result.entries_kept, 1);
        assert_eq!(result.entries_removed, 1);
        assert!(device.join(&keep_id).is_file());
        assert!(!device.join(&drop_id).exists());
        let encoded = fs::read(device.join("Manifest.mbdb")).expect("manifest bytes");
        let parsed = MbdbManifest::parse(&encoded).expect("patched MBDB");
        assert_eq!(parsed.records.len(), 1);
        assert_eq!(parsed.records[0].properties.len(), 1);

        let output = root.join("expanded");
        let extracted = unback_backup(&root, "device-id", Some(&output), None).expect("unback");
        assert_eq!(extracted.files_extracted, 1);
        assert_eq!(
            fs::read(output.join("MyTestDomain/Media/Test.txt")).expect("payload"),
            b"keep"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_legacy_manifest_path_escape_before_creating_output() {
        let (root, device, _, _) = write_legacy_fixture("legacy-path-escape", false);
        let manifest_path = device.join("Manifest.mbdb");
        let mut manifest = MbdbManifest::parse(&fs::read(&manifest_path).expect("manifest"))
            .expect("parse manifest");
        manifest.records[0].relative_path = Some("../outside".into());
        fs::write(
            &manifest_path,
            manifest
                .serialize(None)
                .expect("serialize malicious manifest"),
        )
        .expect("write malicious manifest");

        let error = unback_backup(&root, "device-id", None, None)
            .expect_err("legacy traversal must be rejected");
        assert!(error.to_string().contains("escapes backup root"));
        assert!(!root.join("outside").exists());
        assert!(!root.join("device-id.unback").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn supports_pyiosbackup_legacy_encrypted_payload_without_manifest_key() {
        let (root, device, keep_id, drop_id) = write_legacy_fixture("legacy-encrypted", true);
        let output = root.join("expanded");
        let extracted = unback_backup(&root, "device-id", Some(&output), Some("0000"))
            .expect("encrypted legacy unback");
        assert_eq!(extracted.files_extracted, 2);
        assert_eq!(
            fs::read(output.join("MyTestDomain/Media/Test.txt")).expect("payload"),
            b"Test data"
        );
        let wrong_output = root.join("wrong-password");
        assert!(unback_backup(&root, "device-id", Some(&wrong_output), Some("wrong")).is_err());
        assert!(!wrong_output.join("MyTestDomain/Media/Test.txt").exists());

        let filter = super::super::build_backup_filter(&[], &["Media/Test\\.txt".into()])
            .expect("selection")
            .expect("filter");
        let result = patch_backup_directory(&root, "device-id", &filter, Some("0000"))
            .expect("encrypted legacy patch");
        assert_eq!(result.entries_kept, 1);
        assert!(device.join(&keep_id).is_file());
        assert!(!device.join(&drop_id).exists());
        assert!(device.join("Manifest.mbdb").is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_oversized_manifest_archive_before_blob_decode_or_output_creation() {
        let (root, device, _, _) = write_fixture("oversized-archive");
        let connection = Connection::open(device.join("Manifest.db")).expect("manifest db");
        let archive = vec![0u8; MAX_ARCHIVE_BYTES + 1];
        connection
            .execute(
                "INSERT INTO Files (fileID, domain, relativePath, file) VALUES (?1, ?2, ?3, ?4)",
                params![
                    "3333333333333333333333333333333333333333",
                    "HomeDomain",
                    "Library/Notes/too-large",
                    archive,
                ],
            )
            .expect("insert oversized row");
        drop(connection);
        let error = unback_backup(&root, "device-id", None, None)
            .expect_err("oversized archive must be rejected");
        assert!(error.to_string().contains("metadata archive"));
        assert!(!root.join("device-id.unback").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_payload_without_following_outside_root() {
        use std::os::unix::fs::symlink;

        let (root, device, keep_id, _) = write_fixture("symlink");
        let outside = root.join("outside");
        fs::write(&outside, b"outside").expect("outside");
        let payload = device.join(&keep_id[..2]).join(&keep_id);
        fs::remove_file(&payload).expect("remove payload");
        symlink(&outside, &payload).expect("symlink payload");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        assert!(patch_backup_directory(&root, "device-id", &filter, None)
            .expect_err("symlink must be rejected")
            .to_string()
            .contains("symlink"));
        assert_eq!(fs::read(&outside).expect("outside bytes"), b"outside");
        let connection = Connection::open(device.join("Manifest.db")).expect("manifest db");
        let row_count: i64 = connection
            .query_row("SELECT count(*) FROM Files", [], |row| row.get(0))
            .expect("manifest row count");
        assert_eq!(
            row_count, 2,
            "failed prune must not commit a partial manifest patch"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_manifest_sidecar_before_sqlite_opens_it() {
        use std::os::unix::fs::symlink;

        let (root, device, _, _) = write_fixture("sidecar-symlink");
        let outside = root.join("outside-wal");
        fs::write(&outside, b"outside").expect("outside wal");
        symlink(&outside, device.join("Manifest.db-wal")).expect("wal symlink");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let error = patch_backup_directory(&root, "device-id", &filter, None)
            .expect_err("symlink manifest sidecar must be rejected");
        assert!(error.to_string().contains("symlink"));
        assert_eq!(fs::read(&outside).expect("outside bytes"), b"outside");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_oversized_manifest_sidecar_before_sqlite_reads_it() {
        let (root, device, _, _) = write_fixture("oversized-sidecar");
        let sidecar = device.join("Manifest.db-wal");
        let sidecar_file = fs::File::create(&sidecar).expect("wal sidecar");
        sidecar_file
            .set_len(MAX_MANIFEST_DB_BYTES + 1)
            .expect("sparse wal sidecar");
        let filter = super::super::build_backup_filter(&["sms".into()], &[])
            .expect("selection")
            .expect("filter");
        let error = patch_backup_directory(&root, "device-id", &filter, None)
            .expect_err("oversized manifest sidecar must be rejected");
        assert!(error.to_string().contains("sidecar is too large"));
        fs::remove_dir_all(root).expect("cleanup");
    }
}

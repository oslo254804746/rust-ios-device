//! Backup root/path policy and host filesystem operations.
//!
//! Every operation consumes an explicit layout/root/path and retains the same
//! symlink, staging and metadata limits. Blocking work has a shared typed runner.

use super::protocol::plist_value_to_bool;
use super::{BackupDirectoryLayout, Mobilebackup2Error};
use std::{
    fs::{self, File, OpenOptions},
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};
use time::{OffsetDateTime, UtcOffset};

// Metadata plists are small control-plane documents. Bound them before invoking the plist
// decoder so a sparse or hostile backup cannot make parsing allocate without limit. Payload and
// Manifest.db data have separate, substantially larger limits in backup2::manifest.
const MAX_BACKUP_METADATA_BYTES: u64 = 16 * 1024 * 1024;

const INCREMENTAL_BACKUP_REQUIRED_FILES: &[&str] = &["Manifest.plist", "Status.plist"];
// 978_307_200 seconds = 2001-01-01T00:00:00Z Unix timestamp
// This is the Apple Core Data / NSDate epoch offset (seconds between Unix epoch and Apple epoch)
const APPLE_EPOCH_OFFSET: Duration = Duration::from_secs(978_307_200);

pub(super) fn cleanup_discarded_files(
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

    let (mut info_file, info_pending) =
        crate::staged_file::create(&device_directory.join("Info.plist"))?;
    let result = plist::to_writer_xml(
        &mut info_file,
        &plist::Value::Dictionary(info_plist.clone()),
    )
    .map_err(Mobilebackup2Error::from)
    .and_then(|()| info_file.sync_all().map_err(Mobilebackup2Error::from));
    drop(info_file);
    result?;
    info_pending.commit()?;

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
        let (mut status_file, status_pending) = crate::staged_file::create(&status_path)?;
        let result = plist::to_writer_binary(&mut status_file, &plist::Value::Dictionary(status))
            .map_err(Mobilebackup2Error::from)
            .and_then(|()| status_file.sync_all().map_err(Mobilebackup2Error::from));
        drop(status_file);
        result?;
        status_pending.commit()?;
    }

    let manifest_path = device_directory.join("Manifest.plist");
    let create_manifest = match fs::symlink_metadata(&manifest_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(symlink_path_error(&manifest_path));
        }
        Ok(_) if full => true,
        Ok(_) => false,
        Err(error) if error.kind() == ErrorKind::NotFound => true,
        Err(error) => return Err(error.into()),
    };
    // Reset the manifest only when it is new or a full backup requests it. In incremental mode
    // its existing plist is part of the backup state and must not be truncated.
    if create_manifest {
        let (file, pending) = crate::staged_file::create(&manifest_path)?;
        let result = file.sync_all();
        drop(file);
        result?;
        pending.commit()?;
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
pub(super) async fn run_blocking<T, F>(op: F) -> Result<T, Mobilebackup2Error>
where
    F: FnOnce() -> Result<T, Mobilebackup2Error> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| Mobilebackup2Error::Io(std::io::Error::other(err)))?
}

/// Own the layout across a host filesystem step off the async worker thread.
pub(super) async fn with_layout<T, F>(
    layout: &BackupDirectoryLayout,
    operation: F,
) -> Result<T, Mobilebackup2Error>
where
    F: FnOnce(BackupDirectoryLayout) -> Result<T, Mobilebackup2Error> + Send + 'static,
    T: Send + 'static,
{
    let layout = layout.clone();
    run_blocking(move || operation(layout)).await
}

pub(super) fn create_runtime_layout(
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

pub(super) fn ensure_backup_directory(
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

pub(super) fn read_backup_dictionary(path: &Path) -> Result<plist::Dictionary, Mobilebackup2Error> {
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

pub(super) fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

pub(super) fn sanitize_relative_path(path: &str) -> Result<PathBuf, Mobilebackup2Error> {
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
pub(super) fn manifest_relative_path_string(path: &Path) -> String {
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

pub(super) fn resolve_relative_path(
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

pub(super) fn canonical_backup_root(root: &Path) -> Result<PathBuf, Mobilebackup2Error> {
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
pub(super) fn canonicalize_simplified(path: &Path) -> std::io::Result<PathBuf> {
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

pub(super) fn symlink_path_error(path: &Path) -> Mobilebackup2Error {
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

pub(super) fn ensure_no_symlink_components_at_root(
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

pub(super) fn create_dir_all_no_symlink(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, Mobilebackup2Error> {
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

pub(super) fn reject_symlink_components(path: &Path) -> Result<(), Mobilebackup2Error> {
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
pub(super) fn open_file_for_read(path: &Path) -> Result<File, Mobilebackup2Error> {
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

#[cfg(feature = "backup2-manifest")]
pub(super) fn open_file_for_write(path: &Path) -> Result<File, Mobilebackup2Error> {
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

pub(super) fn metadata_file_path(
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

pub(super) fn layout_relative_path(
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

pub(super) fn create_layout_directory(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    create_dir_all_no_symlink(&layout.root, &relative)?;
    Ok(())
}

pub(super) fn create_layout_parent_directory(
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

pub(super) fn create_layout_staged_file(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(File, crate::staged_file::PendingFile), Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    Ok(crate::staged_file::create(path)?)
}

pub(super) fn open_layout_file_for_read(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<File, Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    open_file_for_read(path)
}

fn rename_layout_path(
    layout: &BackupDirectoryLayout,
    src: &Path,
    dst: &Path,
) -> Result<(), Mobilebackup2Error> {
    let src_relative = layout_relative_path(layout, src)?;
    let dst_relative = layout_relative_path(layout, dst)?;
    ensure_no_symlink_components_at_root(&layout.root, &src_relative)?;
    ensure_no_symlink_components_at_root(&layout.root, &dst_relative)?;
    fs::rename(src, dst)?;
    Ok(())
}

pub(super) fn ensure_tree_has_no_symlinks(path: &Path) -> Result<(), Mobilebackup2Error> {
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

fn remove_layout_path(
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
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) struct LayoutPathChange {
    pub(super) source: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) is_directory: bool,
}

fn resolve_layout_change(
    layout: &BackupDirectoryLayout,
    source: &str,
    destination: &str,
    allow_missing: bool,
) -> Result<Option<(PathBuf, PathBuf)>, Mobilebackup2Error> {
    let source = resolve_relative_path(layout, source)?;
    let destination = resolve_relative_path(layout, destination)?;
    // Filtered uploads can be absent when the device subsequently moves/copies
    // them. Preserve the existing missing-source policy, after both path checks.
    if allow_missing
        && matches!(fs::symlink_metadata(&source), Err(error) if error.kind() == ErrorKind::NotFound)
    {
        return Ok(None);
    }
    Ok(Some((source, destination)))
}

pub(super) fn move_layout_item(
    layout: &BackupDirectoryLayout,
    source: &str,
    destination: &str,
    allow_missing: bool,
) -> Result<Option<LayoutPathChange>, Mobilebackup2Error> {
    let Some((source, destination)) =
        resolve_layout_change(layout, source, destination, allow_missing)?
    else {
        return Ok(None);
    };
    create_layout_parent_directory(layout, &destination)?;
    let is_directory = fs::symlink_metadata(&source)?.is_dir();
    rename_layout_path(layout, &source, &destination)?;
    Ok(Some(LayoutPathChange {
        source,
        destination,
        is_directory,
    }))
}

pub(super) fn copy_layout_item(
    layout: &BackupDirectoryLayout,
    source: &str,
    destination: &str,
    allow_missing: bool,
) -> Result<Option<LayoutPathChange>, Mobilebackup2Error> {
    let Some((source, destination)) =
        resolve_layout_change(layout, source, destination, allow_missing)?
    else {
        return Ok(None);
    };
    let is_directory = fs::symlink_metadata(&source)?.is_dir();
    copy_item(&layout.root, &source, &destination)?;
    Ok(Some(LayoutPathChange {
        source,
        destination,
        is_directory,
    }))
}

pub(super) fn remove_layout_item(
    layout: &BackupDirectoryLayout,
    relative: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let path = resolve_relative_path(layout, relative)?;
    remove_layout_path(layout, &path)?;
    Ok(path)
}

pub(super) fn list_layout_directory(
    layout: &BackupDirectoryLayout,
    relative: &str,
) -> Result<plist::Dictionary, Mobilebackup2Error> {
    let path = resolve_relative_path(layout, relative)?;
    contents_of_directory(&layout.root, &path)
}

pub(super) fn commit_uploaded_file(
    layout: &BackupDirectoryLayout,
    output: &Path,
    pending: crate::staged_file::PendingFile,
) -> Result<(), Mobilebackup2Error> {
    ensure_no_symlink_components_at_root(&layout.root, &layout_relative_path(layout, output)?)?;
    pending.commit()?;
    Ok(())
}

pub(super) fn copy_item(root: &Path, src: &Path, dst: &Path) -> Result<(), Mobilebackup2Error> {
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
        let (mut destination, pending) = crate::staged_file::create(dst)?;
        let result =
            std::io::copy(&mut source, &mut destination).and_then(|_| destination.sync_all());
        drop(destination);
        result?;
        pending.commit()?;
    } else {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup copy source is neither a file nor a directory: {}",
            src.display()
        )));
    }

    Ok(())
}

pub(super) fn contents_of_directory(
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
            tracing::debug!(path = %crate::redaction::diagnostic_id(&entry_path.to_string_lossy()), error = %crate::redaction::diagnostic_error(&err), "cannot read mtime");
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
    // 2001 epoch, then serializes that wall-clock timestamp as if it were UTC.
    let modified = device_link_local_wall_clock(modified);
    let shifted = modified
        .checked_sub(APPLE_EPOCH_OFFSET)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    plist::Date::from(shifted)
}

fn device_link_local_wall_clock(modified: SystemTime) -> SystemTime {
    let utc = OffsetDateTime::from(modified);
    let local_offset = UtcOffset::local_offset_at(utc).unwrap_or(UtcOffset::UTC);
    let local_wall_clock = utc.to_offset(local_offset).replace_offset(UtcOffset::UTC);
    local_wall_clock.into()
}

#[cfg(windows)]
pub(super) fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
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
pub(super) fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn host_work_does_not_block_timer_progress_on_single_thread_runtime() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("synthetic-root"),
            device_directory: PathBuf::from("synthetic-root/source"),
            target_identifier: "source".into(),
        };
        let runtime_thread = std::thread::current().id();
        let (release, wait) = std::sync::mpsc::channel();
        let (result, ()) = tokio::join!(
            with_layout(&layout, move |owned_layout| {
                let released = wait.recv_timeout(Duration::from_secs(2)).is_ok();
                Ok((owned_layout, released, std::thread::current().id()))
            }),
            async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                let _ = release.send(());
            }
        );
        let (owned_layout, released, worker_thread) = result.unwrap();
        assert!(
            released,
            "a blocked host operation must leave the timer runnable"
        );
        assert_ne!(worker_thread, runtime_thread);
        assert_eq!(owned_layout, layout);
    }

    #[tokio::test]
    async fn host_worker_failure_preserves_original_io_and_join_error_sources() {
        let error =
            run_blocking::<(), _>(|| Err(std::io::Error::from(ErrorKind::PermissionDenied).into()))
                .await
                .unwrap_err();
        let Mobilebackup2Error::Io(error) = error else {
            panic!("expected typed IO failure");
        };
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);

        let error = run_blocking::<(), _>(|| panic!("synthetic host worker failure"))
            .await
            .unwrap_err();
        let Mobilebackup2Error::Io(error) = error else {
            panic!("expected join failure wrapped in IO");
        };
        assert!(error
            .get_ref()
            .and_then(|source| source.downcast_ref::<tokio::task::JoinError>())
            .is_some_and(tokio::task::JoinError::is_panic));
    }

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

        let err = create_layout_staged_file(&layout, &device.join("final"))
            .err()
            .expect("symlink output must be rejected");
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
    fn device_link_modification_date_preserves_subsecond_apple_epoch_timestamp() {
        let modified = SystemTime::UNIX_EPOCH
            + APPLE_EPOCH_OFFSET
            + Duration::from_secs(123)
            + Duration::from_millis(900);
        let encoded = device_link_modification_date(modified);
        let shifted: SystemTime = encoded.into();
        let expected = device_link_local_wall_clock(modified)
            .checked_sub(APPLE_EPOCH_OFFSET)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        assert_eq!(shifted, expected);
    }
}

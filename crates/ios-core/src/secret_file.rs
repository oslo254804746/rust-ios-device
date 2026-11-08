//! Restricted-permission persistence for files holding private-key material.
//!
//! Pair records and host identities contain long-lived private keys: anyone who
//! can read them can impersonate this host to the device. Apple's own
//! `/var/db/lockdown` records are `0600` for that reason, so every write path
//! that persists key material goes through here rather than `std::fs::write`.
//!
//! On Windows the mode bits do not apply; the files inherit the parent ACL, as
//! they did before.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroizing;

/// Owner read/write only.
#[cfg(unix)]
const SECRET_FILE_MODE: u32 = 0o600;
/// Owner read/write/traverse only.
#[cfg(unix)]
const SECRET_DIR_MODE: u32 = 0o700;

/// Create `dir` and its parents, restricting `dir` itself to the owner on Unix.
///
/// Only the leaf is tightened: the parents may legitimately be shared locations
/// such as `~/.config` or `%APPDATA%`.
pub fn create_secret_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(dir)?.permissions();
        if perms.mode() & 0o077 != 0 {
            perms.set_mode(SECRET_DIR_MODE);
            fs::set_permissions(dir, perms)?;
        }
    }

    Ok(())
}

/// Write `contents` to `path` so that only the owner can read it on Unix.
///
/// The bytes land in a sibling temporary file first and are renamed into place,
/// so a concurrent reader never observes a half-written record and a failed
/// write leaves the previous record intact.
pub fn write_secret(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_secret_dir(parent)?;
    }

    let tmp = temp_path(path);
    // A stale temp file from a crashed run would otherwise be reused with
    // whatever permissions it already has.
    let _ = fs::remove_file(&tmp);

    let write = || -> io::Result<()> {
        let mut file = create_secret_file(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()
    };

    if let Err(err) = write() {
        let _ = fs::remove_file(&tmp);
        return Err(err);
    }

    replace_secret_file(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Read a regular, private secret file into a buffer wiped on drop.
///
/// The limit must be between one byte and 1 MiB. Unix checks the opened file's
/// owner and mode, rejects leaf symlinks and opens nonblocking before checking
/// its type so a FIFO cannot hang the reader. Windows rejects reparse points;
/// its owner/read permissions still depend on the configured parent ACL.
/// Callers must trust the parent path; this is not a directory-race guarantee.
pub fn read_secret(path: &Path, limit: u64) -> io::Result<Zeroizing<Vec<u8>>> {
    if !(1..=1024 * 1024).contains(&limit) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secret read limit must be between one byte and 1 MiB",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the leaf itself instead of following a reparse point.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secret path must be a regular file",
        ));
    }
    #[cfg(unix)]
    ensure_private_owner(&metadata)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secret path must not be a reparse point",
            ));
        }
    }
    if metadata.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secret file exceeds size limit",
        ));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secret file exceeds size limit",
        ));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn ensure_private_owner(metadata: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no arguments or memory preconditions.
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "secret must belong to the effective user with no group/other access",
        ));
    }
    Ok(())
}

/// Publish a new owner-only secret without replacing an existing destination.
///
/// Creates a missing private parent, but rejects an existing nonprivate parent
/// on Unix instead of changing its permissions. The caller must trust all
/// parent components. Publication requires hard links; unsupported filesystems
/// fail explicitly. A reported cleanup failure can occur after publication.
pub fn write_secret_new(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match fs::symlink_metadata(parent) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "secret parent must be a directory, not a symlink",
                ));
            }
            #[cfg(unix)]
            ensure_private_owner(&metadata)?;
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_secret_dir(parent)?,
        Err(error) => return Err(error),
    }
    let temporary = parent.join(format!(".ios-secret-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = create_secret_file(&temporary)?;
    let result = (|| -> io::Result<()> {
        file.write_all(contents)?;
        file.sync_all()?;
        fs::hard_link(&temporary, path)
    })();
    // Close before cleanup, including on platforms that restrict open-file
    // deletion. Only this invocation's create_new file is removed.
    drop(file);
    let cleanup = fs::remove_file(temporary);
    result.and(cleanup)
}

#[cfg(not(windows))]
fn replace_secret_file(tmp: &Path, path: &Path) -> io::Result<()> {
    fs::rename(tmp, path)
}

#[cfg(windows)]
fn replace_secret_file(tmp: &Path, path: &Path) -> io::Result<()> {
    // `std::fs::rename` cannot replace an existing file on Windows. Move the
    // old record aside first, then install the fully-written temporary file;
    // if installation fails, restore the old record so a transient sharing or
    // ACL error does not destroy the previous credentials.
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if metadata
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_dir())
    {
        return Err(io::Error::new(
            // `IsADirectory` is newer than this crate's Rust 1.80 MSRV.
            io::ErrorKind::InvalidInput,
            format!("secret file path is a directory: {}", path.display()),
        ));
    }

    let Some(()) = metadata.map(|_| ()) else {
        return fs::rename(tmp, path);
    };

    let backup = temp_path(path);
    fs::rename(path, &backup)?;
    match fs::rename(tmp, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, path);
            Err(error)
        }
    }
}

fn create_secret_file(path: &Path) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(SECRET_FILE_MODE);
    }

    opts.open(path)
}

fn temp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{seq}.tmp", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ios_rs_secret_file_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn writes_contents_and_creates_missing_directories() {
        let dir = temp_dir("write");
        let path = dir.join("nested").join("record.plist");

        write_secret(&path, b"secret").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"secret");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn overwrites_an_existing_record_and_leaves_no_temp_files() {
        let dir = temp_dir("overwrite");
        let path = dir.join("record.json");

        write_secret(&path, b"first").unwrap();
        write_secret(&path, b"second").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"second");
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "tmp").unwrap_or(false))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_secret_preserves_existing_target_and_cleans_owned_staging() {
        let dir = std::env::temp_dir().join(format!("ios-new-secret-{}", uuid::Uuid::new_v4()));
        let path = dir.join("token");
        write_secret_new(&path, b"synthetic-original-token").unwrap();
        let error = write_secret_new(&path, b"synthetic-replacement-token").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"synthetic-original-token");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        let directory_target = dir.join("directory-target");
        fs::create_dir(&directory_target).unwrap();
        assert!(write_secret_new(&directory_target, b"synthetic-token").is_err());
        assert!(directory_target.is_dir());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bounded_secret_reader_rejects_invalid_limits_size_and_directory() {
        let dir = std::env::temp_dir().join(format!("ios-read-secret-{}", uuid::Uuid::new_v4()));
        let path = dir.join("token");
        write_secret_new(&path, b"synthetic-token").unwrap();
        assert_eq!(
            read_secret(&path, 15).unwrap().as_slice(),
            b"synthetic-token"
        );
        assert_eq!(
            read_secret(&path, 4).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        for limit in [0, 1024 * 1024 + 1] {
            assert_eq!(
                read_secret(&path, limit).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        assert!(read_secret(&dir, 16).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn private_secret_reader_rejects_public_files_symlinks_and_fifo_without_blocking() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("ios-private-secret-{}", uuid::Uuid::new_v4()));
        let path = dir.join("token");
        write_secret_new(&path, b"synthetic-token").unwrap();
        let link = dir.join("link");
        symlink(&path, &link).unwrap();
        assert!(read_secret(&link, 64).is_err());
        assert_eq!(
            write_secret_new(&link, b"synthetic-replacement")
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&path).unwrap(), b"synthetic-token");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            read_secret(&path, 64).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        let fifo = dir.join("fifo");
        use std::os::unix::ffi::OsStrExt;
        let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: a NUL-terminated path within this owned fixture is supplied.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        assert_eq!(
            read_secret(&fifo, 64).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn new_secret_does_not_tighten_existing_shared_directory_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ios-shared-secret-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            write_secret_new(&dir.join("token"), b"synthetic-token")
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn restricts_file_and_directory_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp_dir("perms");
        let path = dir.join("key.pem");

        write_secret(&path, b"-----BEGIN PRIVATE KEY-----").unwrap();

        let file_mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, SECRET_FILE_MODE);
        let dir_mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, SECRET_DIR_MODE);

        let _ = fs::remove_dir_all(&dir);
    }
}

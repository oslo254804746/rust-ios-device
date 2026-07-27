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
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
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

//! Same-directory, private staging for internal host file writers.
//!
//! Callers retain their path/root checks and own the file handle. Close that
//! handle after flushing and syncing, then consume the pending replacement.
//! This provides per-file publication, not a multi-file transaction or a
//! defense against replacement of untrusted intermediate directories.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct PendingFile {
    temporary: PathBuf,
    destination: PathBuf,
    armed: bool,
}

fn validate_destination(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => Err(
            io::Error::new(io::ErrorKind::InvalidInput, "output must be a regular file"),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub(crate) fn create(path: &Path) -> io::Result<(File, PendingFile)> {
    path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "output path must name a file")
    })?;
    validate_destination(path)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    for _ in 0..16 {
        // Independent of the destination component's length and encoding.
        let temporary = parent.join(format!(".ios-output-{}.tmp", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        match options.open(&temporary) {
            Ok(file) => {
                // File first: on cancellation/drop the handle closes before
                // its cleanup guard, including an unclaimed worker result.
                return Ok((
                    file,
                    PendingFile {
                        temporary,
                        destination: path.to_owned(),
                        armed: true,
                    },
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary output path",
    ))
}

impl PendingFile {
    pub(crate) fn commit(mut self) -> io::Result<()> {
        validate_destination(&self.destination)?;
        crate::fs_replace::move_file_replace(&self.temporary, &self.destination)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for PendingFile {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = fs::remove_file(&self.temporary) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(kind = ?error.kind(), "temporary output cleanup failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn staged_file_write_failure_preserves_destination_in_isolated_process() {
        const CHILD_ENV: &str = "IOS_STAGING_WRITE_FAILURE_CHILD";
        const TEST_NAME: &str =
            "staged_file::tests::staged_file_write_failure_preserves_destination_in_isolated_process";
        let isolated_child = std::env::var_os(CHILD_ENV).is_some()
            && std::env::args().any(|argument| argument == "--exact");
        if !isolated_child {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
                .env(CHILD_ENV, "1")
                .output()
                .expect("spawn isolated write-failure fixture");
            assert!(
                result.status.success(),
                "fixture failed: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            return;
        }
        // Process-wide limits/signals affect only this dedicated child, never
        // the parallel parent harness or other repository operations.
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: a valid writable rlimit pointer is supplied.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) },
            0
        );
        limit.rlim_cur = limit.rlim_cur.min(1024);
        // SAFETY: the limit pointer is valid and the hard limit is unchanged.
        assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_FSIZE, &limit) }, 0);
        // SAFETY: SIG_IGN is a platform-provided handler value. Ignoring the
        // signal makes the real write return EFBIG instead of killing the child.
        assert_ne!(
            unsafe { libc::signal(libc::SIGXFSZ, libc::SIG_IGN) },
            libc::SIG_ERR
        );
        let directory = std::env::temp_dir().join(format!("ios-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("out.bin");
        fs::write(&destination, b"old complete output").unwrap();
        let (mut file, pending) = create(&destination).unwrap();
        let result = file.write_all(&[0x41; 4096]);
        drop(file);
        drop(pending);
        assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EFBIG));
        assert_eq!(fs::read(&destination).unwrap(), b"old complete output");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn staged_file_drop_preserves_old_and_commit_consumes_the_temporary() {
        let directory = std::env::temp_dir().join(format!("ios-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("报告😀.bin");
        fs::write(&destination, b"old").unwrap();
        let (mut file, pending) = create(&destination).unwrap();
        file.write_all(b"partial").unwrap();
        drop(file);
        drop(pending);
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);

        let (mut file, pending) = create(&destination).unwrap();
        file.write_all(b"complete").unwrap();
        file.sync_all().unwrap();
        drop(file);
        pending.commit().unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"complete");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn staged_file_rejects_directory_and_cleans_after_late_commit_failure() {
        let directory = std::env::temp_dir().join(format!("ios-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        assert!(create(&directory).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
        let destination = directory.join("out.bin");
        let (file, pending) = create(&destination).unwrap();
        drop(file);
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("marker"), b"keep").unwrap();
        assert!(pending.commit().is_err());
        assert_eq!(fs::read(destination.join("marker")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn staged_file_readonly_commit_failure_keeps_old_and_cleans() {
        let directory = std::env::temp_dir().join(format!("ios-stage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&directory).unwrap();
        let destination = directory.join("out.bin");
        fs::write(&destination, b"old").unwrap();
        let (mut file, pending) = create(&destination).unwrap();
        file.write_all(b"new").unwrap();
        file.sync_all().unwrap();
        drop(file);
        let mut permissions = fs::metadata(&destination).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&destination, permissions).unwrap();
        assert!(pending.commit().is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        let mut permissions = fs::metadata(&destination).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(destination, permissions).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
}

//! Real filesystem admission without changing pairing-store ownership rules.
use super::*;
use std::{fs, io, path::Path};

const PRIVATE: &str = "synthetic-private-pair-record-material";

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ios-pair-record-admission-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn plist_bytes() -> Vec<u8> {
    let dictionary = plist::Dictionary::from_iter([
        ("DeviceCertificate", plist::Value::Data(vec![1, 2, 3])),
        ("HostCertificate", plist::Value::Data(vec![4, 5, 6])),
        (
            "HostPrivateKey",
            plist::Value::Data(PRIVATE.as_bytes().to_vec()),
        ),
        ("RootCertificate", plist::Value::Data(vec![10, 11, 12])),
        ("HostID", plist::Value::String(PRIVATE.into())),
        ("SystemBUID", plist::Value::String(PRIVATE.into())),
    ]);
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &plist::Value::Dictionary(dictionary)).unwrap();
    bytes
}

fn primary(error: &PairRecordError, path: &Path, kind: io::ErrorKind) {
    let PairRecordError::Read {
        path: actual,
        source,
    } = error
    else {
        panic!("pair-record admission did not retain a typed read error");
    };
    assert_eq!(actual, path);
    assert_eq!(source.kind(), kind);
    let retained = std::error::Error::source(error)
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert!(std::ptr::eq(source, retained));
    for output in [
        format!("{error} {error:?} {error:#?}"),
        crate::redaction::diagnostic_error(error).to_string(),
    ] {
        assert!(!output.contains(PRIVATE));
        assert!(!output.contains(&path.to_string_lossy().to_string()));
    }
}

fn loaded_record(path: &Path, bytes: &[u8]) {
    let raw: Zeroizing<Vec<u8>> = read_pair_record_bytes(path).unwrap();
    assert_eq!(raw.as_slice(), bytes);
    let record = PairRecord::load_from_path(path, PRIVATE).unwrap();
    let secret: &Zeroizing<Vec<u8>> = &record.host_private_key;
    assert_eq!(secret.as_slice(), PRIVATE.as_bytes());
    assert_eq!(record.device_certificate, [1, 2, 3]);
    assert_eq!(record.host_certificate, [4, 5, 6]);
    assert_eq!(record.root_certificate, [10, 11, 12]);
    assert_eq!(record.host_id, PRIVATE);
    assert_eq!(record.system_buid, PRIVATE);
    assert!(!format!("{record:?} {record:#?}").contains(PRIVATE));
}

#[test]
fn regular_pair_record_preserves_data_and_secret_wrappers() {
    let directory = Directory::new();
    let path = directory.path(&format!("{PRIVATE}.plist"));
    let bytes = plist_bytes();
    fs::write(&path, &bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // System-managed pairing stores can be readable outside the current
        // owner. The reader must not inherit the private token-file policy.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    loaded_record(&path, &bytes);
}

#[cfg(unix)]
#[test]
fn symlink_to_regular_pair_record_keeps_existing_acceptance() {
    let directory = Directory::new();
    let path = directory.path("record.plist");
    let link = directory.path(&format!("{PRIVATE}-link.plist"));
    let bytes = plist_bytes();
    fs::write(&path, &bytes).unwrap();
    std::os::unix::fs::symlink(&path, &link).unwrap();
    loaded_record(&link, &bytes);
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn pair_record_directory_is_rejected_before_read_with_safe_primary() {
    let directory = Directory::new();
    let path = directory.path(PRIVATE);
    fs::create_dir(&path).unwrap();
    assert_eq!(
        read_pair_record_bytes(&path).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    let error = PairRecord::load_from_path(&path, PRIVATE).unwrap_err();
    primary(&error, &path, io::ErrorKind::InvalidInput);
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::InvalidInput
    );
}

#[cfg(unix)]
#[test]
fn opened_pair_record_directory_is_rejected_before_read() {
    let directory = Directory::new();
    assert_eq!(
        open_pair_record_file(&directory.0).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn pair_record_exact_byte_limit_is_accepted_and_one_extra_byte_is_rejected() {
    let directory = Directory::new();
    let path = directory.path(&format!("{PRIVATE}.plist"));
    let bytes = vec![0x41; MAX_PAIR_RECORD_BYTES as usize];
    fs::write(&path, &bytes).unwrap();
    let raw: Zeroizing<Vec<u8>> = read_pair_record_bytes(&path).unwrap();
    assert_eq!(raw.as_slice(), bytes.as_slice());
    drop(raw);
    let mut oversized = bytes;
    oversized.push(0x42);
    fs::write(&path, oversized).unwrap();
    assert_eq!(
        read_pair_record_bytes(&path).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    let error = PairRecord::load_from_path(&path, PRIVATE).unwrap_err();
    primary(&error, &path, io::ErrorKind::InvalidData);
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
}

#[test]
fn pair_record_missing_file_keeps_native_os_error_and_safe_primary() {
    let directory = Directory::new();
    let path = directory.path(&format!("{PRIVATE}.plist"));
    let native = fs::metadata(&path).unwrap_err();
    let error = PairRecord::load_from_path(&path, PRIVATE).unwrap_err();
    primary(&error, &path, io::ErrorKind::NotFound);
    let PairRecordError::Read { source, .. } = &error else {
        unreachable!();
    };
    assert_eq!(source.raw_os_error(), native.raw_os_error());
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
}

#[cfg(unix)]
#[test]
fn unix_pair_record_fifo_admission_finishes_without_writer() {
    use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const CHILD_ENV: &str = "IOS_PAIR_RECORD_FIFO_ADMISSION_CHILD";
    const TEST_NAME: &str = "lockdown::pair_record::read_admission_tests::unix_pair_record_fifo_admission_finishes_without_writer";
    const FINISHED: &[u8] = b"pair-record FIFO admission checked";

    if let Some(path) = std::env::var_os(CHILD_ENV) {
        assert!(std::env::args().any(|argument| argument == "--exact"));
        let path = PathBuf::from(path);
        assert!(fs::metadata(&path).unwrap().file_type().is_fifo());
        assert_eq!(
            read_pair_record_bytes(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // Exercise the opened-handle check independently of the initial path
        // check, as if a regular path had been replaced before open. There is
        // no writer: without O_NONBLOCK this real open would wait indefinitely.
        assert_eq!(
            open_pair_record_file(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let error = PairRecord::load_from_path(&path, PRIVATE).unwrap_err();
        primary(&error, &path, io::ErrorKind::InvalidInput);
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::InvalidInput
        );
        assert!(fs::metadata(&path).unwrap().file_type().is_fifo());
        fs::write(path.with_extension("finished"), FINISHED).unwrap();
        return;
    }

    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            // Only this fixture's explicitly spawned process is stopped. A
            // broken blocking-open regression cannot strand a harness thread.
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let directory = Directory::new();
    let path = directory.path(&format!("{PRIVATE}.plist"));
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: a valid NUL-terminated path belongs to this fixture directory.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let mut child = OwnedChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD_ENV, &path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            drop(child);
            panic!("pair-record FIFO admission child exceeded its observation budget");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "pair-record FIFO admission child failed");
    // An incorrect exact filter would otherwise report success with zero tests.
    assert_eq!(fs::read(path.with_extension("finished")).unwrap(), FINISHED);
}

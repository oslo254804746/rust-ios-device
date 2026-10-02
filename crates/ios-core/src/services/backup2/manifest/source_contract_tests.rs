use super::*;
use crate::{classify_error, proto::nskeyedarchiver::ArchiveError, ErrorClass};
use std::{error::Error, io};

struct Fixture {
    root: PathBuf,
    device: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ios-backup-sources-{}", Uuid::new_v4()));
        let device = root.join("owned-device");
        fs::create_dir_all(&device).unwrap();
        let fixture = Self { root, device };
        fs::write(fixture.device.join("Info.plist"), b"owned-info").unwrap();
        fs::write(fixture.device.join("Status.plist"), b"owned-status").unwrap();
        fixture.version("17.0");
        fixture
    }
    fn version(&self, version: &str) {
        let lockdown = plist::Dictionary::from_iter([("ProductVersion", version)]);
        plist::to_file_xml(
            self.device.join("Manifest.plist"),
            &plist::Dictionary::from_iter([
                ("IsEncrypted", plist::Value::Boolean(false)),
                ("Lockdown", plist::Value::Dictionary(lockdown)),
            ]),
        )
        .unwrap();
    }
    fn database(&self, archive: &[u8], mode: u32) {
        let connection = Connection::open(self.device.join("Manifest.db")).unwrap();
        connection
            .execute(
                "CREATE TABLE Files(fileID TEXT, domain TEXT, relativePath TEXT, file BLOB)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO Files VALUES(?1, 'HomeDomain', 'owned-file', ?2)",
                params!["1111111111111111111111111111111111111111", archive],
            )
            .unwrap();
        let payload = self.device.join("11");
        fs::create_dir(&payload).unwrap();
        fs::write(
            payload.join("1111111111111111111111111111111111111111"),
            if mode == MODE_TYPE_SYMLINK {
                b"\xff".as_slice()
            } else {
                b"owned-payload".as_slice()
            },
        )
        .unwrap();
    }
    fn list(&self) -> Result<Vec<BackupManifestEntry>, Mobilebackup2Error> {
        crate::backup2::list_backup_entries(&self.root, "owned-device", None)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn io_source(error: &Mobilebackup2Error, class: ErrorClass) -> &io::Error {
    let Mobilebackup2Error::Io(source) = error else {
        panic!("backup native cause lost")
    };
    assert_eq!(classify_error(error), class);
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    source
}

fn cyclic_archive() -> Vec<u8> {
    let value = plist::Dictionary::from_iter([
        ("$archiver", plist::Value::String("NSKeyedArchiver".into())),
        (
            "$objects",
            plist::Value::Array(vec![
                plist::Value::String("$null".into()),
                plist::Value::Uid(plist::Uid::new(1)),
            ]),
        ),
        (
            "$top",
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "root",
                plist::Value::Uid(plist::Uid::new(1)),
            )])),
        ),
    ]);
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, &value).unwrap();
    bytes
}

#[test]
fn actual_public_metadata_errors_keep_archive_chain_and_depth_category_without_output() {
    for (bytes, class) in [
        (b"<plist><dict>owned-private".to_vec(), ErrorClass::Protocol),
        (cyclic_archive(), ErrorClass::ResourceLimit),
    ] {
        let fixture = Fixture::new();
        fixture.database(&bytes, MODE_TYPE_FILE);
        let error = fixture.list().unwrap_err();
        let archive = io_source(&error, class)
            .source()
            .unwrap()
            .downcast_ref::<ArchiveError>()
            .unwrap();
        match archive {
            ArchiveError::Plist(_) => {
                assert!(archive.source().unwrap().is::<plist::Error>());
            }
            ArchiveError::TooDeep(depth) => assert_eq!(*depth, 64),
            ArchiveError::Invalid(_) => panic!("fixture did not reach intended archive failure"),
        }
        let output = fixture.root.join("owned-output");
        let error =
            crate::backup2::unback_backup(&fixture.root, "owned-device", Some(&output), None)
                .unwrap_err();
        io_source(&error, class);
        assert!(!output.exists());
        assert_eq!(
            fs::read(
                fixture
                    .device
                    .join("11/1111111111111111111111111111111111111111")
            )
            .unwrap(),
            b"owned-payload"
        );
    }
}

#[test]
fn actual_sqlite_database_and_schema_failures_keep_codes_and_unknown_without_prose_matching() {
    let fixture = Fixture::new();
    fs::write(
        fixture.device.join("Manifest.db"),
        b"owned-private not a database",
    )
    .unwrap();
    let error = fixture.list().unwrap_err();
    let native = io_source(&error, ErrorClass::Protocol)
        .source()
        .unwrap()
        .downcast_ref::<rusqlite::Error>()
        .unwrap();
    assert_eq!(
        native.sqlite_error_code(),
        Some(rusqlite::ErrorCode::NotADatabase)
    );
    fs::remove_file(fixture.device.join("Manifest.db")).unwrap();
    drop(Connection::open(fixture.device.join("Manifest.db")).unwrap());
    let error = fixture.list().unwrap_err();
    let native = io_source(&error, ErrorClass::Unknown)
        .source()
        .unwrap()
        .downcast_ref::<rusqlite::Error>()
        .unwrap();
    assert_eq!(
        native.sqlite_error_code(),
        Some(rusqlite::ErrorCode::Unknown)
    );
}

#[test]
fn actual_sqlite_busy_constraint_and_readonly_keep_typed_outcomes_and_original_extended_codes() {
    let fixture = Fixture::new();
    let path = fixture.device.join("owned.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute("CREATE TABLE owned(value INTEGER UNIQUE)", [])
        .unwrap();
    connection
        .execute("INSERT INTO owned VALUES(1)", [])
        .unwrap();
    let duplicate = connection
        .execute("INSERT INTO owned VALUES(1)", [])
        .unwrap_err();
    let extended = duplicate.sqlite_error().unwrap().extended_code;
    let error = sqlite_error(duplicate);
    let native = io_source(&error, ErrorClass::Conflict)
        .source()
        .unwrap()
        .downcast_ref::<rusqlite::Error>()
        .unwrap();
    assert_eq!(native.sqlite_error().unwrap().extended_code, extended);
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let other = Connection::open(&path).unwrap();
    other.busy_timeout(std::time::Duration::ZERO).unwrap();
    let error = sqlite_error(
        other
            .query_row("SELECT value FROM owned", [], |row| row.get::<_, i64>(0))
            .unwrap_err(),
    );
    io_source(&error, ErrorClass::Conflict);
    connection.execute_batch("ROLLBACK").unwrap();
    let readonly = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let error = sqlite_error(
        readonly
            .execute("INSERT INTO owned VALUES(2)", [])
            .unwrap_err(),
    );
    io_source(&error, ErrorClass::Precondition);
}

#[test]
fn actual_sqlite_authorizer_and_bounded_blob_limit_report_permission_and_resource_limit() {
    unsafe extern "C" fn deny_select(
        _: *mut std::ffi::c_void,
        action: std::os::raw::c_int,
        _: *const std::os::raw::c_char,
        _: *const std::os::raw::c_char,
        _: *const std::os::raw::c_char,
        _: *const std::os::raw::c_char,
    ) -> std::os::raw::c_int {
        if action == rusqlite::ffi::SQLITE_SELECT {
            rusqlite::ffi::SQLITE_DENY
        } else {
            rusqlite::ffi::SQLITE_OK
        }
    }
    let connection = Connection::open_in_memory().unwrap();
    // This test exclusively owns the live connection. The static callback has
    // no borrowed data; the bounded limit rejects the BLOB before allocation.
    unsafe {
        let handle = connection.handle();
        assert_eq!(
            rusqlite::ffi::sqlite3_set_authorizer(handle, Some(deny_select), std::ptr::null_mut()),
            rusqlite::ffi::SQLITE_OK
        );
    }
    let error = sqlite_error(
        connection
            .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .unwrap_err(),
    );
    io_source(&error, ErrorClass::PermissionDenied);
    unsafe {
        let handle = connection.handle();
        assert_eq!(
            rusqlite::ffi::sqlite3_set_authorizer(handle, None, std::ptr::null_mut()),
            rusqlite::ffi::SQLITE_OK
        );
        rusqlite::ffi::sqlite3_limit(handle, rusqlite::ffi::SQLITE_LIMIT_LENGTH, 64);
    }
    let error = sqlite_error(
        connection
            .query_row("SELECT zeroblob(65)", [], |row| row.get::<_, Vec<u8>>(0))
            .unwrap_err(),
    );
    io_source(&error, ErrorClass::ResourceLimit);
}

#[test]
fn ambiguous_sqlite_open_and_interrupt_codes_do_not_infer_permission_or_cancellation() {
    for code in [
        rusqlite::ffi::SQLITE_CANTOPEN,
        rusqlite::ffi::SQLITE_INTERRUPT,
    ] {
        let native = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            Some("owned-private cancelled permission denied timeout".into()),
        );
        let error = sqlite_error(native);
        let source = io_source(&error, ErrorClass::Unknown)
            .source()
            .unwrap()
            .downcast_ref::<rusqlite::Error>()
            .unwrap();
        assert_eq!(source.sqlite_error().unwrap().extended_code, code);
    }
}

#[test]
fn actual_metadata_integer_conversions_retain_native_error_without_output_creation() {
    for (mode, size) in [(-1, 0), (MODE_TYPE_FILE as i64, -1)] {
        let fixture = Fixture::new();
        let archive = crate::archive_dict(vec![
            ("Mode".into(), plist::Value::Integer(mode.into())),
            ("Size".into(), plist::Value::Integer(size.into())),
        ]);
        fixture.database(&archive, MODE_TYPE_FILE);
        let error = fixture.list().unwrap_err();
        assert!(io_source(&error, ErrorClass::Protocol)
            .source()
            .unwrap()
            .is::<std::num::TryFromIntError>());
        let output = fixture.root.join("owned-output");
        assert!(
            crate::backup2::unback_backup(&fixture.root, "owned-device", Some(&output), None)
                .is_err()
        );
        assert!(!output.exists());
    }
}

#[test]
fn actual_legacy_keybag_and_crypto_io_failures_keep_typed_sources() {
    let fixture = Fixture::new();
    fixture.version("10.2");
    fs::write(
        fixture.device.join("Manifest.mbdb"),
        b"owned-private invalid magic",
    )
    .unwrap();
    let error = fixture.list().unwrap_err();
    assert!(io_source(&error, ErrorClass::Protocol)
        .source()
        .unwrap()
        .is::<MbdbError>());
    let crypto =
        BackupCrypto::from_keybag(b"owned-private truncated keybag", "owned-password", false)
            .err()
            .unwrap();
    let error = crypto_error(crypto);
    assert!(io_source(&error, ErrorClass::Protocol)
        .source()
        .unwrap()
        .is::<CryptoError>());
    let missing = fs::File::open(fixture.device.join("owned-private missing")).unwrap_err();
    let code = missing.raw_os_error();
    let error = crypto_error(CryptoError::from(missing));
    let crypto = io_source(&error, ErrorClass::NotFound)
        .source()
        .unwrap()
        .downcast_ref::<CryptoError>()
        .unwrap();
    let original = crypto
        .source()
        .unwrap()
        .downcast_ref::<io::Error>()
        .unwrap();
    assert_eq!(original.raw_os_error(), code);
}

#[test]
fn actual_manifest_version_overflow_retains_parse_int_error() {
    let fixture = Fixture::new();
    fixture.version("18446744073709551616.0");
    let error = fixture.list().unwrap_err();
    assert!(io_source(&error, ErrorClass::Protocol)
        .source()
        .unwrap()
        .is::<std::num::ParseIntError>());
    assert!(!fixture.device.join("Manifest.db").exists());
}

#[cfg(unix)]
#[test]
fn actual_symlink_utf8_failure_keeps_original_bytes_and_never_creates_link() {
    let fixture = Fixture::new();
    let archive = crate::archive_dict(vec![
        (
            "Mode".into(),
            plist::Value::Integer((MODE_TYPE_SYMLINK as i64).into()),
        ),
        ("Size".into(), plist::Value::Integer(1.into())),
    ]);
    fixture.database(&archive, MODE_TYPE_SYMLINK);
    let output = fixture.root.join("owned-output");
    let error = crate::backup2::unback_backup(&fixture.root, "owned-device", Some(&output), None)
        .unwrap_err();
    let native = io_source(&error, ErrorClass::Protocol)
        .source()
        .unwrap()
        .downcast_ref::<std::string::FromUtf8Error>()
        .unwrap();
    assert_eq!(native.as_bytes(), b"\xff");
    assert!(!output.join("HomeDomain/owned-file").exists());
}

#[test]
fn actual_public_list_and_extract_keep_valid_metadata_payload_and_result_fields() {
    let fixture = Fixture::new();
    let archive = crate::archive_dict(vec![
        (
            "Mode".into(),
            plist::Value::Integer((MODE_TYPE_FILE as i64 | 0o600).into()),
        ),
        ("Size".into(), plist::Value::Integer(13.into())),
    ]);
    fixture.database(&archive, MODE_TYPE_FILE);
    let entries = fixture.list().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].domain, "HomeDomain");
    assert_eq!(entries[0].relative_path, "owned-file");
    assert_eq!(entries[0].mode, MODE_TYPE_FILE | 0o600);
    assert_eq!(entries[0].size, 13);
    let output = fixture.root.join("owned-output");
    let result =
        crate::backup2::unback_backup(&fixture.root, "owned-device", Some(&output), None).unwrap();
    assert_eq!(result.files_extracted, 1);
    assert_eq!(result.bytes_extracted, 13);
    assert_eq!(
        fs::read(output.join("HomeDomain/owned-file")).unwrap(),
        b"owned-payload"
    );
}

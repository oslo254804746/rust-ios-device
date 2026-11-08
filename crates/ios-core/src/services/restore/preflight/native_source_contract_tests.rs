use std::{
    error::Error,
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use super::*;
use crate::services::restore::test_assets::{os_failure, zip};
use crate::{test_native::has_source, ErrorClass};

fn selector() -> IpswIdentitySelector<'static> {
    IpswIdentitySelector {
        board_id: 1,
        chip_id: 2,
        device_class: "owned",
        restore_behavior: "Update",
    }
}

struct Reader {
    bytes: Cursor<Vec<u8>>,
    fail: Arc<AtomicBool>,
}
impl Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(os_failure());
        }
        self.bytes.read(bytes)
    }
}
impl Seek for Reader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(os_failure());
        }
        self.bytes.seek(position)
    }
}

fn zip_failure(error: &RestoreError, class: ErrorClass) {
    assert!(has_source::<zip::result::ZipError>(error));
    assert_eq!(crate::classify_error(error), class);
    assert!(!format!("{error:?}").contains("owned-private"));
    let RestoreError::Io(outer) = error else {
        panic!("ZIP cause flattened")
    };
    let original = outer
        .source()
        .unwrap()
        .downcast_ref::<zip::result::ZipError>()
        .unwrap();
    if let zip::result::ZipError::Io(original) = original {
        assert_eq!(outer.kind(), original.kind());
        assert!(original.raw_os_error().is_some());
    }
}

#[test]
fn native_source_ipsw_open_retains_zip_format_and_io_failures() {
    let bad = IpswPreflight::open(Cursor::new(b"owned-private bad ZIP"), selector())
        .err()
        .unwrap();
    let fail = Arc::new(AtomicBool::new(true));
    let reader = Reader {
        bytes: Cursor::new(zip(&[("BuildManifest.plist", b"bytes")])),
        fail,
    };
    let error = IpswPreflight::open(reader, selector()).err().unwrap();
    zip_failure(&bad, ErrorClass::InvalidInput);
    zip_failure(&error, ErrorClass::NotFound);
}

#[test]
fn native_source_ipsw_scan_retains_nested_zip_io() {
    let fail = Arc::new(AtomicBool::new(false));
    let reader = Reader {
        bytes: Cursor::new(zip(&[("BuildManifest.plist", b"bytes")])),
        fail: fail.clone(),
    };
    let mut archive = zip::ZipArchive::new(reader).unwrap();
    fail.store(true, Ordering::SeqCst);
    let error = read_zip_member(&mut archive, "BuildManifest.plist", 1024).unwrap_err();
    zip_failure(&error, ErrorClass::NotFound);
}

#[test]
fn native_source_ipsw_entry_retains_zip_password_error() {
    let mut bytes = zip(&[("BuildManifest.plist", b"bytes")]);
    bytes[6] |= 1;
    let central = bytes
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    bytes[central + 8] |= 1;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let error = read_zip_member(&mut archive, "BuildManifest.plist", 1024).unwrap_err();
    zip_failure(&error, ErrorClass::InvalidInput);
}

#[test]
fn native_source_ipsw_crc_read_retains_original_io() {
    let body = b"owned-private unique CRC payload";
    let mut bytes = zip(&[("BuildManifest.plist", body)]);
    let at = bytes
        .windows(body.len())
        .position(|bytes| bytes == body)
        .unwrap();
    bytes[at] ^= 1;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let error = read_zip_member(&mut archive, "BuildManifest.plist", 1024).unwrap_err();
    let RestoreError::Io(outer) = &error else {
        panic!("read IO flattened")
    };
    let original = outer.source().unwrap().downcast_ref::<io::Error>().unwrap();
    assert_eq!(outer.kind(), original.kind());
    assert_eq!(
        crate::classify_error(&error),
        crate::classify_error(original)
    );
}

#[test]
fn native_source_ipsw_manifest_retains_local_plist_error() {
    let bytes = zip(&[(
        "BuildManifest.plist",
        b"<?xml version=\"1.0\"?><plist><dict><key>owned-private</key>",
    )]);
    let error = IpswPreflight::open(Cursor::new(bytes), selector())
        .err()
        .unwrap();
    assert!(has_source::<plist::Error>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    assert!(!format!("{error:?}").contains("owned-private"));
}

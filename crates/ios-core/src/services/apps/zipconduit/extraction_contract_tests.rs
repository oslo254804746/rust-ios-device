//! Real ZIP decoder faults, not physical installation acceptance.

use super::*;
use std::error::Error;

pub(super) fn stored_zip() -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer
        .add_directory("Payload/owned-private.app/", options)
        .unwrap();
    writer
        .start_file("Payload/owned-private.app/file.txt", options)
        .unwrap();
    writer.write_all(b"owned-payload").unwrap();
    writer.finish().unwrap().into_inner()
}

pub(super) fn damaged_packages() -> [(Vec<u8>, crate::ErrorClass, &'static str); 4] {
    let bytes = stored_zip();
    let (header_start, data_start, central_header_start) = {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let entry = archive.by_index(1).unwrap();
        (
            entry.header_start() as usize,
            entry.data_start() as usize,
            entry.central_header_start() as usize,
        )
    };
    let mut header = bytes.clone();
    header[header_start..header_start + 4].fill(0);
    // Mark password-required metadata. This is not a real encrypted-payload fixture.
    let mut password_required = bytes.clone();
    password_required[header_start + 6] |= 1;
    password_required[central_header_start + 8] |= 1;
    let mut checksum = bytes;
    checksum[data_start] ^= 1;
    [
        (
            vec![0; 12],
            crate::ErrorClass::InvalidInput,
            "open IPA ZIP archive",
        ),
        (
            header,
            crate::ErrorClass::InvalidInput,
            "open IPA ZIP archive",
        ),
        (
            password_required,
            crate::ErrorClass::InvalidInput,
            "open IPA ZIP entry",
        ),
        (checksum, crate::ErrorClass::Protocol, "read IPA ZIP entry"),
    ]
}

#[test]
fn malformed_archive_retains_original_typed_zip_error() {
    let error = match extract_zip_entries(&[0; 12]) {
        Err(error) => error,
        Ok(_) => panic!("invalid archive accepted"),
    };
    let mut current: &(dyn Error + 'static) = &error;
    let mut original = None;
    while let Some(source) = current.source() {
        if let Some(zip) = source.downcast_ref::<zip::result::ZipError>() {
            original = Some(zip);
            break;
        }
        current = source;
    }
    assert!(
        original.is_some(),
        "archive's original typed ZIP error was discarded"
    );
    assert!(matches!(
        original.unwrap(),
        zip::result::ZipError::InvalidArchive(_)
    ));
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::InvalidInput
    );
}

#[test]
fn invalid_local_entry_header_retains_original_zip_error() {
    let [_, (bytes, class, phase), _, _] = damaged_packages();
    // Locked zip 2.4.2 checks local headers while constructing the archive.
    let error = extract_zip_entries(&bytes)
        .err()
        .expect("invalid header accepted");
    let ZipConduitError::Io(outer) = &error else {
        panic!("entry ZIP cause discarded");
    };
    assert!(matches!(
        outer
            .source()
            .unwrap()
            .downcast_ref::<zip::result::ZipError>(),
        Some(zip::result::ZipError::InvalidArchive(_))
    ));
    assert_eq!(crate::classify_error(&error), class);
    assert!(error.to_string().contains(phase));
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[test]
fn password_required_entry_retains_original_zip_reason_without_device_inference() {
    let [_, _, (bytes, class, phase), _] = damaged_packages();
    zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
    let error = extract_zip_entries(&bytes)
        .err()
        .expect("password-required entry accepted");
    let ZipConduitError::Io(outer) = &error else {
        panic!("entry ZIP cause discarded");
    };
    assert!(matches!(
        outer
            .source()
            .unwrap()
            .downcast_ref::<zip::result::ZipError>(),
        Some(zip::result::ZipError::UnsupportedArchive(_))
    ));
    assert_eq!(crate::classify_error(&error), class);
    assert!(error.to_string().contains(phase));
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[test]
fn checksum_failure_retains_native_read_error_and_fixed_context() {
    let [_, _, _, (bytes, class, phase)] = damaged_packages();
    let error = extract_zip_entries(&bytes)
        .err()
        .expect("invalid checksum accepted");
    let ZipConduitError::Io(outer) = &error else {
        panic!("entry read cause discarded");
    };
    assert_eq!(outer.kind(), std::io::ErrorKind::InvalidData);
    let native = outer
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(native.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(native.to_string(), "Invalid checksum");
    assert_eq!(crate::classify_error(&error), class);
    assert!(error.to_string().contains(phase));
    assert!(!error.to_string().contains("owned-private"));
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[test]
fn valid_and_empty_archives_preserve_entry_order_names_and_contents() {
    let entries = extract_zip_entries(&stored_zip()).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "Payload/owned-private.app/");
    assert!(entries[0].is_dir);
    assert!(entries[0].data.is_empty());
    assert_eq!(entries[1].name, "Payload/owned-private.app/file.txt");
    assert!(!entries[1].is_dir);
    assert_eq!(entries[1].data, b"owned-payload");
    let empty = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()))
        .finish()
        .unwrap()
        .into_inner();
    assert!(extract_zip_entries(&empty).unwrap().is_empty());
}

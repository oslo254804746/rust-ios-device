use super::*;
use crate::{test_native::has_source, ErrorClass};

#[test]
fn input_native_zip_integer_ranges_keep_conversion_errors() {
    let errors = [
        checked_zip_u16_len("filename", usize::from(u16::MAX) + 1).unwrap_err(),
        checked_zip_i64("RecordCount", u64::MAX).unwrap_err(),
        build_zip_metadata(1, u64::MAX).unwrap_err(),
    ];
    for error in errors {
        assert!(has_source::<std::num::TryFromIntError>(&error));
        assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
        assert!(matches!(error, ZipConduitError::Io(_)));
    }
    assert_eq!(
        checked_zip_u16_len("filename", usize::from(u16::MAX)).unwrap(),
        u16::MAX
    );
    assert_eq!(
        checked_zip_i64("RecordCount", i64::MAX as u64).unwrap(),
        i64::MAX
    );
}

#[cfg(target_pointer_width = "64")]
#[test]
fn input_native_zip_u32_keeps_scalar_overflow_without_giant_allocation() {
    let error = checked_zip_u32_len("file data", u32::MAX as usize + 1).unwrap_err();
    assert!(has_source::<std::num::TryFromIntError>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    assert_eq!(
        checked_zip_u32_len("file data", u32::MAX as usize).unwrap(),
        u32::MAX
    );
}

#[tokio::test]
async fn input_native_zip_filename_fails_before_header_write() {
    let mut wire = Vec::new();
    let error = write_local_file_header(&mut wire, &"x".repeat(usize::from(u16::MAX) + 1), 0, 0, 0)
        .await
        .unwrap_err();
    assert!(has_source::<std::num::TryFromIntError>(&error));
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    assert!(wire.is_empty());
}

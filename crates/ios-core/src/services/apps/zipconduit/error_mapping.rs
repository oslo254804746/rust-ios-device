//! Retain decoder causes through existing error variants; context is fixed.

use super::ZipConduitError;
use std::io;
use zip::result::ZipError;

#[derive(thiserror::Error)]
#[error("{phase}: {source}")]
struct ZipOperationError {
    phase: &'static str,
    #[source]
    source: ZipError,
}

impl std::fmt::Debug for ZipOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipOperationError")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

#[derive(thiserror::Error)]
#[error("read IPA ZIP entry: {source}")]
struct EntryReadError {
    #[source]
    source: io::Error,
}

impl std::fmt::Debug for EntryReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EntryReadError { .. }")
    }
}

pub(super) fn zip_error(phase: &'static str, source: ZipError) -> ZipConduitError {
    let kind = match &source {
        ZipError::Io(error) => error.kind(),
        // These describe the local package, not a device capability response.
        ZipError::InvalidArchive(_)
        | ZipError::UnsupportedArchive(_)
        | ZipError::FileNotFound
        | ZipError::InvalidPassword => io::ErrorKind::InvalidInput,
        // Preserve unknown future sources without inventing a precise reason.
        _ => io::ErrorKind::Other,
    };
    ZipConduitError::Io(io::Error::new(kind, ZipOperationError { phase, source }))
}

pub(super) fn entry_read_error(source: io::Error) -> ZipConduitError {
    ZipConduitError::Io(io::Error::new(source.kind(), EntryReadError { source }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn decoder_adapters_retain_native_kinds_causes_and_safe_debug() {
        for (kind, class) in [
            (io::ErrorKind::TimedOut, crate::ErrorClass::Timeout),
            (
                io::ErrorKind::PermissionDenied,
                crate::ErrorClass::PermissionDenied,
            ),
            (io::ErrorKind::BrokenPipe, crate::ErrorClass::Disconnected),
            (io::ErrorKind::InvalidData, crate::ErrorClass::Protocol),
            (io::ErrorKind::UnexpectedEof, crate::ErrorClass::Io),
            (io::ErrorKind::Interrupted, crate::ErrorClass::Io),
            (io::ErrorKind::Other, crate::ErrorClass::Io),
        ] {
            let secret = "owned-private-zip-native-source";
            let error = zip_error(
                "open IPA ZIP entry",
                ZipError::Io(io::Error::new(kind, secret)),
            );
            assert_eq!(crate::classify_error(&error), class);
            let ZipConduitError::Io(outer) = &error else {
                panic!("ZIP source lost");
            };
            assert_eq!(outer.kind(), kind);
            let payload = outer
                .get_ref()
                .unwrap()
                .downcast_ref::<ZipOperationError>()
                .unwrap();
            assert_eq!(payload.phase, "open IPA ZIP entry");
            let ZipError::Io(native) = payload
                .source()
                .unwrap()
                .downcast_ref::<ZipError>()
                .unwrap()
            else {
                panic!("native source lost");
            };
            assert_eq!(native.kind(), kind);
            assert_eq!(native.to_string(), secret);
            assert!(!format!("{error:?} {payload:?}").contains(secret));
            let error = entry_read_error(io::Error::new(kind, secret));
            assert_eq!(crate::classify_error(&error), class);
            let ZipConduitError::Io(outer) = &error else {
                panic!("read source lost");
            };
            assert_eq!(outer.kind(), kind);
            let payload = outer
                .get_ref()
                .unwrap()
                .downcast_ref::<EntryReadError>()
                .unwrap();
            let native = payload
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap();
            assert_eq!(native.kind(), kind);
            assert_eq!(native.to_string(), secret);
            assert!(!format!("{error:?} {payload:?}").contains(secret));
        }
    }

    #[test]
    fn non_io_package_reasons_retain_input_class_and_original_zip_variant() {
        for source in [
            ZipError::InvalidArchive("owned-private-invalid-archive"),
            ZipError::UnsupportedArchive("owned-private-unsupported-archive"),
            ZipError::FileNotFound,
            ZipError::InvalidPassword,
        ] {
            let original_kind = std::mem::discriminant(&source);
            let error = zip_error("open IPA ZIP archive", source);
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::InvalidInput
            );
            let ZipConduitError::Io(outer) = &error else {
                panic!("ZIP source lost");
            };
            let source = outer.source().unwrap().downcast_ref::<ZipError>().unwrap();
            assert_eq!(std::mem::discriminant(source), original_kind);
            assert!(!format!("{error:?}").contains("owned-private"));
        }
        assert_eq!(
            crate::classify_error(&ZipConduitError::Zip("legacy private reason".into())),
            crate::ErrorClass::InvalidInput
        );
    }
}

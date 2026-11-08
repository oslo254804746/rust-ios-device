//! Fixed owner phases retain H2/XPC causes through the existing IO variant.

use super::FetchSymbolsError;
use crate::xpc::h2_raw::H2Error;
use crate::XpcError;
use std::io;

#[derive(thiserror::Error)]
#[error("{phase}: {source}")]
struct OperationError<E: std::error::Error + 'static> {
    phase: &'static str,
    #[source]
    source: E,
}

impl<E: std::error::Error> std::fmt::Debug for OperationError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteSymbolsOperationError")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

pub(super) fn h2_error(phase: &'static str, source: H2Error) -> FetchSymbolsError {
    let kind = match &source {
        H2Error::Io(error) => error.kind(),
        H2Error::Protocol(_) => io::ErrorKind::InvalidData,
    };
    FetchSymbolsError::Io(io::Error::new(kind, OperationError { phase, source }))
}

pub(super) fn xpc_error(phase: &'static str, source: XpcError) -> FetchSymbolsError {
    let kind = match &source {
        XpcError::Io(error) => error.kind(),
        // Let the retained typed source select protocol/missing/disconnected.
        _ => io::ErrorKind::Other,
    };
    FetchSymbolsError::Io(io::Error::new(kind, OperationError { phase, source }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn native_h2_kinds_original_causes_and_protocol_remain_typed() {
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
            let error = h2_error(
                "file stream read failed",
                H2Error::Io(io::Error::new(kind, "owned-private-h2-cause")),
            );
            assert_eq!(crate::classify_error(&error), class);
            let FetchSymbolsError::Io(outer) = &error else {
                panic!("H2 source discarded");
            };
            assert_eq!(outer.kind(), kind);
            let H2Error::Io(native) = outer.source().unwrap().downcast_ref::<H2Error>().unwrap()
            else {
                panic!("IO source discarded");
            };
            assert_eq!(native.kind(), kind);
            assert_eq!(native.to_string(), "owned-private-h2-cause");
            assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
        }
        let error = h2_error(
            "H2 error",
            H2Error::Protocol("owned-private-protocol".into()),
        );
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(error.source().unwrap().source().unwrap().is::<H2Error>());
    }

    #[test]
    fn xpc_adapters_preserve_original_variants_without_text_classification() {
        for (source, class) in [
            (
                XpcError::Tls("owned-private-timeout-unsupported".into()),
                crate::ErrorClass::Protocol,
            ),
            (XpcError::NotConnected, crate::ErrorClass::Disconnected),
            (
                XpcError::ServiceNotFound("owned-private-service".into()),
                crate::ErrorClass::ServiceMissing,
            ),
            (
                XpcError::Io(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "owned-private-source",
                )),
                crate::ErrorClass::PermissionDenied,
            ),
        ] {
            let original = std::mem::discriminant(&source);
            let error = xpc_error("control decode failed", source);
            assert_eq!(crate::classify_error(&error), class);
            let FetchSymbolsError::Io(outer) = &error else {
                panic!("XPC source discarded");
            };
            let retained = outer.source().unwrap().downcast_ref::<XpcError>().unwrap();
            assert_eq!(std::mem::discriminant(retained), original);
            assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
        }
    }
}

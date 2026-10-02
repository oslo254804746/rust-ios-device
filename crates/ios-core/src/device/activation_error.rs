//! Preserve modern activation/deactivation causes under the existing CoreError.

use crate::mobileactivation::MobileActivationError;
use crate::CoreError;
use std::io;

#[derive(thiserror::Error)]
#[error("mobile activation: {source}")]
struct ActivationError {
    #[source]
    source: MobileActivationError,
}

impl std::fmt::Debug for ActivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActivationError { .. }")
    }
}

pub(super) fn service_error(source: MobileActivationError) -> CoreError {
    let kind = match &source {
        MobileActivationError::Io(error) => error.kind(),
        _ => io::ErrorKind::Other,
    };
    CoreError::Io(io::Error::new(kind, ActivationError { source }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[tokio::test]
    async fn real_modern_service_write_failure_retains_native_source_through_core_mapping() {
        let (stream, peer) = tokio::io::duplex(64);
        drop(peer);
        let mut client = crate::mobileactivation::MobileActivationClient::new(stream);
        let native = client
            .activate(b"owned-private-record", &std::collections::BTreeMap::new())
            .await
            .unwrap_err();
        let error = service_error(native);
        assert_eq!(error.class(), crate::ErrorClass::Disconnected);
        let original = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<MobileActivationError>()
            .unwrap();
        let MobileActivationError::Io(io) = original else {
            panic!("service write cause lost");
        };
        assert_eq!(io.kind(), std::io::ErrorKind::BrokenPipe);
        assert!(!format!("{error:?}").contains("owned-private"));
    }

    #[test]
    fn modern_activation_mapping_retains_native_io_sources_and_safe_context() {
        for (kind, class) in [
            (io::ErrorKind::TimedOut, crate::ErrorClass::Timeout),
            (
                io::ErrorKind::PermissionDenied,
                crate::ErrorClass::PermissionDenied,
            ),
            (io::ErrorKind::BrokenPipe, crate::ErrorClass::Disconnected),
            (io::ErrorKind::InvalidData, crate::ErrorClass::Protocol),
            (io::ErrorKind::Other, crate::ErrorClass::Io),
        ] {
            let error = service_error(MobileActivationError::Io(io::Error::new(
                kind,
                "owned-private-activation",
            )));
            assert_eq!(error.class(), class);
            let CoreError::Io(outer) = &error else {
                panic!("native source lost");
            };
            let MobileActivationError::Io(native) = outer
                .source()
                .unwrap()
                .downcast_ref::<MobileActivationError>()
                .unwrap()
            else {
                panic!("service source lost");
            };
            assert_eq!(outer.kind(), kind);
            assert_eq!(native.to_string(), "owned-private-activation");
            assert!(error.to_string().contains("mobile activation"));
            assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
        }
    }

    #[test]
    fn modern_activation_mapping_keeps_typed_non_io_and_inner_plist_causes() {
        let invalid_xml =
            b"<?xml version=\"1.0\"?><plist version=\"1.0\"><integer>bad</integer></plist>";
        for (source, class) in [
            (
                MobileActivationError::Timeout(std::time::Duration::from_secs(2)),
                crate::ErrorClass::Timeout,
            ),
            (
                MobileActivationError::Http("owned-private-timeout".into()),
                crate::ErrorClass::ServiceFailure,
            ),
            (
                MobileActivationError::Protocol("owned-private-unsupported".into()),
                crate::ErrorClass::Protocol,
            ),
            (
                MobileActivationError::Plist(
                    plist::from_bytes::<plist::Value>(invalid_xml).unwrap_err(),
                ),
                crate::ErrorClass::Protocol,
            ),
            (
                MobileActivationError::Plist(plist::from_bytes::<plist::Value>(b"?").unwrap_err()),
                crate::ErrorClass::Io,
            ),
        ] {
            let original = std::mem::discriminant(&source);
            let error = service_error(source);
            assert_eq!(error.class(), class);
            let retained = error
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<MobileActivationError>()
                .unwrap();
            assert_eq!(std::mem::discriminant(retained), original);
            assert!(!format!("{error:?}").contains("owned-private"));
        }
    }
}

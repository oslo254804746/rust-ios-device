//! Preserve both failures while retaining the final fallback's category.

use crate::{CoreError, ErrorClass};
use std::future::Future;

/// Both failed attempts in `ConnectedDevice::deactivate`, without retry authority.
///
/// Available as the custom payload of the returned `CoreError::Io` through
/// `io::Error::get_ref().downcast_ref()`. Its source is the final fallback error;
/// the earlier service error is a separate borrowed observation. Category
/// snapshots are taken before wrapping and do not perform IO or certify cleanup.
#[derive(thiserror::Error)]
#[error("mobileactivationd deactivation failed ({service_error}); lockdown fallback failed ({fallback_error})")]
pub struct DeactivationFallbackError {
    service_error: CoreError,
    #[source]
    fallback_error: CoreError,
    service_class: ErrorClass,
    fallback_class: ErrorClass,
}

impl DeactivationFallbackError {
    /// Original mobileactivation connection or service failure.
    pub fn service_error(&self) -> &CoreError {
        &self.service_error
    }
    /// Original final lockdown connection or request failure; also the source.
    pub fn fallback_error(&self) -> &CoreError {
        &self.fallback_error
    }
    /// Category observed before the first failure was wrapped.
    pub fn service_class(&self) -> ErrorClass {
        self.service_class
    }
    /// Final category used by `classify_error`, including an opaque Unknown.
    pub fn fallback_class(&self) -> ErrorClass {
        self.fallback_class
    }
}

impl std::fmt::Debug for DeactivationFallbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeactivationFallbackError")
            .field("service_class", &self.service_class)
            .field("fallback_class", &self.fallback_class)
            .finish_non_exhaustive()
    }
}

fn combine(service_error: CoreError, fallback_error: CoreError) -> CoreError {
    let service_class = service_error.class();
    let fallback_class = fallback_error.class();
    let kind = match &fallback_error {
        CoreError::Io(error) => error.kind(),
        _ => std::io::ErrorKind::Other,
    };
    CoreError::Io(std::io::Error::new(
        kind,
        DeactivationFallbackError {
            service_error,
            fallback_error,
            service_class,
            fallback_class,
        },
    ))
}

pub(super) async fn deactivate_with_fallback<F, B, BF>(
    service: F,
    fallback: B,
) -> Result<(), CoreError>
where
    F: Future<Output = Result<(), CoreError>>,
    B: FnOnce() -> BF,
    BF: Future<Output = Result<(), CoreError>>,
{
    match service.await {
        Ok(()) => Ok(()),
        Err(service_error) => match fallback().await {
            Ok(()) => Ok(()),
            Err(fallback_error) => Err(combine(service_error, fallback_error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn successful_service_skips_fallback_and_successful_fallback_runs_once() {
        let calls = AtomicUsize::new(0);
        for (service, expected_calls) in [
            (Ok(()), 0),
            (Err(CoreError::Other("owned-private".into())), 1),
        ] {
            deactivate_with_fallback(std::future::ready(service), || {
                calls.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok(()))
            })
            .await
            .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
        }
    }

    #[tokio::test]
    async fn both_errors_remain_borrowed_with_final_category_including_unknown() {
        for (fallback, expected) in [
            (
                CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "owned-private",
                )),
                ErrorClass::Timeout,
            ),
            (
                CoreError::Lockdown(crate::LockdownError::Io(std::io::Error::from(
                    std::io::ErrorKind::BrokenPipe,
                ))),
                ErrorClass::Disconnected,
            ),
            (
                CoreError::Xpc(crate::XpcError::ServiceNotFound("owned-private".into())),
                ErrorClass::ServiceMissing,
            ),
            (
                CoreError::Protocol("owned-private".into()),
                ErrorClass::Protocol,
            ),
            (
                CoreError::Other("owned-private timeout unsupported".into()),
                ErrorClass::Unknown,
            ),
        ] {
            let original = std::mem::discriminant(&fallback);
            let error = deactivate_with_fallback(
                std::future::ready(Err(CoreError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "owned-private",
                )))),
                || std::future::ready(Err(fallback)),
            )
            .await
            .unwrap_err();
            assert_eq!(error.class(), expected);
            let CoreError::Io(outer) = &error else {
                panic!("combined source lost");
            };
            let observation = outer
                .get_ref()
                .unwrap()
                .downcast_ref::<crate::DeactivationFallbackError>()
                .unwrap();
            assert_eq!(observation.service_class(), ErrorClass::PermissionDenied);
            assert_eq!(observation.fallback_class(), expected);
            assert_eq!(
                std::mem::discriminant(observation.fallback_error()),
                original
            );
            assert_eq!(
                observation.service_error().class(),
                ErrorClass::PermissionDenied
            );
            let source = observation
                .source()
                .unwrap()
                .downcast_ref::<CoreError>()
                .unwrap();
            assert!(std::ptr::eq(source, observation.fallback_error()));
            assert!(!format!("{error:?} {observation:?}").contains("owned-private"));
        }
    }

    #[tokio::test]
    async fn fallback_establishment_failure_keeps_first_error_and_native_dial_cause() {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = socket.local_addr().unwrap();
        #[cfg(target_os = "macos")]
        drop(socket);
        let error = deactivate_with_fallback(
            std::future::ready(Err(CoreError::Unsupported("owned-private".into()))),
            || async {
                tokio::net::TcpStream::connect(address)
                    .await
                    .map(|_| ())
                    .map_err(CoreError::Io)
            },
        )
        .await
        .unwrap_err();
        #[cfg(not(target_os = "macos"))]
        drop(socket);
        assert_eq!(error.class(), ErrorClass::Io);
        let CoreError::Io(outer) = &error else {
            panic!("combined source lost");
        };
        let observation = outer
            .get_ref()
            .unwrap()
            .downcast_ref::<crate::DeactivationFallbackError>()
            .unwrap();
        assert_eq!(observation.service_class(), ErrorClass::Unsupported);
        let CoreError::Io(native) = observation.fallback_error() else {
            panic!("dial source lost");
        };
        assert_eq!(native.kind(), std::io::ErrorKind::ConnectionRefused);
    }

    #[tokio::test]
    async fn real_modern_service_and_legacy_frame_failures_keep_both_original_causes() {
        let (modern, peer) = tokio::io::duplex(64);
        drop(peer);
        let (mut legacy, peer) = tokio::io::duplex(64);
        drop(peer);
        let mut modern = crate::mobileactivation::MobileActivationClient::new(modern);
        let error = deactivate_with_fallback(
            async {
                modern
                    .deactivate()
                    .await
                    .map(|_| ())
                    .map_err(super::super::activation_error::service_error)
            },
            || async {
                crate::lockdown::protocol::send_lockdown(
                    &mut legacy,
                    &plist::Dictionary::from_iter([
                        ("Label", plist::Value::String("ios-rs".into())),
                        ("Request", plist::Value::String("Deactivate".into())),
                    ]),
                )
                .await
                .map_err(CoreError::from)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.class(), ErrorClass::Disconnected);
        let CoreError::Io(outer) = &error else {
            panic!("combined source lost");
        };
        let observation = outer
            .get_ref()
            .unwrap()
            .downcast_ref::<crate::DeactivationFallbackError>()
            .unwrap();
        let service = observation
            .service_error()
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<crate::mobileactivation::MobileActivationError>()
            .unwrap();
        let crate::mobileactivation::MobileActivationError::Io(native) = service else {
            panic!("modern write cause lost");
        };
        assert_eq!(native.kind(), std::io::ErrorKind::BrokenPipe);
        let CoreError::Lockdown(crate::LockdownError::Io(native)) = observation.fallback_error()
        else {
            panic!("legacy write cause lost");
        };
        assert_eq!(native.kind(), std::io::ErrorKind::BrokenPipe);
    }
}

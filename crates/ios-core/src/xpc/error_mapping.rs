//! Preserve typed HTTP/2 failures across the existing XPC error boundary.

use super::h2_raw::H2Error;
use super::XpcError;

pub(super) async fn xpc_timeout<T>(
    duration: std::time::Duration,
    phase: &'static str,
    operation: impl std::future::Future<Output = Result<T, XpcError>>,
) -> Result<T, XpcError> {
    tokio::time::timeout(duration, operation)
        .await
        .map_err(|error| super::native_error(std::io::ErrorKind::TimedOut, phase, error))?
}

#[derive(thiserror::Error)]
#[error("{prefix}{source}")]
struct H2OperationError {
    prefix: &'static str,
    #[source]
    source: H2Error,
}

impl std::fmt::Debug for H2OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H2OperationError")
            .field("phase", &self.prefix)
            .finish_non_exhaustive()
    }
}

/// Prefixes are fixed owner context, never a URL, identifier or device value.
/// Keep the original H2Error and its sources; no diagnostic text is classified.
pub(super) fn h2_error(prefix: &'static str, source: H2Error) -> XpcError {
    let kind = match &source {
        H2Error::Io(error) => error.kind(),
        H2Error::Protocol(_) => std::io::ErrorKind::InvalidData,
    };
    XpcError::Io(std::io::Error::new(
        kind,
        H2OperationError { prefix, source },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{classify_error, ErrorClass};
    use std::error::Error;

    #[tokio::test]
    async fn native_stage_timeout_keeps_elapsed_drops_local_future_and_preserves_inner_outcomes() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        struct Pending(Arc<AtomicBool>);
        impl std::future::Future for Pending {
            type Output = Result<(), XpcError>;
            fn poll(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                std::task::Poll::Pending
            }
        }
        impl Drop for Pending {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let error = xpc_timeout(
            std::time::Duration::from_millis(1),
            "owned stage",
            Pending(dropped.clone()),
        )
        .await
        .unwrap_err();
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(classify_error(&error), ErrorClass::Timeout);
        let XpcError::Io(outer) = error else {
            unreachable!()
        };
        assert!(outer.source().unwrap().is::<tokio::time::error::Elapsed>());
        let error = xpc_timeout(std::time::Duration::from_secs(1), "owned stage", async {
            Err::<(), _>(XpcError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "owned-private",
            )))
        })
        .await
        .unwrap_err();
        assert_eq!(classify_error(&error), ErrorClass::Disconnected);
        let XpcError::Io(inner) = error else {
            unreachable!()
        };
        assert_eq!(inner.kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(inner.to_string(), "owned-private");
        assert_eq!(
            xpc_timeout(std::time::Duration::from_secs(1), "owned stage", async {
                Ok(7)
            })
            .await
            .unwrap(),
            7
        );
    }

    #[test]
    fn mapping_retains_h2_native_sources_context_and_safe_debug() {
        for (kind, class) in [
            (std::io::ErrorKind::TimedOut, ErrorClass::Timeout),
            (
                std::io::ErrorKind::PermissionDenied,
                ErrorClass::PermissionDenied,
            ),
            (std::io::ErrorKind::BrokenPipe, ErrorClass::Disconnected),
            (std::io::ErrorKind::Interrupted, ErrorClass::Io),
            (std::io::ErrorKind::UnexpectedEof, ErrorClass::Io),
        ] {
            let error = h2_error(
                "read header: ",
                H2Error::Io(std::io::Error::new(kind, "synthetic-private-h2-source")),
            );
            assert_eq!(classify_error(&error), class);
            let XpcError::Io(outer) = &error else {
                panic!("native H2 error discarded")
            };
            assert_eq!(outer.kind(), kind);
            let payload = outer
                .get_ref()
                .unwrap()
                .downcast_ref::<H2OperationError>()
                .unwrap();
            assert_eq!(payload.prefix, "read header: ");
            let H2Error::Io(source) = payload.source().unwrap().downcast_ref::<H2Error>().unwrap()
            else {
                panic!("original H2 source discarded")
            };
            assert_eq!(source.kind(), kind);
            assert_eq!(source.to_string(), "synthetic-private-h2-source");
            assert!(!format!("{error:?} {payload:?}").contains("synthetic-private"));
        }
        let error = h2_error(
            "RSD H2: ",
            H2Error::Protocol("synthetic-private-frame".into()),
        );
        assert_eq!(classify_error(&error), ErrorClass::Protocol);
        let XpcError::Io(outer) = &error else {
            panic!("protocol source lost")
        };
        let H2Error::Protocol(source) = outer.source().unwrap().downcast_ref::<H2Error>().unwrap()
        else {
            panic!("typed H2 protocol source lost")
        };
        assert_eq!(source, "synthetic-private-frame");
        assert!(!format!("{error:?}").contains("synthetic-private"));
    }
}

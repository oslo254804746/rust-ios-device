//! Fixed context, retained native causes, and bounded category snapshots.

use crate::ErrorClass;
use std::{error::Error, io};

#[derive(thiserror::Error)]
#[error("{phase}: {source}")]
pub(crate) struct RetainedCause {
    phase: &'static str,
    #[source]
    source: Box<dyn Error + Send + Sync>,
    class: ErrorClass,
}

impl RetainedCause {
    pub(crate) fn class(&self) -> ErrorClass {
        self.class
    }
}

impl std::fmt::Debug for RetainedCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceErrorContext")
            .field("phase", &self.phase)
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}

pub(crate) fn retained_io(
    kind: io::ErrorKind,
    phase: &'static str,
    source: impl Error + Send + Sync + 'static,
) -> io::Error {
    // Preserve Unknown as well: adding an IO container is not native IO evidence.
    let class = crate::classify_error(&source);
    io::Error::new(
        kind,
        RetainedCause {
            phase,
            source: Box::new(source),
            class,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreError;

    fn retain(phase: &'static str, source: impl Error + Send + Sync + 'static) -> CoreError {
        CoreError::Io(retained_io(io::ErrorKind::Other, phase, source))
    }

    fn outer(error: &CoreError) -> &io::Error {
        let CoreError::Io(io) = error else {
            panic!("original source lost");
        };
        assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
        io
    }

    #[test]
    fn original_categories_and_native_sources_survive_generic_context() {
        for (kind, class) in [
            (io::ErrorKind::TimedOut, ErrorClass::Timeout),
            (io::ErrorKind::BrokenPipe, ErrorClass::Disconnected),
            (
                io::ErrorKind::PermissionDenied,
                ErrorClass::PermissionDenied,
            ),
            (io::ErrorKind::InvalidInput, ErrorClass::InvalidInput),
            (io::ErrorKind::InvalidData, ErrorClass::Protocol),
            (io::ErrorKind::NotFound, ErrorClass::NotFound),
            (io::ErrorKind::Interrupted, ErrorClass::Io),
            (io::ErrorKind::UnexpectedEof, ErrorClass::Io),
            (io::ErrorKind::Other, ErrorClass::Io),
        ] {
            let error = retain(
                "owned operation",
                io::Error::new(kind, "owned-private timeout cancelled"),
            );
            assert_eq!(error.class(), class);
            let original = outer(&error)
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap();
            assert_eq!(original.kind(), kind);
            assert_eq!(original.to_string(), "owned-private timeout cancelled");
        }
        let error = retain(
            "owned operation",
            CoreError::Other("owned-private timeout unsupported".into()),
        );
        assert_eq!(error.class(), ErrorClass::Unknown);
        assert!(outer(&error).source().unwrap().is::<CoreError>());
    }

    #[test]
    fn cyclic_native_category_snapshot_is_bounded_and_never_formats_source() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        #[derive(Debug)]
        struct Cycle {
            reads: Arc<AtomicUsize>,
            formats: Arc<AtomicUsize>,
        }
        impl std::fmt::Display for Cycle {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.formats.fetch_add(1, Ordering::SeqCst);
                f.write_str("owned-private timeout")
            }
        }
        impl Error for Cycle {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                self.reads.fetch_add(1, Ordering::SeqCst);
                Some(self)
            }
        }
        let reads = Arc::new(AtomicUsize::new(0));
        let formats = Arc::new(AtomicUsize::new(0));
        let error = retain(
            "owned operation",
            Cycle {
                reads: Arc::clone(&reads),
                formats: Arc::clone(&formats),
            },
        );
        let before = reads.load(Ordering::SeqCst);
        assert!(before > 0 && before <= 32);
        assert_eq!(error.class(), ErrorClass::Unknown);
        outer(&error);
        assert_eq!(reads.load(Ordering::SeqCst), before);
        assert_eq!(formats.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn real_cancelled_async_join_and_panicked_blocking_join_keep_original_flags() {
        #[derive(Debug)]
        struct PrivatePanic;
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let cancelled = task.await.unwrap_err();
        let panicked = tokio::task::spawn_blocking(|| std::panic::panic_any(PrivatePanic))
            .await
            .unwrap_err();
        for (native, class, cancelled) in [
            (cancelled, ErrorClass::Cancelled, true),
            (panicked, ErrorClass::Unknown, false),
        ] {
            let error = retain("load owned records", native);
            assert_eq!(error.class(), class);
            let original = outer(&error)
                .source()
                .unwrap()
                .downcast_ref::<tokio::task::JoinError>()
                .unwrap();
            assert_eq!(original.is_cancelled(), cancelled);
            assert_eq!(original.is_panic(), !cancelled);
        }
    }
}

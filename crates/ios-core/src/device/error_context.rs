//! Fixed context, retained native causes, and bounded category snapshots.

use crate::{CoreError, ErrorClass};
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

pub(super) fn with_kind(
    kind: io::ErrorKind,
    phase: &'static str,
    source: impl Error + Send + Sync + 'static,
) -> CoreError {
    CoreError::Io(retained_io(kind, phase, source))
}

pub(super) fn retain(phase: &'static str, source: impl Error + Send + Sync + 'static) -> CoreError {
    with_kind(io::ErrorKind::Other, phase, source)
}

#[cfg(feature = "tunnel")]
pub(super) fn pairing(
    phase: &'static str,
    source: crate::lockdown::pairing::PairingError,
) -> CoreError {
    let kind = match &source {
        crate::lockdown::pairing::PairingError::Io(error) => error.kind(),
        _ => io::ErrorKind::Other,
    };
    with_kind(kind, phase, source)
}

#[cfg(feature = "tunnel")]
pub(super) fn psk(phase: &'static str, source: crate::psk_tls::PskTlsError) -> CoreError {
    let kind = match &source {
        crate::psk_tls::PskTlsError::Io(error) => error.kind(),
        _ => io::ErrorKind::Other,
    };
    with_kind(kind, phase, source)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[cfg(feature = "tunnel")]
    #[test]
    fn pairing_and_psk_adapters_retain_native_kinds_and_typed_unknown_or_unsupported() {
        use crate::{lockdown::pairing::PairingError, psk_tls::PskTlsError};
        for (kind, class) in [
            (io::ErrorKind::TimedOut, ErrorClass::Timeout),
            (io::ErrorKind::BrokenPipe, ErrorClass::Disconnected),
            (
                io::ErrorKind::PermissionDenied,
                ErrorClass::PermissionDenied,
            ),
            (io::ErrorKind::InvalidData, ErrorClass::Protocol),
            (io::ErrorKind::Other, ErrorClass::Io),
        ] {
            let error = pairing(
                "build owned verification",
                PairingError::Io(io::Error::new(kind, "owned-private")),
            );
            assert_eq!(error.class(), class);
            let io = outer(&error);
            assert_eq!(io.kind(), kind);
            assert!(io.source().unwrap().is::<PairingError>());
            let error = psk(
                "connect owned listener",
                PskTlsError::Io(io::Error::new(kind, "owned-private")),
            );
            assert_eq!(error.class(), class);
            let io = outer(&error);
            assert_eq!(io.kind(), kind);
            assert!(io.source().unwrap().is::<PskTlsError>());
        }
        for (source, class) in [
            (
                PairingError::Crypto("owned-private timeout unsupported".into()),
                ErrorClass::Unknown,
            ),
            (
                PairingError::Protocol("owned-private timeout".into()),
                ErrorClass::Protocol,
            ),
        ] {
            let error = pairing("build owned verification", source);
            assert_eq!(error.class(), class);
            assert!(outer(&error).source().unwrap().is::<PairingError>());
        }
        let error = psk(
            "connect owned listener",
            PskTlsError::Openssl(openssl::error::ErrorStack::get()),
        );
        assert_eq!(error.class(), ErrorClass::Unknown);
        outer(&error);
        let mut backend =
            openssl::ssl::SslContext::builder(openssl::ssl::SslMethod::tls_client()).unwrap();
        let cause = backend.set_cipher_list("owned-invalid-cipher").unwrap_err();
        let error = psk(
            "connect owned listener",
            PskTlsError::PskCipherUnsupported(cause),
        );
        assert_eq!(error.class(), ErrorClass::Unsupported);
        let original = outer(&error)
            .source()
            .unwrap()
            .downcast_ref::<PskTlsError>()
            .unwrap();
        assert!(original
            .source()
            .unwrap()
            .is::<openssl::error::ErrorStack>());
    }

    #[cfg(feature = "tunnel")]
    #[tokio::test]
    async fn actual_psk_refusal_and_closed_stream_keep_dial_or_handshake_causes() {
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = socket.local_addr().unwrap();
        let native = crate::psk_tls::connect_psk_tls("127.0.0.1", address.port(), &[7; 32])
            .await
            .expect_err("owned refusal");
        drop(socket);
        let error = psk("connect owned listener", native);
        assert_eq!(error.class(), ErrorClass::Io);
        let original = outer(&error)
            .source()
            .unwrap()
            .downcast_ref::<crate::psk_tls::PskTlsError>()
            .unwrap();
        let crate::psk_tls::PskTlsError::Io(io) = original else {
            panic!("dial cause lost");
        };
        assert_eq!(io.kind(), io::ErrorKind::ConnectionRefused);
        let (stream, peer) = tokio::io::duplex(64);
        drop(peer);
        let native = crate::psk_tls::connect_psk_tls_stream("owned-private", stream, &[7; 32])
            .await
            .expect_err("closed peer");
        let expected = crate::classify_error(&native);
        let error = psk("connect owned listener", native);
        assert_eq!(error.class(), expected);
        assert!(outer(&error)
            .source()
            .unwrap()
            .is::<crate::psk_tls::PskTlsError>());
        assert_eq!(expected, ErrorClass::Disconnected);
    }
}

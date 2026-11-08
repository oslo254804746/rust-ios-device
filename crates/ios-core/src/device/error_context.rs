//! Device adapters using shared retained native causes.

use crate::CoreError;
use std::{error::Error, io};

pub(crate) use crate::error::context::retained_io;

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

#[cfg(all(test, feature = "tunnel"))]
mod tests {
    use super::*;
    use crate::ErrorClass;

    fn outer(error: &CoreError) -> &io::Error {
        let CoreError::Io(io) = error else {
            panic!("original source lost");
        };
        assert!(!format!("{error:?} {:?}", io.get_ref()).contains("owned-private"));
        io
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
        #[cfg(target_os = "macos")]
        drop(socket);
        let native = crate::psk_tls::connect_psk_tls("127.0.0.1", address.port(), &[7; 32])
            .await
            .expect_err("owned refusal");
        #[cfg(not(target_os = "macos"))]
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

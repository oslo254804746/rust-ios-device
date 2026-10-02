//! Exercise source retention through XPC/RSD operations, without devices.
#![cfg(feature = "tunnel")]

use std::error::Error;
use std::fmt;
use std::io::{self, ErrorKind};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::xpc::h2_raw::{H2Error, H2Framer};
use crate::xpc::rsd::{
    handshake_on_framer, initialize_xpc_connection_on_framer,
    queue_rsd_handshake_bootstrap_on_framer, send_device_handshake_on_framer, XpcConnection,
};
use crate::xpc::XpcClient;
use crate::{classify_error, ErrorClass, XpcError, XpcValue};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

struct OriginalFailure;

impl fmt::Display for OriginalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("synthetic-private-transport-source")
    }
}

impl fmt::Debug for OriginalFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OriginalFailure")
    }
}

impl Error for OriginalFailure {}

struct FaultStream {
    input: Vec<u8>,
    offset: usize,
    kind: ErrorKind,
    fail_write: Arc<AtomicBool>,
    drops: Arc<AtomicUsize>,
}

impl FaultStream {
    fn new(input: Vec<u8>, kind: ErrorKind) -> Self {
        Self {
            input,
            offset: 0,
            kind,
            fail_write: Arc::new(AtomicBool::new(false)),
            drops: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl AsyncRead for FaultStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.offset == self.input.len() {
            return Poll::Ready(Err(io::Error::new(self.kind, OriginalFailure)));
        }
        let end = (self.offset + buf.remaining()).min(self.input.len());
        buf.put_slice(&self.input[self.offset..end]);
        self.offset = end;
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for FaultStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.fail_write.load(Ordering::SeqCst) {
            Poll::Ready(Err(io::Error::new(self.kind, OriginalFailure)))
        } else {
            Poll::Ready(Ok(buf.len()))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl Drop for FaultStream {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn frame(frame_type: u8, stream: u32, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, data.len() as u8, frame_type, 0];
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(data);
    out
}

fn wire(stream: u32, partial_body: bool) -> Vec<u8> {
    let mut out = frame(4, 0, &[]);
    if partial_body {
        // A complete XPC wrapper declares a body, but that body's DATA never
        // arrives. Force the native read failure after header acceptance.
        let mut header = [0; 24];
        header[8..16].copy_from_slice(&8_u64.to_le_bytes());
        out.extend_from_slice(&frame(0, stream, &header));
    }
    out
}

fn assert_native(error: &XpcError, kind: ErrorKind, class: ErrorClass, phase: &str) {
    // Preserve exhaustive matches against the existing public enum.
    match error {
        XpcError::Io(outer) => assert_eq!(outer.kind(), kind),
        XpcError::NotConnected | XpcError::ServiceNotFound(_) | XpcError::Tls(_) => {
            panic!("native source flattened")
        }
    }
    assert_eq!(classify_error(error), class);
    assert!(error.to_string().contains(phase));
    assert!(!format!("{error:?}").contains("synthetic-private"));
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let Some(source) = current else { break };
        if source.is::<OriginalFailure>() {
            return;
        }
        // std::io::Error exposes its custom payload through get_ref; source()
        // may skip that payload and return the payload's own source instead.
        if let Some(io) = source.downcast_ref::<io::Error>() {
            if io
                .get_ref()
                .is_some_and(|payload| payload.is::<OriginalFailure>())
            {
                assert_eq!(io.kind(), kind);
                return;
            }
        }
        current = source.source();
    }
    panic!("original transport source missing from error chain");
}

#[tokio::test]
async fn connect_stream_preserves_h2_read_write_and_protocol_failures() {
    for (kind, class, writing) in [
        (ErrorKind::TimedOut, ErrorClass::Timeout, false),
        (
            ErrorKind::PermissionDenied,
            ErrorClass::PermissionDenied,
            true,
        ),
    ] {
        let stream = FaultStream::new(Vec::new(), kind);
        stream.fail_write.store(writing, Ordering::SeqCst);
        let drops = Arc::clone(&stream.drops);
        let error = match XpcClient::connect_stream(stream).await {
            Err(error) => error,
            Ok(_) => panic!("faulting stream connected"),
        };
        assert_native(&error, kind, class, "H2: ");
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    let stream = FaultStream::new(frame(4, 0, &[0]), ErrorKind::TimedOut);
    let drops = Arc::clone(&stream.drops);
    let error = match XpcClient::connect_stream(stream).await {
        Err(error) => error,
        Ok(_) => panic!("malformed SETTINGS accepted"),
    };
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    let XpcError::Io(outer) = error else {
        panic!("typed protocol source lost")
    };
    assert_eq!(outer.kind(), ErrorKind::InvalidData);
    assert!(matches!(
        outer.source().unwrap().downcast_ref::<H2Error>(),
        Some(H2Error::Protocol(_))
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn rsd_reads_preserve_header_and_partial_body_transport_failures() {
    for partial_body in [false, true] {
        let stream = FaultStream::new(wire(1, partial_body), ErrorKind::TimedOut);
        let drops = Arc::clone(&stream.drops);
        let mut framer = H2Framer::connect(stream).await.unwrap();
        let error = handshake_on_framer(&mut framer).await.unwrap_err();
        assert_native(
            &error,
            ErrorKind::TimedOut,
            ErrorClass::Timeout,
            if partial_body {
                "read body: "
            } else {
                "read header: "
            },
        );
        drop(framer);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn bootstrap_and_send_preserve_native_write_failures() {
    for operation in 0..4 {
        let stream = FaultStream::new(wire(1, false), ErrorKind::BrokenPipe);
        let failing = Arc::clone(&stream.fail_write);
        let drops = Arc::clone(&stream.drops);
        let mut framer = H2Framer::connect(stream).await.unwrap();
        failing.store(true, Ordering::SeqCst);
        let (error, phase) = match operation {
            0 => (
                initialize_xpc_connection_on_framer(&mut framer)
                    .await
                    .unwrap_err(),
                "xpc init write 1: ",
            ),
            1 => (
                queue_rsd_handshake_bootstrap_on_framer(&mut framer)
                    .await
                    .unwrap_err(),
                "rsd bootstrap write 1: ",
            ),
            2 => (
                send_device_handshake_on_framer(&mut framer, uuid::Uuid::from_bytes([0x4a; 16]))
                    .await
                    .unwrap_err(),
                "RSD host handshake: ",
            ),
            _ => {
                let mut connection = XpcConnection::new(framer);
                let error = connection.send(XpcValue::Bool(true)).await.unwrap_err();
                assert_native(&error, ErrorKind::BrokenPipe, ErrorClass::Disconnected, "");
                drop(connection);
                assert_eq!(drops.load(Ordering::SeqCst), 1);
                continue;
            }
        };
        assert_native(
            &error,
            ErrorKind::BrokenPipe,
            ErrorClass::Disconnected,
            phase,
        );
        drop(framer);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn connection_receivers_keep_sources_and_drop_owned_transport() {
    for receiver in 0..3 {
        for partial_body in [false, true] {
            let stream_id = if receiver == 1 { 1 } else { 3 };
            let stream =
                FaultStream::new(wire(stream_id, partial_body), ErrorKind::PermissionDenied);
            let drops = Arc::clone(&stream.drops);
            let framer = H2Framer::connect(stream).await.unwrap();
            let mut connection = XpcConnection::new(framer);
            let result = match receiver {
                0 => connection.recv().await,
                1 => connection.recv_client_server().await,
                _ => connection.recv_any_stream().await,
            };
            assert_native(
                &result.unwrap_err(),
                ErrorKind::PermissionDenied,
                ErrorClass::PermissionDenied,
                "",
            );
            assert_eq!(connection.pending_memory_bytes_used(), 0);
            drop(connection);
            assert_eq!(drops.load(Ordering::SeqCst), 1);
        }
    }
}

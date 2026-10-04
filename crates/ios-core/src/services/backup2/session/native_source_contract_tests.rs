use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
};

use tokio::io::{DuplexStream, ReadBuf};

use super::*;
use crate::{
    backup2::{DeviceOperation, DeviceOperationTimeoutPhase},
    test_native::has_source,
    ErrorClass,
};

struct OwnedRoot(PathBuf);
impl Drop for OwnedRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("owned backup cleanup");
    }
}

struct GatedStream {
    stream: DuplexStream,
    blocked: Arc<AtomicBool>,
    blocked_writes: Arc<AtomicUsize>,
}
impl AsyncRead for GatedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for GatedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.blocked.load(Ordering::SeqCst) {
            self.blocked_writes.fetch_add(1, Ordering::SeqCst);
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

fn request(operation: DeviceOperation) -> DeviceOperationRequest<'static> {
    match operation {
        DeviceOperation::Unback => DeviceOperationRequest::Unback(UnbackRequest {
            message_name: "Unback",
            target_identifier: "owned-device",
            source_identifier: None,
            password: Some("owned-private-password"),
        }),
        DeviceOperation::Extract => DeviceOperationRequest::Extract(ExtractRequest {
            message_name: "Extract",
            target_identifier: "owned-device",
            domain_name: "HomeDomain",
            relative_path: "Library/owned.txt",
            source_identifier: None,
            password: Some("owned-private-password"),
        }),
    }
}

fn evidence(
    error: &Mobilebackup2Error,
    operation: DeviceOperation,
    phase: DeviceOperationTimeoutPhase,
    duration: Duration,
) -> &super::super::DeviceOperationTimeout {
    assert!(matches!(error, Mobilebackup2Error::Io(io) if io.kind() == ErrorKind::TimedOut));
    assert_eq!(crate::classify_error(error), ErrorClass::Timeout);
    assert!(has_source::<tokio::time::error::Elapsed>(error));
    let evidence = error.device_operation_timeout().expect("issued deadline");
    assert_eq!(evidence.operation(), operation);
    assert_eq!(evidence.phase(), phase);
    assert_eq!(evidence.duration(), duration);
    assert!(!format!("{error:?} {evidence:?}").contains("owned-private"));
    evidence
}

async fn operation_timeout(operation: DeviceOperation, duration: Duration) {
    let root = OwnedRoot(tests::test_backup_root("native-deadline", "owned-device"));
    let (stream, mut peer) = tokio::io::duplex(16 * 1024);
    let mut client = Mobilebackup2Client::new(stream);
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        client.run_device_operation(&root.0, request(operation), duration),
    )
    .await
    .expect("owned operation must finish")
    .unwrap_err();
    assert!(evidence(
        &error,
        operation,
        DeviceOperationTimeoutPhase::Operation,
        duration
    )
    .operation_error()
    .is_none());
    // No transport bytes were accepted; retain idle framing, but never send
    // disconnect after the total deadline (including the genuine zero timer).
    assert!(client.is_usable());
    assert!(
        tokio::time::timeout(Duration::from_millis(5), peer.read(&mut [0]))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn native_source_unback_operation_timeout() {
    operation_timeout(DeviceOperation::Unback, Duration::from_millis(10)).await;
}

#[tokio::test]
async fn native_source_extract_operation_timeout() {
    operation_timeout(DeviceOperation::Extract, Duration::from_millis(10)).await;
}

#[tokio::test]
async fn native_source_zero_budget_retains_real_elapsed() {
    operation_timeout(DeviceOperation::Unback, Duration::ZERO).await;
}

#[derive(Clone, Copy)]
enum Completion {
    Protocol,
    Eof,
    Success,
    Pending,
}

async fn disconnect_timeout(operation: DeviceOperation, completion: Completion) {
    let root = OwnedRoot(tests::test_backup_root("native-disconnect", "owned-device"));
    let (stream, mut peer) = tokio::io::duplex(16 * 1024);
    let blocked = Arc::new(AtomicBool::new(false));
    let blocked_writes = Arc::new(AtomicUsize::new(0));
    let mut client = Mobilebackup2Client::new(GatedStream {
        stream,
        blocked: Arc::clone(&blocked),
        blocked_writes: Arc::clone(&blocked_writes),
    });
    let duration = Duration::from_millis(250);
    let run = client.run_device_operation(&root.0, request(operation), duration);
    let remote = async {
        tests::complete_device_link_handshake(&mut peer).await;
        let frame = tests::read_test_frame(&mut peer).await;
        assert_eq!(
            frame.as_array().unwrap()[1]
                .as_dictionary()
                .unwrap()
                .get("MessageName")
                .and_then(plist::Value::as_string),
            Some(match operation {
                DeviceOperation::Unback => "Unback",
                DeviceOperation::Extract => "Extract",
            })
        );
        blocked.store(true, Ordering::SeqCst);
        match completion {
            Completion::Protocol => tests::complete_process_message(&mut peer, 77, None).await,
            Completion::Success => {
                tests::complete_process_message(
                    &mut peer,
                    0,
                    Some(plist::Value::String("owned-success".into())),
                )
                .await;
            }
            Completion::Eof => return None,
            Completion::Pending => {}
        }
        // Hold the peer until the local operation returns; no detached tasks.
        Some(peer)
    };
    let (result, _held_peer) =
        tokio::time::timeout(Duration::from_secs(3), async { tokio::join!(run, remote) })
            .await
            .expect("owned peers must finish");
    let error = result.unwrap_err();
    if matches!(completion, Completion::Eof) {
        // Native EOF invalidates DeviceLink before cleanup. Keep the original
        // failure directly and never try another write on that connection.
        assert_eq!(blocked_writes.load(Ordering::SeqCst), 0);
        assert!(!client.device_link.is_usable());
        assert!(has_source::<io::Error>(&error));
        assert!(
            matches!(&error, Mobilebackup2Error::DeviceLink(DeviceLinkError::Io(io)) if io.kind() == ErrorKind::UnexpectedEof)
        );
        assert!(error.device_operation_timeout().is_none());
        return;
    }
    if matches!(completion, Completion::Pending) {
        // A complete request is still owned during an empty receive wait.
        // Its expired operation rejects cleanup instead of issuing a second
        // timer while attempting another message on that abandoned session.
        assert_eq!(blocked_writes.load(Ordering::SeqCst), 0);
        assert!(!client.is_usable());
        assert!(evidence(
            &error,
            operation,
            DeviceOperationTimeoutPhase::Operation,
            duration,
        )
        .operation_error()
        .is_none());
        return;
    }
    assert!(blocked_writes.load(Ordering::SeqCst) > 0);
    let timeout = evidence(
        &error,
        operation,
        DeviceOperationTimeoutPhase::Disconnect,
        duration,
    );
    match completion {
        Completion::Protocol => {
            let original = timeout.operation_error().expect("prior protocol failure");
            assert!(
                matches!(original, Mobilebackup2Error::Protocol(message) if message.contains("ErrorCode=77"))
            );
            assert_eq!(crate::classify_error(original), ErrorClass::Protocol);
            assert!(original.device_operation_timeout().is_none());
        }
        Completion::Eof => unreachable!("EOF returns its original failure without cleanup I/O"),
        Completion::Success => assert!(timeout.operation_error().is_none()),
        Completion::Pending => unreachable!("expired sent operation performs no cleanup I/O"),
    }
}

#[tokio::test]
async fn native_source_unback_disconnect_retains_protocol_failure() {
    disconnect_timeout(DeviceOperation::Unback, Completion::Protocol).await;
}

#[tokio::test]
async fn native_source_extract_disconnect_retains_protocol_failure() {
    disconnect_timeout(DeviceOperation::Extract, Completion::Protocol).await;
}

#[tokio::test]
async fn native_source_disconnect_retains_native_eof() {
    disconnect_timeout(DeviceOperation::Unback, Completion::Eof).await;
}

#[tokio::test]
async fn native_source_success_then_disconnect_timeout_has_no_prior_error() {
    disconnect_timeout(DeviceOperation::Extract, Completion::Success).await;
}

#[tokio::test]
async fn native_source_sent_operation_timeout_keeps_elapsed_without_disconnect() {
    disconnect_timeout(DeviceOperation::Extract, Completion::Pending).await;
}

//! Actual raw forwarding and reader task outcomes, with a local async stream.
use super::*;
use std::error::Error;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{DuplexStream, ReadBuf};
use tokio::sync::Notify;

const OBSERVE: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct ReaderPanic;

#[derive(Debug)]
struct WorkCause(Arc<()>);
impl std::fmt::Display for WorkCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("synthetic-private-forward-work")
    }
}
impl Error for WorkCause {}

struct PanickingStream {
    stream: DuplexStream,
    packet: Option<[u8; 40]>,
    panicked: Arc<Notify>,
    dropped: Arc<AtomicUsize>,
}
impl AsyncRead for PanickingStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if let Some(packet) = self.packet.take() {
            output.put_slice(&packet);
            Poll::Ready(Ok(()))
        } else {
            self.panicked.notify_one();
            std::panic::panic_any(ReaderPanic)
        }
    }
}
impl AsyncWrite for PanickingStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
impl Drop for PanickingStream {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

struct FaultTun {
    panicked: Arc<Notify>,
    work: Mutex<Option<TunnelError>>,
}
impl TunDevice for FaultTun {
    async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        std::future::pending().await
    }
    async fn write_packet(&mut self, _: &[u8]) -> Result<(), TunnelError> {
        self.panicked.notified().await;
        Err(self
            .work
            .lock()
            .unwrap()
            .take()
            .expect("one forwarding write"))
    }
}

fn find<'a, E: Error + 'static>(error: &'a (dyn Error + 'static)) -> Option<&'a E> {
    let mut current = Some(error);
    for _ in 0..32 {
        let cause = current?;
        if let Some(found) = cause.downcast_ref::<E>() {
            return Some(found);
        }
        current = if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            io.get_ref()
                .map(|cause| cause as &(dyn Error + 'static))
                .or_else(|| io.source())
        } else {
            cause.source()
        };
    }
    None
}

async fn exercise(work_fault: bool) -> (Result<(), TunnelError>, Arc<()>, bool) {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (stream, mut peer) = tokio::io::duplex(1280);
    let panicked = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicUsize::new(0));
    let identity = Arc::new(());
    let mut packet = [0; 40];
    packet[0] = 0x60;
    let stream = PanickingStream {
        stream,
        packet: work_fault.then_some(packet),
        panicked: panicked.clone(),
        dropped: dropped.clone(),
    };
    let tun = FaultTun {
        panicked,
        work: Mutex::new(Some(TunnelError::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            WorkCause(identity.clone()),
        )))),
    };
    let (_cancel, receiver) = watch::channel(());
    let result = forward_packets(stream, tun, 1280, receiver).await;
    let recovered = tokio::time::timeout(OBSERVE, async {
        let eof = peer
            .read_u8()
            .await
            .is_err_and(|error| error.kind() == std::io::ErrorKind::UnexpectedEof);
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
        eof && dropped.load(Ordering::SeqCst) == 1
    })
    .await
    .unwrap_or(false);
    (result, identity, recovered)
}

#[tokio::test]
async fn native_raw_forwarding_retains_both_work_io_and_original_reader_panic_after_recovery() {
    let (result, identity, recovered) = exercise(true).await;
    assert!(recovered);
    let error = result.unwrap_err();
    let dual = find::<ForwardReaderCleanup>(&error).expect("dual native outcome missing");
    let TunnelError::Io(io) = dual.primary() else {
        panic!("original forwarding IO missing")
    };
    assert_eq!(io.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(Arc::ptr_eq(
        &io.get_ref().unwrap().downcast_ref::<WorkCause>().unwrap().0,
        &identity
    ));
    assert!(dual.cleanup().is_panic() && !dual.cleanup().is_cancelled());
    assert!(std::ptr::eq(
        dual.source()
            .unwrap()
            .downcast_ref::<TunnelError>()
            .unwrap(),
        dual.primary()
    ));
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::Disconnected
    );
    assert!(!format!("{dual:?} {dual}").contains("synthetic-private"));
}

#[tokio::test]
async fn native_raw_forwarding_preserves_original_reader_join_panic_when_forwarding_ends_cleanly() {
    let (result, _, recovered) = exercise(false).await;
    assert!(recovered);
    let error = result.unwrap_err();
    assert!(find::<ForwardReaderCleanup>(&error).is_none());
    let native =
        find::<tokio::task::JoinError>(&error).expect("native reader join failure missing");
    assert!(native.is_panic() && !native.is_cancelled());
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Unknown);
}

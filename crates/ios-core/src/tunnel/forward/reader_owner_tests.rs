//! Caller cancellation must release the reader's actual in-memory stream.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio::sync::{watch, Notify};

use super::forward_packets;
use crate::tunnel::tun::TunDevice;
use crate::tunnel::TunnelError;

const OBSERVATION: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Probe {
    read_bytes: AtomicUsize,
    write_bytes: AtomicUsize,
    drops: AtomicUsize,
    read_pending: Notify,
    write_pending: Notify,
    flush_pending: Notify,
    released: Notify,
}

struct TrackedStream {
    stream: DuplexStream,
    probe: Arc<Probe>,
    stall_flush: bool,
}

impl AsyncRead for TrackedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let result = Pin::new(&mut this.stream).poll_read(cx, buf);
        this.probe
            .read_bytes
            .fetch_add(buf.filled().len() - before, Ordering::SeqCst);
        if result.is_pending() {
            this.probe.read_pending.notify_one();
        }
        result
    }
}

impl AsyncWrite for TrackedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.stream).poll_write(cx, bytes);
        match &result {
            Poll::Ready(Ok(size)) => {
                this.probe.write_bytes.fetch_add(*size, Ordering::SeqCst);
            }
            Poll::Pending => this.probe.write_pending.notify_one(),
            _ => {}
        }
        result
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.stall_flush {
            // Duplex flush is always ready; this is a controlled transport wait.
            this.probe.flush_pending.notify_one();
            Poll::Pending
        } else {
            Pin::new(&mut this.stream).poll_flush(cx)
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

impl Drop for TrackedStream {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
        self.probe.released.notify_one();
    }
}

enum Tun {
    Idle,
    Output(Option<Vec<u8>>),
    WritePending(Arc<Notify>),
    WritePanic,
}

#[derive(Debug)]
struct TunPanic;

impl TunDevice for Tun {
    async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        if let Self::Output(packet) = self {
            if let Some(packet) = packet.take() {
                return Ok(packet);
            }
        }
        std::future::pending().await
    }

    async fn write_packet(&mut self, _packet: &[u8]) -> Result<(), TunnelError> {
        match self {
            Self::WritePending(entered) => {
                entered.notify_one();
                std::future::pending().await
            }
            Self::WritePanic => std::panic::panic_any(TunPanic),
            _ => Ok(()),
        }
    }
}

fn packet() -> Vec<u8> {
    let mut packet = vec![0; 40];
    packet[0] = 0x60;
    packet
}

fn tracked(capacity: usize, stall_flush: bool) -> (DuplexStream, TrackedStream, Arc<Probe>) {
    let (peer, stream) = tokio::io::duplex(capacity);
    let probe = Arc::new(Probe::default());
    (
        peer,
        TrackedStream {
            stream,
            probe: probe.clone(),
            stall_flush,
        },
        probe,
    )
}

async fn checkpoint<F: Future<Output = Result<(), TunnelError>>>(
    work: &mut Pin<Box<F>>,
    entered: &Notify,
) {
    tokio::time::timeout(OBSERVATION, async {
        tokio::select! {
            biased;
            result = work => panic!("forwarding ended before checkpoint: {result:?}"),
            _ = entered.notified() => {}
        }
    })
    .await
    .expect("forwarding never reached its actual wait");
}

struct Observation {
    released: bool,
    eof: bool,
    retained_tasks: usize,
    received: Vec<u8>,
}

async fn observe_then_recover(
    mut peer: DuplexStream,
    probe: &Probe,
    baseline: usize,
) -> Observation {
    let released = tokio::time::timeout(OBSERVATION, async {
        while probe.drops.load(Ordering::SeqCst) == 0 {
            probe.released.notified().await;
        }
    })
    .await
    .is_ok();
    let mut received = Vec::new();
    let eof = matches!(
        tokio::time::timeout(OBSERVATION, peer.read_to_end(&mut received)).await,
        Ok(Ok(_))
    );
    let retained_tasks = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
        .saturating_sub(baseline);

    // A negative control retains the reader. Release its pending read and wait
    // for actual stream/task recovery before making the regression assertions.
    drop(peer);
    tokio::time::timeout(OBSERVATION, async {
        loop {
            if probe.drops.load(Ordering::SeqCst) == 1
                && tokio::runtime::Handle::current()
                    .metrics()
                    .num_alive_tasks()
                    == baseline
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reader did not recover after releasing its peer");
    Observation {
        released,
        eof,
        retained_tasks,
        received,
    }
}

fn assert_released(observed: Observation, probe: &Probe) {
    assert!(
        observed.released,
        "reader still owns the dropped caller's stream"
    );
    assert!(observed.eof, "reader prevented peer EOF");
    assert_eq!(observed.retained_tasks, 0);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn forward_reader_owner_dropped_future_releases_partial_header() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()[..20]).await.unwrap();
    let (_cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(stream, Tun::Idle, 1280, receiver));
    checkpoint(&mut work, &probe.read_pending).await;
    let consumed = probe.read_bytes.load(Ordering::SeqCst);
    drop(work);
    let observed = observe_then_recover(peer, &probe, baseline).await;
    assert_eq!(consumed, 20);
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_reader_owner_dropped_future_releases_during_tun_write() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()).await.unwrap();
    let entered = Arc::new(Notify::new());
    let (_cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::WritePending(entered.clone()),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &entered).await;
    let consumed = probe.read_bytes.load(Ordering::SeqCst);
    drop(work);
    let observed = observe_then_recover(peer, &probe, baseline).await;
    assert_eq!(consumed, 40);
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_reader_owner_dropped_future_releases_during_stream_write() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (peer, stream, probe) = tracked(1, false);
    let (_cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::Output(Some(packet())),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &probe.write_pending).await;
    let written = probe.write_bytes.load(Ordering::SeqCst);
    drop(work);
    let observed = observe_then_recover(peer, &probe, baseline).await;
    assert_eq!(written, 1);
    assert_eq!(observed.received, [0x60]);
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_reader_owner_dropped_future_releases_during_controlled_flush() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (peer, stream, probe) = tracked(128, true);
    let (_cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::Output(Some(packet())),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &probe.flush_pending).await;
    let written = probe.write_bytes.load(Ordering::SeqCst);
    drop(work);
    let observed = observe_then_recover(peer, &probe, baseline).await;
    assert_eq!(written, 40);
    assert_eq!(observed.received, packet());
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_reader_owner_parent_task_abort_releases_reader() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()[..20]).await.unwrap();
    let (_cancel, receiver) = watch::channel(());
    let task = tokio::spawn(forward_packets(stream, Tun::Idle, 1280, receiver));
    let reached = tokio::time::timeout(OBSERVATION, probe.read_pending.notified())
        .await
        .is_ok();
    task.abort();
    let result = task.await;
    let observed = observe_then_recover(peer, &probe, baseline).await;
    assert!(reached);
    assert!(result.unwrap_err().is_cancelled());
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_reader_owner_parent_task_unwind_releases_reader() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()).await.unwrap();
    let (_cancel, receiver) = watch::channel(());
    let mut task = tokio::spawn(forward_packets(stream, Tun::WritePanic, 1280, receiver));
    let result = tokio::time::timeout(OBSERVATION, &mut task).await;
    // Recover the parent before any assertion, including on a failed checkpoint.
    let joined = match result {
        Ok(joined) => joined,
        Err(_) => {
            task.abort();
            task.await
        }
    };
    let observed = observe_then_recover(peer, &probe, baseline).await;
    let error = joined.unwrap_err();
    assert!(error.is_panic());
    assert!(error.into_panic().is::<TunPanic>());
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 40);
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_reader_owner_normal_watch_close_keeps_success_and_releases_reader() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()[..20]).await.unwrap();
    let (cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(stream, Tun::Idle, 1280, receiver));
    checkpoint(&mut work, &probe.read_pending).await;
    drop(cancel);
    let result = tokio::time::timeout(OBSERVATION, &mut work).await;
    drop(work);
    let observed = observe_then_recover(peer, &probe, baseline).await;
    result.unwrap().unwrap();
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 20);
    assert_released(observed, &probe);
}

// Save watch-completion evidence before dropping work for fixture recovery.
// An old blocked branch must fail even though caller drop then releases it.
async fn finish_watch<F: Future<Output = Result<(), TunnelError>>>(
    mut work: Pin<Box<F>>,
    peer: DuplexStream,
    probe: &Probe,
    baseline: usize,
) -> (
    Result<Result<(), TunnelError>, tokio::time::error::Elapsed>,
    Observation,
) {
    let result = tokio::time::timeout(OBSERVATION, &mut work).await;
    drop(work);
    let observed = observe_then_recover(peer, probe, baseline).await;
    (result, observed)
}

#[tokio::test]
async fn forward_watch_shutdown_closes_pending_tun_write() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()).await.unwrap();
    let entered = Arc::new(Notify::new());
    let (cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::WritePending(entered.clone()),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &entered).await;
    drop(cancel);
    let (result, observed) = finish_watch(work, peer, &probe, baseline).await;
    result
        .expect("watch close did not end pending TUN write")
        .unwrap();
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 40);
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_watch_shutdown_closes_pending_stream_write() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (peer, stream, probe) = tracked(1, false);
    let (cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::Output(Some(packet())),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &probe.write_pending).await;
    drop(cancel);
    let (result, observed) = finish_watch(work, peer, &probe, baseline).await;
    result
        .expect("watch close did not end pending stream write")
        .unwrap();
    assert_eq!(probe.write_bytes.load(Ordering::SeqCst), 1);
    assert_eq!(observed.received, [0x60]);
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_watch_shutdown_closes_controlled_flush() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (peer, stream, probe) = tracked(128, true);
    let (cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::Output(Some(packet())),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &probe.flush_pending).await;
    drop(cancel);
    let (result, observed) = finish_watch(work, peer, &probe, baseline).await;
    result
        .expect("watch close did not end pending flush")
        .unwrap();
    assert_eq!(probe.write_bytes.load(Ordering::SeqCst), 40);
    assert_eq!(observed.received, packet());
    assert_released(observed, &probe);
}

#[tokio::test]
async fn forward_watch_shutdown_changed_event_closes_pending_tun_write() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()).await.unwrap();
    let entered = Arc::new(Notify::new());
    let (cancel, receiver) = watch::channel(());
    let mut work = Box::pin(forward_packets(
        stream,
        Tun::WritePending(entered.clone()),
        1280,
        receiver,
    ));
    checkpoint(&mut work, &entered).await;
    let receivers_before = cancel.receiver_count();
    let sent = cancel.send(());
    let (result, observed) = finish_watch(work, peer, &probe, baseline).await;
    sent.unwrap();
    result
        .expect("watch event did not end pending TUN write")
        .unwrap();
    assert_eq!(receivers_before, 1);
    assert_eq!(cancel.receiver_count(), 0);
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 40);
    assert_released(observed, &probe);
}

#[derive(thiserror::Error)]
#[error("controlled forwarding IO failure")]
struct ForwardingFault;

impl std::fmt::Debug for ForwardingFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ForwardingFault")
    }
}

struct CloseOnRead {
    cancel: Option<watch::Sender<()>>,
    reads: Arc<AtomicUsize>,
}

impl TunDevice for CloseOnRead {
    async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = self.cancel.take() {
            // Everything in this step is Ready. The next step must recheck the
            // watch rather than looping through another source read in one poll.
            drop(cancel);
            Ok(packet())
        } else {
            // Make a wrong whole-loop outer-select control terminate safely.
            Err(TunnelError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                ForwardingFault,
            )))
        }
    }

    async fn write_packet(&mut self, _packet: &[u8]) -> Result<(), TunnelError> {
        Ok(())
    }
}

#[tokio::test]
async fn forward_watch_shutdown_checks_each_ready_iteration() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (peer, stream, probe) = tracked(128, false);
    let (cancel, receiver) = watch::channel(());
    let reads = Arc::new(AtomicUsize::new(0));
    let work = Box::pin(forward_packets(
        stream,
        CloseOnRead {
            cancel: Some(cancel),
            reads: reads.clone(),
        },
        1280,
        receiver,
    ));
    let (result, observed) = finish_watch(work, peer, &probe, baseline).await;
    result.unwrap().unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(probe.write_bytes.load(Ordering::SeqCst), 40);
    assert_eq!(observed.received, packet());
    assert_released(observed, &probe);
}

struct FailOnWrite {
    cancel: Option<watch::Sender<()>>,
}

impl TunDevice for FailOnWrite {
    async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
        std::future::pending().await
    }

    async fn write_packet(&mut self, _packet: &[u8]) -> Result<(), TunnelError> {
        // This error was obtained within the same poll that closed the watch.
        // Do not replace the acquired primary with a successful cancellation.
        drop(self.cancel.take());
        Err(TunnelError::Io(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            ForwardingFault,
        )))
    }
}

#[tokio::test]
async fn forward_watch_shutdown_keeps_io_obtained_in_closing_poll() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (mut peer, stream, probe) = tracked(128, false);
    peer.write_all(&packet()).await.unwrap();
    let (cancel, receiver) = watch::channel(());
    let work = Box::pin(forward_packets(
        stream,
        FailOnWrite {
            cancel: Some(cancel),
        },
        1280,
        receiver,
    ));
    let (result, observed) = finish_watch(work, peer, &probe, baseline).await;
    let error = result.unwrap().unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::Disconnected
    );
    let TunnelError::Io(io) = error else {
        panic!("acquired forwarding IO was replaced");
    };
    assert_eq!(io.kind(), std::io::ErrorKind::BrokenPipe);
    assert!(io.get_ref().unwrap().is::<ForwardingFault>());
    assert_eq!(probe.read_bytes.load(Ordering::SeqCst), 40);
    assert_released(observed, &probe);
}

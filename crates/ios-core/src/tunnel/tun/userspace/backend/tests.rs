use super::super::*;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

const TOTAL: usize = 16 * 1024 * 1024;
const OBSERVE: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Traffic {
    read: AtomicUsize,
    written: AtomicUsize,
    drops: AtomicUsize,
    block_flush: AtomicBool,
    flush_ready: AtomicBool,
    flush_attempts: AtomicUsize,
    flush_done: AtomicUsize,
    flush_waker: std::sync::Mutex<Option<std::task::Waker>>,
    flush_error: std::sync::Mutex<Option<std::io::Error>>,
}
struct Original {
    stream: DuplexStream,
    traffic: Arc<Traffic>,
}
impl AsyncRead for Original {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buffer);
        if matches!(&result, Poll::Ready(Ok(()))) {
            self.traffic
                .read
                .fetch_add(buffer.filled().len() - before, Ordering::SeqCst);
        }
        result
    }
}
impl AsyncWrite for Original {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(n)) = &result {
            self.traffic.written.fetch_add(*n, Ordering::SeqCst);
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.traffic.flush_attempts.fetch_add(1, Ordering::SeqCst);
        let error = { self.traffic.flush_error.lock().unwrap().take() };
        if let Some(error) = error {
            return Poll::Ready(Err(error));
        }
        if self.traffic.block_flush.load(Ordering::SeqCst)
            && !self.traffic.flush_ready.load(Ordering::SeqCst)
        {
            *self.traffic.flush_waker.lock().unwrap() = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let result = Pin::new(&mut self.stream).poll_flush(cx);
        if matches!(&result, Poll::Ready(Ok(()))) {
            self.traffic.flush_done.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[derive(Debug)]
struct OriginalFlush {
    io: std::io::Error,
    drops: Arc<AtomicUsize>,
}
impl std::fmt::Display for OriginalFlush {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("scripted flush carries original localhost refusal")
    }
}
impl std::error::Error for OriginalFlush {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.io)
    }
}
impl Drop for OriginalFlush {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn failure(error: &TunnelError) -> &owner::TunnelShutdownFailure {
    let mut cause: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(report) = cause.downcast_ref::<owner::TunnelShutdownFailure>() {
            return report;
        }
        cause = cause.source().expect("original barrier native report");
    }
}

#[tokio::test]
async fn flush_failure_keeps_original_deep_native_io_and_actual_joins_without_ack_or_late_quota() {
    let reserved = tokio::net::TcpSocket::new_v4().unwrap();
    reserved.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = reserved.local_addr().unwrap();
    #[cfg(target_os = "macos")]
    drop(reserved);
    let io = tokio::net::TcpStream::connect(addr).await.unwrap_err();
    assert_eq!(io.kind(), std::io::ErrorKind::ConnectionRefused);
    let error_drops = Arc::new(AtomicUsize::new(0));
    let error = std::io::Error::new(
        io.kind(),
        OriginalFlush {
            io,
            drops: error_drops.clone(),
        },
    );
    let pointer = &error
        .get_ref()
        .unwrap()
        .downcast_ref::<OriginalFlush>()
        .unwrap()
        .io as *const std::io::Error as usize;
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(8);
    let traffic = Arc::new(Traffic::default());
    *traffic.flush_error.lock().unwrap() = Some(error);
    let runtime = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        Original {
            stream,
            traffic: traffic.clone(),
        },
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let result = tokio::time::timeout(OBSERVE, runtime.quiesce_and_flush()).await;
    let cleanup = runtime.shutdown(OBSERVE).await;
    let eof = tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await;
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let first = result
        .unwrap()
        .err()
        .expect("native flush error is not an ACK");
    let cleanup = cleanup.unwrap_err();
    for report in [failure(&first), failure(&cleanup)] {
        let original = report.primary().downcast_ref::<TunnelError>().unwrap();
        let TunnelError::Io(io) = original else {
            panic!("original flush IO variant")
        };
        let original = &io
            .get_ref()
            .unwrap()
            .downcast_ref::<OriginalFlush>()
            .unwrap()
            .io;
        assert_eq!(original as *const std::io::Error as usize, pointer);
        assert_eq!(original.kind(), std::io::ErrorKind::ConnectionRefused);
    }
    assert!(failure(&cleanup).joins_complete());
    assert_eq!(traffic.flush_done.load(Ordering::SeqCst), 0);
    assert!(matches!(eof, Ok(Ok(0))));
    assert_eq!(traffic.drops.load(Ordering::SeqCst), 1);
    assert_eq!(error_drops.load(Ordering::SeqCst), 0);
    drop((first, cleanup, held));
    assert_eq!(bank.available_permits(), TOTAL - runtime_owner_bytes());
    drop(runtime);
    assert_eq!(error_drops.load(Ordering::SeqCst), 1);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn same_fifo_barrier_wait_survives_cancel_before_and_during_original_flush_without_duplicate()
{
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(8);
    let traffic = Arc::new(Traffic::default());
    traffic.block_flush.store(true, Ordering::SeqCst);
    let runtime = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        Original {
            stream,
            traffic: traffic.clone(),
        },
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let successor = packet(b"final backend begins after the actual local flush");
    let mut paid = OwnedBuffer::zeroed(successor.len(), Some(&bank)).unwrap();
    paid.copy_from_slice(&successor);
    let mut client =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    let mut preamble = [0; LOCAL_PROXY_PREAMBLE_LEN];
    preamble[..16].copy_from_slice(&"fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets());
    preamble[16..].copy_from_slice(&58783u32.to_le_bytes());
    client.write_all(&preamble).await.unwrap();
    tokio::time::timeout(OBSERVE, async {
        while traffic.written.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let observed = tokio::time::timeout(OBSERVE, async {
        let mut first = Box::pin(runtime.quiesce_and_flush());
        let mut pending_before_write = true;
        loop {
            if futures_util::poll!(first.as_mut()).is_ready() {
                pending_before_write = false;
                break;
            }
            if matches!(
                runtime.backend.0.state.lock().unwrap().port,
                super::PortState::Leased
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
        let no_early_flush = traffic.flush_attempts.load(Ordering::SeqCst) == 0
            && traffic.written.load(Ordering::SeqCst) < IPV6_HEADER_LEN;
        drop(first);
        let mut header = [0; IPV6_HEADER_LEN];
        peer.read_exact(&mut header).await.unwrap();
        let mut body = vec![0; u16::from_be_bytes([header[4], header[5]]) as usize];
        peer.read_exact(&mut body).await.unwrap();
        while traffic.flush_attempts.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let mut second = Box::pin(runtime.quiesce_and_flush());
        let pending_during_flush = futures_util::poll!(second.as_mut()).is_pending();
        drop(second);
        let mut third = Box::pin(runtime.quiesce_and_flush());
        let still_pending = futures_util::poll!(third.as_mut()).is_pending()
            && traffic.flush_done.load(Ordering::SeqCst) == 0
            && !runtime.task_handles.get(1).unwrap().is_finished()
            && traffic.drops.load(Ordering::SeqCst) == 0;
        traffic.flush_ready.store(true, Ordering::SeqCst);
        let wake = { traffic.flush_waker.lock().unwrap().take() };
        if let Some(wake) = wake {
            wake.wake();
        }
        let lease = third.as_mut().await.unwrap();
        drop(third);
        let mut lease = lease;
        lease.port().outgoing.send(paid).await.unwrap();
        let mut received = vec![0; successor.len()];
        peer.read_exact(&mut received).await.unwrap();
        drop(lease);
        (
            pending_before_write,
            no_early_flush,
            pending_during_flush,
            still_pending,
            received == successor,
            traffic.flush_done.load(Ordering::SeqCst),
        )
    })
    .await;
    let joined = runtime.shutdown(OBSERVE).await;
    let eof = tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await;
    let client_eof = tokio::time::timeout(OBSERVE, client.read(&mut [0; 1])).await;
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    joined.unwrap();
    runtime.shutdown_status().unwrap();
    assert_eq!(observed.unwrap(), (true, true, true, true, true, 1));
    assert!(matches!(eof, Ok(Ok(0))));
    assert!(matches!(client_eof, Ok(Ok(0))));
    assert_eq!(traffic.drops.load(Ordering::SeqCst), 1);
    drop(held);
    assert_eq!(bank.available_permits(), TOTAL - runtime_owner_bytes());
    drop(runtime);
    assert_eq!(bank.available_permits(), TOTAL);
}
impl Drop for Original {
    fn drop(&mut self) {
        self.traffic.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn packet(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; IPV6_HEADER_LEN + payload.len()];
    bytes[0] = 0x60;
    bytes[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    bytes[IPV6_HEADER_LEN..].copy_from_slice(payload);
    bytes
}

#[tokio::test]
async fn actual_private_joins_return_same_port_while_original_partial_transport_survives_closed_zero(
) {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(8);
    let traffic = Arc::new(Traffic::default());
    let runtime = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        Original {
            stream,
            traffic: traffic.clone(),
        },
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let reader = runtime.task_handles.get(0).unwrap() as *const OwnedTask as usize;
    let writer = runtime.task_handles.get(1).unwrap() as *const OwnedTask as usize;
    let inbound = packet(b"original partially read payload crosses private handoff");
    let outbound = packet(b"successor packet follows original partial SYN");
    let mut paid_outbound = OwnedBuffer::zeroed(outbound.len(), Some(&bank)).unwrap();
    paid_outbound.copy_from_slice(&outbound);
    // The whole ingress allocation exists before closing the byte pool. Its
    // original read_exact has already consumed two payload bytes and stays live.
    peer.write_all(&inbound[..IPV6_HEADER_LEN + 2])
        .await
        .unwrap();
    tokio::time::timeout(OBSERVE, async {
        while traffic.read.load(Ordering::SeqCst) != IPV6_HEADER_LEN + 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut client =
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, runtime.local_port))
            .await
            .unwrap();
    let mut preamble = [0; LOCAL_PROXY_PREAMBLE_LEN];
    preamble[..16].copy_from_slice(&"fd00::1".parse::<std::net::Ipv6Addr>().unwrap().octets());
    preamble[16..].copy_from_slice(&58783u32.to_le_bytes());
    client.write_all(&preamble).await.unwrap();
    tokio::time::timeout(OBSERVE, async {
        while traffic.written.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let partial_write = traffic.written.load(Ordering::SeqCst) < IPV6_HEADER_LEN;
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let observed = tokio::time::timeout(OBSERVE, async {
        let first = runtime.quiesce_backend().await.unwrap();
        let still_original = reader
            == runtime.task_handles.get(0).unwrap() as *const OwnedTask as usize
            && writer == runtime.task_handles.get(1).unwrap() as *const OwnedTask as usize
            && !runtime.task_handles.get(0).unwrap().is_finished()
            && !runtime.task_handles.get(1).unwrap().is_finished()
            && runtime.transport_is_alive()
            && !runtime.is_alive()
            && traffic.drops.load(Ordering::SeqCst) == 0;
        drop(first);
        // A cancelled owner parks the original receiver; a retry takes that
        // same port after collecting the same already-completed private joins.
        let mut second = runtime.quiesce_backend().await.unwrap();
        peer.write_all(&inbound[IPV6_HEADER_LEN + 2..])
            .await
            .unwrap();
        let received = second.port().incoming.recv().await.unwrap();
        let intact = received.as_ref() == inbound.as_slice();
        drop(received);
        second.port().outgoing.send(paid_outbound).await.unwrap();
        let mut header = [0; IPV6_HEADER_LEN];
        peer.read_exact(&mut header).await.unwrap();
        let mut payload = vec![0; u16::from_be_bytes([header[4], header[5]]) as usize];
        peer.read_exact(&mut payload).await.unwrap();
        let original_syn = header[0] >> 4 == 6 && header[6] == 6;
        let mut next = vec![0; outbound.len()];
        peer.read_exact(&mut next).await.unwrap();
        drop(second);
        (still_original, intact, original_syn, next == outbound)
    })
    .await;
    // Even observation timeout must end with the actual original joins.
    let joined = runtime.shutdown(OBSERVE).await;
    let peer_eof = tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await;
    let client_eof = tokio::time::timeout(OBSERVE, client.read(&mut [0; 1])).await;
    tokio::time::timeout(OBSERVE, async {
        while tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks()
            != baseline
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    joined.unwrap();
    runtime.shutdown_status().unwrap();
    assert!(partial_write);
    assert_eq!(observed.unwrap(), (true, true, true, true));
    assert!(matches!(peer_eof, Ok(Ok(0))));
    assert!(matches!(client_eof, Ok(Ok(0))));
    assert_eq!(traffic.drops.load(Ordering::SeqCst), 1);
    drop(held);
    assert_eq!(bank.available_permits(), TOTAL - runtime_owner_bytes());
    drop(runtime);
    assert_eq!(bank.available_permits(), TOTAL);
}

#[tokio::test]
async fn collecting_original_joins_keeps_parked_port_until_explicit_final_release() {
    let bank = Arc::new(Semaphore::new(TOTAL));
    let (stream, mut peer) = tokio::io::duplex(1280);
    let runtime = UserspaceTunDevice::start_validated_with_bank(
        "fd00::2",
        "fd00::1",
        ValidatedMtu::new(1280).unwrap(),
        stream,
        Some(bank.clone()),
    )
    .await
    .unwrap();
    let lease = runtime.quiesce_and_flush().await.unwrap();
    assert!(runtime.transport_is_alive());
    assert!(!runtime.is_alive());
    drop(lease);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    runtime.request_stop();
    tokio::time::timeout(OBSERVE, runtime.collect_original())
        .await
        .unwrap();
    runtime.shutdown_status().unwrap();
    assert!(!runtime.transport_is_alive());
    // Actual joins alone cannot close a mailbox still needed by native undo.
    let parked = runtime.backend.lease().unwrap();
    assert!(runtime.backend.lease().is_err());
    drop(parked);
    assert!(matches!(
        tokio::time::timeout(OBSERVE, peer.read(&mut [0; 1])).await,
        Ok(Ok(0))
    ));
    runtime.release_original();
    assert!(runtime.backend.lease().is_err());
    runtime.release_original();
    runtime.shutdown_status().unwrap();
    assert!(bank.is_closed());
    drop((runtime, held));
    assert_eq!(bank.available_permits(), TOTAL);
}

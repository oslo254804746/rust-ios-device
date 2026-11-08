//! Actual packet reader, duplex transport, queue, RxToken and typed task joins.
//! These fixtures never create a listener, physical device or native TUN.

use super::*;
use std::error::Error;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::task::{Context, Poll};

use smoltcp::phy::RxToken;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

const OBSERVE: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Probe {
    bytes_read: AtomicUsize,
    read_polls: AtomicUsize,
    drops: AtomicUsize,
}

#[derive(Debug)]
struct NativeCause {
    marker: usize,
}

impl std::fmt::Display for NativeCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("synthetic private receive IO cause")
    }
}

impl Error for NativeCause {}

struct Tracked {
    inner: DuplexStream,
    probe: Arc<Probe>,
    failure: Option<std::io::Error>,
    fail_after: usize,
}

impl AsyncRead for Tracked {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.probe.read_polls.fetch_add(1, Ordering::SeqCst);
        let remaining = self
            .fail_after
            .saturating_sub(self.probe.bytes_read.load(Ordering::SeqCst));
        if remaining == 0 {
            if let Some(original) = self.failure.take() {
                return Poll::Ready(Err(original));
            }
        }
        let limit = if self.failure.is_some() {
            remaining.min(buf.remaining())
        } else {
            buf.remaining()
        };
        // Preserve the original native IO error and use the actual caller's
        // destination buffer. No fixture heap buffer or synthetic read result.
        let unfilled = buf.initialize_unfilled();
        let mut limited = ReadBuf::new(&mut unfilled[..limit]);
        let result = Pin::new(&mut self.inner).poll_read(cx, &mut limited);
        let count = limited.filled().len();
        buf.advance(count);
        self.probe.bytes_read.fetch_add(count, Ordering::SeqCst);
        result
    }
}

impl AsyncWrite for Tracked {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn stream(
    failure: Option<std::io::Error>,
    fail_after: usize,
) -> (Tracked, DuplexStream, Arc<Probe>) {
    let (inner, peer) = tokio::io::duplex(128);
    let probe = Arc::new(Probe::default());
    (
        Tracked {
            inner,
            probe: Arc::clone(&probe),
            failure,
            fail_after,
        },
        peer,
        probe,
    )
}

fn alive() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

async fn until(condition: impl Fn() -> bool) -> bool {
    tokio::time::timeout(OBSERVE, async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok()
}

fn packet(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; IPV6_HEADER_LEN + payload.len()];
    bytes[0] = 0x60;
    bytes[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    bytes[IPV6_HEADER_LEN..].copy_from_slice(payload);
    bytes
}

fn reader_task(
    mut reader: Tracked,
    payload_capacity: usize,
    capacity: usize,
    bank: &Arc<Semaphore>,
) -> (OwnedTask, Ledger, mpsc::Receiver<OwnedBuffer>) {
    let header = OwnedBuffer::zeroed(IPV6_HEADER_LEN, Some(bank)).unwrap();
    let (tx, rx) = mpsc::channel(capacity);
    let byte_bank = Some(Arc::clone(bank));
    let ledger = Ledger::new();
    let handle = tokio::spawn(async move {
        read_ios_packets_owned(&mut reader, payload_capacity, tx, header, byte_bank).await
    });
    let task = OwnedTask::new(handle, ParentRole::ForwardOrReader, ledger.clone());
    (task, ledger, rx)
}

async fn finish(
    task: &OwnedTask,
    ledger: &Ledger,
    peer: &mut DuplexStream,
    probe: &Probe,
    baseline: usize,
) -> Result<(), TunnelError> {
    task.abort();
    let result = owner::join_until(owner::deadline(OBSERVE), ledger, task.join()).await;
    // Capture actual completion before caller/task/ledger Drop. Recovery cannot
    // hide an owner whose join returned while the transport or task was alive.
    let drops = probe.drops.load(Ordering::SeqCst);
    let tasks = alive();
    let mut byte = [0];
    let eof = tokio::time::timeout(OBSERVE, peer.read(&mut byte)).await;
    assert_eq!(
        drops, 1,
        "original reader stream survived the completed join"
    );
    assert_eq!(
        tasks, baseline,
        "join returned before the reader task was reclaimed"
    );
    assert!(
        matches!(eof, Ok(Ok(0))),
        "strict native peer EOF missing: {eof:?}"
    );
    result
}

fn report(error: &TunnelError) -> &crate::TunnelShutdownFailure {
    let mut current: Option<&(dyn Error + 'static)> = Some(error);
    for _ in 0..32 {
        let cause = current.expect("typed native shutdown report missing");
        if let Some(report) = cause.downcast_ref() {
            return report;
        }
        current = cause.source();
    }
    panic!("native source chain exceeded its fixture bound");
}

#[tokio::test]
async fn receive_header_one_byte_short_rejects_before_listener_workers_or_io() {
    let baseline = alive();
    let bank = Arc::new(Semaphore::new(IPV6_HEADER_LEN - 1));
    let (reader, mut peer, probe) = stream(None, usize::MAX);
    let result = UserspaceTunDevice::start_validated_with_bank(
        "fd00::1",
        "fd00::2",
        ValidatedMtu::new(1280).unwrap(),
        reader,
        Some(Arc::clone(&bank)),
    )
    .await;
    let mut byte = [0];
    let eof = tokio::time::timeout(OBSERVE, peer.read(&mut byte)).await;
    assert!(matches!(eof, Ok(Ok(0))));
    assert_eq!(alive(), baseline);
    assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    assert_eq!(probe.read_polls.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), IPV6_HEADER_LEN - 1);
    let error = result
        .err()
        .expect("header admission unexpectedly started workers");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
}

#[tokio::test]
async fn receive_packet_one_byte_short_reads_only_header_and_keeps_typed_join_failure() {
    let baseline = alive();
    let bytes = packet(b"abcd");
    let quota = IPV6_HEADER_LEN + bytes.len() - 1;
    let bank = Arc::new(Semaphore::new(quota));
    let (reader, mut peer, probe) = stream(None, usize::MAX);
    let (task, ledger, mut rx) = reader_task(reader, 1240, 1, &bank);
    let write = tokio::time::timeout(OBSERVE, peer.write_all(&bytes)).await;
    let terminal = until(|| task.is_finished()).await;
    let result = finish(&task, &ledger, &mut peer, &probe, baseline).await;
    write.unwrap().unwrap();
    assert!(
        terminal,
        "quota refusal left the original packet reader pending"
    );
    assert_eq!(probe.bytes_read.load(Ordering::SeqCst), IPV6_HEADER_LEN);
    assert!(
        rx.recv().await.is_none(),
        "refused packet reached the channel"
    );
    assert_eq!(bank.available_permits(), quota);
    let error = result.expect_err("original reader admission error was lost");
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    assert!(report(&error).joins_complete());
    let primary = report(&error)
        .primary()
        .downcast_ref::<TunnelError>()
        .unwrap();
    let TunnelError::Io(io) = primary else {
        panic!("original reader error variant was replaced");
    };
    assert!(matches!(
        io.get_ref()
            .unwrap()
            .downcast_ref::<owned_buffer::BufferAdmission>(),
        Some(owned_buffer::BufferAdmission::Exhausted)
    ));
}

#[tokio::test]
async fn receive_blocked_send_keeps_full_packet_fee_until_queue_or_pending_owner_drops() {
    let baseline = alive();
    let bytes = packet(b"abcd");
    let quota = IPV6_HEADER_LEN + bytes.len() * 2;
    let bank = Arc::new(Semaphore::new(quota));
    let (reader, mut peer, probe) = stream(None, usize::MAX);
    let (task, ledger, mut rx) = reader_task(reader, 1240, 1, &bank);
    let mut input = bytes.clone();
    input.extend_from_slice(&bytes);
    let write = tokio::time::timeout(OBSERVE, peer.write_all(&input)).await;
    let blocked = until(|| {
        rx.len() == 1
            && probe.bytes_read.load(Ordering::SeqCst) == input.len()
            && bank.available_permits() == 0
    })
    .await;
    let pending_fee = bank.available_permits();
    let result = finish(&task, &ledger, &mut peer, &probe, baseline).await;
    let joined_fee = bank.available_permits();
    let retained = rx.recv().await;
    write.unwrap().unwrap();
    result.unwrap();
    assert!(
        blocked,
        "actual second packet never waited behind the full channel"
    );
    assert_eq!(
        pending_fee, 0,
        "blocked send released its live Vec capacity"
    );
    assert_eq!(
        joined_fee,
        quota - bytes.len(),
        "reader stop refunded the queued buffer"
    );
    let retained = retained.unwrap();
    assert_eq!(retained.capacity(), bytes.len());
    assert_eq!(&*retained, bytes.as_slice());
    assert_eq!(bank.available_permits(), quota - bytes.len());
    drop(retained);
    assert_eq!(bank.available_permits(), quota);
    assert!(rx.recv().await.is_none());
    assert!(ledger.result(None).is_ok());
}

#[tokio::test]
async fn receive_actual_8192_channel_device_and_last_rx_token_share_the_same_buffer_fee() {
    let baseline = alive();
    let bytes = packet(b"abcd");
    let mtu = ValidatedMtu::new(1280).unwrap();
    let backing = 4 * std::mem::size_of::<OwnedBuffer>();
    let quota = IPV6_HEADER_LEN + bytes.len() * 2 + mtu.as_usize() + backing * 2;
    let bank = Arc::new(Semaphore::new(quota));
    let (reader, mut peer, probe) = stream(None, usize::MAX);
    let (task, ledger, mut rx) = reader_task(reader, 1240, IOS_PACKET_CHANNEL_CAPACITY, &bank);
    let mut input = bytes.clone();
    input.extend_from_slice(&bytes);
    let write = tokio::time::timeout(OBSERVE, peer.write_all(&input)).await;
    let queued = until(|| {
        rx.len() == 2 && bank.available_permits() == quota - IPV6_HEADER_LEN - bytes.len() * 2
    })
    .await;
    let result = finish(&task, &ledger, &mut peer, &probe, baseline).await;
    let first = rx.recv().await;
    let second = rx.recv().await;
    write.unwrap().unwrap();
    result.unwrap();
    assert!(queued);
    assert_eq!(bank.available_permits(), quota - bytes.len() * 2);
    let first = first.unwrap();
    let second = second.unwrap();
    let original_ptr = second.as_ptr();
    let (stop, _) = watch::channel(ledger.clone());
    let storage = RuntimeStorage {
        bank: Some(Arc::clone(&bank)),
        ledger: ledger.clone(),
        stop,
    };
    let mut device = ChannelDevice::new(mtu, storage);
    device.rx_buf.push_back(first).unwrap();
    device.rx_buf.push_back(second).unwrap();
    drop(rx);
    let first_token = device.receive(smol_now()).unwrap().0;
    let queue_fee = device.rx_buf.backing_bytes() + device.tx_buf.backing_bytes();
    assert_eq!(
        bank.available_permits(),
        quota - bytes.len() * 2 - queue_fee
    );
    first_token.consume(|packet| {
        assert_eq!(
            bank.available_permits(),
            quota - bytes.len() * 2 - queue_fee
        );
        assert_eq!(packet, bytes.as_slice());
    });
    assert_eq!(bank.available_permits(), quota - bytes.len() - queue_fee);
    let last_token = device.receive(smol_now()).unwrap().0;
    drop(device);
    assert_eq!(bank.available_permits(), quota - bytes.len());
    last_token.consume(|packet| {
        assert_eq!(
            packet.as_ptr(),
            original_ptr,
            "RxToken replaced the actual packet Vec"
        );
        assert_eq!(bank.available_permits(), quota - bytes.len());
        packet[IPV6_HEADER_LEN] = b'z';
        assert_eq!(&packet[IPV6_HEADER_LEN..], b"zbcd");
    });
    assert_eq!(
        bank.available_permits(),
        quota,
        "final RxToken did not release its byte fee"
    );
}

#[tokio::test]
async fn receive_partial_payload_native_error_keeps_original_object_and_joins_before_refund() {
    let baseline = alive();
    let bytes = packet(b"abcdefgh");
    let quota = IPV6_HEADER_LEN + bytes.len();
    let bank = Arc::new(Semaphore::new(quota));
    let native = std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        NativeCause { marker: 0xdec0de },
    );
    let original_ptr = native
        .get_ref()
        .unwrap()
        .downcast_ref::<NativeCause>()
        .unwrap() as *const NativeCause as usize;
    let (reader, mut peer, probe) = stream(Some(native), IPV6_HEADER_LEN + 4);
    let (task, ledger, mut rx) = reader_task(reader, 1240, 1, &bank);
    let write = tokio::time::timeout(OBSERVE, peer.write_all(&bytes)).await;
    let terminal = until(|| task.is_finished()).await;
    let result = finish(&task, &ledger, &mut peer, &probe, baseline).await;
    write.unwrap().unwrap();
    assert!(terminal);
    assert_eq!(probe.bytes_read.load(Ordering::SeqCst), IPV6_HEADER_LEN + 4);
    assert!(rx.recv().await.is_none());
    assert_eq!(bank.available_permits(), quota);
    let error = result.expect_err("native partial read error was flattened or dropped");
    for observed in [error, ledger.result(None).unwrap_err()] {
        assert_eq!(
            crate::classify_error(&observed),
            crate::ErrorClass::Disconnected
        );
        let evidence = report(&observed);
        assert!(evidence.joins_complete());
        assert_eq!(evidence.pending_joins(), 0);
        let TunnelError::Io(original) = evidence.primary().downcast_ref::<TunnelError>().unwrap()
        else {
            panic!("original partial-reader IO error variant lost");
        };
        assert_eq!(original.kind(), std::io::ErrorKind::ConnectionReset);
        let cause = original
            .get_ref()
            .unwrap()
            .downcast_ref::<NativeCause>()
            .unwrap();
        assert_eq!(cause.marker, 0xdec0de);
        assert_eq!(cause as *const NativeCause as usize, original_ptr);
    }
}

#[tokio::test]
async fn receive_oversized_discard_does_not_allocate_or_charge_a_rejected_packet() {
    let baseline = alive();
    let expected = packet(b"abcd");
    let quota = IPV6_HEADER_LEN + expected.len();
    let bank = Arc::new(Semaphore::new(quota));
    let (reader, mut peer, probe) = stream(None, usize::MAX);
    let (task, ledger, mut rx) = reader_task(reader, 4, 1, &bank);
    let mut input = packet(b"oversize");
    input.extend_from_slice(&expected);
    let write = tokio::time::timeout(OBSERVE, peer.write_all(&input)).await;
    let queued = until(|| rx.len() == 1 && bank.available_permits() == 0).await;
    let result = finish(&task, &ledger, &mut peer, &probe, baseline).await;
    write.unwrap().unwrap();
    result.unwrap();
    assert!(queued);
    assert_eq!(probe.bytes_read.load(Ordering::SeqCst), input.len());
    let retained = rx.recv().await.unwrap();
    assert_eq!(&*retained, expected.as_slice());
    assert_eq!(bank.available_permits(), IPV6_HEADER_LEN);
    drop(retained);
    assert_eq!(bank.available_permits(), quota);
    assert!(rx.recv().await.is_none());
}

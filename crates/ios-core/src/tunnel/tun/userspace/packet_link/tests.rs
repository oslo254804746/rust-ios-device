use super::super::{watch, PreparedWorker};
use super::*;
use crate::tunnel::owner::Ledger;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{DuplexStream, ReadBuf};

#[tokio::test]
async fn borrowed_pending_packet_survives_cancelled_full_send_and_closed_receiver() {
    const TOTAL: usize = 1024 * 1024;
    for metered in [false, true] {
        let bank = Arc::new(Semaphore::new(TOTAL));
        let (sender, mut receiver) =
            mpsc::with_bank::<WriteItem>(1, metered.then_some(&bank)).unwrap();
        let sender = PacketSender {
            sender,
            barrier: Barrier::prepare(metered.then_some(&bank)).unwrap(),
        };
        let queued = OwnedBuffer::zeroed(17, Some(&bank)).unwrap();
        let packet = OwnedBuffer::zeroed(29, Some(&bank)).unwrap();
        let pointer = packet.as_ptr();
        let closed_packet = OwnedBuffer::zeroed(31, Some(&bank)).unwrap();
        let closed_pointer = closed_packet.as_ptr();
        sender.send(queued).await.unwrap();
        let mut pending = Some(packet);
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        assert_eq!(bank.available_permits(), 0);
        tokio::select! {
            biased;
            _ = sender.send_from(&mut pending) => panic!("full original FIFO accepted packet"),
            _ = std::future::ready(()) => {},
        }
        assert_eq!(pending.as_ref().unwrap().as_ptr(), pointer);
        drop(receiver.recv().await.unwrap());
        sender.send_from(&mut pending).await.unwrap();
        assert!(pending.is_none());
        let WriteItem::Packet(received) = receiver.recv().await.unwrap() else {
            panic!("original packet")
        };
        assert_eq!(received.as_ptr(), pointer);
        drop(received);
        let mut pending = Some(closed_packet);
        drop(receiver);
        assert!(sender.send_from(&mut pending).await.is_err());
        assert_eq!(pending.as_ref().unwrap().as_ptr(), closed_pointer);
        drop((sender, pending, held));
        assert_eq!(bank.available_permits(), TOTAL);
    }
}

#[derive(Default)]
struct Traffic {
    read: AtomicUsize,
    written: AtomicUsize,
    drops: AtomicUsize,
}

struct OriginalStream {
    inner: DuplexStream,
    traffic: Arc<Traffic>,
}

impl AsyncRead for OriginalStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buffer);
        if let Poll::Ready(Ok(())) = &result {
            self.traffic
                .read
                .fetch_add(buffer.filled().len() - before, Ordering::SeqCst);
        }
        result
    }
}

impl AsyncWrite for OriginalStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.inner).poll_write(cx, bytes);
        if let Poll::Ready(Ok(bytes)) = &result {
            self.traffic.written.fetch_add(*bytes, Ordering::SeqCst);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Drop for OriginalStream {
    fn drop(&mut self) {
        self.traffic.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn packet(payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0; IPV6_HEADER_LEN + payload.len()];
    packet[0] = 0x60;
    packet[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    packet[IPV6_HEADER_LEN..].copy_from_slice(payload);
    packet
}

fn owned(bytes: &[u8], bank: &Arc<Semaphore>) -> OwnedBuffer {
    let mut buffer = OwnedBuffer::zeroed(bytes.len(), Some(bank)).unwrap();
    buffer.copy_from_slice(bytes);
    buffer
}

#[tokio::test]
async fn same_packet_workers_keep_partial_read_and_write_progress_when_unique_port_moves() {
    const TOTAL: usize = 2 * 1024 * 1024;
    const OBSERVE: Duration = Duration::from_secs(2);
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let bank = Arc::new(Semaphore::new(TOTAL));
    let mtu = ValidatedMtu::new(1280).unwrap();
    let (link, mut port) =
        PreparedPacketLink::prepare(mtu, mtu.ipv6_payload_capacity().unwrap(), Some(&bank))
            .unwrap();
    let outcomes = Ledger::with_metadata(Some(&bank)).unwrap();
    let (stop, _) = watch::channel(outcomes.clone());
    let storage = RuntimeStorage {
        bank: Some(bank.clone()),
        ledger: outcomes.clone(),
        stop: stop.clone(),
    };
    // Both directions have genuine short transport progress before the port
    // moves. The original two workers remain owned throughout that move.
    let (stream, mut peer) = tokio::io::duplex(8);
    let traffic = Arc::new(Traffic::default());
    let original = OriginalStream {
        inner: stream,
        traffic: traffic.clone(),
    };
    let (reader, writer) = tokio::io::split(original);
    let (reader, writer) = link.workers(reader, writer, Some(bank.clone()), storage);
    let reader = PreparedWorker::prepare(reader, Some(&bank)).unwrap();
    let writer = PreparedWorker::prepare(writer, Some(&bank)).unwrap();
    let tasks = [
        reader.spawn(ParentRole::ForwardOrReader, outcomes.clone()),
        writer.spawn(ParentRole::Writer, outcomes.clone()),
    ];
    let inbound = packet(b"original partial inbound frame");
    let first = packet(b"original partial outbound frame");
    let second = packet(b"next backend outbound frame");
    let observed = tokio::time::timeout(OBSERVE, async {
        port.outgoing.send(owned(&first, &bank)).await.unwrap();
        while traffic.written.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let partial_write = traffic.written.load(Ordering::SeqCst) < first.len();
        peer.write_all(&inbound[..17]).await.unwrap();
        while traffic.read.load(Ordering::SeqCst) != 17 {
            tokio::task::yield_now().await;
        }
        let no_incomplete_packet = port.incoming.try_recv().is_err();
        // A new consumer future takes the same receiver, never the transport.
        let successor = async move {
            let packet = port.incoming.recv().await.unwrap();
            (port, packet)
        };
        tokio::pin!(successor);
        let consumer_waits = futures_util::poll!(successor.as_mut()).is_pending();
        peer.write_all(&inbound[17..]).await.unwrap();
        let (port, received) = successor.await;
        let intact_read = received.as_ref() == inbound.as_slice();
        let original_still_owned = traffic.drops.load(Ordering::SeqCst) == 0;
        port.outgoing.send(owned(&second, &bank)).await.unwrap();
        let mut written = vec![0; first.len() + second.len()];
        peer.read_exact(&mut written).await.unwrap();
        let intact_write = written[..first.len()] == first && written[first.len()..] == second;
        drop((port, received));
        (
            partial_write,
            no_incomplete_packet,
            consumer_waits,
            intact_read,
            original_still_owned,
            intact_write,
        )
    })
    .await;
    // Always collect the actual original joins before evaluating observations.
    outcomes.request_runtime_stop();
    stop.send_replace(());
    for task in &tasks {
        task.abort();
    }
    for task in &tasks {
        task.join().await;
    }
    let joined = outcomes.result(None);
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
    joined.unwrap();
    assert!(matches!(eof, Ok(Ok(0))));
    assert_eq!(traffic.drops.load(Ordering::SeqCst), 1);
    assert_eq!(observed.unwrap(), (true, true, true, true, true, true));
    drop((tasks, stop, outcomes));
    assert_eq!(bank.available_permits(), TOTAL);
}

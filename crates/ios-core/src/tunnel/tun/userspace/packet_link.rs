//! One original transport worker pair and its single backend packet port.
//! Backend changes never receive, split, or reconstruct the byte stream.
use std::{future::Future, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;

use super::{
    mpsc, OwnedBuffer, ParentRole, RuntimeStorage, TunnelError, ValidatedMtu,
    IOS_PACKET_CHANNEL_CAPACITY, IPV6_HEADER_LEN,
};

mod barrier;
use barrier::{Barrier, WriterEnd};

pub(super) enum WriteItem {
    Packet(OwnedBuffer),
    Barrier(Barrier),
}

impl From<OwnedBuffer> for WriteItem {
    fn from(packet: OwnedBuffer) -> Self {
        Self::Packet(packet)
    }
}

#[derive(Clone)]
pub(super) struct PacketSender {
    sender: mpsc::Sender<WriteItem>,
    barrier: Barrier,
}

impl PacketSender {
    /// Preserve the bridge's original in-flight packet if a selected wait is
    /// cancelled or the actual transport receiver has already ended.
    pub(super) async fn send_from(&self, pending: &mut Option<OwnedBuffer>) -> Result<(), ()> {
        let permit = self.sender.reserve().await?;
        let packet = pending.take().expect("owned pending egress packet");
        match permit.send(WriteItem::Packet(packet)) {
            Ok(()) => Ok(()),
            Err(error) => {
                let WriteItem::Packet(packet) = error.0 else {
                    unreachable!("original egress packet")
                };
                *pending = Some(packet);
                Err(())
            }
        }
    }

    pub(super) async fn send(
        &self,
        packet: OwnedBuffer,
    ) -> Result<(), mpsc::error::SendError<OwnedBuffer>> {
        self.sender
            .send(WriteItem::Packet(packet))
            .await
            .map_err(|error| {
                let WriteItem::Packet(packet) = error.0 else {
                    unreachable!("packet submission")
                };
                mpsc::error::SendError(packet)
            })
    }
    pub(super) fn try_send(
        &self,
        packet: OwnedBuffer,
    ) -> Result<(), mpsc::error::TrySendError<OwnedBuffer>> {
        self.sender
            .try_send(WriteItem::Packet(packet))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(WriteItem::Packet(packet)) => {
                    mpsc::error::TrySendError::Full(packet)
                }
                mpsc::error::TrySendError::Closed(WriteItem::Packet(packet)) => {
                    mpsc::error::TrySendError::Closed(packet)
                }
                _ => unreachable!("packet submission"),
            })
    }
    pub(super) async fn flush_barrier(
        &self,
        ledger: &super::Ledger,
        stop: &super::watch::Sender,
    ) -> Result<(), TunnelError> {
        ledger.work_result()?;
        if let Some(mut submission) = self.barrier.submission()? {
            if self
                .sender
                .send_marked(WriteItem::Barrier(self.barrier.clone()), || {
                    submission.queued()
                })
                .await
                .is_err()
            {
                ledger.work_result()?;
                return Err(std::io::Error::other(crate::OperationCancelled::new()).into());
            }
        }
        self.barrier.wait(ledger, stop.subscribe()).await
    }
}

pub(super) trait TrySender<T> {
    fn try_send(&self, value: T) -> Result<(), mpsc::error::TrySendError<T>>;
}
impl<T> TrySender<T> for mpsc::Sender<T> {
    fn try_send(&self, value: T) -> Result<(), mpsc::error::TrySendError<T>> {
        mpsc::Sender::try_send(self, value)
    }
}
impl TrySender<OwnedBuffer> for PacketSender {
    fn try_send(&self, value: OwnedBuffer) -> Result<(), mpsc::error::TrySendError<OwnedBuffer>> {
        PacketSender::try_send(self, value)
    }
}

/// The unique backend-facing receiver and its original egress sender.
/// There is intentionally no Clone or raw transport capability.
pub(super) struct PacketPort {
    pub(super) incoming: mpsc::Receiver<OwnedBuffer>,
    pub(super) outgoing: PacketSender,
}

pub(super) struct PreparedPacketLink {
    mtu: ValidatedMtu,
    payload_capacity: usize,
    header: OwnedBuffer,
    incoming: mpsc::Sender<OwnedBuffer>,
    outgoing: mpsc::Receiver<WriteItem>,
    barrier: Barrier,
}

impl PreparedPacketLink {
    pub(super) fn prepare(
        mtu: ValidatedMtu,
        payload_capacity: usize,
        bank: Option<&Arc<Semaphore>>,
    ) -> Result<(Self, PacketPort), TunnelError> {
        // Preserve the original header -> ingress -> egress admission order.
        // These same paid roots move into the original four-worker runtime.
        let header = OwnedBuffer::zeroed(IPV6_HEADER_LEN, bank)?;
        let (incoming, ingress) = mpsc::with_bank(IOS_PACKET_CHANNEL_CAPACITY, bank)?;
        let (egress, outgoing) = mpsc::with_bank(IOS_PACKET_CHANNEL_CAPACITY, bank)?;
        let barrier = Barrier::prepare(bank)?;
        Ok((
            Self {
                mtu,
                payload_capacity,
                header,
                incoming,
                outgoing,
                barrier: barrier.clone(),
            },
            PacketPort {
                incoming: ingress,
                outgoing: PacketSender {
                    sender: egress,
                    barrier,
                },
            },
        ))
    }

    pub(super) fn workers<R, W>(
        self,
        mut reader: R,
        mut writer: W,
        byte_bank: Option<Arc<Semaphore>>,
        storage: RuntimeStorage,
    ) -> (
        impl Future<Output = Result<(), TunnelError>> + Send + 'static,
        impl Future<Output = Result<(), TunnelError>> + Send + 'static,
    )
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let Self {
            mtu,
            payload_capacity,
            header,
            incoming,
            outgoing,
            barrier,
        } = self;
        let mut reader_stop = storage.stop.subscribe();
        let mut writer_stop = storage.stop.subscribe();
        let reader_storage = storage.clone();
        let reader_future = async move {
            let result = tokio::select! {
                biased;
                result = read_ios_packets_owned(&mut reader, payload_capacity, incoming, header, byte_bank) => result,
                _ = reader_stop.changed() => Ok(()),
            };
            if let Err(original) = result {
                reader_storage.fail(ParentRole::ForwardOrReader, original);
            }
            let result = reader_storage.result();
            tracing::debug!("userspace: iOS packet reader exited");
            result
        };
        let writer_storage = storage;
        let writer_future = async move {
            let _ended = WriterEnd(barrier);
            let _result = tokio::select! {
                biased;
                result = write_ios_packets_owned(&mut writer, mtu, outgoing, Some(&writer_storage)) => result,
                _ = writer_stop.changed() => Ok(()),
            };
            let result = writer_storage.result();
            tracing::debug!("userspace: iOS packet writer exited");
            result
        };
        (reader_future, writer_future)
    }
}

pub(super) async fn read_ios_packets_owned<R>(
    reader: &mut R,
    payload_capacity: usize,
    tx: mpsc::Sender<OwnedBuffer>,
    mut hdr: OwnedBuffer,
    byte_bank: Option<Arc<Semaphore>>,
) -> Result<(), TunnelError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    loop {
        reader.read_exact(&mut hdr).await?;
        let payload_len = u16::from_be_bytes([hdr[4], hdr[5]]) as usize;
        if hdr[0] >> 4 != 6 {
            tracing::warn!("userspace: non-IPv6 packet from iOS, skipping");
            discard_bytes_owned(reader, payload_len).await?;
            continue;
        }
        if payload_len > payload_capacity {
            tracing::warn!(
                "userspace: IPv6 payload {payload_len} exceeds negotiated payload capacity {payload_capacity}"
            );
            discard_bytes_owned(reader, payload_len).await?;
            continue;
        }
        let packet_len = match IPV6_HEADER_LEN.checked_add(payload_len) {
            Some(packet_len) => packet_len,
            None => {
                tracing::warn!("userspace: IPv6 packet length overflow");
                break;
            }
        };
        let mut pkt = OwnedBuffer::zeroed(packet_len, byte_bank.as_ref())?;
        pkt[..IPV6_HEADER_LEN].copy_from_slice(&hdr);
        reader.read_exact(&mut pkt[IPV6_HEADER_LEN..]).await?;
        if byte_bank.is_none() {
            forward_userspace_udp(&pkt);
        }
        if tx.send(pkt).await.is_err() {
            break;
        }
    }
    Ok(())
}

async fn discard_bytes_owned<R>(reader: &mut R, mut remaining: usize) -> Result<(), TunnelError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut discard = [0u8; 4096];
    while remaining > 0 {
        let chunk_len = remaining.min(discard.len());
        reader.read_exact(&mut discard[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

pub(super) async fn write_ios_packets_owned<W, T>(
    writer: &mut W,
    mtu: ValidatedMtu,
    mut rx: mpsc::Receiver<T>,
    storage: Option<&RuntimeStorage>,
) -> Result<(), TunnelError>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Into<WriteItem>,
{
    while let Some(item) = rx.recv().await {
        match item.into() {
            WriteItem::Packet(pkt) => {
                if pkt.len() > mtu.as_usize() {
                    tracing::warn!(
                        "userspace: generated IPv6 packet length {} exceeds negotiated MTU {}",
                        pkt.len(),
                        mtu.as_u16()
                    );
                    continue;
                }
                writer
                    .write_all(&pkt)
                    .await
                    .map_err(|original| capture_write_error(original, storage))?;
            }
            WriteItem::Barrier(barrier) => {
                writer
                    .flush()
                    .await
                    .map_err(|original| capture_write_error(original, storage))?;
                barrier.flushed();
            }
        }
    }
    Ok(())
}

fn capture_write_error(original: std::io::Error, storage: Option<&RuntimeStorage>) -> TunnelError {
    let Some(storage) = storage else {
        return original.into();
    };
    // This executes in the actual Ready(Err) path, before the original packet
    // or future can drop/refund a fee or wake the barrier's caller.
    storage.fail(ParentRole::Writer, original.into());
    storage
        .result()
        .expect_err("captured original writer error")
}

#[cfg(test)]
pub(super) fn egress_bytes() -> usize {
    mpsc::required_bytes::<WriteItem>(IOS_PACKET_CHANNEL_CAPACITY) + Barrier::body_bytes()
}

fn forward_userspace_udp(pkt: &[u8]) {
    if pkt.len() < IPV6_HEADER_LEN + 8 || (pkt[0] >> 4) != 6 || pkt[6] != 17 {
        return;
    }
    let dst_port = u16::from_be_bytes([pkt[42], pkt[43]]);
    let udp_len = u16::from_be_bytes([pkt[44], pkt[45]]) as usize;
    if dst_port == 0 || udp_len < 8 || IPV6_HEADER_LEN + udp_len > pkt.len() {
        return;
    }
    static UDP_FORWARDER: std::sync::OnceLock<Option<std::net::UdpSocket>> =
        std::sync::OnceLock::new();
    let Some(sock) = UDP_FORWARDER.get_or_init(|| {
        let sock = std::net::UdpSocket::bind((std::net::Ipv6Addr::LOCALHOST, 0)).ok()?;
        let _ = sock.set_nonblocking(true);
        Some(sock)
    }) else {
        return;
    };
    let target = std::net::SocketAddrV6::new(std::net::Ipv6Addr::LOCALHOST, dst_port, 0, 0);
    let _ = sock.send_to(&pkt[IPV6_HEADER_LEN + 8..IPV6_HEADER_LEN + udp_len], target);
}

#[cfg(test)]
mod tests;

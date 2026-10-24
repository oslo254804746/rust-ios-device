//! One original transport worker pair and its single backend packet port.
//! Backend changes never receive, split, or reconstruct the byte stream.
use std::{future::Future, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;

use super::{
    mpsc, OwnedBuffer, ParentRole, RuntimeStorage, TunnelError, ValidatedMtu,
    IOS_PACKET_CHANNEL_CAPACITY, IPV6_HEADER_LEN,
};

/// The unique backend-facing receiver and its original egress sender.
/// There is intentionally no Clone or raw transport capability.
pub(super) struct PacketPort {
    pub(super) incoming: mpsc::Receiver<OwnedBuffer>,
    pub(super) outgoing: mpsc::Sender<OwnedBuffer>,
}

pub(super) struct PreparedPacketLink {
    mtu: ValidatedMtu,
    payload_capacity: usize,
    header: OwnedBuffer,
    incoming: mpsc::Sender<OwnedBuffer>,
    outgoing: mpsc::Receiver<OwnedBuffer>,
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
        Ok((
            Self {
                mtu,
                payload_capacity,
                header,
                incoming,
                outgoing,
            },
            PacketPort {
                incoming: ingress,
                outgoing: egress,
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
            let result = tokio::select! {
                biased;
                result = write_ios_packets_owned(&mut writer, mtu, outgoing) => result,
                _ = writer_stop.changed() => Ok(()),
            };
            if let Err(original) = result {
                writer_storage.fail(ParentRole::Writer, original);
            }
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

pub(super) async fn write_ios_packets_owned<W>(
    writer: &mut W,
    mtu: ValidatedMtu,
    mut rx: mpsc::Receiver<OwnedBuffer>,
) -> Result<(), TunnelError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    while let Some(pkt) = rx.recv().await {
        if pkt.len() > mtu.as_usize() {
            tracing::warn!(
                "userspace: generated IPv6 packet length {} exceeds negotiated MTU {}",
                pkt.len(),
                mtu.as_u16()
            );
            continue;
        }
        writer.write_all(&pkt).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

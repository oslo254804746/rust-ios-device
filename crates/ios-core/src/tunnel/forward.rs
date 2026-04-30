use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

use crate::tunnel::tun::TunDevice;
use crate::tunnel::TunnelError;

/// Bidirectionally forward IPv6 packets between an iOS TCP stream and a TUN device.
/// Runs until the `cancel` watch receiver fires or an IO error occurs.
pub async fn forward_packets<S, D>(
    mut stream: S,
    mut tun: D,
    mtu: u32,
    mut cancel: watch::Receiver<()>,
) -> Result<(), TunnelError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    D: TunDevice,
{
    let mut ip6_header = vec![0u8; 40];
    let mut payload_buf = vec![0u8; mtu as usize];

    loop {
        tokio::select! {
            _ = cancel.changed() => break,

            // iOS stream → TUN: read IPv6 packet, inject into TUN
            result = read_ipv6_packet(&mut stream, &mut ip6_header, &mut payload_buf) => {
                let payload_len = result?;
                // Avoid clone on hot path: write header and payload as two separate slices
                let mut packet = Vec::with_capacity(40 + payload_len);
                packet.extend_from_slice(&ip6_header);
                packet.extend_from_slice(&payload_buf[..payload_len]);
                tun.write_packet(&packet).await?;
            }

            // TUN → iOS stream: read packet from OS, forward to device
            result = tun.read_packet() => {
                let packet = result?;
                stream.write_all(&packet).await?;
                stream.flush().await?;
            }
        }
    }
    Ok(())
}

async fn read_ipv6_packet<R: AsyncRead + Unpin>(
    reader: &mut R,
    header_buf: &mut [u8],
    payload_buf: &mut [u8],
) -> Result<usize, TunnelError> {
    reader.read_exact(header_buf).await?;

    if header_buf[0] >> 4 != 6 {
        return Err(TunnelError::Protocol(format!(
            "expected IPv6 packet, got version {}",
            header_buf[0] >> 4
        )));
    }

    let payload_len = u16::from_be_bytes([header_buf[4], header_buf[5]]) as usize;
    if payload_len > payload_buf.len() {
        return Err(TunnelError::Protocol(format!(
            "IPv6 payload {payload_len} exceeds buffer {}",
            payload_buf.len()
        )));
    }
    reader.read_exact(&mut payload_buf[..payload_len]).await?;
    Ok(payload_len)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf};
    use tokio::sync::watch;

    use super::*;

    #[derive(Clone, Default)]
    struct SharedTunState {
        packets_to_read: VecDeque<Vec<u8>>,
        written_packets: Vec<Vec<u8>>,
    }

    #[derive(Clone, Default)]
    struct MockTun {
        state: Arc<Mutex<SharedTunState>>,
    }

    impl MockTun {
        fn with_read_packets(packets: impl IntoIterator<Item = Vec<u8>>) -> Self {
            Self {
                state: Arc::new(Mutex::new(SharedTunState {
                    packets_to_read: packets.into_iter().collect(),
                    written_packets: Vec::new(),
                })),
            }
        }
    }

    impl TunDevice for MockTun {
        async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
            loop {
                if let Some(packet) = self.state.lock().unwrap().packets_to_read.pop_front() {
                    return Ok(packet);
                }
                tokio::task::yield_now().await;
            }
        }

        async fn write_packet(&mut self, packet: &[u8]) -> Result<(), TunnelError> {
            self.state
                .lock()
                .unwrap()
                .written_packets
                .push(packet.to_vec());
            Ok(())
        }
    }

    struct PendingTun {
        state: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl PendingTun {
        fn new() -> (Self, Arc<Mutex<Vec<Vec<u8>>>>) {
            let state = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    state: state.clone(),
                },
                state,
            )
        }
    }

    impl TunDevice for PendingTun {
        async fn read_packet(&mut self) -> Result<Vec<u8>, TunnelError> {
            std::future::pending().await
        }

        async fn write_packet(&mut self, packet: &[u8]) -> Result<(), TunnelError> {
            self.state.lock().unwrap().push(packet.to_vec());
            Ok(())
        }
    }

    struct ChunkedReader {
        data: Vec<u8>,
        pos: usize,
        max_chunk: usize,
    }

    impl ChunkedReader {
        fn new(data: Vec<u8>, max_chunk: usize) -> Self {
            Self {
                data,
                pos: 0,
                max_chunk,
            }
        }
    }

    impl AsyncRead for ChunkedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.pos >= self.data.len() {
                return Poll::Ready(Ok(()));
            }

            let remaining = self.data.len() - self.pos;
            let n = remaining.min(self.max_chunk).min(buf.remaining());
            let end = self.pos + n;
            buf.put_slice(&self.data[self.pos..end]);
            self.pos = end;
            Poll::Ready(Ok(()))
        }
    }

    fn ipv6_packet(payload: &[u8]) -> Vec<u8> {
        let mut packet = vec![0u8; 40 + payload.len()];
        packet[0] = 0x60;
        packet[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        packet[40..].copy_from_slice(payload);
        packet
    }

    async fn run_forward_for_packet_test(
        stream: DuplexStream,
        tun: impl TunDevice,
    ) -> (
        watch::Sender<()>,
        tokio::task::JoinHandle<Result<(), TunnelError>>,
    ) {
        let (cancel_tx, cancel_rx) = watch::channel(());
        let task = tokio::spawn(forward_packets(stream, tun, 1280, cancel_rx));
        (cancel_tx, task)
    }

    #[tokio::test]
    async fn read_ipv6_packet_reassembles_chunked_input() {
        let expected = ipv6_packet(b"payload");
        let mut reader = ChunkedReader::new(expected.clone(), 3);
        let mut header = [0u8; 40];
        let mut payload = [0u8; 1280];

        let payload_len = read_ipv6_packet(&mut reader, &mut header, &mut payload)
            .await
            .unwrap();

        assert_eq!(payload_len, b"payload".len());
        assert_eq!(&header, &expected[..40]);
        assert_eq!(&payload[..payload_len], b"payload");
    }

    #[tokio::test]
    async fn read_ipv6_packet_rejects_non_ipv6_packet() {
        let mut packet = ipv6_packet(b"payload");
        packet[0] = 0x40;
        let mut reader = tokio::io::BufReader::new(&packet[..]);
        let mut header = [0u8; 40];
        let mut payload = [0u8; 1280];

        let err = read_ipv6_packet(&mut reader, &mut header, &mut payload)
            .await
            .unwrap_err();

        match err {
            TunnelError::Protocol(message) => assert!(
                message.contains("expected IPv6 packet"),
                "unexpected error: {message}"
            ),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_ipv6_packet_rejects_payload_larger_than_mtu_buffer() {
        let mut packet = ipv6_packet(&[0xaa; 8]);
        packet[4..6].copy_from_slice(&9u16.to_be_bytes());
        let mut reader = tokio::io::BufReader::new(&packet[..]);
        let mut header = [0u8; 40];
        let mut payload = [0u8; 8];

        let err = read_ipv6_packet(&mut reader, &mut header, &mut payload)
            .await
            .unwrap_err();

        match err {
            TunnelError::Protocol(message) => assert!(
                message.contains("exceeds buffer 8"),
                "unexpected error: {message}"
            ),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn forward_packets_writes_stream_packets_to_tun() {
        let expected = ipv6_packet(b"from-ios");
        let (mut peer, stream) = tokio::io::duplex(4096);
        let (tun, written) = PendingTun::new();
        let (cancel_tx, task) = run_forward_for_packet_test(stream, tun).await;

        peer.write_all(&expected).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if written.lock().unwrap().as_slice() == [expected.clone()] {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        drop(cancel_tx);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn forward_packets_writes_tun_packets_to_stream() {
        let expected = ipv6_packet(b"from-tun");
        let (mut peer, stream) = tokio::io::duplex(4096);
        let tun = MockTun::with_read_packets([expected.clone()]);
        let (cancel_tx, task) = run_forward_for_packet_test(stream, tun).await;

        let mut received = vec![0u8; expected.len()];
        peer.read_exact(&mut received).await.unwrap();
        assert_eq!(received, expected);

        drop(cancel_tx);
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn forward_packets_stops_when_cancelled() {
        let (_peer, stream) = tokio::io::duplex(4096);
        let tun = MockTun::default();
        let (cancel_tx, task) = run_forward_for_packet_test(stream, tun).await;

        drop(cancel_tx);

        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn forward_packets_returns_error_for_invalid_stream_packet() {
        let mut invalid = ipv6_packet(b"bad");
        invalid[0] = 0x40;
        let (mut peer, stream) = tokio::io::duplex(4096);
        let tun = MockTun::default();
        let (cancel_tx, task) = run_forward_for_packet_test(stream, tun).await;

        peer.write_all(&invalid).await.unwrap();
        let err = task.await.unwrap().unwrap_err();
        drop(cancel_tx);

        match err {
            TunnelError::Protocol(message) => assert!(
                message.contains("expected IPv6 packet"),
                "unexpected error: {message}"
            ),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}

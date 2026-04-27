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

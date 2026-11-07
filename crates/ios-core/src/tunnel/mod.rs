//! CDTunnel handshake, TUN device abstraction, and packet forwarding.

crate::redaction::error_debug!(TunnelError);

use std::net::Ipv6Addr;
use std::time::{Duration, Instant};

/// Maximum time allowed for a TCP dial or initial protocol setup through a tunnel.
///
/// A stale kernel route can otherwise inherit the operating system's much
/// longer SYN retry window and stall every RSD/service operation.
pub const TUNNEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long the device may stay silent after the host's last outbound packet
/// before the userspace tunnel nudges it with one ICMPv6 Echo Request (20 ms).
///
/// After a burst, iOS `CoreDeviceProxy` can hold packets it has already
/// received until another host packet arrives; sending a single ICMPv6 echo
/// releases the held window within a few milliseconds without re-arming the
/// watchdog if unanswered.
pub const USERSPACE_NUDGE_DELAY: Duration = Duration::from_millis(20);

/// Total byte length of the IPv6 + ICMPv6 Echo Request nudge packet (40-byte
/// IPv6 header + 12-byte ICMPv6 body).
pub const ICMPV6_ECHO_PACKET_LEN: usize = 52;

/// The minimum MTU required by IPv6 (RFC 8200).
pub const IPV6_MIN_MTU: u32 = 1280;

/// The fixed IPv6 base header included in every packet on a CDTunnel stream.
pub(crate) const IPV6_HEADER_LEN: usize = 40;

/// The largest MTU accepted by the kernel TUN API used by this crate.
pub const MAX_TUNNEL_MTU: u32 = u16::MAX as u32;

/// Build a complete 52-byte IPv6 ICMPv6 Echo Request packet (RFC 4443 §4.1)
/// from `src` to `dst`, matching `pymobiledevice3`'s `_icmp6_echo_request`.
pub fn build_icmpv6_echo_request(src: Ipv6Addr, dst: Ipv6Addr) -> [u8; ICMPV6_ECHO_PACKET_LEN] {
    let src_bytes = src.octets();
    let dst_bytes = dst.octets();
    let mut body = [
        128u8, 0, // Type = 128 (Echo Request), Code = 0
        0, 0, // Checksum placeholder
        0x70, 0x6D, // Identifier = 0x706D ("pm")
        0, 0, // Sequence = 0
        b'p', b'm', b'd', b'3',
    ];

    let mut sum: u32 = 0;
    for chunk in src_bytes.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    for chunk in dst_bytes.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    // Upper-layer packet length (12) + Next Header (58 = ICMPv6)
    sum += body.len() as u32;
    sum += 58u32;
    for chunk in body.chunks_exact(2) {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    let checksum = !(sum as u16);
    body[2..4].copy_from_slice(&checksum.to_be_bytes());

    let mut packet = [0u8; ICMPV6_ECHO_PACKET_LEN];
    // Version = 6, Traffic Class = 0, Flow Label = 0
    packet[0..4].copy_from_slice(&0x6000_0000u32.to_be_bytes());
    // Payload Length = 12
    packet[4..6].copy_from_slice(&(body.len() as u16).to_be_bytes());
    // Next Header = 58 (ICMPv6), Hop Limit = 64
    packet[6] = 58;
    packet[7] = 64;
    packet[8..24].copy_from_slice(&src_bytes);
    packet[24..40].copy_from_slice(&dst_bytes);
    packet[40..52].copy_from_slice(&body);
    packet
}

/// Deterministic state machine for the userspace tunnel burst-silence nudge
/// watchdog (`pymobiledevice3` `UserspaceTun` `NUDGE_DELAY`).
#[derive(Debug, Clone)]
pub struct UserspaceNudgeWatchdog {
    nudge_packet: [u8; ICMPV6_ECHO_PACKET_LEN],
    nudge_delay: Duration,
    last_tx: Option<Instant>,
    rx_since_tx: bool,
    armed: bool,
    closed: bool,
}

impl UserspaceNudgeWatchdog {
    pub fn new(src: Ipv6Addr, dst: Ipv6Addr, nudge_delay: Duration) -> Self {
        Self {
            nudge_packet: build_icmpv6_echo_request(src, dst),
            nudge_delay,
            last_tx: None,
            rx_since_tx: false,
            armed: false,
            closed: false,
        }
    }

    pub fn with_default_delay(src: Ipv6Addr, dst: Ipv6Addr) -> Self {
        Self::new(src, dst, USERSPACE_NUDGE_DELAY)
    }

    pub fn nudge_packet(&self) -> &[u8; ICMPV6_ECHO_PACKET_LEN] {
        &self.nudge_packet
    }

    pub fn is_armed(&self) -> bool {
        self.armed && !self.closed
    }

    /// Record an outbound packet or batch at `now`. The nudge's own packet
    /// does not re-arm the watchdog so an unanswered nudge is sent at most once.
    pub fn record_outbound(&mut self, packet: &[u8], now: Instant) {
        if self.closed || packet == self.nudge_packet {
            return;
        }
        self.last_tx = Some(now);
        self.rx_since_tx = false;
        self.armed = true;
    }

    /// Record an inbound packet from the device.
    pub fn record_inbound(&mut self) {
        if self.closed {
            return;
        }
        self.rx_since_tx = true;
        self.armed = false;
    }

    /// Return the deadline at which the next check/nudge is due, if armed.
    pub fn next_deadline(&self) -> Option<Instant> {
        if self.closed || !self.armed || self.rx_since_tx {
            return None;
        }
        self.last_tx
            .and_then(|last_tx| last_tx.checked_add(self.nudge_delay))
    }

    /// Poll the watchdog at `now`, returning the 52-byte ICMPv6 Echo Request
    /// packet if the device has remained silent for at least `nudge_delay`
    /// since the last non-nudge outbound packet.
    pub fn poll_nudge(&mut self, now: Instant) -> Option<[u8; ICMPV6_ECHO_PACKET_LEN]> {
        if self.closed || !self.armed {
            return None;
        }
        if self.rx_since_tx {
            self.armed = false;
            return None;
        }
        let deadline = self.next_deadline()?;
        if now < deadline {
            return None;
        }
        self.armed = false;
        Some(self.nudge_packet)
    }

    /// Cancel any pending nudge on tunnel teardown.
    pub fn close(&mut self) {
        self.closed = true;
        self.armed = false;
    }
}

/// An MTU that is safe for every tunnel implementation in this crate.
///
/// The wire handshake uses an unsigned 32-bit value, while the kernel TUN
/// implementation accepts a `u16` and all packet buffers need a host `usize`.
/// Keeping the checked conversion in one small type prevents those boundaries
/// from being reimplemented with truncating casts in individual backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidatedMtu(u16);

fn invalid_mtu(
    value: u32,
    reason: String,
    source: Option<std::num::TryFromIntError>,
) -> TunnelError {
    #[derive(thiserror::Error)]
    #[error("invalid tunnel MTU {value}: {reason}")]
    struct MtuInput {
        value: u32,
        reason: String,
        #[source]
        source: Option<std::num::TryFromIntError>,
    }
    impl std::fmt::Debug for MtuInput {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TunnelMtuInput")
                .field("value", &self.value)
                .finish_non_exhaustive()
        }
    }
    TunnelError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        MtuInput {
            value,
            reason,
            source,
        },
    ))
}

impl ValidatedMtu {
    pub(crate) fn new(value: u32) -> Result<Self, TunnelError> {
        if value < IPV6_MIN_MTU {
            return Err(invalid_mtu(
                value,
                format!("IPv6 requires at least {IPV6_MIN_MTU} bytes"),
                None,
            ));
        }

        let value = u16::try_from(value).map_err(|source| {
            invalid_mtu(
                value,
                format!("exceeds the supported maximum {MAX_TUNNEL_MTU} bytes"),
                Some(source),
            )
        })?;
        Ok(Self(value))
    }

    pub(crate) const fn as_u16(self) -> u16 {
        self.0
    }

    pub(crate) fn as_usize(self) -> usize {
        usize::from(self.0)
    }

    /// Return the largest IPv6 payload that fits in this MTU.
    pub(crate) fn ipv6_payload_capacity(self) -> Result<usize, TunnelError> {
        self.as_usize().checked_sub(IPV6_HEADER_LEN).ok_or_else(|| {
            TunnelError::Protocol(format!(
                "invalid tunnel MTU {}: smaller than the IPv6 header",
                self.as_u16()
            ))
        })
    }
}

#[cfg(feature = "tunnel")]
pub mod forward;
#[cfg(feature = "tunnel")]
pub mod handshake;
pub mod manager;
#[cfg(feature = "tunnel-private-ip")]
pub(crate) mod owner;
#[cfg(feature = "tunnel-private-ip")]
pub use owner::TunnelShutdownFailure;
#[cfg(feature = "tunnel")]
pub mod tun;

#[cfg(feature = "tunnel")]
pub use handshake::TunnelInfo;
#[cfg(not(feature = "tunnel"))]
#[derive(Clone)]
pub struct TunnelInfo {
    pub server_address: String,
    pub server_rsd_port: u16,
    pub client_address: String,
    pub client_mtu: u32,
}

#[cfg(not(feature = "tunnel"))]
impl std::fmt::Debug for TunnelInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunnelInfo")
            .field(
                "server",
                &crate::redaction::diagnostic_id(&self.server_address),
            )
            .field("rsd_port", &self.server_rsd_port)
            .field(
                "client",
                &crate::redaction::diagnostic_id(&self.client_address),
            )
            .field("mtu", &self.client_mtu)
            .finish()
    }
}
pub use manager::{TunMode, TunnelHandle, TunnelManager};
#[cfg(feature = "tunnel-private-ip")]
pub use owner::phase_outcomes::{PhaseCleanup, PhaseFailure};

/// Errors from tunnel operations.
#[derive(thiserror::Error)]
pub enum TunnelError {
    #[cfg(feature = "tunnel-private-ip")]
    #[error(transparent)]
    Phases(#[from] PhaseFailure),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("TUN device error: {0}")]
    TunDevice(String),
}

#[cfg(test)]
mod nudge_tests {
    use super::*;

    const HOST_ADDR: &str = "fd57:6e64:6572:7761::2";
    const DEVICE_ADDR: &str = "fd57:6e64:6572:7761::1";

    #[test]
    fn icmpv6_echo_request_has_valid_header_and_rfc4443_checksum() {
        let src: Ipv6Addr = HOST_ADDR.parse().unwrap();
        let dst: Ipv6Addr = DEVICE_ADDR.parse().unwrap();
        let packet = build_icmpv6_echo_request(src, dst);
        assert_eq!(packet.len(), ICMPV6_ECHO_PACKET_LEN);
        assert_eq!(packet[0] >> 4, 6);
        assert_eq!(packet[6], 58);
        let payload_len = u16::from_be_bytes([packet[4], packet[5]]) as usize;
        assert_eq!(payload_len, packet.len() - IPV6_HEADER_LEN);
        let body = &packet[IPV6_HEADER_LEN..];
        assert_eq!(body[0], 128);
        assert_eq!(body[1], 0);
        assert_eq!(&body[8..12], b"pmd3");

        let mut total: u32 = 0;
        for chunk in packet[8..40].chunks_exact(2) {
            total += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
        }
        total += body.len() as u32;
        total += 58u32;
        for chunk in body.chunks_exact(2) {
            total += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
        }
        while (total >> 16) != 0 {
            total = (total & 0xFFFF) + (total >> 16);
        }
        assert_eq!(total as u16, 0xFFFF);
    }

    #[test]
    fn silent_device_is_nudged_once_and_nudge_does_not_rearm() {
        let src: Ipv6Addr = HOST_ADDR.parse().unwrap();
        let dst: Ipv6Addr = DEVICE_ADDR.parse().unwrap();
        let mut watchdog = UserspaceNudgeWatchdog::with_default_delay(src, dst);
        let t0 = Instant::now();
        let mut data_pkt = [0u8; 60];
        data_pkt[0] = 0x60;

        watchdog.record_outbound(&data_pkt, t0);
        assert!(watchdog.is_armed());
        assert!(watchdog
            .poll_nudge(t0 + Duration::from_millis(10))
            .is_none());

        let nudge = watchdog
            .poll_nudge(t0 + Duration::from_millis(20))
            .expect("expected single ICMPv6 nudge after 20ms silence");
        assert_eq!(&nudge, watchdog.nudge_packet());
        assert!(!watchdog.is_armed());

        // The nudge's own egress does not re-arm the watchdog.
        watchdog.record_outbound(&nudge, t0 + Duration::from_millis(21));
        assert!(!watchdog.is_armed());
        assert!(watchdog
            .poll_nudge(t0 + Duration::from_millis(100))
            .is_none());
    }

    #[test]
    fn device_that_answers_or_closed_watchdog_is_not_nudged() {
        let src: Ipv6Addr = HOST_ADDR.parse().unwrap();
        let dst: Ipv6Addr = DEVICE_ADDR.parse().unwrap();
        let mut watchdog = UserspaceNudgeWatchdog::with_default_delay(src, dst);
        let t0 = Instant::now();
        let mut data_pkt = [0u8; 60];
        data_pkt[0] = 0x60;

        watchdog.record_outbound(&data_pkt, t0);
        watchdog.record_inbound();
        assert!(!watchdog.is_armed());
        assert!(watchdog
            .poll_nudge(t0 + Duration::from_millis(50))
            .is_none());

        watchdog.record_outbound(&data_pkt, t0 + Duration::from_millis(60));
        assert!(watchdog.is_armed());
        watchdog.close();
        assert!(!watchdog.is_armed());
        assert!(watchdog
            .poll_nudge(t0 + Duration::from_millis(100))
            .is_none());
    }
}

//! CDTunnel handshake, TUN device abstraction, and packet forwarding.

use std::time::Duration;

/// Maximum time allowed for a TCP dial or initial protocol setup through a tunnel.
///
/// A stale kernel route can otherwise inherit the operating system's much
/// longer SYN retry window and stall every RSD/service operation.
pub const TUNNEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// The minimum MTU required by IPv6 (RFC 8200).
pub const IPV6_MIN_MTU: u32 = 1280;

/// The fixed IPv6 base header included in every packet on a CDTunnel stream.
pub(crate) const IPV6_HEADER_LEN: usize = 40;

/// The largest MTU accepted by the kernel TUN API used by this crate.
pub const MAX_TUNNEL_MTU: u32 = u16::MAX as u32;

/// An MTU that is safe for every tunnel implementation in this crate.
///
/// The wire handshake uses an unsigned 32-bit value, while the kernel TUN
/// implementation accepts a `u16` and all packet buffers need a host `usize`.
/// Keeping the checked conversion in one small type prevents those boundaries
/// from being reimplemented with truncating casts in individual backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidatedMtu(u16);

impl ValidatedMtu {
    pub(crate) fn new(value: u32) -> Result<Self, TunnelError> {
        if value < IPV6_MIN_MTU {
            return Err(TunnelError::Protocol(format!(
                "invalid tunnel MTU {value}: IPv6 requires at least {IPV6_MIN_MTU} bytes"
            )));
        }

        let value = u16::try_from(value).map_err(|_| {
            TunnelError::Protocol(format!(
                "invalid tunnel MTU {value}: exceeds the supported maximum {MAX_TUNNEL_MTU} bytes"
            ))
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
#[cfg(feature = "tunnel")]
pub mod tun;

#[cfg(feature = "tunnel")]
pub use handshake::TunnelInfo;
#[cfg(not(feature = "tunnel"))]
#[derive(Debug, Clone)]
pub struct TunnelInfo {
    pub server_address: String,
    pub server_rsd_port: u16,
    pub client_address: String,
    pub client_mtu: u32,
}
pub use manager::{TunMode, TunnelHandle, TunnelManager};

/// Errors from tunnel operations.
#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("TUN device error: {0}")]
    TunDevice(String),
}

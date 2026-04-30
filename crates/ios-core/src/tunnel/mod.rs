//! CDTunnel handshake, TUN device abstraction, and packet forwarding.

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

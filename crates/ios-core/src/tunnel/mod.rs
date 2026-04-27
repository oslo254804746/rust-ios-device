//! CDTunnel handshake, TUN device abstraction, and packet forwarding.

pub mod forward;
pub mod handshake;
pub mod manager;
pub mod tun;

pub use handshake::TunnelInfo;
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

//! ios-xpc: XPC binary protocol over HTTP/2 for iOS 17+ service connections.
//!
//! Architecture:
//!   h2_raw   – minimal raw HTTP/2 framer (streams 1 + 3)
//!   message  – XPC binary message encode/decode
//!   rsd      – RSD handshake (service discovery)
//!   client   – High-level XpcClient

pub mod client;
pub mod h2_raw;
pub mod message;
pub mod rsd;

// Keep the old codec as a compatibility shim (still compiles)
pub mod codec;

pub use client::XpcClient;
pub use message::{XpcMessage, XpcValue};
pub use rsd::{RsdHandshake, ServiceDescriptor};

/// Errors from XPC operations.
#[derive(Debug, thiserror::Error)]
pub enum XpcError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not connected")]
    NotConnected,
    #[error("service not found: {0}")]
    ServiceNotFound(String),
    #[error("TLS / protocol error: {0}")]
    Tls(String),
}

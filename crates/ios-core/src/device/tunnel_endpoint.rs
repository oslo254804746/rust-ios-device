//! A routable endpoint inside an established CoreDevice tunnel.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::str::FromStr;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::error::CoreError;

/// Network path used to reach any port exposed through an RSD tunnel.
///
/// Userspace TUN routes use the go-ios-compatible 20-byte prelude: a 16-byte
/// IPv6 destination followed by a little-endian `u32` port. Kernel TUN routes
/// connect directly to the IPv6 destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TunnelEndpoint {
    UserspaceProxy {
        proxy_port: u16,
        remote_addr: Ipv6Addr,
    },
    DirectIpv6 {
        remote_addr: Ipv6Addr,
    },
}

impl TunnelEndpoint {
    pub(super) fn resolve(
        server_addr: &str,
        userspace_port: Option<u16>,
    ) -> Result<Self, CoreError> {
        let remote_addr = Ipv6Addr::from_str(server_addr)
            .map_err(|e| CoreError::Protocol(format!("invalid IPv6 addr: {e}")))?;

        Ok(match userspace_port {
            Some(proxy_port) => Self::UserspaceProxy {
                proxy_port,
                remote_addr,
            },
            None => Self::DirectIpv6 { remote_addr },
        })
    }

    /// Open a stream to `port` over this tunnel path.
    pub(super) async fn connect(self, port: u16) -> Result<TcpStream, CoreError> {
        match self {
            Self::UserspaceProxy {
                proxy_port,
                remote_addr,
            } => {
                let mut proxy = TcpStream::connect(("127.0.0.1", proxy_port)).await?;
                proxy.write_all(&remote_addr.octets()).await?;
                proxy.write_all(&(port as u32).to_le_bytes()).await?;
                proxy.flush().await?;
                Ok(proxy)
            }
            Self::DirectIpv6 { remote_addr } => {
                let addr = SocketAddr::V6(SocketAddrV6::new(remote_addr, port, 0, 0));
                Ok(TcpStream::connect(addr).await?)
            }
        }
    }
}

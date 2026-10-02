//! A routable endpoint inside an established CoreDevice tunnel.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::str::FromStr;
use std::{future::Future, io, time::Duration};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::error::CoreError;
use crate::tunnel::TUNNEL_CONNECT_TIMEOUT;

async fn timeout_io<T>(
    duration: Duration,
    operation: impl Future<Output = io::Result<T>>,
    context: &'static str,
) -> io::Result<T> {
    tokio::time::timeout(duration, operation)
        .await
        .map_err(|elapsed| {
            super::error_context::retained_io(io::ErrorKind::TimedOut, context, elapsed)
        })?
}

/// Network path used to reach any port exposed through an RSD tunnel.
///
/// Dial/prelude errors cross the XPC boundary through connection_error_to_xpc.
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

/// Retain native dial/prelude sources across the existing XPC error boundary.
pub(super) fn connection_error_to_xpc(error: CoreError) -> crate::XpcError {
    match error {
        CoreError::Io(error) => crate::XpcError::Io(error),
        other => crate::XpcError::Io(io::Error::other(other)),
    }
}

impl TunnelEndpoint {
    pub(super) fn resolve(
        server_addr: &str,
        userspace_port: Option<u16>,
    ) -> Result<Self, CoreError> {
        let remote_addr = Ipv6Addr::from_str(server_addr).map_err(|error| {
            super::error_context::with_kind(
                io::ErrorKind::InvalidData,
                "parse tunnel IPv6 address",
                error,
            )
        })?;

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
                let mut proxy = timeout_io(
                    TUNNEL_CONNECT_TIMEOUT,
                    TcpStream::connect(("127.0.0.1", proxy_port)),
                    "userspace tunnel proxy dial timed out",
                )
                .await?;
                proxy.write_all(&remote_addr.octets()).await?;
                proxy.write_all(&(port as u32).to_le_bytes()).await?;
                proxy.flush().await?;
                Ok(proxy)
            }
            Self::DirectIpv6 { remote_addr } => {
                let addr = SocketAddr::V6(SocketAddrV6::new(remote_addr, port, 0, 0));
                Ok(timeout_io(
                    TUNNEL_CONNECT_TIMEOUT,
                    TcpStream::connect(addr),
                    "tunnel service dial timed out",
                )
                .await?)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xpc_connection_adapter_retains_native_io_and_other_sources() {
        for (kind, class) in [
            (io::ErrorKind::TimedOut, crate::ErrorClass::Timeout),
            (
                io::ErrorKind::PermissionDenied,
                crate::ErrorClass::PermissionDenied,
            ),
            (io::ErrorKind::ConnectionRefused, crate::ErrorClass::Io),
        ] {
            let error = connection_error_to_xpc(CoreError::Io(io::Error::new(
                kind,
                "synthetic-private-dial",
            )));
            assert_eq!(crate::classify_error(&error), class);
            let crate::XpcError::Io(source) = &error else {
                panic!("native IO lost")
            };
            assert_eq!(source.kind(), kind);
            assert_eq!(source.to_string(), "synthetic-private-dial");
            assert!(!format!("{error:?}").contains("synthetic-private"));
        }
        let error =
            connection_error_to_xpc(CoreError::Protocol("synthetic-private-prelude".into()));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        let crate::XpcError::Io(source) = error else {
            panic!("prelude source lost")
        };
        assert!(source
            .get_ref()
            .unwrap()
            .downcast_ref::<CoreError>()
            .is_some());
    }

    #[tokio::test]
    async fn timeout_io_classifies_elapsed_dials() {
        let err = timeout_io(
            Duration::from_millis(1),
            std::future::pending::<io::Result<()>>(),
            "test dial timed out",
        )
        .await
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(err.to_string().starts_with("test dial timed out: "));
        assert!(std::error::Error::source(&err)
            .unwrap()
            .is::<tokio::time::error::Elapsed>());
    }
}

#[cfg(test)]
mod routing_tests {
    use super::*;

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint_uses_userspace_proxy_when_available() {
        let endpoint = TunnelEndpoint::resolve("fd00::1", Some(60105)).expect("valid endpoint");

        assert_eq!(
            endpoint,
            TunnelEndpoint::UserspaceProxy {
                proxy_port: 60105,
                remote_addr: Ipv6Addr::from_str("fd00::1").expect("valid IPv6"),
            }
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint_falls_back_to_direct_ipv6() {
        let endpoint = TunnelEndpoint::resolve("fd00::2", None).expect("valid endpoint");

        assert_eq!(
            endpoint,
            TunnelEndpoint::DirectIpv6 {
                remote_addr: Ipv6Addr::from_str("fd00::2").expect("valid IPv6"),
            }
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint_rejects_invalid_ipv6() {
        let err = TunnelEndpoint::resolve("not-an-ipv6", Some(60105))
            .expect_err("invalid IPv6 should fail");

        assert_eq!(err.class(), crate::ErrorClass::Protocol);
        let CoreError::Io(outer) = err else {
            panic!("address parser cause lost");
        };
        assert!(std::error::Error::source(&outer)
            .unwrap()
            .is::<std::net::AddrParseError>());
    }

    #[tokio::test]
    #[cfg(feature = "tunnel")]
    async fn userspace_tunnel_endpoint_writes_go_ios_routing_header() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy listener");
        let proxy_port = listener.local_addr().expect("listener address").port();
        let remote_addr = Ipv6Addr::from_str("fd00::42").expect("valid IPv6");
        let endpoint = TunnelEndpoint::UserspaceProxy {
            proxy_port,
            remote_addr,
        };

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept proxy connection");
            let mut header = [0u8; 20];
            stream
                .read_exact(&mut header)
                .await
                .expect("read routing header");
            header
        });

        let _stream = endpoint
            .connect(62000)
            .await
            .expect("connect through proxy");
        let header = server.await.expect("join proxy task");

        assert_eq!(&header[..16], &remote_addr.octets());
        assert_eq!(u32::from_le_bytes(header[16..].try_into().unwrap()), 62000);
    }
}

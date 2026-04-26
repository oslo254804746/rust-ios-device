use std::io;

use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;

/// Resolve the usbmuxd socket address from the environment or platform defaults.
pub fn usbmuxd_socket_address() -> String {
    let env = std::env::var("USBMUXD_SOCKET_ADDRESS").ok();
    socket_address_from_env(env.as_deref(), cfg!(windows))
}

pub(crate) fn socket_address_from_env(env: Option<&str>, is_windows: bool) -> String {
    if let Some(addr) = env {
        if addr.contains(':') {
            return format!("tcp://{addr}");
        } else {
            return format!("unix://{addr}");
        }
    }
    if is_windows {
        "tcp://127.0.0.1:27015".to_string()
    } else {
        "unix:///var/run/usbmuxd".to_string()
    }
}

/// Unified async stream over TCP or Unix socket.
pub enum UsbmuxStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl UsbmuxStream {
    pub async fn connect(addr: &str) -> io::Result<Self> {
        if let Some(tcp_addr) = addr.strip_prefix("tcp://") {
            Ok(Self::Tcp(TcpStream::connect(tcp_addr).await?))
        } else if let Some(path) = addr.strip_prefix("unix://") {
            #[cfg(unix)]
            return Ok(Self::Unix(UnixStream::connect(path).await?));
            #[cfg(not(unix))]
            {
                let _ = path;
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Unix sockets not supported on this platform",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown scheme: {addr}"),
            ))
        }
    }
}

impl tokio::io::AsyncRead for UsbmuxStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl tokio::io::AsyncWrite for UsbmuxStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_addr_windows_default() {
        let addr = socket_address_from_env(None, true);
        assert_eq!(addr, "tcp://127.0.0.1:27015");
    }

    #[test]
    fn test_socket_addr_env_override() {
        let addr = socket_address_from_env(Some("192.168.1.1:27015"), false);
        assert_eq!(addr, "tcp://192.168.1.1:27015");
    }

    #[test]
    fn test_socket_addr_env_unix() {
        let addr = socket_address_from_env(Some("/tmp/usbmuxd.sock"), false);
        assert_eq!(addr, "unix:///tmp/usbmuxd.sock");
    }
}

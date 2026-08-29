use std::future::Future;
use std::io;
use std::time::Duration;

use tokio::net::TcpStream;
#[cfg(unix)]
use tokio::net::UnixStream;

/// A usbmuxd TCP endpoint is local control-plane infrastructure; do not let a
/// black-holed address hold every caller forever. Unix sockets intentionally
/// retain their previous unbounded connect semantics below.
const USBMUXD_TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Resolve the usbmuxd socket address from the environment or platform defaults.
pub fn usbmuxd_socket_address() -> String {
    let env = std::env::var("USBMUXD_SOCKET_ADDRESS").ok();
    socket_address_from_env(env.as_deref(), cfg!(windows))
}

pub(crate) fn socket_address_from_env(env: Option<&str>, is_windows: bool) -> String {
    if let Some(addr) = env {
        if addr.starts_with("tcp://") || addr.starts_with("unix://") {
            return addr.to_string();
        } else if addr.contains(':') {
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
            Ok(Self::Tcp(
                connect_tcp_with_timeout(
                    addr,
                    USBMUXD_TCP_CONNECT_TIMEOUT,
                    TcpStream::connect(tcp_addr),
                )
                .await?,
            ))
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

async fn connect_tcp_with_timeout<F>(
    address: &str,
    timeout: Duration,
    connector: F,
) -> io::Result<TcpStream>
where
    F: Future<Output = io::Result<TcpStream>>,
{
    match tokio::time::timeout(timeout, connector).await {
        Ok(Ok(stream)) => Ok(stream),
        Ok(Err(error)) => Err(io::Error::new(
            error.kind(),
            format!("usbmuxd TCP connect to {address} failed: {error}"),
        )),
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "usbmuxd TCP connect to {address} timed out after {} ms",
                timeout.as_millis()
            ),
        )),
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
    fn test_socket_addr_explicit_schemes_are_not_rewritten() {
        assert_eq!(
            socket_address_from_env(Some("tcp://192.168.1.1:27015"), false),
            "tcp://192.168.1.1:27015"
        );
        assert_eq!(
            socket_address_from_env(Some("unix:///tmp/usbmuxd.sock"), true),
            "unix:///tmp/usbmuxd.sock"
        );
    }

    #[test]
    fn test_socket_addr_env_unix() {
        let addr = socket_address_from_env(Some("/tmp/usbmuxd.sock"), false);
        assert_eq!(addr, "unix:///tmp/usbmuxd.sock");
    }

    #[tokio::test]
    async fn test_tcp_connect_timeout_is_bounded_and_names_address() {
        let pending = std::future::pending::<io::Result<TcpStream>>();
        let error =
            connect_tcp_with_timeout("198.51.100.7:27015", Duration::from_millis(5), pending)
                .await
                .expect_err("injected black-hole connector should time out");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("198.51.100.7:27015"));
        assert!(error.to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_tcp_connect_error_names_address() {
        let connector = async {
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "injected refusal",
            ))
        };
        let error = connect_tcp_with_timeout("127.0.0.1:27015", Duration::from_secs(1), connector)
            .await
            .expect_err("injected connector should fail");

        assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
        assert!(error.to_string().contains("127.0.0.1:27015"));
        assert!(error.to_string().contains("injected refusal"));
    }
}

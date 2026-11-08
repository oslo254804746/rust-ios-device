//! Owned native-source and transport fixtures for crate unit tests.

#[cfg(any(
    feature = "display",
    feature = "hid",
    feature = "cryptex",
    feature = "pasteboard"
))]
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
#[cfg(any(
    feature = "display",
    feature = "hid",
    feature = "cryptex",
    feature = "pasteboard"
))]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) fn has_source<T: std::error::Error + 'static>(
    error: &(dyn std::error::Error + 'static),
) -> bool {
    let mut source = Some(error);
    for _ in 0..32 {
        let Some(error) = source else { break };
        if error.is::<T>() {
            return true;
        }
        source = error.source();
    }
    false
}

/// Initialize the actual XPC client over an owned peer, then allow tests to
/// stall its writes without changing ambient sockets or production timers.
#[cfg(any(
    feature = "display",
    feature = "hid",
    feature = "cryptex",
    feature = "pasteboard"
))]
pub(crate) async fn connected_xpc_pair() -> (
    crate::XpcClient,
    tokio::io::DuplexStream,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    struct Gated {
        stream: tokio::io::DuplexStream,
        open: Arc<AtomicBool>,
    }
    impl AsyncRead for Gated {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }
    impl AsyncWrite for Gated {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            if !self.open.load(Ordering::SeqCst) {
                return Poll::Pending;
            }
            Pin::new(&mut self.stream).poll_write(cx, buf)
        }
        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.stream).poll_flush(cx)
        }
        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.stream).poll_shutdown(cx)
        }
    }
    let (stream, mut peer) = tokio::io::duplex(64 * 1024);
    let open = Arc::new(AtomicBool::new(true));
    let connect = crate::XpcClient::connect_stream(Gated {
        stream,
        open: Arc::clone(&open),
    });
    let bootstrap = async {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        peer.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0]).await.unwrap();
        let empty = crate::encode_xpc_message(&crate::XpcMessage {
            flags: crate::xpc_message_flags::ALWAYS_SET,
            msg_id: 0,
            body: None,
        })
        .unwrap();
        let mut acknowledged = 0;
        while acknowledged < 3 {
            let mut header = [0; 9];
            peer.read_exact(&mut header).await.unwrap();
            let size = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
            assert!(size <= 64 * 1024);
            let mut payload = vec![0; size];
            peer.read_exact(&mut payload).await.unwrap();
            if header[3] != 0 {
                continue;
            }
            assert!(crate::decode_xpc_message(payload.into()).is_ok());
            header[..3].copy_from_slice(&(empty.len() as u32).to_be_bytes()[1..]);
            header[4] = 0;
            peer.write_all(&header).await.unwrap();
            peer.write_all(&empty).await.unwrap();
            acknowledged += 1;
        }
    };
    let (client, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(connect, bootstrap)
    })
    .await
    .expect("owned bootstrap must finish");
    (client.unwrap(), peer, open)
}

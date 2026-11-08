use super::*;
use std::task::Context;
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

#[derive(Clone, Copy, Default)]
pub(super) enum Behavior {
    #[default]
    Normal,
    Failure(io::ErrorKind),
    SlowDrop(Duration),
}

#[derive(Debug)]
pub(super) struct Marker(pub(super) Arc<()>);
impl std::fmt::Display for Marker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("owned native packet reader marker")
    }
}
impl Error for Marker {}

#[derive(Default)]
pub(super) struct Probe {
    pub(super) reads: AtomicUsize,
    pub(super) drops: AtomicUsize,
    pub(super) marker: Arc<()>,
}

struct Stream {
    inner: DuplexStream,
    behavior: Behavior,
    probe: Arc<Probe>,
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.probe.reads.fetch_add(1, Ordering::SeqCst);
        if let Behavior::Failure(kind) = self.behavior {
            self.behavior = Behavior::Normal;
            return Poll::Ready(Err(io::Error::new(kind, Marker(self.probe.marker.clone()))));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        if let Behavior::SlowDrop(delay) = self.behavior {
            std::thread::sleep(delay);
        }
        self.probe.drops.fetch_add(1, Ordering::SeqCst);
    }
}

pub(super) async fn start(behavior: Behavior) -> (Arc<TunnelHandle>, DuplexStream, Arc<Probe>) {
    let (inner, peer) = tokio::io::duplex(1280);
    let probe = Arc::new(Probe::default());
    let runtime = ios_core::UserspaceTunDevice::start(
        "fd00::2",
        "fd00::1",
        1280,
        Stream {
            inner,
            behavior,
            probe: probe.clone(),
        },
    )
    .await
    .unwrap();
    let handle = Arc::new(TunnelHandle::new_userspace(
        UDID.into(),
        ios_core::TunnelInfo {
            server_address: "fd00::1".into(),
            server_rsd_port: 58783,
            client_address: "fd00::2".into(),
            client_mtu: 1280,
        },
        runtime,
    ));
    (handle, peer, probe)
}

pub(super) fn complete(result: &std::result::Result<(), ios_core::TunnelError>) -> bool {
    result.as_ref().map_or_else(
        |error| original_receipt(error).is_some_and(|receipt| receipt.joins_complete()),
        |_| true,
    )
}

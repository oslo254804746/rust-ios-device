use super::*;
use crate::xpc::raw_storage::MutableBytes;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::AsyncReadExt;

enum Fault {
    None,
    Prefix(Option<std::io::Error>),
    Zero,
}
struct Observed<S> {
    inner: S,
    start: usize,
    len: usize,
    offered: Arc<AtomicBool>,
    calls: Arc<AtomicUsize>,
    fault: Fault,
    prefix_written: bool,
}
impl<S: AsyncRead + Unpin> AsyncRead for Observed<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buffer)
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Observed<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, bytes)
    }
    fn is_write_vectored(&self) -> bool {
        true
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        this.calls.fetch_add(1, Ordering::SeqCst);
        if buffers.len() == 2 && !buffers[1].is_empty() {
            let pointer = buffers[1].as_ptr() as usize;
            assert!(
                pointer >= this.start && pointer + buffers[1].len() <= this.start + this.len,
                "vectored payload is not the original owner"
            );
            if buffers[0].len() == 9 && buffers[1].len() == this.len {
                this.offered.store(true, Ordering::SeqCst);
            }
        }
        match &mut this.fault {
            Fault::None => Pin::new(&mut this.inner).poll_write_vectored(cx, buffers),
            Fault::Zero => Poll::Ready(Ok(0)),
            Fault::Prefix(error) => {
                if this.prefix_written {
                    return Poll::Ready(Err(error.take().expect("original error consumed twice")));
                }
                let result = Pin::new(&mut this.inner).poll_write(cx, &buffers[0][..4]);
                if matches!(result,Poll::Ready(Ok(n))if n>0) {
                    this.prefix_written = true;
                }
                result
            }
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
fn framer<S: AsyncRead + AsyncWrite + Unpin>(
    inner: S,
    payload: &MutableBytes,
    bank: &Arc<Semaphore>,
    fault: Fault,
) -> (H2Framer<Observed<S>>, Arc<AtomicBool>, Arc<AtomicUsize>) {
    let offered = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut framer = H2Framer::new(Observed {
        inner,
        start: payload.bytes.as_ptr() as usize,
        len: payload.bytes.len(),
        offered: Arc::clone(&offered),
        calls: Arc::clone(&calls),
        fault,
        prefix_written: false,
    });
    framer.configure_bank(Some(bank));
    framer.client_server_open = true;
    framer.outbound_stream_windows.prepare_keys(&[1]).unwrap();
    framer.outbound_stream_windows.insert_reserved(1, 65535);
    (framer, offered, calls)
}
fn payload(bank: &Arc<Semaphore>) -> MutableBytes {
    let mut bytes = MutableBytes::zeroed(bank, 64).unwrap();
    bytes.bytes.fill(0x5a);
    bytes
}

#[tokio::test]
async fn actual_localhost_tcp_gets_original_header_and_payload_slices_together() {
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let payload = payload(&bank);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    drop(listener);
    assert!(stream.is_write_vectored());
    let (mut framer, offered, calls) = framer(stream, &payload, &bank, Fault::None);
    assert!(framer.stream.is_write_vectored());
    framer.write_stream(1, &payload.bytes).await.unwrap();
    assert!(offered.load(Ordering::SeqCst));
    assert!(calls.load(Ordering::SeqCst) > 0);
    let mut bytes = [0; 73];
    peer.read_exact(&mut bytes).await.unwrap();
    assert_eq!(bytes, build_data_frame(1, &[0x5a; 64])[..]);
    assert_eq!(
        bank.available_permits(),
        quota - 64 - framer.outbound_stream_windows.backing_bytes()
    );
    drop(framer);
    let mut byte = [0];
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), peer.read(&mut byte)).await,
        Ok(Ok(0))
    ));
    assert_eq!(bank.available_permits(), quota - 64);
    drop(payload);
    assert_eq!(bank.available_permits(), quota);
}

#[tokio::test]
async fn vectored_partial_header_keeps_original_native_error_pointer_and_strict_eof() {
    #[derive(Debug)]
    struct Native(u32);
    impl std::fmt::Display for Native {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("native vectored send fixture")
        }
    }
    impl std::error::Error for Native {}
    let native = std::io::Error::new(std::io::ErrorKind::BrokenPipe, Native(87));
    let pointer = native.get_ref().unwrap().downcast_ref::<Native>().unwrap() as *const Native;
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let payload = payload(&bank);
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (mut framer, offered, calls) = framer(stream, &payload, &bank, Fault::Prefix(Some(native)));
    let error = framer.write_stream(1, &payload.bytes).await.unwrap_err();
    assert!(offered.load(Ordering::SeqCst));
    assert!(calls.load(Ordering::SeqCst) >= 2);
    assert!(!framer.is_usable());
    let H2Error::Io(original) = error else {
        panic!("original vector IO lost");
    };
    assert_eq!(original.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(
        original
            .get_ref()
            .unwrap()
            .downcast_ref::<Native>()
            .unwrap()
            .0,
        87
    );
    assert_eq!(
        original
            .get_ref()
            .unwrap()
            .downcast_ref::<Native>()
            .unwrap() as *const Native,
        pointer
    );
    drop(framer);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert_eq!(bytes, frame_header(FRAME_DATA, 0, 1, 64)[..4]);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
    assert_eq!(bank.available_permits(), quota - 64);
    drop(payload);
    assert_eq!(bank.available_permits(), quota);
    assert_eq!(
        original
            .get_ref()
            .unwrap()
            .downcast_ref::<Native>()
            .unwrap() as *const Native,
        pointer
    );
}

#[tokio::test]
async fn vectored_zero_write_is_original_standard_writezero_and_never_false_success() {
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let payload = payload(&bank);
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (mut framer, offered, _) = framer(stream, &payload, &bank, Fault::Zero);
    let error = framer.write_stream(1, &payload.bytes).await.unwrap_err();
    assert!(offered.load(Ordering::SeqCst));
    assert!(!framer.is_usable());
    let H2Error::Io(original) = error else {
        panic!("writezero IO lost");
    };
    let standard = std::io::Error::from(std::io::ErrorKind::WriteZero);
    assert_eq!(original.kind(), standard.kind());
    assert_eq!(original.to_string(), standard.to_string());
    drop(framer);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
    assert_eq!(bank.available_permits(), quota - 64);
    drop(payload);
    assert_eq!(bank.available_permits(), quota);
}

use super::*;
use crate::xpc::raw_storage::MutableBytes;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::AsyncReadExt;

struct Probe {
    inner: tokio::io::DuplexStream,
    bank: Arc<Semaphore>,
    quota: usize,
    start: usize,
    len: usize,
    seen: Arc<AtomicBool>,
    drops: Arc<AtomicUsize>,
    failure: Option<std::io::Error>,
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl AsyncRead for Probe {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buffer)
    }
}
impl AsyncWrite for Probe {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();
        if !bytes.is_empty() && bytes.iter().all(|byte| *byte == 0x5a) {
            let pointer = bytes.as_ptr() as usize;
            assert!(
                pointer >= this.start && pointer + bytes.len() <= this.start + this.len,
                "metered DATA copied away from original buffer"
            );
            assert!(
                this.bank.available_permits() <= this.quota - this.len,
                "original payload grant returned while writing"
            );
            this.seen.store(true, Ordering::SeqCst);
            if let Some(original) = this.failure.take() {
                return Poll::Ready(Err(original));
            }
        }
        Pin::new(&mut this.inner).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
fn fixture(
    capacity: usize,
    failure: Option<std::io::Error>,
) -> (
    H2Framer<Probe>,
    tokio::io::DuplexStream,
    MutableBytes,
    Arc<Semaphore>,
    Arc<AtomicBool>,
    Arc<AtomicUsize>,
) {
    let quota = 16 * 1024 * 1024;
    let bank = Arc::new(Semaphore::new(quota));
    let mut payload = MutableBytes::zeroed(&bank, 64).unwrap();
    payload.bytes.fill(0x5a);
    let (inner, peer) = tokio::io::duplex(capacity);
    let seen = Arc::new(AtomicBool::new(false));
    let drops = Arc::new(AtomicUsize::new(0));
    let mut framer = H2Framer::new(Probe {
        inner,
        bank: Arc::clone(&bank),
        quota,
        start: payload.bytes.as_ptr() as usize,
        len: 64,
        seen: Arc::clone(&seen),
        drops: Arc::clone(&drops),
        failure,
    });
    framer.configure_bank(Some(&bank));
    framer.client_server_open = true;
    framer.outbound_stream_windows.prepare_keys(&[1]).unwrap();
    framer.outbound_stream_windows.insert_reserved(1, 65535);
    (framer, peer, payload, bank, seen, drops)
}

#[test]
fn inline_control_and_frame_headers_match_original_builders() {
    assert_eq!(
        initial_settings_bytes(),
        build_settings_frame(&[
            (SETTINGS_MAX_CONCURRENT_STREAMS, 100),
            (SETTINGS_INITIAL_WINDOW_SIZE, INITIAL_WINDOW_SIZE)
        ])[..]
    );
    for kind in [
        FRAME_DATA,
        FRAME_HEADERS,
        FRAME_SETTINGS,
        FRAME_WINDOW_UPDATE,
    ] {
        for flags in [0, 1, 4, 255] {
            for id in [0, 1, 3, u32::MAX] {
                for length in [0, 1, 64, MAX_FRAME_PAYLOAD] {
                    let bytes = vec![7; length];
                    assert_eq!(
                        frame_header(kind, flags, id, length),
                        build_frame(kind, flags, id, &bytes)[..9]
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn actual_partial_data_write_borrows_original_pointer_and_keeps_owner_after_write_join() {
    let (mut framer, mut peer, payload, bank, seen, drops) = fixture(16, None);
    let quota = 16 * 1024 * 1024;
    let task = tokio::spawn(async move {
        let result = framer.write_stream(1, &payload.bytes).await;
        (framer, payload, result)
    });
    let mut frame = vec![0; 73];
    peer.read_exact(&mut frame).await.unwrap();
    assert_eq!(frame, build_data_frame(1, &[0x5a; 64]));
    let (framer, payload, result) = task.await.unwrap();
    result.unwrap();
    assert!(seen.load(Ordering::SeqCst));
    assert_eq!(
        bank.available_permits(),
        quota - 64 - framer.outbound_stream_windows.backing_bytes()
    );
    drop(framer);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
    assert_eq!(bank.available_permits(), quota - 64);
    drop(payload);
    assert_eq!(bank.available_permits(), quota);
}

#[tokio::test]
async fn original_payload_write_error_keeps_typed_native_pointer_after_header_progress_and_close() {
    #[derive(Debug)]
    struct Native(u32);
    impl std::fmt::Display for Native {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("original native send fixture")
        }
    }
    impl std::error::Error for Native {}
    let original = std::io::Error::new(std::io::ErrorKind::BrokenPipe, Native(93));
    let pointer = original
        .get_ref()
        .unwrap()
        .downcast_ref::<Native>()
        .unwrap() as *const Native;
    let (mut framer, mut peer, payload, bank, seen, drops) = fixture(128, Some(original));
    let quota = 16 * 1024 * 1024;
    let error = framer.write_stream(1, &payload.bytes).await.unwrap_err();
    assert!(!framer.is_usable());
    assert!(seen.load(Ordering::SeqCst));
    let H2Error::Io(original) = error else {
        panic!("native send IO lost");
    };
    assert_eq!(original.kind(), std::io::ErrorKind::BrokenPipe);
    assert_eq!(
        original
            .get_ref()
            .unwrap()
            .downcast_ref::<Native>()
            .unwrap()
            .0,
        93
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
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let mut wire = Vec::new();
    peer.read_to_end(&mut wire).await.unwrap();
    assert_eq!(wire, frame_header(FRAME_DATA, 0, 1, 64));
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
async fn original_deadline_after_partial_send_invalidates_and_refunds_future_before_payload_owner()
{
    let (mut framer, mut peer, payload, bank, seen, drops) = fixture(16, None);
    let quota = 16 * 1024 * 1024;
    framer.set_operation_timeout(Some(Duration::from_millis(5)));
    let error = framer.write_stream(1, &payload.bytes).await.unwrap_err();
    assert!(!framer.is_usable());
    assert!(seen.load(Ordering::SeqCst));
    let H2Error::Io(original) = error else {
        panic!("original elapsed IO lost");
    };
    assert!(original.get_ref().unwrap().is::<Elapsed>());
    assert_eq!(original.kind(), std::io::ErrorKind::TimedOut);
    drop(framer);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let mut wire = Vec::new();
    peer.read_to_end(&mut wire).await.unwrap();
    assert!(wire.len() > 9 && wire.len() < 73);
    assert_eq!(wire, build_data_frame(1, &[0x5a; 64])[..wire.len()]);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
    assert_eq!(bank.available_permits(), quota - 64);
    drop(payload);
    assert_eq!(bank.available_permits(), quota);
}

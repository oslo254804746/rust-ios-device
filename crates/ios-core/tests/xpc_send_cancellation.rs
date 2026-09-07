#![cfg(feature = "tunnel")]

use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use ios_core::{encode_xpc_message, xpc_message_flags as flags, XpcClient, XpcMessage, XpcValue};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration};

const FRAME_DATA: u8 = 0x00;
const FRAME_HEADERS: u8 = 0x01;
const FRAME_SETTINGS: u8 = 0x04;
const FRAME_WINDOW_UPDATE: u8 = 0x08;
const FLAG_SETTINGS_ACK: u8 = 0x01;
const STREAM_INIT: u32 = 0;
const STREAM_CLIENT_SERVER: u32 = 1;
const STREAM_SERVER_CLIENT: u32 = 3;
const MAX_FRAME_PAYLOAD: usize = 16_384;

fn build_frame(frame_type: u8, frame_flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let len = payload.len();
    let mut frame = Vec::with_capacity(9 + len);
    frame.push(((len >> 16) & 0xff) as u8);
    frame.push(((len >> 8) & 0xff) as u8);
    frame.push((len & 0xff) as u8);
    frame.push(frame_type);
    frame.push(frame_flags);
    frame.extend_from_slice(&(stream_id & 0x7fff_ffff).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn settings_frame() -> Vec<u8> {
    build_frame(FRAME_SETTINGS, 0, STREAM_INIT, &[])
}

fn settings_ack_frame() -> Vec<u8> {
    build_frame(FRAME_SETTINGS, FLAG_SETTINGS_ACK, STREAM_INIT, &[])
}

fn data_frame(stream_id: u32, payload: &[u8]) -> Vec<u8> {
    build_frame(FRAME_DATA, 0, stream_id, payload)
}

fn empty_xpc_message() -> Vec<u8> {
    encode_xpc_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })
    .expect("empty XPC response should encode")
    .to_vec()
}

struct WireFrame {
    frame_type: u8,
    stream_id: u32,
}

async fn read_frame(stream: &mut DuplexStream) -> io::Result<WireFrame> {
    let mut header = [0u8; 9];
    stream.read_exact(&mut header).await?;
    let len = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    Ok(WireFrame {
        frame_type: header[3],
        stream_id: u32::from_be_bytes([header[5] & 0x7f, header[6], header[7], header[8]]),
    })
}

async fn serve_initialization(mut server: DuplexStream, done_rx: oneshot::Receiver<()>) {
    let mut preface = [0u8; 24];
    server.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");

    let mut settings = [0u8; 21];
    server.read_exact(&mut settings).await.unwrap();
    assert_eq!(settings[3], FRAME_SETTINGS);

    let mut window_update = [0u8; 13];
    server.read_exact(&mut window_update).await.unwrap();
    assert_eq!(window_update[3], FRAME_WINDOW_UPDATE);

    server.write_all(&settings_frame()).await.unwrap();
    server.flush().await.unwrap();

    let mut ack = [0u8; 9];
    server.read_exact(&mut ack).await.unwrap();
    assert_eq!(&ack, settings_ack_frame().as_slice());

    let frame = read_frame(&mut server).await.unwrap();
    assert_eq!(
        (frame.frame_type, frame.stream_id),
        (FRAME_HEADERS, STREAM_CLIENT_SERVER)
    );
    let frame = read_frame(&mut server).await.unwrap();
    assert_eq!(
        (frame.frame_type, frame.stream_id),
        (FRAME_DATA, STREAM_CLIENT_SERVER)
    );
    server
        .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty_xpc_message()))
        .await
        .unwrap();
    server.flush().await.unwrap();

    let frame = read_frame(&mut server).await.unwrap();
    assert_eq!(
        (frame.frame_type, frame.stream_id),
        (FRAME_HEADERS, STREAM_SERVER_CLIENT)
    );
    let frame = read_frame(&mut server).await.unwrap();
    assert_eq!(
        (frame.frame_type, frame.stream_id),
        (FRAME_DATA, STREAM_CLIENT_SERVER)
    );
    server
        .write_all(&data_frame(STREAM_CLIENT_SERVER, &empty_xpc_message()))
        .await
        .unwrap();
    server.flush().await.unwrap();

    let frame = read_frame(&mut server).await.unwrap();
    assert_eq!(
        (frame.frame_type, frame.stream_id),
        (FRAME_DATA, STREAM_SERVER_CLIENT)
    );
    server
        .write_all(&data_frame(STREAM_SERVER_CLIENT, &empty_xpc_message()))
        .await
        .unwrap();
    server.flush().await.unwrap();

    // Keep the peer alive while the test cancels the blocked multi-frame send.
    let _ = done_rx.await;
}

struct WriteGate {
    armed: AtomicBool,
    released: AtomicBool,
    data_frames_seen: AtomicUsize,
    first_data_complete: AtomicBool,
    bytes_written: AtomicUsize,
    blocked_tx: Mutex<Option<oneshot::Sender<()>>>,
    blocked_waker: Mutex<Option<Waker>>,
}

impl WriteGate {
    fn new() -> (Arc<Self>, oneshot::Receiver<()>) {
        let (blocked_tx, blocked_rx) = oneshot::channel();
        (
            Arc::new(Self {
                armed: AtomicBool::new(false),
                released: AtomicBool::new(false),
                data_frames_seen: AtomicUsize::new(0),
                first_data_complete: AtomicBool::new(false),
                bytes_written: AtomicUsize::new(0),
                blocked_tx: Mutex::new(Some(blocked_tx)),
                blocked_waker: Mutex::new(None),
            }),
            blocked_rx,
        )
    }

    fn arm(&self) {
        self.data_frames_seen.store(0, Ordering::SeqCst);
        self.first_data_complete.store(false, Ordering::SeqCst);
        self.released.store(false, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        if let Some(waker) = self.blocked_waker.lock().unwrap().take() {
            waker.wake();
        }
    }

    fn bytes_written(&self) -> usize {
        self.bytes_written.load(Ordering::SeqCst)
    }
}

struct GatedIo<S> {
    inner: S,
    gate: Arc<WriteGate>,
}

fn is_full_client_data_frame(buf: &[u8]) -> bool {
    if buf.len() < 9 || buf[3] != FRAME_DATA {
        return false;
    }
    let len = ((buf[0] as usize) << 16) | ((buf[1] as usize) << 8) | buf[2] as usize;
    let stream_id = u32::from_be_bytes([buf[5] & 0x7f, buf[6], buf[7], buf[8]]);
    stream_id == STREAM_CLIENT_SERVER && len == buf.len() - 9
}

impl<S: AsyncRead + Unpin> AsyncRead for GatedIo<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for GatedIo<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let is_data = self.gate.armed.load(Ordering::SeqCst) && is_full_client_data_frame(buf);
        let data_index = is_data.then(|| self.gate.data_frames_seen.fetch_add(1, Ordering::SeqCst));

        if data_index.is_some_and(|index| index >= 1) && !self.gate.released.load(Ordering::SeqCst)
        {
            if let Some(tx) = self.gate.blocked_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            *self.gate.blocked_waker.lock().unwrap() = Some(cx.waker().clone());
            return Poll::Pending;
        }

        let result = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = result {
            self.gate.bytes_written.fetch_add(written, Ordering::SeqCst);
            if data_index == Some(0) && written == buf.len() {
                self.gate.first_data_complete.store(true, Ordering::SeqCst);
            }
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn assert_requires_reconnect(error: ios_core::XpcError) {
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("re-establish")
            || message.contains("reconnect")
            || message.contains("frame-aligned")
            || message.contains("cancel"),
        "expected a reconnect/poison error, got: {error}"
    );
}

#[tokio::test]
async fn cancelled_public_call_after_second_xpc_data_frame_requires_reconnect() {
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    let (gate, mut blocked_rx) = WriteGate::new();
    let gated_client = GatedIo {
        inner: client_io,
        gate: Arc::clone(&gate),
    };
    let (done_tx, done_rx) = oneshot::channel();
    let server_task = tokio::spawn(serve_initialization(server_io, done_rx));

    let mut client = timeout(
        Duration::from_secs(5),
        XpcClient::connect_stream(gated_client),
    )
    .await
    .expect("XPC initialization watchdog")
    .expect("XPC initialization should succeed");
    gate.arm();

    let large_body = XpcValue::String("x".repeat(MAX_FRAME_PAYLOAD * 3));
    let mut call_fut = Box::pin(client.call(large_body));
    timeout(Duration::from_secs(5), async {
        tokio::select! {
            biased;
            _ = &mut blocked_rx => {}
            result = &mut call_fut => {
                panic!("multi-frame call completed before the second DATA write was blocked: {result:?}");
            }
        }
    })
    .await
    .expect("second DATA write must become pending within the watchdog");

    assert!(
        gate.first_data_complete.load(Ordering::SeqCst),
        "the first XPC DATA frame must be fully written before the second is blocked"
    );
    let bytes_after_cancel = gate.bytes_written();
    drop(call_fut);

    gate.release();

    let send_result = timeout(
        Duration::from_secs(5),
        client.send(XpcValue::String("after-cancel".into())),
    )
    .await
    .expect("send after a cancelled whole-message write must not hang");
    assert_requires_reconnect(
        send_result.expect_err("a cancelled multi-frame XPC send must poison the public client"),
    );
    assert_eq!(
        gate.bytes_written(),
        bytes_after_cancel,
        "a rejected send must not write another byte after the cancellation"
    );

    let recv_result = timeout(Duration::from_secs(5), client.recv())
        .await
        .expect("recv after a cancelled whole-message write must not hang");
    assert_requires_reconnect(recv_result.expect_err(
        "recv must observe the same poisoned connection instead of reading a new message",
    ));
    assert_eq!(
        gate.bytes_written(),
        bytes_after_cancel,
        "a rejected recv must not trigger any wire write"
    );

    let _ = done_tx.send(());
    timeout(Duration::from_secs(5), server_task)
        .await
        .expect("server shutdown watchdog")
        .expect("server task should finish cleanly");
}

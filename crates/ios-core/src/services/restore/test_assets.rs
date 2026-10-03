//! Owned local fixtures, independent of ambient proxy, daemon or Apple state.

use std::{
    io::{self, Cursor, Write},
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

pub(super) struct Directory(pub(super) PathBuf);
impl Directory {
    pub(super) fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("ios-restore-native-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("owned restore directory cleanup");
    }
}

pub(super) fn os_failure() -> io::Error {
    let directory = Directory::new();
    std::fs::File::open(directory.0.join("owned-private-missing")).unwrap_err()
}

pub(super) fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub(super) fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= 16 * 1024);
    let mut bytes = vec![0, (payload.len() >> 8) as u8, payload.len() as u8, kind, 0];
    bytes.extend_from_slice(&(if kind == 0 { 1u32 } else { 0u32 }).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

pub(super) struct Stream {
    input: Cursor<Vec<u8>>,
    pub(super) fail: Arc<AtomicBool>,
    pub(super) writes: Arc<AtomicUsize>,
    allowed_writes: usize,
    failure: Option<io::Error>,
}
impl Stream {
    pub(super) fn new(input: Vec<u8>, allowed_writes: usize) -> Self {
        Self {
            input: Cursor::new(input),
            fail: Arc::new(AtomicBool::new(false)),
            writes: Arc::new(AtomicUsize::new(0)),
            allowed_writes,
            failure: Some(os_failure()),
        }
    }
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let at = self.input.position() as usize;
        let end = (at + buf.remaining()).min(self.input.get_ref().len());
        buf.put_slice(&self.input.get_ref()[at..end]);
        self.input.set_position(end as u64);
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let at = self.writes.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) || at >= self.allowed_writes {
            return Poll::Ready(Err(self.failure.take().expect("one injected native fault")));
        }
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

pub(super) async fn http<T, F, Fut>(
    response: Option<&[u8]>,
    hold: bool,
    operation: F,
) -> (T, Vec<u8>)
where
    F: FnOnce(reqwest::Client, String) -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/owned-private", listener.local_addr().unwrap());
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap();
    let peer = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            assert!(request.len() <= 64 * 1024);
        }
        if let Some(response) = response {
            stream.write_all(response).await.unwrap();
        }
        if !hold {
            stream.shutdown().await.unwrap();
        }
        let mut remaining = Vec::new();
        // Client drop terminates held local responses; tolerate reset as well as EOF.
        let _ = stream.read_to_end(&mut remaining).await;
        request.extend(remaining);
        request
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(operation(client, url), peer)
    })
    .await
    .expect("owned HTTP futures must finish")
}

use super::*;
use std::{
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::io::ReadBuf;

struct Abrupt;
impl AsyncRead for Abrupt {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()))
    }
}

async fn check_close(local_eof: bool, pending: &[u8], abrupt: bool) -> Result<(), DproxyError> {
    let path = std::env::temp_dir().join(format!("dproxy-eof-{}", uuid::Uuid::new_v4()));
    let mut recorder = ProxyRecorder::new(&path, ProxyProtocol::Lockdown).unwrap();
    recorder
        .record_chunk(Direction::DeviceToHost, pending)
        .unwrap();
    let recording = Arc::new(tokio::sync::Mutex::new(Recording {
        recorder,
        observer: None::<fn(DecodeFailure)>,
        local_eof,
    }));
    let result = if abrupt {
        pump(
            Abrupt,
            tokio::io::sink(),
            Direction::DeviceToHost,
            recording.clone(),
        )
        .await
    } else {
        pump(
            tokio::io::empty(),
            tokio::io::sink(),
            Direction::DeviceToHost,
            recording.clone(),
        )
        .await
    };
    drop(recording);
    for name in ["events.ndjson", "host-to-device.bin", "device-to-host.bin"] {
        fs::remove_file(path.join(name)).unwrap();
    }
    fs::remove_dir(path).unwrap();
    result
}

#[tokio::test]
async fn client_eof_allows_remote_tls_close_only_at_frame_boundary() {
    check_close(true, &[], true).await.unwrap();
    assert!(
        matches!(check_close(false, &[], true).await, Err(DproxyError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof)
    );
    check_close(false, &[], false).await.unwrap();
}

#[tokio::test]
async fn remote_eof_keeps_truncated_plist_header_and_body_errors() {
    for pending in [&[0, 0][..], &[0, 0, 0, 4, b'a'][..]] {
        for abrupt in [false, true] {
            assert!(
                matches!(check_close(true, pending, abrupt).await, Err(DproxyError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof)
            );
        }
    }
}

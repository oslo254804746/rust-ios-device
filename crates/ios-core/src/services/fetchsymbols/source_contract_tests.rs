//! Real public remote-symbols calls on owned host streams, without a device.

use super::*;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::io::ReadBuf;

#[derive(Default)]
struct State {
    written: Vec<u8>,
    drops: usize,
}
struct Service {
    replies: io::Cursor<Vec<u8>>,
    fail_after: Option<usize>,
    state: Arc<Mutex<State>>,
}
impl AsyncRead for Service {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let count = io::Read::read(&mut self.replies, buf.initialize_unfilled())?;
        buf.advance(count);
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for Service {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let mut state = self.state.lock().unwrap();
        let count = if let Some(limit) = self.fail_after {
            if state.written.len() == limit {
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "owned-private-write",
                )));
            }
            bytes.len().min(limit - state.written.len())
        } else {
            bytes.len()
        };
        state.written.extend_from_slice(&bytes[..count]);
        Poll::Ready(Ok(count))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.state.lock().unwrap().drops += 1;
    }
}
fn service(replies: Vec<u8>, fail_after: Option<usize>) -> (Service, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    (
        Service {
            replies: io::Cursor::new(replies),
            fail_after,
            state: Arc::clone(&state),
        },
        state,
    )
}
fn settings() -> Vec<u8> {
    vec![0, 0, 0, 4, 0, 0, 0, 0, 0]
}
fn data(stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for chunk in payload.chunks(crate::xpc::h2_raw::MAX_FRAME_PAYLOAD) {
        bytes.extend_from_slice(&(chunk.len() as u32).to_be_bytes()[1..]);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&stream_id.to_be_bytes());
        bytes.extend_from_slice(chunk);
    }
    bytes
}
fn assert_cause(error: &FetchSymbolsError, class: crate::ErrorClass, phase: &str) {
    assert_eq!(crate::classify_error(error), class);
    assert!(error.to_string().contains(phase));
    let FetchSymbolsError::Io(outer) = error else {
        panic!("typed cause discarded");
    };
    let source = std::error::Error::source(outer).expect("original cause discarded");
    assert!(source.is::<crate::xpc::h2_raw::H2Error>() || source.is::<crate::XpcError>());
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[tokio::test]
async fn settings_read_and_bootstrap_write_keep_original_public_failures() {
    for (replies, limit, class, phase) in [
        (vec![], None, crate::ErrorClass::Io, "H2 error"),
        // Preface/settings/window-update and settings ACK total 67 bytes.
        (
            settings(),
            Some(67),
            crate::ErrorClass::PermissionDenied,
            "remote XPC bootstrap step 1 failed",
        ),
    ] {
        let (stream, state) = service(replies, limit);
        let error = RemoteFetchSymbolsClient::connect(stream)
            .await
            .err()
            .expect("fault accepted");
        assert_cause(&error, class, phase);
        assert_eq!(state.lock().unwrap().drops, 1);
    }
}

#[tokio::test]
async fn catalog_eof_and_malformed_control_keep_native_h2_and_xpc_causes() {
    for malformed in [false, true] {
        let replies = if malformed {
            [settings(), data(1, &[0; 24])].concat()
        } else {
            settings()
        };
        let (stream, state) = service(replies, None);
        let mut client = RemoteFetchSymbolsClient::connect(stream).await.unwrap();
        let error = client.list_files().await.unwrap_err();
        assert_cause(
            &error,
            if malformed {
                crate::ErrorClass::Protocol
            } else {
                crate::ErrorClass::Io
            },
            if malformed {
                "control decode failed"
            } else {
                "control frame read failed"
            },
        );
        assert!(state.lock().unwrap().written.len() > 67);
        drop(client);
        assert_eq!(state.lock().unwrap().drops, 1);
    }
}

#[tokio::test]
async fn partial_file_keeps_prior_complete_chunk_and_no_success_on_eof() {
    let first = vec![b'x'; MAX_CHUNK];
    let replies = [settings(), data(2, &first), data(2, b"ab")].concat();
    let (stream, state) = service(replies, None);
    let mut client = RemoteFetchSymbolsClient::connect(stream).await.unwrap();
    let file = RemoteSymbolFile {
        path: "owned-private-symbol".into(),
        size: (MAX_CHUNK + 6) as u64,
    };
    let mut output = Vec::new();
    let error = client
        .download_known(0, &file, &mut output, None)
        .await
        .unwrap_err();
    assert_cause(&error, crate::ErrorClass::Io, "file stream read failed");
    assert_eq!(output, first);
    drop(client);
    assert_eq!(state.lock().unwrap().drops, 1);
}

#[tokio::test]
async fn successful_remote_download_retains_bytes_and_requested_limit() {
    let (stream, state) = service([settings(), data(2, b"abcdef")].concat(), None);
    let mut client = RemoteFetchSymbolsClient::connect(stream).await.unwrap();
    let file = RemoteSymbolFile {
        path: "owned-private-symbol".into(),
        size: 6,
    };
    let mut output = Vec::new();
    assert_eq!(
        client
            .download_known(0, &file, &mut output, Some(3))
            .await
            .unwrap(),
        3
    );
    assert_eq!(output, b"abc");
    drop(client);
    assert_eq!(state.lock().unwrap().drops, 1);
}

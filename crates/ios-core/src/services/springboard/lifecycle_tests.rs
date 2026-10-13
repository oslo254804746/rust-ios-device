use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

use super::*;

const BOUND: Duration = Duration::from_secs(2);
const SHORT: Duration = Duration::from_millis(30);

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, future)
        .await
        .expect("owned springboard fixture stalled")
}

fn frame(value: &plist::Value) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, value).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend(body);
    bytes
}

fn orientation() -> Vec<u8> {
    frame(&plist::Value::Dictionary(plist::Dictionary::from_iter([(
        "interfaceOrientation".to_string(),
        plist::Value::Integer(3.into()),
    )])))
}

async fn request<S: AsyncRead + Unpin>(peer: &mut S) -> plist::Dictionary {
    let size = bounded(peer.read_u32()).await.unwrap() as usize;
    assert!(size < 16 * 1024);
    let mut body = vec![0; size];
    bounded(peer.read_exact(&mut body)).await.unwrap();
    plist::from_bytes(&body).unwrap()
}

async fn eof<S: AsyncRead + Unpin>(peer: &mut S) {
    assert_eq!(bounded(peer.read(&mut [0; 1])).await.unwrap(), 0);
}

fn issued_timer(error: &SpringboardError) {
    assert_eq!(crate::classify_error(error), crate::ErrorClass::Timeout);
    assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(error));
}

async fn all_rejected<S: AsyncRead + AsyncWrite + Unpin>(client: &mut SpringboardClient<S>) {
    assert!(!client.is_usable());
    assert!(client.stream.is_none());
    let state = plist::Value::Array(vec![]);
    let errors = [
        bounded(client.list_icons()).await.unwrap_err(),
        bounded(client.get_icon_state_raw("2")).await.unwrap_err(),
        bounded(client.get_icon_png_data("owned"))
            .await
            .unwrap_err(),
        bounded(client.get_interface_orientation())
            .await
            .unwrap_err(),
        bounded(client.get_homescreen_icon_metrics())
            .await
            .unwrap_err(),
        bounded(client.get_wallpaper_info("homescreen"))
            .await
            .unwrap_err(),
        bounded(client.get_wallpaper_preview_image("homescreen"))
            .await
            .unwrap_err(),
        bounded(client.get_homescreen_wallpaper_pngdata())
            .await
            .unwrap_err(),
        bounded(client.set_icon_state(&state)).await.unwrap_err(),
    ];
    assert!(errors
        .into_iter()
        .all(|error| matches!(error, SpringboardError::Protocol(_))));
}

#[tokio::test]
async fn lifecycle_springboard_all_preflight_budgets_and_unpolled_calls_do_no_io() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = SpringboardClient::new(stream);
    let state = plist::Value::Array(vec![]);
    drop(client.list_icons());
    drop(client.get_icon_state_raw("2"));
    drop(client.get_icon_png_data("owned"));
    drop(client.get_interface_orientation());
    drop(client.get_homescreen_icon_metrics());
    drop(client.get_wallpaper_info("homescreen"));
    drop(client.get_wallpaper_preview_image("homescreen"));
    drop(client.get_homescreen_wallpaper_pngdata());
    drop(client.set_icon_state(&state));
    for (budget, class) in [
        (Duration::ZERO, crate::ErrorClass::Timeout),
        (Duration::MAX, crate::ErrorClass::InvalidInput),
    ] {
        client.set_operation_timeout(Some(budget));
        let errors = [
            client.list_icons().await.unwrap_err(),
            client.get_icon_state_raw("2").await.unwrap_err(),
            client.get_icon_png_data("owned").await.unwrap_err(),
            client.get_interface_orientation().await.unwrap_err(),
            client.get_homescreen_icon_metrics().await.unwrap_err(),
            client.get_wallpaper_info("homescreen").await.unwrap_err(),
            client
                .get_wallpaper_preview_image("homescreen")
                .await
                .unwrap_err(),
            client.get_homescreen_wallpaper_pngdata().await.unwrap_err(),
            client.set_icon_state(&state).await.unwrap_err(),
        ];
        for error in errors {
            assert_eq!(crate::classify_error(&error), class);
            assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        }
        assert!(client.is_usable());
    }
    client.set_operation_timeout(None);
    assert!(tokio::time::timeout(SHORT, peer.read_u8()).await.is_err());
    let server = async {
        assert_eq!(
            request(&mut peer).await["command"].as_string(),
            Some("getInterfaceOrientation")
        );
        bounded(peer.write_all(&orientation())).await.unwrap();
        peer
    };
    let (result, mut peer) =
        bounded(async { tokio::join!(client.get_interface_orientation(), server) }).await;
    assert_eq!(result.unwrap(), InterfaceOrientation::Landscape);
    assert!(client.is_usable());
    drop(client);
    eof(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_springboard_no_progress_expiry_or_cancel_keeps_the_original_stream() {
    for expiry in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(64);
        let mut client = SpringboardClient::new(stream);
        bounded(client.stream.as_mut().unwrap().write_all(&[0x5a; 64]))
            .await
            .unwrap();
        if expiry {
            client.set_operation_timeout(Some(SHORT));
            issued_timer(
                &bounded(client.get_interface_orientation())
                    .await
                    .unwrap_err(),
            );
        } else {
            let mut operation = Box::pin(client.get_interface_orientation());
            assert!(tokio::time::timeout(SHORT, &mut operation).await.is_err());
            drop(operation);
        }
        assert!(client.is_usable());
        let mut seed = [0; 64];
        bounded(peer.read_exact(&mut seed)).await.unwrap();
        assert_eq!(seed, [0x5a; 64]);
        client.set_operation_timeout(None);
        let server = async {
            request(&mut peer).await;
            bounded(peer.write_all(&orientation())).await.unwrap();
            peer
        };
        let (result, mut peer) =
            bounded(async { tokio::join!(client.get_interface_orientation(), server) }).await;
        assert_eq!(result.unwrap(), InterfaceOrientation::Landscape);
        drop(client);
        eof(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_springboard_partial_send_expiry_or_cancel_drops_the_original_stream() {
    for expiry in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(64);
        let mut client = SpringboardClient::new(stream);
        if expiry {
            client.set_operation_timeout(Some(SHORT));
            issued_timer(
                &bounded(client.get_interface_orientation())
                    .await
                    .unwrap_err(),
            );
        } else {
            let mut operation = Box::pin(client.get_interface_orientation());
            assert!(tokio::time::timeout(SHORT, &mut operation).await.is_err());
            drop(operation);
        }
        all_rejected(&mut client).await;
        let expected = frame(&plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "command".to_string(),
            plist::Value::String("getInterfaceOrientation".into()),
        )])));
        let mut sent = vec![0; 64];
        bounded(peer.read_exact(&mut sent)).await.unwrap();
        assert_eq!(sent.as_slice(), &expected[..64]);
        eof(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_springboard_fragmented_cancel_or_expiry_rejects_late_replies() {
    let bytes = orientation();
    for prefix in [2, 4, 6] {
        for expiry in [false, true] {
            let (stream, mut peer) = tokio::io::duplex(4096);
            let mut client = SpringboardClient::new(stream);
            let server = async {
                request(&mut peer).await;
                bounded(peer.write_all(&bytes[..prefix])).await.unwrap();
                peer
            };
            if expiry {
                client.set_operation_timeout(Some(SHORT));
                let (result, returned_peer) =
                    bounded(async { tokio::join!(client.get_interface_orientation(), server) })
                        .await;
                issued_timer(&result.unwrap_err());
                peer = returned_peer;
            } else {
                let caller = async {
                    let mut operation = Box::pin(client.get_interface_orientation());
                    assert!(tokio::time::timeout(SHORT, &mut operation).await.is_err());
                    drop(operation);
                };
                let ((), returned_peer) = bounded(async { tokio::join!(caller, server) }).await;
                peer = returned_peer;
            }
            all_rejected(&mut client).await;
            assert!(bounded(peer.write_all(&bytes[prefix..])).await.is_err());
            eof(&mut peer).await;
        }
    }
}

#[tokio::test]
async fn lifecycle_springboard_send_and_receive_share_one_total() {
    let (stream, mut peer) = tokio::io::duplex(512);
    let mut client = SpringboardClient::new(stream);
    bounded(client.stream.as_mut().unwrap().write_all(&[0x5a; 512]))
        .await
        .unwrap();
    client.set_operation_timeout(Some(Duration::from_millis(400)));
    let server = async {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let mut seed = [0; 512];
        bounded(peer.read_exact(&mut seed)).await.unwrap();
        assert_eq!(seed, [0x5a; 512]);
        request(&mut peer).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(bounded(peer.write_all(&orientation())).await.is_err());
        peer
    };
    let (result, mut peer) =
        bounded(async { tokio::join!(client.get_interface_orientation(), server) }).await;
    issued_timer(&result.unwrap_err());
    all_rejected(&mut client).await;
    eof(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_springboard_malformed_oversized_and_typed_failures_retire_queued_reply() {
    let invalid_frames = [
        {
            let mut bytes = 3u32.to_be_bytes().to_vec();
            bytes.extend_from_slice(b"bad");
            bytes
        },
        (16u32 * 1024 * 1024 + 1).to_be_bytes().to_vec(),
        frame(&plist::Value::String("wrong response shape".into())),
    ];
    for invalid in invalid_frames {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = SpringboardClient::new(stream);
        let server = async {
            request(&mut peer).await;
            let mut responses = invalid;
            responses.extend(orientation());
            bounded(peer.write_all(&responses)).await.unwrap();
            peer
        };
        let (result, mut peer) =
            bounded(async { tokio::join!(client.get_interface_orientation(), server) }).await;
        assert!(matches!(
            result,
            Err(SpringboardError::Plist(_) | SpringboardError::Protocol(_))
        ));
        all_rejected(&mut client).await;
        eof(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_springboard_legacy_set_eof_success_is_terminal_and_query_eof_is_error() {
    for partial in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = SpringboardClient::new(stream);
        let server = async {
            assert_eq!(
                request(&mut peer).await["command"].as_string(),
                Some("setIconState")
            );
            if partial {
                bounded(peer.write_all(&[0, 0])).await.unwrap();
            }
            peer.shutdown().await.unwrap();
            peer
        };
        let state = plist::Value::Array(vec![]);
        let (result, mut peer) =
            bounded(async { tokio::join!(client.set_icon_state(&state), server) }).await;
        result.unwrap();
        all_rejected(&mut client).await;
        eof(&mut peer).await;
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = SpringboardClient::new(stream);
    let server = async {
        request(&mut peer).await;
        peer.shutdown().await.unwrap();
        peer
    };
    let (result, mut peer) =
        bounded(async { tokio::join!(client.get_interface_orientation(), server) }).await;
    assert!(
        matches!(result, Err(SpringboardError::Io(ref error)) if error.kind() == std::io::ErrorKind::UnexpectedEof)
    );
    all_rejected(&mut client).await;
    eof(&mut peer).await;
}

#[derive(thiserror::Error)]
#[error("owned springboard read failure")]
struct ReadFailure(Arc<()>);

impl std::fmt::Debug for ReadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ReadFailure")
    }
}

struct FailedRead {
    stream: DuplexStream,
    identity: Arc<()>,
    delay: Duration,
}

impl AsyncRead for FailedRead {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        std::thread::sleep(self.delay);
        Poll::Ready(Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            ReadFailure(Arc::clone(&self.identity)),
        )))
    }
}

impl AsyncWrite for FailedRead {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn lifecycle_springboard_primary_io_retains_the_original_native_payload() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let identity = Arc::new(());
    let mut client = SpringboardClient::new(FailedRead {
        stream,
        identity: Arc::clone(&identity),
        delay: Duration::ZERO,
    });
    client.set_operation_timeout(Some(BOUND));
    let error = bounded(client.get_interface_orientation())
        .await
        .unwrap_err();
    let SpringboardError::Io(error) = error else {
        panic!("original IO lost")
    };
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    let original = error
        .get_ref()
        .unwrap()
        .downcast_ref::<ReadFailure>()
        .unwrap();
    assert!(Arc::ptr_eq(&original.0, &identity));
    request(&mut peer).await;
    all_rejected(&mut client).await;
    eof(&mut peer).await;
}

struct CrossClock {
    stream: DuplexStream,
    delay_write: bool,
    calls: Arc<[AtomicUsize; 3]>,
}

impl AsyncRead for CrossClock {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.calls[2].fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for CrossClock {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let previous = self.calls[0].fetch_add(1, Ordering::SeqCst);
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if self.delay_write && previous == 0 && matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            std::thread::sleep(SHORT * 2);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.calls[1].fetch_add(1, Ordering::SeqCst);
        let result = Pin::new(&mut self.stream).poll_flush(cx);
        if !self.delay_write && matches!(result, Poll::Ready(Ok(()))) {
            std::thread::sleep(SHORT * 2);
        }
        result
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn lifecycle_springboard_clock_crossing_ready_io_cannot_start_the_next_wire_stage() {
    for delay_write in [true, false] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let calls = Arc::new([
            AtomicUsize::new(0),
            AtomicUsize::new(0),
            AtomicUsize::new(0),
        ]);
        let mut client = SpringboardClient::new(CrossClock {
            stream,
            delay_write,
            calls: Arc::clone(&calls),
        });
        client.set_operation_timeout(Some(SHORT));
        issued_timer(
            &bounded(client.get_interface_orientation())
                .await
                .unwrap_err(),
        );
        assert_eq!(calls[2].load(Ordering::SeqCst), 0);
        if delay_write {
            assert_eq!(calls[0].load(Ordering::SeqCst), 1);
            assert_eq!(calls[1].load(Ordering::SeqCst), 0);
            // Only the accepted prefix reached the actual original peer.
            assert!(bounded(peer.read_u32()).await.unwrap() > 0);
        } else {
            assert_eq!(calls[0].load(Ordering::SeqCst), 2);
            assert_eq!(calls[1].load(Ordering::SeqCst), 1);
            request(&mut peer).await;
        }
        all_rejected(&mut client).await;
        eof(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_springboard_native_ready_failure_crossing_clock_keeps_original_cause() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let identity = Arc::new(());
    let mut client = SpringboardClient::new(FailedRead {
        stream,
        identity: Arc::clone(&identity),
        delay: SHORT * 2,
    });
    client.set_operation_timeout(Some(SHORT));
    let error = bounded(client.get_interface_orientation())
        .await
        .unwrap_err();
    assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
    let SpringboardError::Io(error) = error else {
        panic!("original IO lost after clock crossing")
    };
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    let original = error
        .get_ref()
        .unwrap()
        .downcast_ref::<ReadFailure>()
        .unwrap();
    assert!(Arc::ptr_eq(&original.0, &identity));
    request(&mut peer).await;
    all_rejected(&mut client).await;
    eof(&mut peer).await;
}

#[tokio::test]
async fn lifecycle_springboard_parse_completion_checks_total_and_preserves_primary() {
    for fail in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = SpringboardClient::new(stream);
        client.set_operation_timeout(Some(SHORT));
        let server = async {
            request(&mut peer).await;
            bounded(peer.write_all(&orientation())).await.unwrap();
            peer
        };
        let command = CommandRequest {
            command: "getInterfaceOrientation",
        };
        let operation = client.request(&command, |response| {
            let value = response?;
            // A timer cannot interrupt synchronous response decoding. The
            // completed result must still be checked against the same clock.
            std::thread::sleep(SHORT * 2);
            if fail {
                Err(SpringboardError::Service("original parse failure".into()))
            } else {
                parse_interface_orientation(value)
            }
        });
        let (result, mut peer) = bounded(async { tokio::join!(operation, server) }).await;
        let error = result.unwrap_err();
        if fail {
            assert!(
                matches!(error, SpringboardError::Service(ref message) if message == "original parse failure")
            );
        } else {
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
            assert!(!crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        }
        all_rejected(&mut client).await;
        eof(&mut peer).await;
    }
}

#[tokio::test]
async fn lifecycle_springboard_owned_loopback_failure_reaches_strict_eof_and_joins() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    for expiry in [false, true, false, true] {
        let (connected, accepted) = bounded(async {
            tokio::join!(tokio::net::TcpStream::connect(address), listener.accept())
        })
        .await;
        let mut client = SpringboardClient::new(connected.unwrap());
        let mut peer = accepted.unwrap().0;
        let server = tokio::spawn(async move {
            assert_eq!(
                request(&mut peer).await["command"].as_string(),
                Some("getInterfaceOrientation")
            );
            bounded(peer.write_all(&orientation()[..6])).await.unwrap();
            eof(&mut peer).await;
        });
        if expiry {
            client.set_operation_timeout(Some(Duration::from_millis(100)));
            issued_timer(
                &bounded(client.get_interface_orientation())
                    .await
                    .unwrap_err(),
            );
        } else {
            let mut operation = Box::pin(client.get_interface_orientation());
            assert!(
                tokio::time::timeout(Duration::from_millis(100), &mut operation)
                    .await
                    .is_err()
            );
            drop(operation);
        }
        all_rejected(&mut client).await;
        bounded(server).await.unwrap();
        // The peer's original task joined while the unusable client is alive.
        drop(client);
    }
}

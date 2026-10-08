use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::DuplexStream;

const SECRET: &str = "synthetic-private-browse-error";

fn limits() -> BrowseLimits {
    BrowseLimits::new(Duration::from_secs(2))
}

fn frame(status: Option<&str>, ids: &[&str]) -> Vec<u8> {
    let mut dictionary = plist::Dictionary::new();
    if let Some(status) = status {
        dictionary.insert("Status".to_owned(), plist::Value::String(status.into()));
    }
    dictionary.insert(
        "CurrentList".to_owned(),
        plist::Value::Array(
            ids.iter()
                .map(|id| {
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "CFBundleIdentifier".to_owned(),
                            plist::Value::String((*id).into()),
                        ),
                        (
                            "CFBundleDisplayName".to_owned(),
                            plist::Value::String("Example".into()),
                        ),
                        (
                            "ApplicationType".to_owned(),
                            plist::Value::String("User".into()),
                        ),
                        (
                            "Path".to_owned(),
                            plist::Value::String("/Applications/Example.app".into()),
                        ),
                    ]))
                })
                .collect(),
        ),
    );
    dictionary_frame(dictionary)
}

fn dictionary_frame(dictionary: plist::Dictionary) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &dictionary).unwrap();
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(&body);
    bytes
}

async fn read_request<S: AsyncRead + Unpin>(peer: &mut S) -> Vec<u8> {
    let mut prefix = [0; 4];
    peer.read_exact(&mut prefix).await.unwrap();
    let length = u32::from_be_bytes(prefix) as usize;
    assert!(length <= 4096);
    let mut body = vec![0; length];
    peer.read_exact(&mut body).await.unwrap();
    let request: plist::Dictionary = plist::from_bytes(&body).unwrap();
    assert_eq!(request["Command"].as_string(), Some("Browse"));
    let options = request["ClientOptions"].as_dictionary().unwrap();
    assert_eq!(options["ApplicationType"].as_string(), Some("User"));
    assert_eq!(options["ShowLaunchProhibitedApps"].as_boolean(), Some(true));
    assert_eq!(options.len(), 2);
    prefix.into_iter().chain(body).collect()
}

fn resource(error: &IpError) -> &BrowseLimit {
    let IpError::Io(io) = error else {
        panic!("caller limit lost its existing error family")
    };
    io.get_ref().unwrap().downcast_ref().unwrap()
}

fn elapsed(error: &IpError) -> &tokio::time::error::Elapsed {
    let IpError::Io(io) = error else {
        panic!("original deadline source lost")
    };
    io.get_ref().unwrap().downcast_ref().unwrap()
}

#[tokio::test]
async fn bounded_browse_empty_exact_and_entry_plus_one_preserve_wire_and_order() {
    for (max_entries, ids) in [
        (0, vec![]),
        (1, vec!["one"]),
        (2, vec!["one", "two"]),
        (1, vec!["one", "two"]),
    ] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let server = async {
            let request = read_request(&mut peer).await;
            peer.write_all(&frame(Some("Complete"), &ids))
                .await
                .unwrap();
            (peer, request)
        };
        let (result, (mut peer, request)) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                client.list_user_apps_with_limits(limits().with_max_entries(max_entries)),
                server
            )
        })
        .await
        .unwrap();
        let mut expected = Vec::new();
        send_plist(
            &mut expected,
            &serde_json::json!({
                "Command": "Browse",
                "ClientOptions": {
                    "ApplicationType": "User",
                    "ShowLaunchProhibitedApps": true,
                },
            }),
        )
        .await
        .unwrap();
        assert_eq!(request, expected);
        if ids.len() <= max_entries {
            assert_eq!(
                result
                    .unwrap()
                    .iter()
                    .map(|app| app.bundle_id.as_str())
                    .collect::<Vec<_>>(),
                ids
            );
            assert!(client.is_usable());
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(resource(&error).kind, "entries");
            assert_eq!(resource(&error).received, 2);
            assert!(!client.is_usable());
        }
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_exact_wire_budget_includes_prefix_and_rejects_before_body() {
    let complete = frame(Some("Complete"), &[]);
    for max_wire_bytes in [complete.len(), complete.len() - 1] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let server = async {
            read_request(&mut peer).await;
            if max_wire_bytes == complete.len() {
                peer.write_all(&complete).await.unwrap();
            } else {
                // Keep the body absent and connection open. A correct refusal
                // comes from declared length before allocating or polling it.
                peer.write_all(&complete[..4]).await.unwrap();
            }
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.list_user_apps_with_limits(limits().with_max_wire_bytes(max_wire_bytes)),
                server
            )
        })
        .await
        .unwrap();
        if max_wire_bytes == complete.len() {
            assert!(result.unwrap().is_empty());
            assert!(client.is_usable());
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert_eq!(resource(&error).received, complete.len() as u64);
            assert!(!client.is_usable());
        }
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_cumulative_wire_and_frame_limits_cover_all_chunks() {
    let replies = [
        frame(Some("BrowsingApplications"), &["one"]),
        frame(Some("BrowsingApplications"), &["two"]),
        frame(Some("Complete"), &["three"]),
    ];
    let total: usize = replies.iter().map(Vec::len).sum();
    for mode in 0..3 {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let configured = limits()
            .with_max_entries(3)
            .with_max_frames(if mode == 2 { 2 } else { 3 })
            .with_max_wire_bytes(if mode == 1 { total - 1 } else { total });
        let server = async {
            read_request(&mut peer).await;
            peer.write_all(&replies[0]).await.unwrap();
            peer.write_all(&replies[1]).await.unwrap();
            if mode == 1 {
                peer.write_all(&replies[2][..4]).await.unwrap();
            } else {
                peer.write_all(&replies[2]).await.unwrap();
            }
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.list_user_apps_with_limits(configured), server)
        })
        .await
        .unwrap();
        if mode == 0 {
            assert_eq!(
                result
                    .unwrap()
                    .iter()
                    .map(|app| app.bundle_id.as_str())
                    .collect::<Vec<_>>(),
                vec!["one", "two", "three"]
            );
            assert!(client.is_usable());
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            assert!(!client.is_usable());
            if mode == 1 {
                assert_eq!(resource(&error).kind, "reply wire bytes");
                assert_eq!(resource(&error).received, total as u64);
            } else {
                assert_eq!(resource(&error).kind, "frames");
                let mut unread = vec![0; replies[2].len()];
                client.stream.read_exact(&mut unread).await.unwrap();
                assert_eq!(unread, replies[2]);
            }
        }
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_zero_overflow_and_unpolled_rejections_perform_no_io() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = InstallationProxy::new(stream);
    for (configured, class) in [
        (
            BrowseLimits::new(Duration::ZERO),
            crate::ErrorClass::Timeout,
        ),
        (
            BrowseLimits::new(Duration::MAX),
            crate::ErrorClass::InvalidInput,
        ),
        (
            limits().with_max_frames(0),
            crate::ErrorClass::ResourceLimit,
        ),
        (
            limits().with_max_wire_bytes(0),
            crate::ErrorClass::ResourceLimit,
        ),
        (
            limits().with_max_wire_bytes(3),
            crate::ErrorClass::ResourceLimit,
        ),
    ] {
        let error = client
            .list_user_apps_with_limits(configured)
            .await
            .unwrap_err();
        assert_eq!(crate::classify_error(&error), class);
        assert!(client.is_usable());
    }
    drop(client.list_user_apps_with_limits(limits()));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
            .await
            .is_err()
    );
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn bounded_browse_larger_caller_frame_limit_cannot_raise_the_existing_hard_limit() {
    let (stream, mut peer) = tokio::io::duplex(1024);
    let mut client = InstallationProxy::new(stream);
    let incomplete = frame(Some("BrowsingApplications"), &[]);
    let server = async {
        read_request(&mut peer).await;
        for _ in 0..MAX_BROWSE_CHUNKS {
            peer.write_all(&incomplete).await.unwrap();
        }
        peer
    };
    let configured = limits()
        .with_max_entries(usize::MAX)
        .with_max_frames(usize::MAX)
        .with_max_wire_bytes(usize::MAX);
    let (result, mut peer) = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(client.list_user_apps_with_limits(configured), server)
    })
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert_eq!(resource(&error).limit, MAX_BROWSE_CHUNKS as u64);
    assert_eq!(resource(&error).received, MAX_BROWSE_CHUNKS as u64 + 1);
    assert!(!client.is_usable());
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn bounded_browse_huge_declared_length_preserves_wire_or_hard_frame_refusal() {
    for declared in [(MAX_PLIST_SIZE + 1) as u32, u32::MAX] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let server = async {
            read_request(&mut peer).await;
            peer.write_all(&declared.to_be_bytes()).await.unwrap();
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.list_user_apps_with_limits(limits().with_max_wire_bytes(usize::MAX)),
                server
            )
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        if declared as u64 + 4 > usize::MAX as u64 {
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
        } else {
            assert!(matches!(error, IpError::Protocol(_)));
        }
        assert!(!client.is_usable());
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_remote_error_is_never_empty_success_and_retains_safe_original_fields() {
    for status in [None, Some("BrowsingApplications"), Some("Complete")] {
        let mut dictionary = plist::Dictionary::from_iter([
            ("Error".to_owned(), plist::Value::String(SECRET.into())),
            (
                "ErrorDescription".to_owned(),
                plist::Value::String(SECRET.into()),
            ),
        ]);
        if let Some(status) = status {
            dictionary.insert("Status".to_owned(), plist::Value::String(status.into()));
        }
        let bytes = dictionary_frame(dictionary);
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let server = async {
            read_request(&mut peer).await;
            peer.write_all(&bytes).await.unwrap();
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.list_user_apps_with_limits(limits()), server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ServiceFailure
        );
        let IpError::Io(io) = &error else {
            panic!("original response source lost")
        };
        let response = io
            .get_ref()
            .unwrap()
            .downcast_ref::<BrowseResponseError>()
            .unwrap();
        assert_eq!(response.code, SECRET);
        assert_eq!(response.description.as_deref(), Some(SECRET));
        assert!(!format!("{error:?}").contains(SECRET));
        assert!(!format!("{error}").contains(SECRET));
        assert_eq!(client.is_usable(), status == Some("Complete"));
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_malformed_typed_fields_do_not_become_empty_results() {
    let cases = [
        plist::Dictionary::new(),
        plist::Dictionary::from_iter([("Status".to_owned(), plist::Value::Boolean(true))]),
        plist::Dictionary::from_iter([("Error".to_owned(), plist::Value::Boolean(false))]),
        plist::Dictionary::from_iter([
            ("Error".to_owned(), plist::Value::String(SECRET.into())),
            ("ErrorDescription".to_owned(), plist::Value::Boolean(false)),
        ]),
        plist::Dictionary::from_iter([
            ("Status".to_owned(), plist::Value::String("Complete".into())),
            ("CurrentList".to_owned(), plist::Value::Boolean(false)),
        ]),
        plist::Dictionary::from_iter([
            ("Status".to_owned(), plist::Value::String("Complete".into())),
            (
                "CurrentList".to_owned(),
                plist::Value::Array(vec![plist::Value::Boolean(false)]),
            ),
        ]),
    ];
    for dictionary in cases {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let server = async {
            read_request(&mut peer).await;
            peer.write_all(&dictionary_frame(dictionary)).await.unwrap();
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.list_user_apps_with_limits(limits()), server)
        })
        .await
        .unwrap();
        assert_eq!(
            crate::classify_error(&result.unwrap_err()),
            crate::ErrorClass::Protocol
        );
        assert!(!client.is_usable());
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_malformed_plist_keeps_original_decode_error() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = InstallationProxy::new(stream);
    let server = async {
        read_request(&mut peer).await;
        peer.write_all(&(SECRET.len() as u32).to_be_bytes())
            .await
            .unwrap();
        peer.write_all(SECRET.as_bytes()).await.unwrap();
        peer
    };
    let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.list_user_apps_with_limits(limits()), server)
    })
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert!(matches!(error, IpError::Plist(_)));
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
    assert!(!format!("{error:?}").contains(SECRET));
    assert!(!client.is_usable());
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn bounded_browse_explicit_complete_success_or_error_allows_a_following_legacy_query() {
    for business_error in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let caller = async {
            let first = client.list_user_apps_with_limits(limits()).await;
            if business_error {
                assert_eq!(
                    crate::classify_error(&first.unwrap_err()),
                    crate::ErrorClass::ServiceFailure
                );
            } else {
                assert!(first.unwrap().is_empty());
            }
            assert!(client.is_usable());
            let next = client.list_user_apps().await.unwrap();
            assert_eq!(next[0].bundle_id, "next");
        };
        let server = async {
            read_request(&mut peer).await;
            if business_error {
                peer.write_all(&dictionary_frame(plist::Dictionary::from_iter([
                    ("Status".to_owned(), plist::Value::String("Complete".into())),
                    ("Error".to_owned(), plist::Value::String(SECRET.into())),
                ])))
                .await
                .unwrap();
            } else {
                peer.write_all(&frame(Some("Complete"), &[])).await.unwrap();
            }
            read_request(&mut peer).await;
            peer.write_all(&frame(Some("Complete"), &["next"]))
                .await
                .unwrap();
        };
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn bounded_browse_partial_reply_and_native_eof_preserve_primary_sources() {
    for body in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = InstallationProxy::new(stream);
        let server = async {
            read_request(&mut peer).await;
            let complete = frame(Some("Complete"), &[]);
            peer.write_all(&complete[..if body { 8 } else { 2 }])
                .await
                .unwrap();
            drop(peer);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.list_user_apps_with_limits(limits()), server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        let IpError::Io(io) = error else {
            panic!("native EOF was flattened")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(!client.is_usable());
    }
}

#[tokio::test]
async fn bounded_browse_partial_blocked_send_times_out_and_invalidates() {
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut client = InstallationProxy::new(stream);
    let error = client
        .list_user_apps_with_limits(BrowseLimits::new(Duration::from_millis(20)))
        .await
        .unwrap_err();
    elapsed(&error);
    assert!(!client.is_usable());
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 1);
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

struct StallSend {
    stream: DuplexStream,
    stalled: Arc<AtomicBool>,
}

impl AsyncRead for StallSend {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for StallSend {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.stalled.load(Ordering::Relaxed) {
            Poll::Pending
        } else {
            Pin::new(&mut self.stream).poll_write(cx, bytes)
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

#[tokio::test]
async fn bounded_browse_no_wire_progress_timeout_preserves_reuse_on_the_original_stream() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let stalled = Arc::new(AtomicBool::new(true));
    let mut client = InstallationProxy::new(StallSend {
        stream,
        stalled: Arc::clone(&stalled),
    });
    let error = client
        .list_user_apps_with_limits(BrowseLimits::new(Duration::from_millis(20)))
        .await
        .unwrap_err();
    elapsed(&error);
    assert!(client.is_usable());
    stalled.store(false, Ordering::Relaxed);
    let server = async {
        read_request(&mut peer).await;
        peer.write_all(&frame(Some("Complete"), &[])).await.unwrap();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.list_user_apps_with_limits(limits()), server)
    })
    .await
    .unwrap();
    assert!(result.unwrap().is_empty());
    assert!(client.is_usable());
}

#[tokio::test]
async fn bounded_browse_cancel_fences_every_legacy_entry_and_does_not_consume_late_reply() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = InstallationProxy::new(stream);
    let (request_sent, cancel) = tokio::sync::oneshot::channel();
    let caller = async {
        let mut operation = Box::pin(client.list_user_apps_with_limits(limits()));
        tokio::select! {
            biased;
            _ = cancel => {},
            result = &mut operation => panic!("unexpected completion: {result:?}"),
        }
        drop(operation);
    };
    let server = async {
        read_request(&mut peer).await;
        request_sent.send(()).unwrap();
        peer
    };
    let ((), mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(caller, server)
    })
    .await
    .unwrap();
    assert!(!client.is_usable());
    let late = frame(Some("Complete"), &["late"]);
    peer.write_all(&late).await.unwrap();
    assert!(matches!(
        client.install("private-path").await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.upgrade("private-path").await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.uninstall("private-id").await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.archive("private-id").await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.restore("private-id").await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.list_all_apps().await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.list_user_apps().await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.lookup_app("private-id").await,
        Err(IpError::Protocol(_))
    ));
    assert!(matches!(
        client.list_user_apps_with_limits(limits()).await,
        Err(IpError::Protocol(_))
    ));
    let mut unread = vec![0; late.len()];
    client.stream.read_exact(&mut unread).await.unwrap();
    assert_eq!(unread, late);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
            .await
            .is_err()
    );
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

struct CrossingRead {
    stream: DuplexStream,
    bytes_read: Arc<AtomicUsize>,
    first_frame_len: usize,
    crossed: bool,
    native_error: bool,
}

impl AsyncRead for CrossingRead {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.crossed && self.bytes_read.load(Ordering::Relaxed) >= self.first_frame_len {
            self.crossed = true;
            std::thread::sleep(Duration::from_millis(80));
            if self.native_error {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    SECRET,
                )));
            }
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buf);
        self.bytes_read
            .fetch_add(buf.filled().len() - before, Ordering::Relaxed);
        result
    }
}

impl AsyncWrite for CrossingRead {
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
async fn bounded_browse_total_crossing_reply_stops_subsequent_poll_and_keeps_original_error_priority(
) {
    for native_error in [false, true] {
        let first = frame(Some("BrowsingApplications"), &["first"]);
        let complete = frame(Some("Complete"), &["second"]);
        let (stream, mut peer) = tokio::io::duplex(4096);
        let bytes_read = Arc::new(AtomicUsize::new(0));
        let mut client = InstallationProxy::new(CrossingRead {
            stream,
            bytes_read: Arc::clone(&bytes_read),
            first_frame_len: first.len(),
            crossed: false,
            native_error,
        });
        let server = async {
            read_request(&mut peer).await;
            peer.write_all(&first).await.unwrap();
            peer.write_all(&complete).await.unwrap();
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.list_user_apps_with_limits(BrowseLimits::new(Duration::from_millis(50))),
                server
            )
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        if native_error {
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::Disconnected
            );
            let IpError::Io(io) = error else {
                panic!("native crossing error lost")
            };
            assert_eq!(io.kind(), std::io::ErrorKind::ConnectionReset);
            assert_eq!(io.get_ref().unwrap().to_string(), SECRET);
            assert_eq!(bytes_read.load(Ordering::Relaxed), first.len());
        } else {
            elapsed(&error);
            assert_eq!(bytes_read.load(Ordering::Relaxed), first.len() + 4);
        }
        assert!(!client.is_usable());
        assert!(client.stream.crossed);
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_browse_native_fresh_sessions_after_cancel_and_drop_reach_eof() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    for cancelled in [true, false, false] {
        let (connected, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
        let mut client = InstallationProxy::new(connected.unwrap());
        let mut peer = accepted.unwrap().0;
        let (sent, cancel) = tokio::sync::oneshot::channel();
        let caller = async {
            if cancelled {
                let mut operation = Box::pin(client.list_user_apps_with_limits(limits()));
                tokio::select! {
                    biased;
                    _ = cancel => {},
                    result = &mut operation => panic!("unexpected completion: {result:?}"),
                }
                drop(operation);
                assert!(!client.is_usable());
            } else {
                assert!(client
                    .list_user_apps_with_limits(limits())
                    .await
                    .unwrap()
                    .is_empty());
                assert!(client.is_usable());
            }
        };
        let server = async {
            read_request(&mut peer).await;
            if cancelled {
                sent.send(()).unwrap();
            } else {
                drop(sent);
                peer.write_all(&frame(Some("Complete"), &[])).await.unwrap();
            }
            peer
        };
        let ((), mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(caller, server)
        })
        .await
        .unwrap();
        drop(client);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), peer.read(&mut [0; 1]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::DuplexStream;

const PATH: &str = "synthetic-private-bounded-read-path";
const FD: u64 = 17;

async fn request<S: AsyncRead + Unpin>(peer: &mut S, opcode: AfcOpcode) -> (u64, Vec<u8>) {
    let mut bytes = [0; AfcHeader::SIZE];
    peer.read_exact(&mut bytes).await.unwrap();
    let header = AfcHeader::ref_from_bytes(&bytes).unwrap();
    assert_eq!(header.magic.get(), AFC_MAGIC);
    assert_eq!(header.operation.get(), opcode as u64);
    assert_eq!(header.entire_len.get(), header.this_len.get());
    let mut hp = vec![0; header.this_len.get() as usize - AfcHeader::SIZE];
    peer.read_exact(&mut hp).await.unwrap();
    (header.packet_num.get(), hp)
}

async fn reply<S: AsyncWrite + Unpin>(
    peer: &mut S,
    id: u64,
    opcode: AfcOpcode,
    hp: &[u8],
    payload: &[u8],
) {
    let header = AfcHeader::new(id, opcode, hp.len(), payload.len());
    peer.write_all(header.as_bytes()).await.unwrap();
    peer.write_all(hp).await.unwrap();
    peer.write_all(payload).await.unwrap();
}

async fn open<S: AsyncRead + AsyncWrite + Unpin>(peer: &mut S) {
    let (id, hp) = request(peer, AfcOpcode::FileRefOpen).await;
    assert_eq!(id, 1);
    assert_eq!(&hp[..8], &1u64.to_le_bytes());
    assert_eq!(&hp[8..], format!("{PATH}\0").as_bytes());
    reply(
        peer,
        id,
        AfcOpcode::FileRefOpenResult,
        &FD.to_le_bytes(),
        &[],
    )
    .await;
}

async fn read_request<S: AsyncRead + Unpin>(peer: &mut S) -> (u64, usize) {
    let (id, hp) = request(peer, AfcOpcode::FileRefRead).await;
    assert_eq!(hp.len(), 16);
    assert_eq!(&hp[..8], &FD.to_le_bytes());
    let requested = u64::from_le_bytes(hp[8..].try_into().unwrap());
    assert!((1..=65_536).contains(&requested));
    (id, requested as usize)
}

async fn close<S: AsyncRead + AsyncWrite + Unpin>(peer: &mut S, code: u64) {
    let (id, hp) = request(peer, AfcOpcode::FileRefClose).await;
    assert_eq!(hp, FD.to_le_bytes());
    reply(peer, id, AfcOpcode::Status, &code.to_le_bytes(), &[]).await;
}

fn limit_source(error: &AfcError) -> &AfcReadLimit {
    let AfcError::Io(io) = error else {
        panic!("bounded decision did not preserve the existing error family")
    };
    io.get_ref().unwrap().downcast_ref().unwrap()
}

fn cleanup_source(error: &AfcError) -> &FileCleanupFailure {
    let AfcError::Io(io) = error else {
        panic!("primary/cleanup aggregate lost")
    };
    io.get_ref().unwrap().downcast_ref().unwrap()
}

#[tokio::test]
async fn bounded_read_exact_limit_empty_zero_and_usize_max_are_real_roundtrips() {
    for (max_bytes, content) in [
        (0, b"".as_slice()),
        (0, b"x".as_slice()),
        (1, b"x".as_slice()),
        (4, b"data".as_slice()),
        (4, b"extra".as_slice()),
        (usize::MAX, b"data".as_slice()),
    ] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        let ceiling = max_bytes.min(AfcClient::<DuplexStream>::MAX_IN_MEMORY_FILE);
        let server = async {
            open(&mut peer).await;
            let (id, requested) = read_request(&mut peer).await;
            assert_eq!(requested, (ceiling + 1).min(65_536));
            reply(&mut peer, id, AfcOpcode::Data, &[], content).await;
            if !content.is_empty() && content.len() <= ceiling {
                let (id, requested) = read_request(&mut peer).await;
                assert_eq!(requested, (ceiling - content.len() + 1).min(65_536));
                reply(&mut peer, id, AfcOpcode::Data, &[], &[]).await;
            }
            close(&mut peer, 0).await;
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client.read_file_with_limit(PATH, max_bytes), server)
        })
        .await
        .unwrap();
        assert!(client.is_usable());
        if content.len() <= ceiling {
            assert_eq!(result.unwrap().as_ref(), content);
        } else {
            let error = result.unwrap_err();
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
            let limit = limit_source(&error);
            assert_eq!(limit.scope, "file data");
            assert_eq!(limit.limit, ceiling);
            assert_eq!(limit.received, content.len() as u64);
            assert!(!format!("{error:?}").contains(PATH));
        }
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_read_multiple_chunks_probe_one_byte_at_the_exact_ceiling() {
    for extra in [false, true] {
        let content = vec![b'x'; 65_536];
        let (stream, mut peer) = tokio::io::duplex(1024);
        let mut client = AfcClient::new(stream);
        let server = async {
            open(&mut peer).await;
            let (id, requested) = read_request(&mut peer).await;
            assert_eq!(requested, 65_536);
            reply(&mut peer, id, AfcOpcode::Data, &[], &content).await;
            let (id, requested) = read_request(&mut peer).await;
            assert_eq!(requested, 1);
            reply(
                &mut peer,
                id,
                AfcOpcode::Data,
                &[],
                if extra { b"x" } else { b"" },
            )
            .await;
            close(&mut peer, 0).await;
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client.read_file_with_limit(PATH, content.len()), server)
        })
        .await
        .unwrap();
        assert!(client.is_usable());
        if extra {
            let error = result.unwrap_err();
            assert_eq!(limit_source(&error).received, 65_537);
            assert_eq!(
                crate::classify_error(&error),
                crate::ErrorClass::ResourceLimit
            );
        } else {
            assert_eq!(result.unwrap().as_ref(), content);
        }
    }
}

#[tokio::test]
async fn bounded_read_consumed_limit_failure_with_successful_close_permits_next_query() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    let caller = async {
        let error = client.read_file_with_limit(PATH, 0).await.unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        assert_eq!(limit_source(&error).received, 1);
        assert!(client.is_usable());
        assert_eq!(client.list_dir("/").await.unwrap(), vec!["next"]);
    };
    let server = async {
        open(&mut peer).await;
        let (id, _) = read_request(&mut peer).await;
        reply(&mut peer, id, AfcOpcode::Data, &[], b"x").await;
        close(&mut peer, 0).await;
        let (id, hp) = request(&mut peer, AfcOpcode::ReadDir).await;
        assert_eq!(hp, b"/\0");
        reply(&mut peer, id, AfcOpcode::Data, &[], b"next\0").await;
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(caller, server)
    })
    .await
    .unwrap();
}

#[derive(Clone, Copy)]
enum MaliciousPhase {
    Open,
    Read,
    Close,
}

#[tokio::test]
async fn bounded_read_rejects_declared_body_before_allocating_or_waiting() {
    for (phase, body, hp_len) in [
        (MaliciousPhase::Open, 9, 9),
        (MaliciousPhase::Open, u64::MAX - 40, u64::MAX - 40),
        (MaliciousPhase::Read, 9, 0),
        // Body fits the control allowance but exceeds the one requested byte.
        (MaliciousPhase::Read, 2, 0),
        (MaliciousPhase::Read, u64::MAX - 40, 0),
        (MaliciousPhase::Close, 9, 9),
    ] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        let server = async {
            let (id, opcode) = match phase {
                MaliciousPhase::Open => {
                    let (id, _) = request(&mut peer, AfcOpcode::FileRefOpen).await;
                    (id, AfcOpcode::FileRefOpenResult)
                }
                MaliciousPhase::Read => {
                    open(&mut peer).await;
                    let (id, requested) = read_request(&mut peer).await;
                    assert_eq!(requested, 1);
                    (id, AfcOpcode::Data)
                }
                MaliciousPhase::Close => {
                    open(&mut peer).await;
                    let (id, _) = read_request(&mut peer).await;
                    reply(&mut peer, id, AfcOpcode::Data, &[], &[]).await;
                    let (id, _) = request(&mut peer, AfcOpcode::FileRefClose).await;
                    (id, AfcOpcode::Status)
                }
            };
            let mut header = AfcHeader::new(id, opcode, 0, 0).as_bytes().to_vec();
            header[8..16].copy_from_slice(&(40 + body).to_le_bytes());
            header[16..24].copy_from_slice(&(40 + hp_len).to_le_bytes());
            peer.write_all(&header).await.unwrap();
            // Keep the connection open with no body. Completion must come from
            // header admission, rather than EOF or a timeout of a giant read.
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.read_file_with_limit(PATH, 0), server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::ResourceLimit
        );
        let original = match phase {
            MaliciousPhase::Read => {
                let failure = cleanup_source(&error);
                assert!(matches!(failure.cleanup, AfcError::Protocol(_)));
                &failure.primary
            }
            _ => &error,
        };
        assert_eq!(limit_source(original).received, body);
        assert!(!client.is_usable());
        assert!(matches!(
            client.list_dir("/").await,
            Err(AfcError::Protocol(_))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
                .await
                .is_err()
        );
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_read_malformed_lengths_and_short_status_keep_protocol_classification() {
    for (entire_len, this_len, opcode, body) in [
        (40u64, 39u64, AfcOpcode::FileRefOpenResult, b"".as_slice()),
        (40, 41, AfcOpcode::FileRefOpenResult, b"".as_slice()),
        (47, 47, AfcOpcode::Status, &[0; 7]),
    ] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        let server = async {
            let (id, _) = request(&mut peer, AfcOpcode::FileRefOpen).await;
            let mut bytes = AfcHeader::new(id, opcode, 0, 0).as_bytes().to_vec();
            bytes[8..16].copy_from_slice(&entire_len.to_le_bytes());
            bytes[16..24].copy_from_slice(&this_len.to_le_bytes());
            peer.write_all(&bytes).await.unwrap();
            peer.write_all(body).await.unwrap();
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.read_file_with_limit(PATH, 0), server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert!(matches!(error, AfcError::Protocol(_)));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        assert!(!client.is_usable());
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }
}

#[tokio::test]
async fn bounded_read_partial_reply_retains_native_eof_and_prevents_cleanup_write() {
    for partial_body in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        let server = async {
            open(&mut peer).await;
            let (id, _) = read_request(&mut peer).await;
            let header = AfcHeader::new(id, AfcOpcode::Data, 0, 3);
            if partial_body {
                peer.write_all(header.as_bytes()).await.unwrap();
                peer.write_all(b"x").await.unwrap();
            } else {
                peer.write_all(&header.as_bytes()[..7]).await.unwrap();
            }
            drop(peer);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.read_file_with_limit(PATH, 4), server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        // Core does not infer a device disconnect from an ambiguous native EOF.
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
        let failure = cleanup_source(&error);
        let AfcError::Io(io) = &failure.primary else {
            panic!("native EOF was flattened")
        };
        assert_eq!(io.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(matches!(failure.cleanup, AfcError::Protocol(_)));
        assert!(!client.is_usable());
    }
}

#[tokio::test]
async fn bounded_read_primary_and_cleanup_business_errors_keep_both_original_statuses() {
    for close_fails in [false, true] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = AfcClient::new(stream);
        let server = async {
            open(&mut peer).await;
            let (id, _) = read_request(&mut peer).await;
            reply(&mut peer, id, AfcOpcode::Status, &8u64.to_le_bytes(), &[]).await;
            close(&mut peer, if close_fails { 10 } else { 0 }).await;
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(client.read_file_with_limit(PATH, 0), server)
        })
        .await
        .unwrap();
        let error = result.unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::NotFound);
        assert_eq!(client.is_usable(), !close_fails);
        if close_fails {
            let failure = cleanup_source(&error);
            assert!(matches!(
                failure.primary,
                AfcError::Status(AfcStatusCode::ObjectNotFound)
            ));
            assert!(matches!(
                failure.cleanup,
                AfcError::Status(AfcStatusCode::PermDenied)
            ));
        } else {
            assert!(matches!(
                error,
                AfcError::Status(AfcStatusCode::ObjectNotFound)
            ));
        }
    }
}

#[tokio::test]
async fn bounded_read_limit_remains_primary_when_close_business_response_fails() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    let server = async {
        open(&mut peer).await;
        let (id, requested) = read_request(&mut peer).await;
        assert_eq!(requested, 1);
        reply(&mut peer, id, AfcOpcode::Data, &[], b"x").await;
        close(&mut peer, 10).await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.read_file_with_limit(PATH, 0), server)
    })
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert_eq!(
        crate::classify_error(&error),
        crate::ErrorClass::ResourceLimit
    );
    let failure = cleanup_source(&error);
    assert_eq!(limit_source(&failure.primary).limit, 0);
    assert!(matches!(
        failure.cleanup,
        AfcError::Status(AfcStatusCode::PermDenied)
    ));
    assert!(!client.is_usable());
}

#[tokio::test]
async fn bounded_read_cleanup_only_failure_is_not_a_successful_empty_result() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    let server = async {
        open(&mut peer).await;
        let (id, _) = read_request(&mut peer).await;
        reply(&mut peer, id, AfcOpcode::Data, &[], &[]).await;
        close(&mut peer, 10).await;
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.read_file_with_limit(PATH, 0), server)
    })
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(AfcError::Status(AfcStatusCode::PermDenied))
    ));
    assert!(!client.is_usable());
}

#[tokio::test]
async fn bounded_read_cancel_leaves_late_reply_unconsumed_and_rejects_reuse() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    let (read_started, cancel) = tokio::sync::oneshot::channel();
    let caller = async {
        let mut operation = Box::pin(client.read_file_with_limit(PATH, 4));
        tokio::select! {
            biased;
            _ = cancel => {},
            result = &mut operation => panic!("unexpected completion: {result:?}"),
        }
        drop(operation);
    };
    let server = async {
        open(&mut peer).await;
        let (id, _) = read_request(&mut peer).await;
        read_started.send(()).unwrap();
        (peer, id)
    };
    let ((), (mut peer, id)) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(caller, server)
    })
    .await
    .unwrap();
    assert!(!client.is_usable());
    let mut late = AfcHeader::new(id, AfcOpcode::Data, 0, 4)
        .as_bytes()
        .to_vec();
    late.extend_from_slice(b"late");
    peer.write_all(&late).await.unwrap();
    assert!(matches!(
        client.read_file_with_limit(PATH, 4).await,
        Err(AfcError::Protocol(_))
    ));
    assert!(matches!(
        client.close_file(FD).await,
        Err(AfcError::Protocol(_))
    ));
    let mut untouched = vec![0; late.len()];
    client
        .stream
        .inner
        .read_exact(&mut untouched)
        .await
        .unwrap();
    assert_eq!(untouched, late);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
            .await
            .is_err()
    );
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

struct StallReadRequest {
    stream: DuplexStream,
    stalled: Arc<AtomicBool>,
    stalled_write: Arc<tokio::sync::Notify>,
}

impl AsyncRead for StallReadRequest {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for StallReadRequest {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.stalled.load(Ordering::Relaxed) {
            self.stalled_write.notify_one();
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
async fn bounded_read_cancel_after_open_invalidates_even_without_read_wire_progress() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let stalled = Arc::new(AtomicBool::new(false));
    let stalled_write = Arc::new(tokio::sync::Notify::new());
    let mut client = AfcClient::new(StallReadRequest {
        stream,
        stalled: Arc::clone(&stalled),
        stalled_write: Arc::clone(&stalled_write),
    });
    let caller = async {
        let mut operation = Box::pin(client.read_file_with_limit(PATH, 4));
        tokio::select! {
            biased;
            _ = stalled_write.notified() => {},
            result = &mut operation => panic!("unexpected completion: {result:?}"),
        }
        drop(operation);
    };
    let server = async {
        let (id, _) = request(&mut peer, AfcOpcode::FileRefOpen).await;
        stalled.store(true, Ordering::Relaxed);
        reply(
            &mut peer,
            id,
            AfcOpcode::FileRefOpenResult,
            &FD.to_le_bytes(),
            &[],
        )
        .await;
        peer
    };
    let ((), mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(caller, server)
    })
    .await
    .unwrap();
    assert!(!client.stream.progressed);
    assert!(!client.is_usable());
    assert!(matches!(
        client.close_file(FD).await,
        Err(AfcError::Protocol(_))
    ));
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn bounded_read_request_timeout_keeps_elapsed_and_does_not_consume_partial_reply() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    client.set_request_timeout(Some(Duration::from_millis(30)));
    let server = async {
        open(&mut peer).await;
        let (id, _) = read_request(&mut peer).await;
        peer.write_all(AfcHeader::new(id, AfcOpcode::Data, 0, 3).as_bytes())
            .await
            .unwrap();
        peer.write_all(b"x").await.unwrap();
        peer
    };
    let (result, mut peer) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(client.read_file_with_limit(PATH, 4), server)
    })
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
    let failure = cleanup_source(&error);
    let AfcError::Io(io) = &failure.primary else {
        panic!("native elapsed lost")
    };
    assert!(io.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
    assert!(matches!(failure.cleanup, AfcError::Protocol(_)));
    assert!(!client.is_usable());
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn bounded_read_zero_overflow_and_unpolled_budget_preserve_no_io_admission() {
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    for (budget, class) in [
        (Duration::ZERO, crate::ErrorClass::Timeout),
        (Duration::MAX, crate::ErrorClass::InvalidInput),
    ] {
        client.set_request_timeout(Some(budget));
        let error = client
            .read_file_with_limit(PATH, usize::MAX)
            .await
            .unwrap_err();
        assert_eq!(crate::classify_error(&error), class);
        assert!(client.is_usable());
        assert_eq!(client.packet_num, 1);
    }
    client.set_request_timeout(None);
    drop(client.read_file_with_limit(PATH, 0));
    assert_eq!(client.packet_num, 1);
    assert!(client.is_usable());
    assert!(
        tokio::time::timeout(Duration::from_millis(10), peer.read(&mut [0; 1]))
            .await
            .is_err()
    );
    drop(client);
    assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
}

#[tokio::test]
async fn bounded_read_native_loopback_drop_reaches_peer_eof_on_repeated_cycles() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    for _ in 0..4 {
        let (connection, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(address), listener.accept(),);
        let mut client = AfcClient::new(connection.unwrap());
        let mut peer = accepted.unwrap().0;
        let server = async {
            open(&mut peer).await;
            let (id, requested) = read_request(&mut peer).await;
            assert_eq!(requested, 5);
            reply(&mut peer, id, AfcOpcode::Data, &[], b"data").await;
            let (id, requested) = read_request(&mut peer).await;
            assert_eq!(requested, 1);
            reply(&mut peer, id, AfcOpcode::Data, &[], &[]).await;
            close(&mut peer, 0).await;
            peer
        };
        let (result, mut peer) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(client.read_file_with_limit(PATH, 4), server)
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap().as_ref(), b"data");
        assert!(client.is_usable());
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

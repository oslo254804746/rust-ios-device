use super::*;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadBuf};

#[derive(Default)]
struct Observation {
    polls: AtomicUsize,
    pending: AtomicUsize,
    read_bytes: AtomicUsize,
    pending_changed: tokio::sync::Notify,
}

impl Observation {
    async fn next_pending(&self, after: usize, consumed: usize) {
        loop {
            let notified = self.pending_changed.notified();
            if self.pending.load(Ordering::Relaxed) > after
                && self.read_bytes.load(Ordering::Relaxed) >= consumed
            {
                return;
            }
            notified.await;
        }
    }
}

struct ObservedReader<S> {
    inner: S,
    observation: Arc<Observation>,
}

impl<S: AsyncRead + Unpin> AsyncRead for ObservedReader<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.observation.polls.fetch_add(1, Ordering::Relaxed);
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.observation
            .read_bytes
            .fetch_add(buf.filled().len() - before, Ordering::Relaxed);
        if result.is_pending() {
            self.observation.pending.fetch_add(1, Ordering::Relaxed);
            self.observation.pending_changed.notify_one();
        }
        result
    }
}

async fn cancel_when_pending<S: AsyncRead + Unpin>(
    stream: &mut SyslogStream<S>,
    observation: &Observation,
) -> Option<std::io::Result<String>> {
    cancel_after_consumed_prefix(stream, observation, 0).await
}

async fn cancel_after_consumed_prefix<S: AsyncRead + Unpin>(
    stream: &mut SyslogStream<S>,
    observation: &Observation,
    consumed: usize,
) -> Option<std::io::Result<String>> {
    let after = observation.pending.load(Ordering::Relaxed);
    let mut operation = Box::pin(stream.next_message());
    let completed = tokio::select! {
        biased;
        _ = observation.next_pending(after, consumed) => None,
        result = &mut operation => Some(result),
    };
    drop(operation);
    completed
}

async fn eof<S: AsyncRead + Unpin>(peer: &mut S) -> std::io::Result<usize> {
    tokio::time::timeout(Duration::from_secs(2), peer.read(&mut [0; 1]))
        .await
        .expect("owned peer did not observe local release")
}

#[tokio::test]
async fn consumed_prefix_survives_pending_cancellation_and_resumes_exact_message() {
    for (raw, split, expected) in [
        (b"prefix tail".as_slice(), 7, "prefix tail"),
        (b"prefix \xe6\x88\x96 tail".as_slice(), 8, "prefix 或 tail"),
        (b"prefix \xe6\x88\x96 tail".as_slice(), 9, "prefix 或 tail"),
        (
            b"prefix \\M-f\\M^H\\M^V tail".as_slice(),
            9,
            "prefix 或 tail",
        ),
    ] {
        let (reader, mut peer) = tokio::io::duplex(4096);
        let observation = Arc::new(Observation::default());
        let mut stream = SyslogStream::new(ObservedReader {
            inner: reader,
            observation: Arc::clone(&observation),
        });
        peer.write_all(&raw[..split]).await.unwrap();
        let cancelled = tokio::time::timeout(
            Duration::from_secs(2),
            cancel_when_pending(&mut stream, &observation),
        )
        .await;
        let usable_after_cancel = stream.is_usable();
        peer.write_all(&raw[split..]).await.unwrap();
        peer.write_all(b"\0next\0").await.unwrap();
        let resumed = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
        let next = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
        let usable_after_success = stream.is_usable();
        drop(stream);
        let peer_eof = eof(&mut peer).await;

        assert_eq!(peer_eof.unwrap(), 0);
        assert!(cancelled.unwrap().is_none());
        assert!(usable_after_cancel && usable_after_success);
        assert_eq!(resumed.unwrap().unwrap(), expected);
        assert_eq!(next.unwrap().unwrap(), "next");
    }
}

#[tokio::test]
async fn repeated_cancellation_keeps_utf8_and_vis_bytes_until_the_same_terminator() {
    for (raw, first, second, expected) in [
        (b"UTF8 \xe6\x88\x96 end".as_slice(), 6, 7, "UTF8 或 end"),
        (b"vis \\M-f\\M^H\\M^V end".as_slice(), 6, 10, "vis 或 end"),
    ] {
        let (reader, mut peer) = tokio::io::duplex(4096);
        let observation = Arc::new(Observation::default());
        let mut stream = SyslogStream::new(ObservedReader {
            inner: reader,
            observation: Arc::clone(&observation),
        });
        peer.write_all(&raw[..first]).await.unwrap();
        let first_cancel = tokio::time::timeout(
            Duration::from_secs(2),
            cancel_when_pending(&mut stream, &observation),
        )
        .await;
        peer.write_all(&raw[first..second]).await.unwrap();
        let second_cancel = tokio::time::timeout(
            Duration::from_secs(2),
            cancel_when_pending(&mut stream, &observation),
        )
        .await;
        peer.write_all(&raw[second..]).await.unwrap();
        peer.write_all(b"\0").await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
        let usable = stream.is_usable();
        drop(stream);
        let peer_eof = eof(&mut peer).await;

        assert_eq!(peer_eof.unwrap(), 0);
        assert!(first_cancel.unwrap().is_none());
        assert!(second_cancel.unwrap().is_none());
        assert_eq!(result.unwrap().unwrap(), expected);
        assert!(usable);
    }
}

#[tokio::test]
async fn empty_idle_cancellation_and_continuous_empty_messages_keep_healthy_framing() {
    let (reader, mut peer) = tokio::io::duplex(4096);
    let observation = Arc::new(Observation::default());
    let mut stream = SyslogStream::new(ObservedReader {
        inner: reader,
        observation: Arc::clone(&observation),
    });
    let idle_cancel = tokio::time::timeout(
        Duration::from_secs(2),
        cancel_when_pending(&mut stream, &observation),
    )
    .await;
    peer.write_all(b"\0one\0\0two\0").await.unwrap();
    let mut results = Vec::new();
    for _ in 0..4 {
        results.push(tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await);
    }
    let usable = stream.is_usable();
    drop(stream);
    let peer_eof = eof(&mut peer).await;

    assert_eq!(peer_eof.unwrap(), 0);
    assert!(idle_cancel.unwrap().is_none());
    assert!(usable);
    assert_eq!(
        results
            .into_iter()
            .map(|result| result.unwrap().unwrap())
            .collect::<Vec<_>>(),
        vec!["", "one", "", "two"]
    );
}

#[tokio::test]
async fn exact_raw_cap_succeeds_and_following_messages_start_without_old_prefix() {
    let (reader, mut peer) = tokio::io::duplex(1024);
    let observation = Arc::new(Observation::default());
    let mut stream = SyslogStream::new(ObservedReader {
        inner: reader,
        observation: Arc::clone(&observation),
    });
    let sender = async {
        peer.write_all(&vec![b'x'; MAX_SYSLOG_MESSAGE])
            .await
            .unwrap();
        peer.write_all(b"\0\0next\0").await.unwrap();
        peer
    };
    let reader = async {
        let exact = stream.next_message().await;
        let empty = stream.next_message().await;
        let next = stream.next_message().await;
        (exact, empty, next)
    };
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        tokio::join!(reader, sender)
    })
    .await;
    let usable = stream.is_usable();
    drop(stream);
    let ((exact, empty, next), mut peer) = result.unwrap();
    let peer_eof = eof(&mut peer).await;

    assert_eq!(peer_eof.unwrap(), 0);
    assert!(usable);
    let exact = exact.unwrap();
    assert_eq!(exact.len(), MAX_SYSLOG_MESSAGE);
    assert!(exact.bytes().all(|byte| byte == b'x'));
    assert_eq!(empty.unwrap(), "");
    assert_eq!(next.unwrap(), "next");
}

#[tokio::test]
async fn raw_cap_plus_one_makes_tail_terminal_without_another_transport_poll() {
    for terminated in [false, true] {
        let (reader, mut peer) = tokio::io::duplex(1024);
        let observation = Arc::new(Observation::default());
        let mut stream = SyslogStream::new(ObservedReader {
            inner: reader,
            observation: Arc::clone(&observation),
        });
        let sender = async {
            peer.write_all(&vec![b'x'; MAX_SYSLOG_MESSAGE + 1])
                .await
                .unwrap();
            if terminated {
                peer.write_all(b"\0tail\0").await.unwrap();
            }
            peer
        };
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(stream.next_message(), sender)
        })
        .await;
        let usable = stream.is_usable();
        let polls_before = observation.polls.load(Ordering::Relaxed);
        let repeated = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
        let polls_after = observation.polls.load(Ordering::Relaxed);
        drop(stream);
        let (first, mut peer) = result.unwrap();
        let peer_eof = eof(&mut peer).await;

        assert_eq!(peer_eof.unwrap(), 0);
        assert!(!usable);
        assert_eq!(first.unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            repeated.unwrap().unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(polls_before, polls_after);
    }
}

#[tokio::test]
async fn idle_or_partial_eof_preserves_original_error_then_rejects_locally() {
    for prefix in [b"".as_slice(), b"unfinished".as_slice()] {
        let (reader, mut peer) = tokio::io::duplex(4096);
        let observation = Arc::new(Observation::default());
        let mut stream = SyslogStream::new(ObservedReader {
            inner: reader,
            observation: Arc::clone(&observation),
        });
        peer.write_all(prefix).await.unwrap();
        peer.shutdown().await.unwrap();
        let first = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
        let usable = stream.is_usable();
        let polls_before = observation.polls.load(Ordering::Relaxed);
        let repeated = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
        let polls_after = observation.polls.load(Ordering::Relaxed);
        drop(stream);
        let peer_eof = eof(&mut peer).await;

        assert_eq!(peer_eof.unwrap(), 0);
        let error = first.unwrap().unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
        assert_eq!(error.to_string(), "syslog stream closed");
        assert!(!usable);
        assert_eq!(
            repeated.unwrap().unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(polls_before, polls_after);
    }
}

#[derive(Debug)]
struct NativeFailure {
    identity: Arc<()>,
}

impl std::fmt::Display for NativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("synthetic original read failure")
    }
}

impl std::error::Error for NativeFailure {}

struct FailingReader {
    reader: ObservedReader<tokio::io::DuplexStream>,
    fail: Arc<AtomicBool>,
    identity: Arc<()>,
}

impl AsyncRead for FailingReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.fail.load(Ordering::Relaxed) {
            self.reader
                .observation
                .polls
                .fetch_add(1, Ordering::Relaxed);
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                NativeFailure {
                    identity: Arc::clone(&self.identity),
                },
            )));
        }
        Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

#[tokio::test]
async fn native_io_failure_after_consumed_prefix_retains_exact_cause_and_terminal_tail() {
    let (reader, mut peer) = tokio::io::duplex(4096);
    let observation = Arc::new(Observation::default());
    let fail = Arc::new(AtomicBool::new(false));
    let identity = Arc::new(());
    let mut stream = SyslogStream::new(FailingReader {
        reader: ObservedReader {
            inner: reader,
            observation: Arc::clone(&observation),
        },
        fail: Arc::clone(&fail),
        identity: Arc::clone(&identity),
    });
    peer.write_all(b"unfinished ").await.unwrap();
    let cancelled = tokio::time::timeout(
        Duration::from_secs(2),
        cancel_when_pending(&mut stream, &observation),
    )
    .await;
    fail.store(true, Ordering::Relaxed);
    peer.write_all(b"tail\0").await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
    let usable = stream.is_usable();
    let polls_before = observation.polls.load(Ordering::Relaxed);
    let repeated = tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await;
    let polls_after = observation.polls.load(Ordering::Relaxed);
    drop(stream);
    let peer_eof = eof(&mut peer).await;

    assert_eq!(peer_eof.unwrap(), 0);
    assert!(cancelled.unwrap().is_none());
    let error = first.unwrap().unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::ConnectionReset);
    let original = error
        .get_ref()
        .unwrap()
        .downcast_ref::<NativeFailure>()
        .unwrap();
    assert!(Arc::ptr_eq(&original.identity, &identity));
    assert!(!usable);
    assert_eq!(
        repeated.unwrap().unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );
    assert_eq!(polls_before, polls_after);
}

#[tokio::test]
async fn fresh_tcp_sessions_after_cancel_and_drop_release_the_original_peer() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    for cancelled in [true, true, false] {
        let (connected, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
        let mut peer = accepted.unwrap().0;
        let observation = Arc::new(Observation::default());
        let mut stream = SyslogStream::new(ObservedReader {
            inner: connected.unwrap(),
            observation: Arc::clone(&observation),
        });
        peer.write_all(if cancelled { b"unfinished" } else { b"fresh\0" })
            .await
            .unwrap();
        let (pending, message) = if cancelled {
            (
                Some(
                    tokio::time::timeout(
                        Duration::from_secs(2),
                        // Wait for the prefix to leave native RX. Closing
                        // unread bytes can produce RST and does not exercise
                        // consumed-prefix continuation.
                        cancel_after_consumed_prefix(
                            &mut stream,
                            &observation,
                            b"unfinished".len(),
                        ),
                    )
                    .await,
                ),
                None,
            )
        } else {
            (
                None,
                Some(tokio::time::timeout(Duration::from_secs(2), stream.next_message()).await),
            )
        };
        let usable = stream.is_usable();
        drop(stream);
        let peer_eof = eof(&mut peer).await;

        assert_eq!(peer_eof.unwrap(), 0);
        assert!(usable);
        if let Some(pending) = pending {
            assert!(pending.unwrap().is_none());
        }
        if let Some(message) = message {
            assert_eq!(message.unwrap().unwrap(), "fresh");
        }
    }
}

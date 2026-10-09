//! Owned peers exercise exact lockdown wire, real TLS, and cleanup before
//! operation admission is released. No device enumeration/pairing is involved.

use std::{
    error::Error,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::Duration,
};

use super::*;
use crate::{
    session::{OperationCancel, OperationLease, OperationOptions, SessionOwner, SharedAdmission},
    ErrorClass,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadBuf};

#[path = "test_assets.rs"]
mod test_assets;

const SERVICE: &str = "owned.private.checked.service";
const SESSION_ID: &str = "owned-private-checked-session";
const BUDGET: Duration = Duration::from_secs(3);
const FIXTURE: Duration = Duration::from_secs(5);

fn record() -> PairRecord {
    PairRecord {
        device_certificate: vec![],
        host_certificate: test_assets::TEST_CERT_PEM.to_vec(),
        host_private_key: test_assets::TEST_KEY_PEM.to_vec().into(),
        root_certificate: vec![],
        host_id: "owned-private-checked-host".into(),
        system_buid: "owned-private-checked-buid".into(),
        wifi_mac_address: None,
    }
}

fn admitted(
    total: Duration,
    cancel: OperationCancel,
) -> (SessionOwner, OperationLease, SharedAdmission) {
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let lease = owner
        .try_begin(OperationOptions::new(total), cancel)
        .unwrap();
    (owner, lease, shared)
}

fn dictionary<const N: usize>(fields: [(&str, plist::Value); N]) -> plist::Dictionary {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn query_reply() -> plist::Dictionary {
    dictionary([
        ("Request", "QueryType".into()),
        ("Type", "com.apple.mobile.lockdown".into()),
    ])
}

fn session_reply(tls: bool) -> plist::Dictionary {
    dictionary([
        ("Request", "StartSession".into()),
        ("SessionID", SESSION_ID.into()),
        ("EnableSessionSSL", tls.into()),
    ])
}

fn service_reply(tls: bool) -> plist::Dictionary {
    dictionary([
        ("Request", "StartService".into()),
        ("Port", 23456u64.into()),
        ("EnableServiceSSL", tls.into()),
    ])
}

async fn request<S: AsyncRead + Unpin>(
    peer: &mut S,
    operation: &str,
    keys: &[&str],
) -> plist::Dictionary {
    let request: plist::Dictionary = recv_lockdown(peer).await.unwrap();
    assert_eq!(request.len(), keys.len());
    assert_eq!(request["Label"].as_string(), Some("ios-rs"));
    assert_eq!(request["Request"].as_string(), Some(operation));
    for key in keys {
        assert!(request.contains_key(key), "unexpected wire shape");
    }
    request
}

async fn query<S: AsyncRead + AsyncWrite + Unpin>(peer: &mut S) {
    request(peer, "QueryType", &["Label", "Request"]).await;
    send_lockdown(peer, &query_reply()).await.unwrap();
}

async fn start_request<S: AsyncRead + Unpin>(peer: &mut S) {
    let request = request(
        peer,
        "StartSession",
        &[
            "Label",
            "Request",
            "ProtocolVersion",
            "HostID",
            "SystemBUID",
        ],
    )
    .await;
    assert_eq!(request["ProtocolVersion"].as_string(), Some("2"));
    assert_eq!(
        request["HostID"].as_string(),
        Some("owned-private-checked-host")
    );
    assert_eq!(
        request["SystemBUID"].as_string(),
        Some("owned-private-checked-buid")
    );
}

async fn established<S: AsyncRead + AsyncWrite + Unpin>(peer: &mut S, tls: bool) {
    query(peer).await;
    start_request(peer).await;
    send_lockdown(peer, &session_reply(tls)).await.unwrap();
}

async fn service_request<S: AsyncRead + Unpin>(peer: &mut S) {
    let request = request(peer, "StartService", &["Label", "Request", "Service"]).await;
    assert_eq!(request["Service"].as_string(), Some(SERVICE));
}

async fn eof<S: AsyncRead + Unpin>(peer: &mut S) {
    let mut tail = vec![];
    peer.read_to_end(&mut tail).await.unwrap();
    assert!(
        tail.is_empty(),
        "owner sent an extra request after completion/failure"
    );
}

fn released(owner: &SessionOwner, lease: OperationLease, shared: &SharedAdmission) {
    assert_eq!(owner.active(), 1, "peer EOF must precede lease retirement");
    assert_eq!(shared.available(), 0);
    drop(lease);
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
}

fn safe(error: &LockdownError) {
    let text = format!("{error:?} {}", crate::redaction::diagnostic_error(error));
    for secret in [
        SERVICE,
        SESSION_ID,
        "owned-private",
        "BEGIN PRIVATE KEY",
        "BEGIN CERTIFICATE",
    ] {
        assert!(
            !text.contains(secret),
            "automatic diagnostics disclosed private data"
        );
    }
}

fn native<'a, T: Error + 'static>(mut error: &'a (dyn Error + 'static)) -> Option<&'a T> {
    for _ in 0..32 {
        if let Some(error) = error.downcast_ref::<T>() {
            return Some(error);
        }
        error = if let Some(io) = error.downcast_ref::<io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| io.source())?
        } else {
            error.source()?
        };
    }
    None
}

async fn failure_at(stage: u8, reply: plist::Dictionary) -> LockdownError {
    let (host, mut peer) = tokio::io::duplex(16_384);
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let record = record();
    let (error, ()) = tokio::time::timeout(FIXTURE, async {
        tokio::join!(
            async {
                match start_existing_trust_session(lease.context(), host, &record).await {
                    Err(error) => {
                        assert!(stage < 2);
                        error
                    }
                    Ok(mut session) => {
                        assert_eq!(stage, 2);
                        let error = session.start_service(SERVICE).await.unwrap_err();
                        assert!(session.stream.is_none());
                        let retired = session.start_service(SERVICE).await.unwrap_err();
                        assert!(matches!(retired, LockdownError::Protocol(_)));
                        drop(session);
                        error
                    }
                }
            },
            async {
                if stage == 0 {
                    request(&mut peer, "QueryType", &["Label", "Request"]).await;
                } else {
                    query(&mut peer).await;
                    start_request(&mut peer).await;
                    if stage == 2 {
                        send_lockdown(&mut peer, &session_reply(false))
                            .await
                            .unwrap();
                        service_request(&mut peer).await;
                    }
                }
                send_lockdown(&mut peer, &reply).await.unwrap();
                eof(&mut peer).await;
            }
        )
    })
    .await
    .expect("owned failure fixture exceeded its bound");
    released(&owner, lease, &shared);
    safe(&error);
    error
}

#[tokio::test]
async fn plaintext_exact_wire_service_ssl_flags_and_cleanup() {
    for ssl in [false, true] {
        let (host, mut peer) = tokio::io::duplex(16_384);
        let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
        let record = record();
        tokio::time::timeout(FIXTURE, async {
            tokio::join!(
                async {
                    let mut session = start_existing_trust_session(lease.context(), host, &record)
                        .await
                        .unwrap();
                    assert!(!session.uses_tls());
                    assert_eq!(session.session_id(), SESSION_ID);
                    let debug = format!("{session:?}");
                    assert!(!debug.contains(SESSION_ID));
                    let info = session.start_service(SERVICE).await.unwrap();
                    assert_eq!(info.port, 23456);
                    assert_eq!(info.enable_service_ssl, ssl);
                    drop(session);
                },
                async {
                    established(&mut peer, false).await;
                    service_request(&mut peer).await;
                    send_lockdown(&mut peer, &service_reply(ssl)).await.unwrap();
                    eof(&mut peer).await;
                }
            )
        })
        .await
        .unwrap();
        released(&owner, lease, &shared);
    }
}

fn acceptor() -> tokio_rustls::TlsAcceptor {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let chain = rustls_pemfile::certs(&mut std::io::Cursor::new(test_assets::TEST_CERT_PEM))
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    let key = rustls_pemfile::private_key(&mut std::io::Cursor::new(test_assets::TEST_KEY_PEM))
        .unwrap()
        .unwrap();
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    // Keep the owned peer idle after the selected response so socket EOF is
    // observed without unread unsolicited TLS tickets causing a Linux RST.
    config.send_tls13_tickets = 0;
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

#[tokio::test]
async fn owned_tcp_real_tls_exact_start_service_and_raw_socket_eof_before_release() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (host, accepted) = tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
    let host = host.unwrap();
    let (mut peer, _) = accepted.unwrap();
    host.set_nodelay(true).unwrap();
    peer.set_nodelay(true).unwrap();
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let record = record();
    tokio::time::timeout(FIXTURE, async {
        tokio::join!(
            async {
                let mut session = start_existing_trust_session(lease.context(), host, &record)
                    .await
                    .unwrap();
                assert!(session.uses_tls());
                assert_eq!(session.session_id(), SESSION_ID);
                let info = session.start_service(SERVICE).await.unwrap();
                assert_eq!(info.port, 23456);
                assert!(!info.enable_service_ssl);
                drop(session);
            },
            async {
                established(&mut peer, true).await;
                let mut tls = acceptor().accept(peer).await.unwrap();
                service_request(&mut tls).await;
                send_lockdown(&mut tls, &service_reply(false))
                    .await
                    .unwrap();
                // Dropping a TLS transport does not send close_notify. Assert the
                // actual owned TCP EOF, without treating TLS UnexpectedEof as EOF.
                eof(&mut tls.get_mut().0).await;
            }
        )
    })
    .await
    .unwrap();
    released(&owner, lease, &shared);
    assert_eq!(listener.local_addr().unwrap(), address);
}

#[tokio::test]
async fn error_only_replies_win_over_absent_or_invalid_success_metadata() {
    for (stage, operation, code, class) in [
        (
            0,
            "QueryType",
            "owned-private-unknown-error",
            ErrorClass::ServiceFailure,
        ),
        (
            1,
            "StartSession",
            "InvalidHostID",
            ErrorClass::PermissionDenied,
        ),
        (
            1,
            "StartSession",
            "UserDeniedPairing",
            ErrorClass::PermissionDenied,
        ),
        (
            1,
            "StartSession",
            "PairingDialogResponsePending",
            ErrorClass::PermissionDenied,
        ),
        (
            1,
            "StartSession",
            "PasswordProtected",
            ErrorClass::PermissionDenied,
        ),
        (
            1,
            "StartSession",
            "owned-private-unknown-error",
            ErrorClass::ServiceFailure,
        ),
        (
            2,
            "StartService",
            "InvalidService",
            ErrorClass::ServiceMissing,
        ),
        (
            2,
            "StartService",
            "PasswordProtected",
            ErrorClass::ServiceFailure,
        ),
        (
            2,
            "StartService",
            "owned-private-unknown-error",
            ErrorClass::ServiceFailure,
        ),
    ] {
        let mut reply = dictionary([("Error", code.into())]);
        // No Request/Type/SessionID/SSL/Port is needed to retain Error-only.
        reply.insert("Port".into(), "owned-private-invalid-port".into());
        let error = failure_at(stage, reply).await;
        let remote = LockdownResponseError::in_error_chain(&error).unwrap();
        assert_eq!(remote.operation(), operation);
        assert_eq!(remote.code(), code);
        assert_eq!(crate::classify_error(&error), class);
    }
    for (operation, code, class) in [
        ("GetValue", "MissingValue", ErrorClass::NotFound),
        ("GetValue", "PasswordProtected", ErrorClass::ServiceFailure),
        ("StartService", "InvalidService", ErrorClass::ServiceMissing),
        (
            "StartService",
            "PasswordProtected",
            ErrorClass::ServiceFailure,
        ),
    ] {
        assert_eq!(
            LockdownResponseError::new(operation, code.into()).class(),
            class
        );
    }
}

#[tokio::test]
async fn checked_request_type_session_id_and_ssl_metadata_are_strict() {
    let mut bad_query = query_reply();
    bad_query.insert("Type".into(), "owned-private-unrelated-type".into());
    let mut no_type = query_reply();
    no_type.remove("Type");
    let mut bad_request = session_reply(false);
    bad_request.insert("Request".into(), "GetValue".into());
    let mut no_id = session_reply(false);
    no_id.remove("SessionID");
    let mut empty_id = session_reply(false);
    empty_id.insert("SessionID".into(), "".into());
    let mut bad_id = session_reply(false);
    bad_id.insert("SessionID".into(), true.into());
    let mut no_ssl = session_reply(false);
    no_ssl.remove("EnableSessionSSL");
    let mut bad_ssl = session_reply(false);
    bad_ssl.insert("EnableSessionSSL".into(), "false".into());
    for (stage, reply) in [
        (0, bad_query),
        (0, no_type),
        (1, bad_request),
        (1, no_id),
        (1, empty_id),
        (1, bad_id),
        (1, no_ssl),
        (1, bad_ssl),
        (1, dictionary([("Error", true.into())])),
    ] {
        let error = failure_at(stage, reply).await;
        assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
        assert!(LockdownResponseError::in_error_chain(&error).is_none());
    }
}

#[tokio::test]
async fn checked_service_port_and_ssl_metadata_are_strict_and_conversion_source_survives() {
    let mut no_port = service_reply(false);
    no_port.remove("Port");
    let mut zero_port = service_reply(false);
    zero_port.insert("Port".into(), 0u64.into());
    let mut bad_port = service_reply(false);
    bad_port.insert("Port".into(), "23456".into());
    let mut negative = service_reply(false);
    negative.insert("Port".into(), (-1i64).into());
    let mut no_ssl = service_reply(false);
    no_ssl.remove("EnableServiceSSL");
    let mut bad_ssl = service_reply(false);
    bad_ssl.insert("EnableServiceSSL".into(), 0u64.into());
    for reply in [no_port, zero_port, bad_port, negative, no_ssl, bad_ssl] {
        assert_eq!(
            crate::classify_error(&failure_at(2, reply).await),
            ErrorClass::Protocol
        );
    }
    let mut over = service_reply(false);
    over.insert("Port".into(), 65536u64.into());
    let error = failure_at(2, over).await;
    assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
    assert!(native::<std::num::TryFromIntError>(&error).is_some());
}

#[tokio::test]
async fn existing_frame_limit_rejects_before_payload_allocation_and_plist_cause_is_retained() {
    for payload in [None, Some(b"owned-private-malformed-plist".as_slice())] {
        let (host, mut peer) = tokio::io::duplex(8192);
        let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
        let record = record();
        let (error, ()) = tokio::time::timeout(FIXTURE, async {
            tokio::join!(
                async {
                    start_existing_trust_session(lease.context(), host, &record)
                        .await
                        .unwrap_err()
                },
                async {
                    request(&mut peer, "QueryType", &["Label", "Request"]).await;
                    let bytes = match payload {
                        None => (4u32 * 1024 * 1024 + 1).to_be_bytes().to_vec(),
                        Some(payload) => super::super::protocol::encode_frame(payload),
                    };
                    peer.write_all(&bytes).await.unwrap();
                    eof(&mut peer).await;
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
        if payload.is_some() {
            assert!(native::<plist::Error>(&error).is_some());
        }
        safe(&error);
        released(&owner, lease, &shared);
    }
}

#[derive(Default)]
struct Counts {
    read: AtomicUsize,
    write: AtomicUsize,
    dropped: AtomicBool,
    retirement: Mutex<Vec<(usize, usize)>>,
}

struct Observe<S> {
    inner: S,
    counts: Arc<Counts>,
    armed: Arc<AtomicBool>,
    consumed: Arc<tokio::sync::Notify>,
    retirement: Option<(SessionOwner, SharedAdmission)>,
}

impl<S> Drop for Observe<S> {
    fn drop(&mut self) {
        if let Some((owner, shared)) = &self.retirement {
            self.counts
                .retirement
                .lock()
                .unwrap()
                .push((owner.active(), shared.available()));
        }
        self.counts.dropped.store(true, Ordering::SeqCst);
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Observe<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.counts.read.fetch_add(1, Ordering::SeqCst);
        let before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if buf.filled().len() > before && self.armed.load(Ordering::SeqCst) {
            self.consumed.notify_one();
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Observe<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.counts.write.fetch_add(1, Ordering::SeqCst);
        Pin::new(&mut self.inner).poll_write(cx, data)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

fn observed<S>(
    inner: S,
) -> (
    Observe<S>,
    Arc<Counts>,
    Arc<AtomicBool>,
    Arc<tokio::sync::Notify>,
) {
    let counts = Arc::new(Counts::default());
    let armed = Arc::new(AtomicBool::new(false));
    let consumed = Arc::new(tokio::sync::Notify::new());
    (
        Observe {
            inner,
            counts: counts.clone(),
            armed: armed.clone(),
            consumed: consumed.clone(),
            retirement: None,
        },
        counts,
        armed,
        consumed,
    )
}

#[tokio::test]
async fn pre_cancel_and_expired_total_perform_no_stream_io_and_drop_before_lease_release() {
    for expired in [false, true] {
        let cancel = OperationCancel::new();
        let (owner, lease, shared) = admitted(
            if expired {
                Duration::from_millis(1)
            } else {
                BUDGET
            },
            cancel.clone(),
        );
        let (host, mut peer) = tokio::io::duplex(8);
        let (host, counts, _, _) = observed(host);
        if expired {
            tokio::time::sleep(Duration::from_millis(10)).await;
        } else {
            cancel.cancel();
        }
        let error = start_existing_trust_session(lease.context(), host, &record())
            .await
            .unwrap_err();
        assert_eq!(
            crate::classify_error(&error),
            if expired {
                ErrorClass::Timeout
            } else {
                ErrorClass::Cancelled
            }
        );
        assert_eq!(counts.read.load(Ordering::SeqCst), 0);
        assert_eq!(counts.write.load(Ordering::SeqCst), 0);
        assert!(counts.dropped.load(Ordering::SeqCst));
        tokio::time::timeout(FIXTURE, eof(&mut peer)).await.unwrap();
        released(&owner, lease, &shared);
    }
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    for (total, class) in [
        (Duration::ZERO, ErrorClass::Timeout),
        (Duration::MAX, ErrorClass::InvalidInput),
    ] {
        let error = owner
            .try_begin(OperationOptions::new(total), OperationCancel::new())
            .err()
            .unwrap();
        assert_eq!(crate::classify_error(&error), class);
        assert_eq!(owner.active(), 0);
        assert_eq!(shared.available(), 1);
    }
}

#[tokio::test]
async fn partial_service_read_total_or_cancellation_closes_stream_and_forbids_reuse() {
    for cancel_now in [false, true] {
        let cancel = OperationCancel::new();
        let (owner, lease, shared) = admitted(Duration::from_millis(500), cancel.clone());
        let (host, mut peer) = tokio::io::duplex(8192);
        let (host, counts, armed, consumed) = observed(host);
        let record = record();
        let (error, ()) = tokio::time::timeout(FIXTURE, async {
            tokio::join!(
                async {
                    let mut session = start_existing_trust_session(lease.context(), host, &record)
                        .await
                        .unwrap();
                    let error = session.start_service(SERVICE).await.unwrap_err();
                    assert!(session.stream.is_none());
                    assert!(counts.dropped.load(Ordering::SeqCst));
                    assert!(matches!(
                        session.start_service(SERVICE).await.unwrap_err(),
                        LockdownError::Protocol(_)
                    ));
                    drop(session);
                    error
                },
                async {
                    established(&mut peer, false).await;
                    service_request(&mut peer).await;
                    armed.store(true, Ordering::SeqCst);
                    peer.write_all(&[0, 0]).await.unwrap();
                    consumed.notified().await;
                    if cancel_now {
                        cancel.cancel();
                    }
                    eof(&mut peer).await;
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(
            crate::classify_error(&error),
            if cancel_now {
                ErrorClass::Cancelled
            } else {
                ErrorClass::Timeout
            }
        );
        if !cancel_now {
            assert!(native::<tokio::time::error::Elapsed>(&error).is_some());
        }
        released(&owner, lease, &shared);
    }
}

#[tokio::test]
async fn blocked_query_write_total_or_cancel_drains_actual_partial_output_after_close() {
    for cancel_now in [false, true] {
        let cancel = OperationCancel::new();
        let (owner, lease, shared) = admitted(Duration::from_millis(500), cancel.clone());
        let (host, mut peer) = tokio::io::duplex(1);
        let (host, counts, _, _) = observed(host);
        let record = record();
        let (closed, wait) = tokio::sync::oneshot::channel();
        let (error, ()) = tokio::time::timeout(FIXTURE, async {
            tokio::join!(
                async {
                    let error = start_existing_trust_session(lease.context(), host, &record)
                        .await
                        .unwrap_err();
                    closed.send(()).unwrap();
                    error
                },
                async {
                    let mut first = [0; 1];
                    peer.read_exact(&mut first).await.unwrap();
                    assert_eq!(first, [0]);
                    if cancel_now {
                        cancel.cancel();
                    }
                    // Keep capacity exhausted until the actual client transport
                    // closes; draining earlier would turn this into a read stall.
                    wait.await.unwrap();
                    let mut remaining = vec![];
                    peer.read_to_end(&mut remaining).await.unwrap();
                    assert!(
                        remaining.len() <= 1,
                        "bounded write continued after cancellation/total"
                    );
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(
            crate::classify_error(&error),
            if cancel_now {
                ErrorClass::Cancelled
            } else {
                ErrorClass::Timeout
            }
        );
        if !cancel_now {
            assert!(native::<tokio::time::error::Elapsed>(&error).is_some());
        }
        assert!(counts.dropped.load(Ordering::SeqCst));
        released(&owner, lease, &shared);
    }
}

#[tokio::test]
async fn blocked_real_tls_handshake_uses_same_total_and_cancellation_without_plaintext_fallback() {
    for cancel_now in [false, true] {
        let cancel = OperationCancel::new();
        let (owner, lease, shared) = admitted(Duration::from_millis(500), cancel.clone());
        let (host, mut peer) = tokio::io::duplex(16_384);
        let record = record();
        let (error, ()) = tokio::time::timeout(FIXTURE, async {
            tokio::join!(
                async {
                    start_existing_trust_session(lease.context(), host, &record)
                        .await
                        .unwrap_err()
                },
                async {
                    established(&mut peer, true).await;
                    let mut header = [0; 5];
                    peer.read_exact(&mut header).await.unwrap();
                    assert_eq!(header[0], 22);
                    let size = usize::from(u16::from_be_bytes(header[3..].try_into().unwrap()));
                    assert!(size <= 16_384);
                    let mut hello = vec![0; size];
                    peer.read_exact(&mut hello).await.unwrap();
                    assert_eq!(hello[0], 1);
                    if cancel_now {
                        cancel.cancel();
                    }
                    eof(&mut peer).await;
                }
            )
        })
        .await
        .unwrap();
        assert_eq!(
            crate::classify_error(&error),
            if cancel_now {
                ErrorClass::Cancelled
            } else {
                ErrorClass::Timeout
            }
        );
        if !cancel_now {
            assert!(native::<tokio::time::error::Elapsed>(&error).is_some());
        }
        released(&owner, lease, &shared);
    }
}

#[tokio::test]
async fn coalesced_start_session_and_tls_alert_keep_real_tls_bytes_and_native_source() {
    let (host, mut peer) = tokio::io::duplex(16_384);
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let record = record();
    let (error, ()) = tokio::time::timeout(FIXTURE, async {
        tokio::join!(
            async {
                start_existing_trust_session(lease.context(), host, &record)
                    .await
                    .unwrap_err()
            },
            async {
                query(&mut peer).await;
                start_request(&mut peer).await;
                let mut plist = vec![];
                plist::to_writer_xml(&mut plist, &session_reply(true)).unwrap();
                let mut coalesced = super::super::protocol::encode_frame(&plist);
                // Real TLS fatal handshake_failure follows the plist in one write.
                // Its rustls source proves bytes were retained, rather than lost at
                // the upgrade and replaced by a stalled-handshake timeout.
                coalesced.extend_from_slice(&[21, 3, 3, 0, 2, 2, 40]);
                peer.write_all(&coalesced).await.unwrap();
                let mut header = [0; 5];
                peer.read_exact(&mut header).await.unwrap();
                assert_eq!(header[0], 22);
                let size = usize::from(u16::from_be_bytes(header[3..].try_into().unwrap()));
                assert!(size <= 16_384);
                let mut hello = vec![0; size];
                peer.read_exact(&mut hello).await.unwrap();
                assert_eq!(hello[0], 1);
                eof(&mut peer).await;
            }
        )
    })
    .await
    .unwrap();
    assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
    assert!(matches!(
        native::<rustls::Error>(&error),
        Some(rustls::Error::AlertReceived(
            rustls::AlertDescription::HandshakeFailure
        ))
    ));
    assert!(native::<tokio::time::error::Elapsed>(&error).is_none());
    safe(&error);
    released(&owner, lease, &shared);
}

#[tokio::test]
async fn tls_certificate_parser_error_keeps_original_cause_without_starting_tls_io() {
    let (host, mut peer) = tokio::io::duplex(8192);
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let mut record = record();
    record.host_certificate =
        b"-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----".to_vec();
    let (error, ()) = tokio::time::timeout(FIXTURE, async {
        tokio::join!(
            async {
                start_existing_trust_session(lease.context(), host, &record)
                    .await
                    .unwrap_err()
            },
            async {
                established(&mut peer, true).await;
                eof(&mut peer).await;
            }
        )
    })
    .await
    .unwrap();
    assert_eq!(crate::classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(io) = &error else {
        panic!("certificate parser source was flattened");
    };
    assert!(io.source().is_some());
    assert!(native::<io::Error>(io.source().unwrap()).is_some());
    safe(&error);
    released(&owner, lease, &shared);
}

#[tokio::test]
async fn owned_tcp_idle_query_total_cancel_or_future_drop_releases_actual_socket() {
    for mode in [0, 1, 2] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (host, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
        let (mut peer, _) = accepted.unwrap();
        let (host, counts, _, _) = observed(host.unwrap());
        let cancel = OperationCancel::new();
        let (owner, lease, shared) = admitted(Duration::from_millis(500), cancel.clone());
        let record = record();
        let mut work = Box::pin(start_existing_trust_session(lease.context(), host, &record));
        tokio::time::timeout(FIXTURE, async {
            tokio::select! {
                result = &mut work => panic!("query completed before peer's idle stage: {result:?}"),
                _ = request(&mut peer, "QueryType", &["Label", "Request"]) => {}
            }
            if mode == 2 { drop(work); } else {
                if mode == 1 { cancel.cancel(); }
                let error = work.await.unwrap_err();
                assert_eq!(crate::classify_error(&error), if mode == 1 { ErrorClass::Cancelled } else { ErrorClass::Timeout });
                if mode == 0 { assert!(native::<tokio::time::error::Elapsed>(&error).is_some()); }
            }
            assert!(counts.dropped.load(Ordering::SeqCst)); eof(&mut peer).await;
        }).await.unwrap();
        assert_eq!(listener.local_addr().unwrap(), address);
        released(&owner, lease, &shared);
    }
}

#[tokio::test]
async fn unpolled_checked_session_future_closes_owned_stream_without_io() {
    let (host, mut peer) = tokio::io::duplex(1);
    let (host, counts, _, _) = observed(host);
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let record = record();
    drop(start_existing_trust_session(lease.context(), host, &record));
    assert_eq!(counts.read.load(Ordering::SeqCst), 0);
    assert_eq!(counts.write.load(Ordering::SeqCst), 0);
    assert!(counts.dropped.load(Ordering::SeqCst));
    tokio::time::timeout(FIXTURE, eof(&mut peer)).await.unwrap();
    released(&owner, lease, &shared);
}

#[derive(Debug)]
struct NativePrimary(Arc<()>);
impl std::fmt::Display for NativePrimary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("owned-private-native-primary")
    }
}
impl Error for NativePrimary {}

struct Crossing {
    counts: Arc<Counts>,
    marker: Option<Arc<()>>,
    cancel: Option<OperationCancel>,
}
impl Drop for Crossing {
    fn drop(&mut self) {
        self.counts.dropped.store(true, Ordering::SeqCst);
    }
}
impl AsyncRead for Crossing {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.counts.read.fetch_add(1, Ordering::SeqCst);
        Poll::Pending
    }
}
impl AsyncWrite for Crossing {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.counts.write.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = &self.cancel {
            cancel.cancel();
        } else {
            std::thread::sleep(Duration::from_millis(40));
        }
        Poll::Ready(match &self.marker {
            Some(marker) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                NativePrimary(marker.clone()),
            )),
            None => Ok(bytes.len()),
        })
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        panic!("late/cancelled write continued to flush")
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        panic!("crossing operation performed shutdown")
    }
}

#[tokio::test]
async fn native_error_in_crossing_poll_wins_and_late_success_is_suppressed() {
    for (cancel_during, original_error) in
        [(false, false), (false, true), (true, false), (true, true)]
    {
        let cancel = OperationCancel::new();
        let (owner, lease, shared) = admitted(
            if cancel_during {
                BUDGET
            } else {
                Duration::from_millis(10)
            },
            cancel.clone(),
        );
        let counts = Arc::new(Counts::default());
        let marker = Arc::new(());
        let stream = Crossing {
            counts: counts.clone(),
            marker: original_error.then(|| marker.clone()),
            cancel: cancel_during.then_some(cancel),
        };
        let error = start_existing_trust_session(lease.context(), stream, &record())
            .await
            .unwrap_err();
        assert!(counts.dropped.load(Ordering::SeqCst));
        assert_eq!(counts.write.load(Ordering::SeqCst), 1);
        assert_eq!(counts.read.load(Ordering::SeqCst), 0);
        if original_error {
            assert_eq!(crate::classify_error(&error), ErrorClass::Disconnected);
            assert!(Arc::ptr_eq(
                &native::<NativePrimary>(&error).unwrap().0,
                &marker
            ));
            assert!(native::<tokio::time::error::Elapsed>(&error).is_none());
        } else {
            assert_eq!(
                crate::classify_error(&error),
                if cancel_during {
                    ErrorClass::Cancelled
                } else {
                    ErrorClass::Timeout
                }
            );
            if !cancel_during {
                assert!(native::<tokio::time::error::Elapsed>(&error).is_some());
            }
        }
        safe(&error);
        released(&owner, lease, &shared);
    }
}

#[tokio::test]
async fn dropping_partial_method_closes_stream_while_borrowed_session_cannot_escape_lease() {
    let (host, mut peer) = tokio::io::duplex(8192);
    let (host, counts, armed, consumed) = observed(host);
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let record = record();
    tokio::time::timeout(FIXTURE, async {
        tokio::join!(async {
            let mut session = start_existing_trust_session(lease.context(), host, &record).await.unwrap();
            let mut method = Box::pin(session.start_service(SERVICE));
            tokio::select! { result = &mut method => panic!("partial method completed: {result:?}"), _ = consumed.notified() => {} }
            drop(method); assert!(counts.dropped.load(Ordering::SeqCst)); assert!(session.stream.is_none()); drop(session);
        }, async {
            established(&mut peer, false).await; service_request(&mut peer).await;
            armed.store(true, Ordering::SeqCst); peer.write_all(&[0, 0]).await.unwrap(); eof(&mut peer).await;
        })
    }).await.unwrap();
    released(&owner, lease, &shared);
}

#[tokio::test]
async fn finite_prepared_selector_limits_reject_before_owned_stream_io_and_release_admission() {
    for (selector, class) in [
        (String::new(), ErrorClass::InvalidInput),
        ("private\0service".into(), ErrorClass::InvalidInput),
        ("é".repeat(2048) + "x", ErrorClass::ResourceLimit),
    ] {
        let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
        let (host, mut peer) = tokio::io::duplex(8);
        let (mut host, counts, _, _) = observed(host);
        host.retirement = Some((owner.clone(), shared.clone()));
        let error = match crate::session::prepared_service_observation(
            host,
            Arc::new(record()),
            lease,
            selector,
        ) {
            Err(error) => Some(error),
            Ok(work) => {
                drop(work);
                None
            }
        };
        assert_eq!(counts.read.load(Ordering::SeqCst), 0);
        assert_eq!(counts.write.load(Ordering::SeqCst), 0);
        assert!(counts.dropped.load(Ordering::SeqCst));
        tokio::time::timeout(FIXTURE, eof(&mut peer)).await.unwrap();
        assert_eq!(*counts.retirement.lock().unwrap(), [(1, 0)]);
        assert_eq!(owner.active(), 0);
        assert_eq!(shared.available(), 1);
        assert_eq!(
            crate::classify_error(&error.expect("invalid selector was accepted")),
            class
        );
    }
}

#[tokio::test]
async fn finite_prepared_exact_selector_limit_uses_real_wire_and_closes_before_release() {
    let selector = "é".repeat(2048);
    assert_eq!(selector.len(), 4096);
    let (owner, lease, shared) = admitted(BUDGET, OperationCancel::new());
    let (host, mut peer) = tokio::io::duplex(16_384);
    let (mut host, counts, _, _) = observed(host);
    host.retirement = Some((owner.clone(), shared.clone()));
    let prepared = crate::session::prepared_service_observation(
        host,
        Arc::new(record()),
        lease,
        selector.clone(),
    )
    .unwrap();
    let (result, ()) = tokio::time::timeout(FIXTURE, async {
        tokio::join!(prepared, async {
            established(&mut peer, false).await;
            let request =
                request(&mut peer, "StartService", &["Label", "Request", "Service"]).await;
            assert_eq!(request["Service"].as_string(), Some(selector.as_str()));
            send_lockdown(&mut peer, &service_reply(false))
                .await
                .unwrap();
            eof(&mut peer).await;
        })
    })
    .await
    .unwrap();
    let observation = result.unwrap();
    assert_eq!(observation.port, 23456);
    assert!(!observation.enable_service_ssl);
    assert!(!observation.enable_session_ssl);
    assert!(counts.dropped.load(Ordering::SeqCst));
    assert_eq!(*counts.retirement.lock().unwrap(), [(1, 0)]);
    assert_eq!(owner.active(), 0);
    assert_eq!(shared.available(), 1);
}

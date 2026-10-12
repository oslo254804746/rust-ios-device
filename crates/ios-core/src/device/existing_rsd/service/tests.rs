use super::super::tests::{
    active_bootstrap, assert_control_tail, directory, frame, wire, SERVICE, UDID,
};
use super::*;
use crate::xpc::XpcValue;
use std::collections::VecDeque;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const OBSERVE: Duration = Duration::from_secs(3);
struct PeerSource {
    streams: StdMutex<VecDeque<ServiceStream>>,
    routes: StdMutex<Vec<ExistingRsdRoute>>,
}
impl Source for PeerSource {
    fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>> {
        Box::pin(async { Ok(uuid::Uuid::from_bytes([0x55; 16])) })
    }
    fn dial(&self, route: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>> {
        Box::pin(async move {
            self.routes.lock().unwrap().push(route);
            Ok(self
                .streams
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected additional service socket"))
        })
    }
}
async fn prepared(
    mut dictionary: indexmap::IndexMap<String, XpcValue>,
    route: ExistingRsdRoute,
) -> (ExistingRsdSession, PeerSource, tokio::io::DuplexStream) {
    let (rsd, mut rsd_peer) = tokio::io::duplex(16_384);
    let (service, service_peer) = tokio::io::duplex(16_384);
    let source = PeerSource {
        streams: StdMutex::new(VecDeque::from([
            Box::new(rsd) as ServiceStream,
            Box::new(service),
        ])),
        routes: StdMutex::new(Vec::new()),
    };
    dictionary.insert("MessageType".into(), XpcValue::String("Handshake".into()));
    let (session, ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(
            ExistingRsdSession::observe_from_source(&source, route, OBSERVE),
            async {
                if route.proxy_port.is_some() {
                    let mut prelude = [0; 20];
                    rsd_peer.read_exact(&mut prelude).await.unwrap();
                    assert_eq!(&prelude[..16], &route.endpoint.ip().octets());
                    assert_eq!(
                        &prelude[16..],
                        &(route.endpoint.port() as u32).to_le_bytes()
                    );
                }
                active_bootstrap(&mut rsd_peer).await;
                rsd_peer
                    .write_all(&frame(0, 0, 1, &wire(dictionary)))
                    .await
                    .unwrap();
                let mut tail = Vec::new();
                rsd_peer.read_to_end(&mut tail).await.unwrap();
                assert_control_tail(&tail);
            }
        )
    })
    .await
    .unwrap();
    (session.unwrap(), source, service_peer)
}
fn direct() -> ExistingRsdRoute {
    ExistingRsdRoute::direct(SocketAddrV6::new(
        "fe80::1234".parse().unwrap(),
        12345,
        7,
        2,
    ))
    .unwrap()
}

#[tokio::test]
async fn existing_rsd_service_session_uses_current_canonical_port_scope_and_real_io_then_eof() {
    let (session, source, mut peer) = prepared(directory(), direct()).await;
    assert_eq!(session.selection().handshake_mode, RsdHandshakeMode::Direct);
    assert_eq!(
        session.selection().attempted_modes,
        [RsdHandshakeMode::Direct]
    );
    let mut connection = session
        .open_from_source(&source, UDID, SERVICE, ExistingRsdServiceKind::RawDtx, 64)
        .await
        .unwrap();
    let selected = source.routes.lock().unwrap()[1].endpoint();
    assert_eq!(
        (selected.port(), selected.scope_id(), selected.flowinfo()),
        (23456, 2, 7)
    );
    let (result, ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(
            session.run(async {
                connection.stream.write_all(b"query").await?;
                let mut response = [0; 5];
                connection.stream.read_exact(&mut response).await?;
                assert_eq!(&response, b"reply");
                Ok(())
            }),
            async {
                let mut query = [0; 5];
                peer.read_exact(&mut query).await.unwrap();
                assert_eq!(&query, b"query");
                peer.write_all(b"reply").await.unwrap();
            }
        )
    })
    .await
    .unwrap();
    result.unwrap();
    drop(connection);
    let (cleanup, ()) = tokio::join!(session.close(OBSERVE), async {
        let mut tail = Vec::new();
        peer.read_to_end(&mut tail).await.unwrap();
        assert!(tail.is_empty());
        peer.shutdown().await.unwrap();
    });
    let cleanup = cleanup.unwrap();
    assert_eq!(cleanup.closed_sockets, 1);
    assert!(cleanup.strict_eof);
}

#[tokio::test]
async fn existing_rsd_service_missing_canonical_port_never_falls_back_to_shim_or_dials() {
    let mut value = directory();
    let XpcValue::Dictionary(services) = value.get_mut("Services").unwrap() else {
        unreachable!()
    };
    let XpcValue::Dictionary(canonical) = services.get_mut(SERVICE).unwrap() else {
        unreachable!()
    };
    canonical.shift_remove("Port");
    services.insert(
        format!("{SERVICE}.shim.remote"),
        XpcValue::Dictionary(indexmap::IndexMap::from([(
            "Port".into(),
            XpcValue::Uint64(34567),
        )])),
    );
    let (session, source, _peer) = prepared(value, direct()).await;
    let error = session
        .open_from_source(&source, UDID, SERVICE, ExistingRsdServiceKind::RawDtx, 64)
        .await
        .unwrap_err();
    assert_eq!(crate::classify_error(&error), ErrorClass::Protocol);
    assert_eq!(source.routes.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn existing_rsd_service_xpc_shim_and_wrong_identity_reject_before_socket_io() {
    let mut value = directory();
    let XpcValue::Dictionary(services) = value.get_mut("Services").unwrap() else {
        unreachable!()
    };
    let canonical = services.shift_remove(SERVICE).unwrap();
    services.insert(format!("{SERVICE}.shim.remote"), canonical);
    let (session, source, _peer) = prepared(value, direct()).await;
    assert_eq!(
        crate::classify_error(
            &session
                .open_from_source(
                    &source,
                    UDID,
                    SERVICE,
                    ExistingRsdServiceKind::RemoteXpc,
                    64
                )
                .await
                .unwrap_err()
        ),
        ErrorClass::Unsupported
    );
    assert_eq!(
        crate::classify_error(
            &session
                .open_from_source(
                    &source,
                    "another-target",
                    SERVICE,
                    ExistingRsdServiceKind::RawDtx,
                    64
                )
                .await
                .unwrap_err()
        ),
        ErrorClass::Protocol
    );
    assert_eq!(source.routes.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn existing_rsd_service_proxy_prelude_and_cumulative_read_write_guard_have_separate_cleanup()
{
    let route =
        ExistingRsdRoute::via_userspace_proxy("[fd00::1]:12345".parse().unwrap(), 54321).unwrap();
    let (session, source, mut peer) = prepared(directory(), route).await;
    let (connection, ()) = tokio::join!(
        session.open_from_source(&source, UDID, SERVICE, ExistingRsdServiceKind::RawDtx, 24),
        async {
            let mut prelude = [0; 20];
            peer.read_exact(&mut prelude).await.unwrap();
            assert_eq!(&prelude[..16], &route.endpoint.ip().octets());
            assert_eq!(&prelude[16..], &23456u32.to_le_bytes());
        }
    );
    let mut connection = connection.unwrap();
    connection.stream.write_all(b"1234").await.unwrap();
    let error = connection.stream.write_all(b"x").await.unwrap_err();
    assert_eq!(crate::classify_error(&error), ErrorClass::ResourceLimit);
    drop(connection);
    let (closed, ()) = tokio::join!(session.close(OBSERVE), async {
        let mut tail = Vec::new();
        peer.read_to_end(&mut tail).await.unwrap();
        assert_eq!(&tail, b"1234");
        peer.write_all(b"late unread reply").await.unwrap();
        peer.shutdown().await.unwrap();
    });
    let closed = closed.unwrap();
    assert_eq!(closed.drained_bytes, 17);
    assert!(closed.strict_eof);
}

#[tokio::test]
async fn existing_rsd_service_tcp_deadline_keeps_original_elapsed_and_closes_after_unread_reply() {
    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let mut peer = listener.accept().await.unwrap().0;
    let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
    let owner = ExistingRsdServiceOwner(Arc::new(StdMutex::new(SocketState {
        stream: Some(Box::new(stream)),
        remaining: 64,
        terminal: false,
        write_closed: false,
        eof_seen: false,
        drained_bytes: 0,
        close_error: None,
        permit: Some(permit),
    })));
    let clock = Clock::new(Duration::from_millis(40)).unwrap();
    let mut guarded = ServiceWireStream {
        owner: owner.clone(),
        deadline: clock.deadline,
        timer: Box::pin(tokio::time::sleep_until(clock.deadline)),
        registry: None,
    };
    let phase = clock
        .run(async {
            let mut byte = [0];
            guarded.read_exact(&mut byte).await?;
            Ok(())
        })
        .await
        .unwrap_err();
    assert_eq!(crate::classify_error(&phase), ErrorClass::Timeout);
    let mut cause: Option<&(dyn Error + 'static)> = Some(&phase);
    let mut elapsed = false;
    for _ in 0..32 {
        let Some(current) = cause else { break };
        if current.is::<tokio::time::error::Elapsed>() {
            elapsed = true;
            break;
        }
        cause = if let Some(io) = current.downcast_ref::<io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| current.source())
        } else {
            current.source()
        };
    }
    assert!(elapsed, "original business timer object must survive");
    drop(guarded);
    let (closed, ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(owner.close(OBSERVE), async {
            let mut input = Vec::new();
            peer.read_to_end(&mut input).await.unwrap();
            assert!(input.is_empty());
            peer.write_all(b"reply after business expiry")
                .await
                .unwrap();
            peer.shutdown().await.unwrap();
        })
    })
    .await
    .unwrap();
    let closed = closed.unwrap();
    assert!(closed.strict_eof);
    assert_eq!(closed.drained_bytes, 27);
    assert!(owner.closed());
}

#[tokio::test]
async fn existing_rsd_service_cleanup_timeout_retains_original_owner_and_can_resume_eof() {
    let (stream, mut peer) = tokio::io::duplex(128);
    let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
    let owner = ExistingRsdServiceOwner(Arc::new(StdMutex::new(SocketState {
        stream: Some(Box::new(stream)),
        remaining: 64,
        terminal: true,
        write_closed: false,
        eof_seen: false,
        drained_bytes: 0,
        close_error: None,
        permit: Some(permit),
    })));
    let error = owner.close(Duration::from_millis(20)).await.unwrap_err();
    let CoreError::Io(io) = &error else {
        panic!("typed retained cleanup expected")
    };
    let retained = io
        .get_ref()
        .unwrap()
        .downcast_ref::<ExistingRsdCleanupFailure>()
        .unwrap();
    assert_eq!(retained.pending_sockets(), 1);
    assert_eq!(
        crate::classify_error(retained.primary()),
        ErrorClass::Timeout
    );
    let (closed, ()) = tokio::join!(retained.retry(OBSERVE), async {
        assert_eq!(
            peer.read_u8().await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        peer.shutdown().await.unwrap();
    });
    assert!(closed.unwrap().strict_eof);
    assert!(owner.closed());
}

#[tokio::test]
async fn existing_rsd_service_close_seals_new_opens_and_collects_same_poll_late_tcp_socket() {
    struct LateSource {
        address: std::net::SocketAddr,
        ready: AtomicBool,
        gate: Semaphore,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl Source for LateSource {
        fn identity(&self) -> BoxFuture<'_, Result<uuid::Uuid, XpcError>> {
            unreachable!("session already observed")
        }
        fn dial(&self, _: ExistingRsdRoute) -> BoxFuture<'_, Result<ServiceStream, CoreError>> {
            Box::pin(async {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let stream = tokio::net::TcpStream::connect(self.address).await?;
                self.ready.store(true, Ordering::SeqCst);
                self.gate.acquire().await.unwrap().forget();
                Ok(Box::new(stream) as ServiceStream)
            })
        }
    }
    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let source = LateSource {
        address: listener.local_addr().unwrap(),
        ready: AtomicBool::new(false),
        gate: Semaphore::new(0),
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let session = ExistingRsdSession {
        route: ExistingRsdRoute::direct("[::1]:12345".parse().unwrap()).unwrap(),
        clock: Clock::new(OBSERVE).unwrap(),
        observation: ExistingRsdObservation::new(
            Some(UDID.into()),
            Some(std::collections::HashMap::from([(
                SERVICE.into(),
                ExistingRsdService::new(Some(23456), None),
            )])),
        ),
        selection: RsdSelectionReceipt {
            handshake_mode: RsdHandshakeMode::Direct,
            attempted_modes: vec![RsdHandshakeMode::Direct],
        },
        connections: Arc::new(Semaphore::new(4)),
        sockets: ServiceRegistry::new(),
    };
    let mut open = Box::pin(session.open_from_source(
        &source,
        UDID,
        SERVICE,
        ExistingRsdServiceKind::RawDtx,
        64,
    ));
    tokio::time::timeout(OBSERVE, async {
        while !source.ready.load(Ordering::SeqCst) {
            assert!(futures_util::poll!(&mut open).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut peer = listener.accept().await.unwrap().0;
    let mut close = Box::pin(session.close(OBSERVE));
    assert!(futures_util::poll!(&mut close).is_pending());
    let rejected = session
        .open_from_source(&source, UDID, SERVICE, ExistingRsdServiceKind::RawDtx, 64)
        .await
        .unwrap_err();
    assert_eq!(crate::classify_error(&rejected), ErrorClass::Cancelled);
    assert_eq!(source.calls.load(Ordering::SeqCst), 1);
    source.gate.add_permits(1);
    let original = open.await.unwrap_err();
    assert_eq!(crate::classify_error(&original), ErrorClass::Cancelled);
    let (closed, ()) = tokio::time::timeout(OBSERVE, async {
        tokio::join!(close, async {
            let mut bytes = Vec::new();
            peer.read_to_end(&mut bytes).await.unwrap();
            assert!(
                bytes.is_empty(),
                "closed admission must emit no service protocol bytes"
            );
            peer.shutdown().await.unwrap();
        })
    })
    .await
    .unwrap();
    let closed = closed.unwrap();
    assert_eq!((closed.opened_sockets, closed.closed_sockets), (1, 1));
    assert!(closed.strict_eof);
    assert_eq!(session.connections.available_permits(), 4);
}

use super::*;

#[tokio::test]
async fn lifecycle_tunnel_server_shutdown_is_retained_for_late_waiter() {
    let source = FakeSource::new();
    let state = state(source, TUNNEL_CONNECT_TIMEOUT);
    let mut shutdown = state.prepare_cutover::<1>().unwrap();
    state.notify_shutdown(&mut shutdown);
    drop(shutdown);
    tokio::time::timeout(Duration::from_millis(50), state.wait_shutdown())
        .await
        .expect("shutdown was lost before the waiter subscribed");
}

async fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

struct ScannerFrameSource {
    base: Arc<FakeSource>,
    frame_drops: AtomicUsize,
}

impl Source for ScannerFrameSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            struct Frame<'a>(&'a AtomicUsize);
            impl Drop for Frame<'_> {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
            }
            let _original_frame = Frame(&self.frame_drops);
            self.base.connect(udid, route, mode).await
        })
    }

    fn usb_devices(&self) -> BoxFuture<'_, source::UsbSnapshot> {
        self.base.usb_devices()
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        self.base.paired_devices()
    }
}

#[tokio::test]
async fn lifecycle_tunnel_server_inline_scanner_drops_original_frame_before_blocked_cleanup_finishes(
) {
    let base = FakeSource::new();
    base.usb.lock().unwrap().push(A.into());
    base.script(A, [Behavior::Pending]);
    let source = Arc::new(ScannerFrameSource {
        base: base.clone(),
        frame_drops: AtomicUsize::new(0),
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let (listener, port) = listener().await;
    let signal = Arc::new(Semaphore::new(0));
    let entered_signal = signal.clone();
    let mut owner = Box::pin(
        server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_millis(5),
            server::CLEANUP_TIMEOUT,
            async move {
                entered_signal.acquire().await.unwrap().forget();
                Ok(())
            },
        )
        .unwrap(),
    );
    tokio::time::timeout(LOCAL_BUDGET, async {
        while base.count(A) == 0 {
            assert!(futures_util::poll!(&mut owner).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.frame_drops.load(Ordering::SeqCst), 0);
    let table = state.inner.tunnels.write().await;
    signal.add_permits(1);
    // The parent is still alive and cleanup cannot pass this table lock.
    // Only explicit destruction of its original scanner can drop this frame.
    assert!(futures_util::poll!(&mut owner).is_pending());
    assert_eq!(source.frame_drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    drop(table);
    tokio::time::timeout(LOCAL_BUDGET, owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.frame_drops.load(Ordering::SeqCst), 1);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test]
async fn lifecycle_tunnel_server_shutdown_before_start_does_not_poll_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut shutdown = state.prepare_cutover::<1>().unwrap();
    state.notify_shutdown(&mut shutdown);
    drop(shutdown);
    let (listener, port) = listener().await;
    tokio::time::timeout(
        LOCAL_BUDGET,
        server::prepare(
            listener,
            agent_router(state.clone()),
            state,
            Duration::from_millis(5),
            server::CLEANUP_TIMEOUT,
            std::future::pending(),
        )
        .unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(A), 0);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test]
async fn lifecycle_tunnel_server_http_shutdown_drops_pending_scan_and_clears_owned_device() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), B).await.unwrap();
    let (listener, port) = listener().await;
    let signal_polls = Arc::new(AtomicUsize::new(0));
    let signal_entered = signal_polls.clone();
    let owner_state = state.clone();
    let owner = tokio::spawn(async move {
        server::prepare(
            listener,
            agent_router(owner_state.clone()),
            owner_state,
            Duration::from_millis(5),
            server::CLEANUP_TIMEOUT,
            async move {
                signal_entered.fetch_add(1, Ordering::SeqCst);
                std::future::pending().await
            },
        )
        .unwrap()
        .await
    });
    source.entered(A, 1).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/shutdown"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"],
        true
    );
    tokio::time::timeout(LOCAL_BUDGET, owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(signal_polls.load(Ordering::SeqCst), 1);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test]
async fn lifecycle_tunnel_server_caller_drop_owns_scanner_without_detached_monitor() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), B).await.unwrap();
    let (listener, port) = listener().await;
    let mut owner = Box::pin(
        server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_millis(5),
            server::CLEANUP_TIMEOUT,
            std::future::pending(),
        )
        .unwrap(),
    );
    tokio::time::timeout(LOCAL_BUDGET, async {
        while source.count(A) == 0 {
            assert!(futures_util::poll!(&mut owner).is_pending());
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(source.active.load(Ordering::SeqCst), 1);
    drop(owner);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    tokio::time::timeout(LOCAL_BUDGET, state.wait_shutdown())
        .await
        .unwrap();
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

#[tokio::test]
async fn lifecycle_tunnel_server_signal_failure_keeps_native_io_and_clears_state() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    start(state.clone(), B).await.unwrap();
    let (listener, _) = listener().await;
    let error = server::prepare(
        listener,
        agent_router(state.clone()),
        state.clone(),
        Duration::from_millis(5),
        server::CLEANUP_TIMEOUT,
        async {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "synthetic-private-signal-input",
            ))
        },
    )
    .unwrap()
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    assert!(!crate::output::error_message(&error).contains("synthetic-private"));
}

#[tokio::test]
async fn lifecycle_tunnel_server_cleanup_lock_deadline_preserves_primary_and_elapsed() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut clear = state.prepare_cutover::<2>().unwrap();
    start(state.clone(), B).await.unwrap();
    let held = state.inner.tunnels.write().await;
    let (listener, _) = listener().await;
    let error = tokio::time::timeout(
        LOCAL_BUDGET,
        server::prepare(
            listener,
            agent_router(state.clone()),
            state.clone(),
            Duration::from_millis(5),
            Duration::from_millis(30),
            async {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "synthetic-private-signal-input",
                ))
            },
        )
        .unwrap(),
    )
    .await
    .unwrap()
    .unwrap_err();
    let dual = error.downcast_ref::<server::CleanupFailure>().unwrap();
    assert_eq!(
        dual.primary
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::BrokenPipe
    );
    assert!(dual.cleanup.is::<tokio::time::error::Elapsed>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Disconnected
    );
    assert!(!format!("{dual:?} {dual}").contains("synthetic-private"));
    assert!(!crate::output::error_message(&error).contains("synthetic-private"));
    // Caller Drop cannot clear a table held by another owner. The fixture
    // releases that lock and explicitly completes its remaining cleanup.
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    drop(held);
    state.clear(&mut clear).await;
    drop(clear);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_server_graceful_http_drain_uses_cleanup_deadline() {
    let source = FakeSource::new();
    let state = state(source, TUNNEL_CONNECT_TIMEOUT);
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let route_entered = entered.clone();
    let route_release = release.clone();
    let app = agent_router(state.clone()).route(
        "/synthetic-stall",
        get(move || {
            let entered = route_entered.clone();
            let release = route_release.clone();
            async move {
                entered.add_permits(1);
                release.acquire().await.unwrap().forget();
                "released"
            }
        }),
    );
    let signal = Arc::new(Semaphore::new(0));
    let owner_signal = signal.clone();
    let (listener, port) = listener().await;
    let owner = tokio::spawn(async move {
        server::prepare(
            listener,
            app,
            state,
            Duration::from_secs(10),
            Duration::from_millis(30),
            async move {
                owner_signal.acquire().await.unwrap().forget();
                Ok(())
            },
        )
        .unwrap()
        .await
    });
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream
        .write_all(b"GET /synthetic-stall HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    tokio::time::timeout(LOCAL_BUDGET, entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    signal.add_permits(1);
    let error = tokio::time::timeout(LOCAL_BUDGET, owner)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    // Axum's delegated request task is still pending after the capped drain.
    // Release it and observe response/EOF; do not claim it was forcibly joined.
    release.add_permits(1);
    let mut response = Vec::new();
    tokio::time::timeout(LOCAL_BUDGET, stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(String::from_utf8(response).unwrap().contains("released"));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn lifecycle_tunnel_server_real_sigint_cancels_scan_and_exits_runtime() {
    const HELPER: &str = "RUST_IOS_TUNNEL_SERVER_SIGINT_HELPER";
    if std::env::var_os(HELPER).is_some() {
        let source = FakeSource::new();
        source.usb.lock().unwrap().push(A.into());
        source.script(A, [Behavior::Pending]);
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        start(state.clone(), B).await.unwrap();
        let (listener, _) = listener().await;
        let mut owner = Box::pin(
            server::prepare(
                listener,
                agent_router(state.clone()),
                state.clone(),
                Duration::from_millis(5),
                server::CLEANUP_TIMEOUT,
                tokio::signal::ctrl_c(),
            )
            .unwrap(),
        );
        while source.count(A) == 0 {
            assert!(futures_util::poll!(&mut owner).is_pending());
            tokio::task::yield_now().await;
        }
        // serve polls its one real signal before the scanner. Thus observing
        // source entry means Ctrl-C registration already happened.
        println!("SYNTHETIC_TUNNEL_SERVER_SIGNAL_READY");
        use std::io::Write;
        std::io::stdout().flush().unwrap();
        owner.await.unwrap();
        assert_eq!(source.active.load(Ordering::SeqCst), 0);
        assert_eq!(source.drops.load(Ordering::SeqCst), 1);
        assert!(state.list_records().await.unwrap().as_slice().is_empty());
        return;
    }

    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cmd::tunnel::tests::lifecycle_tests::server_tests::lifecycle_tunnel_server_real_sigint_cancels_scan_and_exits_runtime", "--nocapture"])
        .env(HELPER, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    tokio::time::timeout(LOCAL_BUDGET, async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("child exited before signal registration");
            if line == "SYNTHETIC_TUNNEL_SERVER_SIGNAL_READY" {
                break;
            }
        }
    })
    .await
    .unwrap();
    // Only the child created above receives the real OS signal.
    assert_eq!(unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT) }, 0);
    let status = tokio::time::timeout(LOCAL_BUDGET, child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "signal child failed: {status}");
}

//! Acquired loopback IO and same-poll lifecycle cancellation through the scanner.
use super::*;

struct PairedSource {
    base: Arc<LocalSource>,
    control: StdMutex<Option<TunnelAgentState>>,
    stream: StdMutex<Option<TcpStream>>,
    terminal: bool,
    failure: bool,
    paired_calls: AtomicUsize,
}

impl Source for PairedSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.base.connect(udid, route, mode)
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.base.usb_devices()
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        Box::pin(async move {
            self.paired_calls.fetch_add(1, Ordering::SeqCst);
            let mut stream = self.stream.lock().unwrap().take().unwrap();
            let state = self.control.lock().unwrap().take().unwrap();
            let error = stream
                .read_exact(&mut [0; 2])
                .await
                .expect_err("actual one-byte loopback EOF fixture");
            // Acquire the original IO first; change the epoch before the same
            // discovery poll returns, with no hook mutex/watch Ref retained.
            if self.terminal {
                state.notify_shutdown();
            } else {
                state.clear().await;
            }
            if self.failure {
                Err(error.into())
            } else {
                Ok(Vec::new())
            }
        })
    }
}

async fn exercise(terminal: bool, failure: bool) -> Result<()> {
    let expected = baseline();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (stream, peer) = tokio::join!(
        TcpStream::connect(listener.local_addr().unwrap()),
        listener.accept()
    );
    let stream = stream.unwrap();
    let (mut peer, _) = peer.unwrap();
    peer.write_all(&[7]).await.unwrap();
    peer.shutdown().await.unwrap();
    drop(peer);
    drop(listener);
    let base = LocalSource::new();
    let source = Arc::new(PairedSource {
        base: base.clone(),
        control: StdMutex::new(None),
        stream: StdMutex::new(Some(stream)),
        terminal,
        failure,
        paired_calls: AtomicUsize::new(0),
    });
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::trusted_test_local(),
        Upstreams::default(),
        source.clone(),
        OBSERVE,
    );
    publish(&state, Route::Wifi(None)).await.unwrap();
    let native = base.handle();
    *source.control.lock().unwrap() = Some(state.clone());
    let result = state.refresh_once().await;
    let calls = (
        base.connect_calls.load(Ordering::SeqCst),
        base.usb_calls.load(Ordering::SeqCst),
        source.paired_calls.load(Ordering::SeqCst),
    );
    let stopped = !native.is_alive();
    let not_ready = !state.is_ready().await;
    // Release any unconsumed one-shot control if admission unexpectedly failed.
    let pending = source.control.lock().unwrap().take();
    drop(pending);
    let recovery = finish(&state, &base, expected).await;
    assert!(recovery && native.shutdown_status().is_ok());
    assert!(stopped && not_ready);
    assert_eq!(calls, (1, 1, 1));
    result
}

fn original_io(result: Result<()>) {
    let error = result.unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::UnexpectedEof
    );
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Io
    );
}

#[tokio::test]
async fn paired_discovery_acquired_native_io_survives_same_poll_clear() {
    original_io(exercise(false, true).await);
}

#[tokio::test]
async fn paired_discovery_acquired_native_io_survives_same_poll_close() {
    original_io(exercise(true, true).await);
}

#[tokio::test]
async fn paired_discovery_late_success_after_clear_stays_unpublished() {
    exercise(false, false).await.unwrap();
}

#[tokio::test]
async fn paired_discovery_late_success_after_close_stays_unpublished() {
    exercise(true, false).await.unwrap();
}

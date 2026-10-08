//! Epoch changes after the first paired reply check, before the final fence.
use super::*;
use futures_util::FutureExt;

struct FenceSource {
    base: Arc<LocalSource>,
    stream: StdMutex<Option<TcpStream>>,
    failure: bool,
    terminal: bool,
    paired_calls: AtomicUsize,
}

impl Source for FenceSource {
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
            let error = stream
                .read_exact(&mut [0; 2])
                .await
                .expect_err("actual partial loopback EOF");
            if self.failure {
                Err(error.into())
            } else if self.terminal {
                Ok(Vec::new())
            } else {
                Ok(vec![ios_core::device::PairedMobdev2Device {
                    udid: "stale-paired-target".into(),
                    host: "192.0.2.19".into(),
                }])
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
    peer.write_all(&[9]).await.unwrap();
    peer.shutdown().await.unwrap();
    drop(peer);
    drop(listener);
    let base = LocalSource::new();
    let source = Arc::new(FenceSource {
        base: base.clone(),
        stream: StdMutex::new(Some(stream)),
        failure,
        terminal,
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
    let control = state.clone();
    let fired = Arc::new(AtomicUsize::new(0));
    let observed = fired.clone();
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        // The first epoch check has passed and no watch/table/hook guard is held.
        // Poll actual clear to completion on these uncontended metadata locks.
        // No alternative clear implementation or synthetic error is used.
        if terminal {
            control.notify_shutdown();
        } else {
            control
                .clear()
                .now_or_never()
                .expect("actual clear completes without a held metadata lock");
        }
        observed.fetch_add(1, Ordering::SeqCst);
    }));
    let result = state.refresh_once().await;
    let calls = (
        base.connect_calls.load(Ordering::SeqCst),
        base.usb_calls.load(Ordering::SeqCst),
        source.paired_calls.load(Ordering::SeqCst),
    );
    let stopped = !native.is_alive();
    let not_ready = !state.is_ready().await;
    // Break the test-only one-shot state cycle even on unexpected early return.
    let unused = state.inner.paired_after_first_check.lock().unwrap().take();
    drop(unused);
    let recovery = finish(&state, &base, expected).await;
    assert!(recovery && native.shutdown_status().is_ok());
    assert!(stopped && not_ready);
    assert_eq!(fired.load(Ordering::SeqCst), 1);
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
async fn paired_final_fence_preserves_acquired_native_io_after_clear() {
    original_io(exercise(false, true).await);
}

#[tokio::test]
async fn paired_final_fence_preserves_acquired_native_io_after_close() {
    original_io(exercise(true, true).await);
}

#[tokio::test]
async fn paired_final_fence_discards_nonempty_stale_success_after_clear() {
    exercise(false, false).await.unwrap();
}

#[tokio::test]
async fn paired_final_fence_discards_empty_stale_success_after_close() {
    exercise(true, false).await.unwrap();
}

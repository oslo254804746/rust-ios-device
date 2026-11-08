use super::*;
use std::sync::Condvar;

const HOST_BUDGET: Duration = Duration::from_secs(2);

struct PublicationPause {
    entered: AtomicBool,
    changed: Notify,
    released: StdMutex<bool>,
    release: Condvar,
}

impl PublicationPause {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            entered: AtomicBool::new(false),
            changed: Notify::new(),
            released: StdMutex::new(false),
            release: Condvar::new(),
        })
    }

    fn pause(&self) {
        self.entered.store(true, Ordering::SeqCst);
        self.changed.notify_waiters();
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.release.wait(released).unwrap();
        }
    }

    async fn entered(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.entered.load(Ordering::SeqCst) {
                return;
            }
            changed.await;
        }
    }

    fn release(&self) {
        let mut released = self
            .released
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *released = true;
        self.release.notify_all();
    }
}

struct ResumePublication(Arc<PublicationPause>);

impl Drop for ResumePublication {
    fn drop(&mut self) {
        // A failed observation or assertion cannot leave a worker parked in the
        // fixture's synchronous checkpoint.
        self.0.release();
    }
}

struct PausedDevice {
    device: Box<dyn source::FakeDevice>,
    checks: AtomicUsize,
    pause: Arc<PublicationPause>,
}

impl source::FakeDevice for PausedDevice {
    fn record(&self) -> TunnelRecord {
        self.device.record()
    }

    fn is_alive(&self) -> bool {
        if self.checks.fetch_add(1, Ordering::SeqCst) == 1 {
            // The actual manager has its table guard and reached its final
            // local liveness check. In the old implementation the epoch check
            // has already succeeded. No lifecycle watch is borrowed here.
            self.pause.pause();
        }
        self.device.is_alive()
    }

    fn device_id(&self) -> u32 {
        self.device.device_id()
    }
}

struct PausedSource {
    source: Arc<FakeSource>,
    pause: Arc<PublicationPause>,
}

impl Source for PausedSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            match self.source.connect(udid, route, mode).await? {
                ManagedDevice::Fake(device) => Ok(ManagedDevice::Fake(Box::new(PausedDevice {
                    device,
                    checks: AtomicUsize::new(0),
                    pause: self.pause.clone(),
                }))),
                ManagedDevice::Native(_) => unreachable!("fixture returned a native device"),
            }
        })
    }

    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.source.usb_devices()
    }

    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<PairedMobdev2Device>>> {
        self.source.paired_devices()
    }
}

fn assert_cancelled(error: &anyhow::Error) {
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(!crate::output::error_message(error).contains(A));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lifecycle_tunnel_shutdown_fences_publication_close_after_final_check_rejects_all_routes() {
    for (route, fallback) in [
        (Some(StartTunnelConnectionType::Usbmux), false),
        (Some(StartTunnelConnectionType::Usb), false),
        (Some(StartTunnelConnectionType::Wifi), false),
        (None, true),
    ] {
        let source = FakeSource::new();
        if fallback {
            source.script(
                A,
                [
                    Behavior::Failure(std::io::ErrorKind::BrokenPipe),
                    Behavior::Success,
                ],
            );
            source.paired.lock().unwrap().push(PairedMobdev2Device {
                udid: A.into(),
                host: "192.0.2.2".into(),
            });
        }
        let pause = PublicationPause::new();
        let resume = ResumePublication(pause.clone());
        let state = TunnelAgentState::with_source(
            TunMode::Userspace,
            AccessControl::trusted_test_local(),
            Upstreams::default(),
            Arc::new(PausedSource {
                source: source.clone(),
                pause: pause.clone(),
            }),
            TUNNEL_CONNECT_TIMEOUT,
        );
        let mut prepared_outer_0 = state.prepare_cutover::<2>().unwrap();
        let mut prepared_outer_1 = state.prepare_cutover::<2>().unwrap();

        let mut close_input = state.prepare_cutover::<1>().unwrap();
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let request_state = state.clone();
        let mut request =
            tokio::spawn(async move { request_state.ensure_tunnel(A, None, route).await });
        let entered = tokio::time::timeout(HOST_BUDGET, pause.entered()).await;

        let (closed, observed_close) = tokio::sync::oneshot::channel();
        let close_state = state.clone();
        let closer = std::thread::spawn(move || {
            close_state.notify_shutdown(&mut close_input);
            let _ = closed.send(());
        });
        // Wait for close to finish before allowing the manager to insert. If a
        // future change incorrectly borrows the watches around is_alive, the
        // bounded observation still releases the checkpoint before any joins.
        let closed = tokio::time::timeout(HOST_BUDGET, observed_close).await;
        pause.release();
        let outcome = match tokio::time::timeout(HOST_BUDGET, &mut request).await {
            Ok(outcome) => Some(outcome),
            Err(_) => {
                request.abort();
                let _ = request.await;
                None
            }
        };
        let close_thread = closer.join();
        drop(resume);

        // Capture the old publication before cleanup so cleanup cannot hide an
        // insertion that happened after the completed close.
        let published = state.inner.tunnels.read().await.contains_key(A);
        let released = source.drops.load(Ordering::SeqCst);
        let serial_released = tokio::time::timeout(HOST_BUDGET, ticket.serial())
            .await
            .is_ok();
        state.clear(&mut prepared_outer_0).await;
        drop(prepared_outer_0);
        let restarted = state.ensure_tunnel(A, None, route).await;
        state.clear(&mut prepared_outer_1).await;
        drop(prepared_outer_1);

        // Every task and OS thread is reclaimed before checking outcomes,
        // including the expected failure when running the old implementation.
        assert!(entered.is_ok(), "manager missed its publication checkpoint");
        assert!(
            matches!(closed, Ok(Ok(()))),
            "close waited for local liveness"
        );
        assert!(close_thread.is_ok(), "shutdown thread panicked");
        let error = outcome
            .expect("manager worker did not finish after checkpoint release")
            .expect("manager worker panicked")
            .unwrap_err();
        assert_cancelled(&error);
        assert_cancelled(&restarted.unwrap_err());
        assert!(!published, "old admission inserted after close completed");
        assert_eq!(released, 1);
        assert!(serial_released);
        assert!(state.inner.tunnels.read().await.is_empty());
        assert!(state.inner.failed_attempts.read().await.is_empty());
        assert_eq!(source.count(A), 1 + usize::from(fallback));
        assert_eq!(source.active.load(Ordering::SeqCst), 0);
        assert_eq!(
            source.exited.load(Ordering::SeqCst),
            1 + usize::from(fallback)
        );
        assert_eq!(
            source.usb_calls.load(Ordering::SeqCst),
            usize::from(!fallback && route != Some(StartTunnelConnectionType::Wifi))
        );
        assert_eq!(
            source.paired_calls.load(Ordering::SeqCst),
            usize::from(fallback)
        );
        assert_eq!(
            state.inner.lifecycle.connects.available_permits(),
            lifecycle::MAX_CONNECTS
        );
    }
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_publication_success_before_close_is_released_by_clear() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<1>().unwrap();
    let mut control_input_1 = state.prepare_cutover::<1>().unwrap();

    let record = start(state.clone(), A).await.unwrap();
    assert_eq!(state.get_record(A).await.unwrap(), Some(record));
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.notify_shutdown(&mut control_input_0);
    drop(control_input_0);
    // A commit accepted before close remains owned until table cleanup.
    assert!(state.inner.tunnels.read().await.contains_key(A));
    state.clear(&mut control_input_1).await;
    drop(control_input_1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_cancelled(&start(state.clone(), A).await.unwrap_err());
    assert_eq!(source.count(A), 1);
    assert!(state.inner.tunnels.read().await.is_empty());
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_publication_stale_stop_and_clear_tickets_skip_action() {
    for clear in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let mut clear_input = clear.then(|| state.prepare_cutover::<2>().unwrap());
        let mut stop_input = (!clear).then(|| state.prepare_cutover::<3>().unwrap());

        let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        if clear {
            state.inner.lifecycle.clear(clear_input.as_mut().unwrap());
        } else {
            assert!(!state.remove_tunnel(A, stop_input.as_mut().unwrap()).await);
        }
        let called = AtomicBool::new(false);
        let error = ticket
            .commit(|| {
                called.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err();
        assert_cancelled(&error);
        assert!(!called.load(Ordering::SeqCst));
        let fresh = state.inner.lifecycle.ticket(A).await.unwrap();
        fresh
            .commit(|| {
                called.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        assert!(called.load(Ordering::SeqCst));
        assert!(source.calls.lock().unwrap().is_empty());
        state.clear(&mut control_input_0).await;
        drop(control_input_0);
    }
}

#[tokio::test]
async fn lifecycle_tunnel_shutdown_fences_publication_holds_shutdown_guard_through_action() {
    let lifecycle = Arc::new(Lifecycle::new());
    let mut close_input = lifecycle
        .prepare_cutover::<1>(lifecycle.byte_bank())
        .unwrap();
    let mut clear_input = lifecycle
        .prepare_cutover::<1>(lifecycle.byte_bank())
        .unwrap();
    let ticket = lifecycle.ticket(A).await.unwrap();
    let pause = PublicationPause::new();
    let resume = ResumePublication(pause.clone());
    let sequence = Arc::new(AtomicUsize::new(0));
    let publication_order = Arc::new(AtomicUsize::new(usize::MAX));
    let close_order = Arc::new(AtomicUsize::new(usize::MAX));
    let commit_pause = pause.clone();
    let commit_sequence = sequence.clone();
    let committed = publication_order.clone();
    let publisher = std::thread::spawn(move || {
        ticket.commit(|| {
            commit_pause.pause();
            committed.store(
                commit_sequence.fetch_add(1, Ordering::SeqCst),
                Ordering::SeqCst,
            );
            Ok(())
        })
    });
    let entered = tokio::time::timeout(HOST_BUDGET, pause.entered()).await;
    let (started, observed_start) = tokio::sync::oneshot::channel();
    let (closed, observed_close) = tokio::sync::oneshot::channel();
    let closing = lifecycle.clone();
    let closing_sequence = sequence.clone();
    let closed_order = close_order.clone();
    let closer = std::thread::spawn(move || {
        let _ = started.send(());
        closing.close(&mut close_input);
        closed_order.store(
            closing_sequence.fetch_add(1, Ordering::SeqCst),
            Ordering::SeqCst,
        );
        let _ = closed.send(());
    });
    let started = tokio::time::timeout(HOST_BUDGET, observed_start).await;
    // This is an observation of a blocked synchronous writer, not a scheduling
    // sleep used to create the publication race. Always release before joining.
    let closed_while_held = tokio::time::timeout(Duration::from_millis(100), observed_close).await;
    pause.release();
    let publication = publisher.join();
    let close_thread = closer.join();
    drop(resume);
    lifecycle.clear(&mut clear_input);
    let restarted = lifecycle.ticket(A).await;

    assert!(entered.is_ok(), "commit did not enter its guarded action");
    assert!(matches!(started, Ok(Ok(()))), "close thread did not start");
    assert!(
        closed_while_held.is_err(),
        "close completed inside the guarded action"
    );
    publication.expect("publication thread panicked").unwrap();
    assert!(close_thread.is_ok(), "close thread panicked");
    assert_eq!(publication_order.load(Ordering::SeqCst), 0);
    assert_eq!(close_order.load(Ordering::SeqCst), 1);
    match restarted {
        Err(error) => assert_cancelled(&error),
        Ok(_) => panic!("clear reopened terminal admission"),
    }
}

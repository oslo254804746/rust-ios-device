//! Real HTTP expiry at the final manual-intent commit, with original native IO.
use super::*;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

const REQUEST_BUDGET: Duration = Duration::from_secs(8);
const AUTHORITY_BUDGET: Duration = Duration::from_secs(6);
const CONNECTION_BUDGET: Duration = Duration::from_millis(250);
const CONNECTION_CROSS: Duration = Duration::from_millis(350);

#[derive(Clone, Copy)]
enum Clock {
    Authority(u64),
    Connection,
}

#[derive(Clone, Copy)]
enum Boundary {
    StateLock,
    PendingLock,
    Candidate,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn cross(clock: Clock) -> bool {
    match clock {
        Clock::Authority(expiry) => {
            let limit = std::time::Instant::now() + AUTHORITY_BUDGET;
            while unix_now() < expiry && std::time::Instant::now() < limit {
                std::thread::sleep(Duration::from_millis(10));
            }
            unix_now() >= expiry
        }
        Clock::Connection => {
            let started = std::time::Instant::now();
            std::thread::sleep(CONNECTION_CROSS);
            started.elapsed() >= CONNECTION_CROSS
        }
    }
}

type CrossingWorker = Arc<StdMutex<Option<std::thread::JoinHandle<()>>>>;

// A separate OS thread owns the synchronous Cell lock. The Tokio current-thread
// executor can wait for it without also owning the only task that releases it.
fn install_crossing(
    ticket: &Ticket,
    boundary: Boundary,
    clock: Clock,
) -> (Arc<AtomicBool>, CrossingWorker) {
    let crossed = Arc::new(AtomicBool::new(false));
    let worker = Arc::new(StdMutex::new(None));
    let result = crossed.clone();
    let thread_slot = worker.clone();
    let held_ticket = ticket.clone();
    ticket.on_next_manual_establish(matches!(boundary, Boundary::Candidate), move || {
        if matches!(boundary, Boundary::Candidate) {
            result.store(cross(clock), Ordering::SeqCst);
        } else {
            let (locked, acquired) = mpsc::sync_channel(0);
            let thread = std::thread::spawn(move || {
                held_ticket.hold_manual_intent(matches!(boundary, Boundary::PendingLock), || {
                    locked.send(()).unwrap();
                    result.store(cross(clock), Ordering::SeqCst);
                });
            });
            *thread_slot.lock().unwrap() = Some(thread);
            acquired.recv_timeout(AUTHORITY_BUDGET).unwrap();
        }
    });
    (crossed, worker)
}

// A concurrent publisher may restore the original row after the HTTP caller's
// initial absence check. Insert that exact owner before the new Source returns,
// so the final commit really displaces it and must undo that displacement.
struct PublicationSource {
    base: Arc<IntentSource>,
    displaced: StdMutex<Option<(TunnelAgentState, ManagedTunnel)>>,
}

impl Source for PublicationSource {
    fn connect<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        self.base.connect(udid, route, mode)
    }
    fn connect_owned<'a>(
        &'a self,
        udid: &'a str,
        route: Route,
        mode: TunMode,
        owner: &'a dyn TunnelOwner,
    ) -> BoxFuture<'a, Result<ManagedDevice>> {
        Box::pin(async move {
            let device = self.base.connect_owned(udid, route, mode, owner).await?;
            let displaced = self.displaced.lock().unwrap().take();
            if let Some((state, row)) = displaced {
                let old = state
                    .inner
                    .tunnels
                    .write()
                    .await
                    .insert(udid.to_owned(), row);
                drop(old);
            }
            Ok(device)
        })
    }
    fn usb_devices(&self) -> BoxFuture<'_, UsbSnapshot> {
        self.base.usb_devices()
    }
    fn paired_devices(&self) -> BoxFuture<'_, Result<Vec<ios_core::device::PairedMobdev2Device>>> {
        self.base.paired_devices()
    }
}

fn same_pair(left: &Option<(Arc<()>, Arc<()>)>, right: &Option<(Arc<()>, Arc<()>)>) -> bool {
    match (left, right) {
        (Some((a, b)), Some((c, d))) => Arc::ptr_eq(a, c) && Arc::ptr_eq(b, d),
        (None, None) => true,
        _ => false,
    }
}

fn pending(ticket: &Ticket) -> Option<(Arc<()>, Arc<()>)> {
    ticket
        .pending_success()
        .map(|(desired, success)| (desired, success.version))
}

async fn exercise(publication: bool, boundary: Boundary, authorization: bool) {
    let expected = baseline();
    let expiry = unix_now() + 4;
    let fixture = PolicyFixture::new(vec![
        grant(TOKEN, vec![Permission::ManageTunnels]),
        grant(REPLACEMENT, vec![Permission::ManageTunnels]).expires_at(if authorization {
            expiry
        } else {
            unix_now() + 3600
        }),
    ]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let base_source = IntentSource::new();
    let source = Arc::new(PublicationSource {
        base: base_source.clone(),
        displaced: StdMutex::new(None),
    });
    let budget = if authorization {
        AUTHORITY_BUDGET
    } else {
        CONNECTION_BUDGET
    };
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        Upstreams::default(),
        source.clone(),
        budget,
    );

    // Seed a real original failure, then establish a native manual owner while
    // its failure writer is held. This leaves a nonempty pending-success tuple
    // and a failure stamp whose exact identities an expired start must preserve.
    base_source.script(Fate::Failure);
    let failure_ticket = state
        .inner
        .lifecycle
        .ticket(AUTH_DEVICE)
        .await
        .unwrap()
        .fenced();
    let failed = state
        .refresh_connect_deferred(AUTH_DEVICE, Route::Wifi(None), &failure_ticket, None)
        .await
        .unwrap();
    state
        .flush_failure_intents(vec![FailureIntent {
            _input: None,
            udid: AUTH_DEVICE.into(),
            ticket: failure_ticket,
            failed,
        }])
        .await
        .unwrap();
    let held_failure = state.inner.failed_attempts.write().await;
    let old_failure = held_failure[AUTH_DEVICE].clone();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(REQUEST_BUDGET)
        .build()
        .unwrap();
    let started = post(
        &client,
        &base,
        AUTH_DEVICE,
        Some(TOKEN),
        Some("192.0.2.120"),
    )
    .await;
    let original = base_source.a.handle();
    let ticket = state.inner.lifecycle.ticket(AUTH_DEVICE).await.unwrap();
    let before = ticket.desired();
    let old_pending = pending(&ticket);
    let old_stamp = ticket.failure_stamp();
    let old_fenced = ticket.fenced();
    let changes = ticket.subscribe_intent();
    let watched = changes.borrow().clone();
    let table = state.inner.tunnels.read().await;
    let old_publication = table[AUTH_DEVICE].publication.clone();
    let old_route = table[AUTH_DEVICE].route_intent.clone();
    let old_owner = table[AUTH_DEVICE]._ticket.clone();
    drop(table);
    let before_native = state.inner.native.snapshot();
    let before_gates = state.inner.lifecycle.coordinator_count().await;
    let before_failure_slots = state.inner.failure_slots.available_permits();
    let before_calls = base_source.a.connect_calls.load(Ordering::SeqCst);
    let before_routes = base_source.routes.lock().unwrap().clone();
    if publication {
        let row = state
            .inner
            .tunnels
            .write()
            .await
            .remove(AUTH_DEVICE)
            .unwrap();
        *source.displaced.lock().unwrap() = Some((state.clone(), row));
    }
    let clock = if authorization {
        Clock::Authority(expiry)
    } else {
        Clock::Connection
    };
    let (crossed, worker) = install_crossing(&ticket, boundary, clock);
    let began = std::time::Instant::now();
    let response = client
        .post(format!("{base}/start-tunnel"))
        .bearer_auth(REPLACEMENT)
        .json(&json!({"udid": AUTH_DEVICE, "connection_type": "wifi", "ip": "192.0.2.121"}))
        .send()
        .await;
    let elapsed = began.elapsed();
    let response = match response {
        Ok(response) => {
            let status = response.status();
            let class = response.headers().get("x-ios-error-class").cloned();
            Ok((status, class, response.text().await))
        }
        Err(error) => Err(error),
    };
    let joined = worker
        .lock()
        .unwrap()
        .take()
        .map(|worker| worker.join())
        .transpose();
    let after = ticket.desired();
    let after_pending = pending(&ticket);
    let after_stamp = ticket.failure_stamp();
    let old_ticket_current = old_fenced.is_current();
    let watch_unchanged =
        !changes.has_changed().unwrap() && Arc::ptr_eq(&watched, &changes.borrow());
    let table = state.inner.tunnels.read().await;
    let owner_preserved = table.get(AUTH_DEVICE).is_some_and(|row| {
        Arc::ptr_eq(&row.publication, &old_publication)
            && row.route_intent == old_route
            && row._ticket.same_incarnation(&old_owner)
            && row
                .device
                .native_handle()
                .is_some_and(|native| Arc::ptr_eq(native, &original))
    });
    drop(table);
    let original_alive = original.is_alive();
    let fresh = if publication {
        Some(base_source.a.handle())
    } else {
        None
    };
    let fresh_stopped = fresh.as_ref().map_or(true, |fresh| !fresh.is_alive());
    let after_native = state.inner.native.snapshot();
    let after_gates = state.inner.lifecycle.coordinator_count().await;
    let after_failure_slots = state.inner.failure_slots.available_permits();
    let after_connect_slots = state.inner.lifecycle.connects.available_permits();
    let after_calls = base_source.a.connect_calls.load(Ordering::SeqCst);
    let after_routes = base_source.routes.lock().unwrap().clone();
    let preserved_failure = held_failure.get(AUTH_DEVICE).is_some_and(|row| {
        Arc::ptr_eq(&row.version, &old_failure.version)
            && row.last_attempt == old_failure.last_attempt
            && row.fail_count == old_failure.fail_count
            && row.identity == old_failure.identity
            && row
                .slot
                .as_ref()
                .zip(old_failure.slot.as_ref())
                .is_some_and(|(current, old)| Arc::ptr_eq(current, old))
    });
    let no_retained_injection = source.displaced.lock().unwrap().is_none();
    let abandoned_injection = source.displaced.lock().unwrap().take();
    drop(abandoned_injection);
    drop(held_failure);
    drop(old_failure);
    drop(client);
    server.abort();
    let _ = server.await;
    let originals = base_source.a.handles.lock().unwrap().clone();
    cleanup(&state, &base_source, expected).await;
    // Certify the actual native peers' strict UnexpectedEof, original joins,
    // registry/permit release and exact Tokio task baseline before assertions
    // about desired state or preservation. cleanup uses the original IO peers.
    assert!(originals
        .iter()
        .all(|native| native.shutdown_status().is_ok()));
    assert_eq!(baseline(), expected);
    assert!(joined.is_ok());

    assert_eq!(started.unwrap(), StatusCode::OK);
    assert!(old_pending.is_some() && old_stamp.is_some());
    assert!(
        crossed.load(Ordering::SeqCst),
        "the real clock did not cross at the selected commit boundary"
    );
    assert!(elapsed < REQUEST_BUDGET);
    if !authorization {
        assert!(elapsed >= CONNECTION_CROSS);
    }
    let (status, class, body) = response.unwrap();
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        class.unwrap(),
        if authorization {
            "cancelled"
        } else {
            "timeout"
        }
    );
    let body = body.unwrap();
    assert!(!body.contains(TOKEN) && !body.contains(REPLACEMENT) && !body.contains(AUTH_DEVICE));
    assert!(
        Arc::ptr_eq(&before.epoch, &after.epoch) && Arc::ptr_eq(&before.version, &after.version)
    );
    assert!(
        matches!(after.desired, intent::Desired::Manual { route, authority } if route == old_route && authority.current())
    );
    assert!(same_pair(&old_pending, &after_pending) && same_pair(&old_stamp, &after_stamp));
    assert!(
        watch_unchanged && old_ticket_current && owner_preserved && original_alive && fresh_stopped
    );
    assert!(preserved_failure && no_retained_injection);
    assert_eq!(after_native.active, before_native.active);
    assert_eq!(
        after_native.retired,
        before_native.retired + usize::from(publication)
    );
    assert_eq!(
        after_native.native_handles,
        before_native.native_handles + usize::from(publication)
    );
    assert_eq!(
        after_native.occupied(),
        before_native.occupied() + usize::from(publication)
    );
    assert_eq!(
        (
            after_native.reserved,
            after_native.inflight,
            after_native.unpublished
        ),
        (0, 0, 0)
    );
    assert_eq!(after_native.capacity, before_native.capacity);
    assert_eq!(after_gates, before_gates);
    assert_eq!(after_failure_slots, before_failure_slots);
    assert_eq!(before_failure_slots, MAX_FAILURE_RECORDS - 1);
    assert_eq!(after_connect_slots, lifecycle::MAX_CONNECTS);
    assert_eq!(after_calls, before_calls + usize::from(publication));
    assert_eq!(
        &after_routes[..before_routes.len()],
        before_routes.as_slice()
    );
    if publication {
        assert_eq!(
            after_routes.last().unwrap(),
            &(AUTH_DEVICE.into(), Route::Wifi(Some("192.0.2.121".into())))
        );
        assert!(!Arc::ptr_eq(fresh.as_ref().unwrap(), &original));
    }
}

#[tokio::test]
async fn intent_native_http_live_authority_expiry_preserves_original_intent_owner_and_pending() {
    for boundary in [
        Boundary::StateLock,
        Boundary::PendingLock,
        Boundary::Candidate,
    ] {
        exercise(false, boundary, true).await;
    }
}

#[tokio::test]
async fn intent_native_http_new_publication_authority_expiry_restores_exact_displaced_native_row() {
    for boundary in [
        Boundary::StateLock,
        Boundary::PendingLock,
        Boundary::Candidate,
    ] {
        exercise(true, boundary, true).await;
    }
}

#[tokio::test]
async fn intent_native_http_live_connection_expiry_preserves_original_intent_owner_and_pending() {
    for boundary in [
        Boundary::StateLock,
        Boundary::PendingLock,
        Boundary::Candidate,
    ] {
        exercise(false, boundary, false).await;
    }
}

#[tokio::test]
async fn intent_native_http_new_publication_connection_expiry_restores_exact_displaced_native_row()
{
    for boundary in [
        Boundary::StateLock,
        Boundary::PendingLock,
        Boundary::Candidate,
    ] {
        exercise(true, boundary, false).await;
    }
}

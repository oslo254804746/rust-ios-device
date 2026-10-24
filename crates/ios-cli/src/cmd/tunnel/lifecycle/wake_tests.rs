use super::super::{intent::Desired, metadata_vec, source, TunMode, TunnelAgentState, Upstreams};
use super::*;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
use crate::access::{AccessControl, Permission};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

struct TicketWake {
    state: TunnelAgentState,
    ticket: Ticket,
    calls: AtomicUsize,
    unlocked: AtomicBool,
}

impl Wake for TicketWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        let unlocked = self.state.inner.tunnels.try_write().is_ok()
            && self.state.inner.lifecycle.devices.try_lock().is_ok()
            && self.ticket.gate.serial.try_lock().is_ok()
            && self.ticket.gate.generation.value_is_unlocked()
            && self.state.inner.lifecycle.shutdown.value_is_unlocked()
            && self.ticket.gate.intent.mutation_guards_are_unlocked();
        self.unlocked.fetch_and(unlocked, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn ticket_authority_rollback_marker_refund_crosses_original_table_serial_and_watch_guards() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::ManageTunnels])]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {TOKEN}").parse().unwrap(),
    );
    let authority = access
        .authenticate(&headers)
        .unwrap()
        .device_authority(Permission::ManageTunnels, DEVICE)
        .ok()
        .unwrap();
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access,
        Upstreams::default(),
        Arc::new(source::NoDevices),
        Duration::from_secs(1),
    );
    let bank = state.inner.global_metadata_bytes.clone();
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let original = ticket.desired();
    let marker_bytes = ControlIdentity::body_bytes();
    let mut cutover = state.prepare_cutover::<1>().unwrap();
    let original_error = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "original late control check",
    ));
    let pointer = original_error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let pending_error = std::sync::Mutex::new(Some(original_error));
    let checks = AtomicUsize::new(0);
    let observed = Arc::new(TicketWake {
        state: state.clone(),
        ticket: ticket.clone(),
        calls: AtomicUsize::new(0),
        unlocked: AtomicBool::new(true),
    });
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(marker_bytes as u32));
    assert!(!bank.is_closed());
    assert_eq!(bank.available_permits(), 0);
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    let serial = ticket.serial().await;
    let table = state.inner.tunnels.write().await;
    let error = metadata_vec::with_allocation_hook(
        |_, _| panic!("prepaid guarded rollback allocated a backing"),
        || {
            authority
                .commit(DEVICE, || {
                    ticket.commit(|| {
                        ticket.gate.intent.checked_replace(
                            || Ok(Desired::Stopped),
                            None,
                            || {
                                if checks.fetch_add(1, Ordering::SeqCst) == 0 {
                                    Ok(())
                                } else {
                                    Err(pending_error.lock().unwrap().take().unwrap())
                                }
                            },
                            &mut cutover,
                        )
                    })
                })
                .unwrap_err()
        },
    );
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(ControlIdentity::ptr_eq(
        &ticket.desired().version,
        &original.version
    ));
    assert_eq!(observed.calls.load(Ordering::SeqCst), 0);
    assert_eq!(bank.available_permits(), 0);
    drop(table);
    drop(serial);
    drop(cutover);
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.unlocked.load(Ordering::SeqCst));
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("rolled-back actual marker did not refund its exact body"),
    };
    assert_eq!(permit.num_permits(), marker_bytes);
    drop((
        permit, held, original, error, observed, waker, ticket, state,
    ));
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
    // The real authority commit also returned before refund. Its private Tokio
    // revision lock is not exposed by the nonblocking try-lock probes above.
}

struct PruneWake {
    lifecycle: Arc<Lifecycle>,
    calls: AtomicUsize,
    unlocked: AtomicBool,
}

impl Wake for PruneWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.unlocked
            .fetch_and(self.lifecycle.devices.try_lock().is_ok(), Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn capacity_prune_refusal_refunds_all_actual_gate_markers_after_device_guard() {
    let lifecycle = Arc::new(Lifecycle::new());
    let bank = lifecycle.byte_bank().clone();
    let mut refund_bytes = 0;
    for index in 0..MAX_DEVICE_GATES {
        let key = format!("prune-refund-fixture-{index}");
        refund_bytes +=
            Lifecycle::additional_metadata_bytes(&key) + 2 * ControlIdentity::body_bytes();
        drop(lifecycle.ticket(&key).await.unwrap());
    }
    assert_eq!(lifecycle.coordinator_count().await, MAX_DEVICE_GATES);
    let fields = 5 * MAX_DEVICE_GATES;
    let release_bytes = ReleaseBatch::backing_bytes(fields);
    let mut release_fields = ReleaseBatch::new(&bank);
    release_fields.prepare(fields).unwrap();
    tokio::task::yield_now().await;
    let before = bank.available_permits();
    let held = bank.clone().try_acquire_many_owned(before as u32).unwrap();
    let observed = Arc::new(PruneWake {
        lifecycle: lifecycle.clone(),
        calls: AtomicUsize::new(0),
        unlocked: AtomicBool::new(true),
    });
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = std::pin::pin!(bank.clone().acquire_many_owned(refund_bytes as u32));
    assert!(!bank.is_closed());
    assert_eq!(bank.available_permits(), 0);
    assert!(waiter.as_mut().poll(&mut cx).is_pending());
    // All old nonmarker fees feed this same waiter first. Requiring the WHOLE
    // real gate refund isolates its wake until the marker headers also free.
    // The queued grant leaves the new gate's Result admission genuinely short.
    let allocation_guard = metadata_vec::on_allocation(|_, _| {
        panic!("prepaid zero-availability pruning allocated a backing")
    });
    let error = lifecycle
        .ticket_with_inputs("new-gate-after-prune", None, Some(release_fields))
        .await
        .err()
        .unwrap();
    drop(allocation_guard);
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(lifecycle.coordinator_count().await, 0);
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.unlocked.load(Ordering::SeqCst));
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("the whole actual pruned gate refund did not satisfy the waiter"),
    };
    assert_eq!(permit.num_permits(), refund_bytes);
    drop((permit, held));
    assert_eq!(
        bank.available_permits(),
        before + refund_bytes + release_bytes
    );
    drop((observed, waker, lifecycle));
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

#[tokio::test]
async fn partial_stale_reset_preparation_keeps_actual_empty_backings_and_grants_past_device_guard()
{
    type Waiter = std::pin::Pin<
        Box<
            dyn Future<
                    Output = std::result::Result<OwnedSemaphorePermit, tokio::sync::AcquireError>,
                > + Send,
        >,
    >;
    let lifecycle = Arc::new(Lifecycle::new());
    let bank = lifecycle.byte_bank().clone();
    let ticket = lifecycle
        .ticket("partial-reset-refund-fixture")
        .await
        .unwrap();
    let old_generation = ticket.generation.clone();
    let original = ticket.desired();
    let mut clear = lifecycle.prepare_cutover::<2>(&bank).unwrap();
    lifecycle.clear(&mut clear);
    drop(clear);
    let before = bank.available_permits();
    let marker_bytes = ControlIdentity::body_bytes();
    let staged_bytes = 2 * marker_bytes + BirthBatch::backing_bytes(2);
    let held = bank
        .clone()
        .try_acquire_many_owned((before - staged_bytes) as u32)
        .unwrap();
    let observed = Arc::new(PruneWake {
        lifecycle: lifecycle.clone(),
        calls: AtomicUsize::new(0),
        unlocked: AtomicBool::new(true),
    });
    let pending = Arc::new(std::sync::Mutex::new(None::<Waiter>));
    let attempt_count = Arc::new(AtomicUsize::new(0));
    let attempts = attempt_count.clone();
    let refusal = metadata_vec::refuse_allocation(move |layout| {
        layout.size() == marker_bytes && attempts.fetch_add(1, Ordering::SeqCst) == 1
    });
    let waiting = pending.clone();
    let credit = bank.clone();
    let wake = observed.clone();
    let allocations = Arc::new(AtomicUsize::new(0));
    let allocated = allocations.clone();
    let hook = metadata_vec::on_allocation(move |layout, _| {
        if layout.size() != marker_bytes {
            assert_eq!(layout.size(), BirthBatch::backing_bytes(2));
            return;
        }
        assert_eq!(allocated.fetch_add(1, Ordering::SeqCst), 0);
        assert!(!credit.is_closed());
        assert_eq!(credit.available_permits(), 0);
        let mut waiter: Waiter = Box::pin(credit.clone().acquire_many_owned(staged_bytes as u32));
        let waker = Waker::from(wake.clone());
        let mut cx = Context::from_waker(&waker);
        assert!(waiter.as_mut().poll(&mut cx).is_pending());
        *waiting.lock().unwrap() = Some(waiter);
    });
    let error = lifecycle.ticket(ticket.udid()).await.err().unwrap();
    drop((hook, refusal));
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(attempt_count.load(Ordering::SeqCst), 2);
    assert_eq!(allocations.load(Ordering::SeqCst), 1);
    assert!(observed.calls.load(Ordering::SeqCst) > 0);
    assert!(observed.unlocked.load(Ordering::SeqCst));
    // All destinations must be ready before the first original gate reset.
    assert!(ticket.gate.intent.is_epoch(&original.epoch));
    assert!(ControlIdentity::ptr_eq(
        &ticket.gate.generation.borrow(),
        &old_generation
    ));
    let waker = Waker::from(observed.clone());
    let mut cx = Context::from_waker(&waker);
    let mut waiter = pending.lock().unwrap().take().unwrap();
    let permit = match waiter.as_mut().poll(&mut cx) {
        Poll::Ready(Ok(permit)) => permit,
        _ => panic!("the actual partial reset preparation did not refund its whole staging fee"),
    };
    assert_eq!(permit.num_permits(), staged_bytes);
    drop((permit, held));
    assert_eq!(bank.available_permits(), before);
    drop((
        waiter,
        pending,
        observed,
        waker,
        original,
        old_generation,
        ticket,
        lifecycle,
    ));
    assert_eq!(
        bank.available_permits(),
        super::super::MAX_GLOBAL_KNOWN_METADATA_BYTES
    );
}

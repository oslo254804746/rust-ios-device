use super::*;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};

fn principal(state: &TunnelAgentState) -> Principal {
    state
        .inner
        .access
        .authenticate(&axum::http::HeaderMap::new())
        .unwrap()
}
fn counts(source: &FakeSource, uid: &str) -> (usize, usize, usize, usize) {
    (
        source.count(uid),
        source.usb_calls.load(Ordering::SeqCst),
        source.paired_calls.load(Ordering::SeqCst),
        source.drops.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn http_control_actual_last_byte_refusal_precedes_locks_and_keeps_original_publication() {
    for stop in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), LOCAL_BUDGET);
        drop(start(state.clone(), A).await.unwrap());
        let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
        let snapshot = ticket.desired();
        let publication = state.inner.tunnels.read().await[A].publication.clone();
        let original_counts = counts(&source, A);
        let bank = state.inner.global_metadata_bytes.clone();
        let before = bank.available_permits();
        let replies = response_json::Replies::new(&bank).unwrap();
        let success = if stop {
            replies.cancelled(A).unwrap()
        } else {
            replies.operation("epoch").unwrap()
        };
        let prefix = before - bank.available_permits();
        drop(success);
        drop(replies);
        assert_eq!(bank.available_permits(), before);
        let frame = if stop {
            http_control::stop_bytes()
        } else {
            http_control::epoch_bytes()
        };
        let leave = prefix + frame - 1;
        let held = bank
            .clone()
            .try_acquire_many_owned((before - leave) as u32)
            .unwrap();
        let coordinator = if stop {
            Some(state.inner.lifecycle.hold_coordinator().await)
        } else {
            None
        };
        let table = if stop {
            None
        } else {
            Some(state.inner.tunnels.write().await)
        };
        let layouts = Arc::new(StdMutex::new(Vec::new()));
        let observed = layouts.clone();
        let hook = metadata_vec::on_allocation(move |layout, _| {
            observed
                .lock()
                .unwrap()
                .push((layout.size(), layout.align()));
        });
        let (port, server) = spawn_test_router(agent_router(state.clone())).await;
        let response = tokio::time::timeout(LOCAL_BUDGET, async {
            if stop {
                client()
                    .delete(format!("http://127.0.0.1:{port}/tunnel/{A}"))
                    .send()
                    .await
            } else {
                client()
                    .post(format!("http://127.0.0.1:{port}/epoch"))
                    .send()
                    .await
            }
        })
        .await
        .expect("HTTP owner refusal before held control lock")
        .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
        assert!(response.text().await.unwrap().contains(if stop {
            "tunnel stop owner admission failed"
        } else {
            "tunnel epoch owner admission failed"
        }));
        drop(hook);
        {
            let actual = layouts.lock().unwrap();
            assert_eq!(
                actual.len(),
                2,
                "only original Replies diagnostic and success wire buffers"
            );
            assert_eq!(actual[0], (2048, 1));
            assert_eq!(actual.iter().map(|entry| entry.0).sum::<usize>(), prefix);
            assert!(actual.iter().all(|entry| entry.1 == 1));
        }
        assert!(ticket.is_current());
        assert!(crate::cmd::tunnel::ControlIdentity::ptr_eq(
            &snapshot.version,
            &ticket.desired().version
        ));
        assert_eq!(counts(&source, A), original_counts);
        drop(coordinator);
        drop(table);
        assert!(publication.ptr_eq(&state.inner.tunnels.read().await[A].publication));
        assert_eq!(state.inner.lifecycle.coordinator_count().await, 1);
        drop(held);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        state.notify_shutdown();
        state.clear().await;
        state.drain_native(LOCAL_BUDGET).await.unwrap();
        drop(publication);
        drop(snapshot);
        drop(ticket);
        drop(state);
        tokio::time::timeout(LOCAL_BUDGET, async {
            while bank.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn http_stop_two_actual_frames_keep_distinct_old_global_epochs_until_each_drop_at_closed_zero(
) {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let principal = principal(&state);
    let bank = state.inner.global_metadata_bytes.clone();
    let frame = http_control::stop_bytes();
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - 2 * frame) as u32)
        .unwrap();
    let control = state.inner.lifecycle.scan_control(LOCAL_BUDGET).unwrap();
    let a = crate::cmd::tunnel::ControlIdentity::downgrade(&control.epoch.generation);
    drop(control);
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    let observed_bank = bank.clone();
    let addresses = Arc::new(StdMutex::new(Vec::new()));
    let observed = addresses.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        let mut addresses = observed.lock().unwrap();
        assert_eq!(layout.size(), frame);
        assert_eq!(
            observed_bank.available_permits(),
            (1 - addresses.len()) * frame
        );
        addresses.push(pointer);
    });
    let mut first = Box::pin(http_control::prepare_stop(&state, &principal, A).unwrap());
    assert!(futures_util::poll!(&mut first).is_pending());
    state.inner.lifecycle.clear();
    let control = state.inner.lifecycle.scan_control(LOCAL_BUDGET).unwrap();
    let b = crate::cmd::tunnel::ControlIdentity::downgrade(&control.epoch.generation);
    drop(control);
    assert!(!crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &a.upgrade().unwrap(),
        &b.upgrade().unwrap()
    ));
    let mut second = Box::pin(http_control::prepare_stop(&state, &principal, A).unwrap());
    assert!(futures_util::poll!(&mut second).is_pending());
    drop(hook);
    {
        let addresses = addresses.lock().unwrap();
        assert_eq!(addresses.len(), 2);
        assert_ne!(addresses[0], addresses[1]);
    }
    bank.close();
    state.inner.lifecycle.clear();
    assert_eq!(bank.available_permits(), 0);
    assert!(a.upgrade().is_some() && b.upgrade().is_some());
    drop(first);
    assert_eq!(bank.available_permits(), frame);
    assert!(a.upgrade().is_none() && b.upgrade().is_some());
    drop(second);
    assert_eq!(bank.available_permits(), 2 * frame);
    assert!(b.upgrade().is_none());
    drop(coordinator);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert_eq!(counts(&source, A), (0, 0, 0, 0));
    drop(held);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn http_epoch_frame_exact_pending_drop_and_core_clear_closed_zero_have_independent_rules() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let epoch = state.inner.lifecycle.global_epoch().unwrap();
    let bank = state.inner.global_metadata_bytes.clone();
    let frame = http_control::epoch_bytes();
    let mut held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - frame) as u32)
        .unwrap();
    let table = state.inner.tunnels.write().await;
    let observed_bank = bank.clone();
    let allocated = Arc::new(AtomicUsize::new(0));
    let observed = allocated.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        assert_eq!(layout.size(), frame);
        assert_ne!(pointer, 0);
        assert_eq!(observed.fetch_add(1, Ordering::SeqCst), 0);
        assert_eq!(observed_bank.available_permits(), 0);
    });
    let mut pending = Box::pin(http_control::prepare_epoch(&state).unwrap());
    assert!(futures_util::poll!(&mut pending).is_pending());
    drop(hook);
    assert_eq!(allocated.load(Ordering::SeqCst), 1);
    drop(pending);
    assert!(epoch.check().is_ok());
    assert_eq!(bank.available_permits(), frame);
    held.merge(
        bank.clone()
            .try_acquire_many_owned(frame.try_into().unwrap())
            .unwrap(),
    );
    assert_eq!(bank.available_permits(), 0);
    bank.close();
    drop(table);
    let hook = metadata_vec::on_allocation(|_, _| panic!("core clear requested a new backing"));
    state.clear().await;
    assert!(epoch.check().is_err());
    state.notify_shutdown();
    state.clear().await;
    drop(hook);
    assert_eq!(bank.available_permits(), 0);
    assert_eq!(counts(&source, A), (0, 0, 0, 0));
    drop(epoch);
    drop(held);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn http_stop_frame_does_not_add_admission_to_core_stop_of_existing_closed_zero_gate() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    drop(start(state.clone(), A).await.unwrap());
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let snapshot = ticket.desired();
    let original_calls = source.count(A);
    let bank = state.inner.global_metadata_bytes.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    bank.close();
    let hook = metadata_vec::on_allocation(|_, _| panic!("core known stop requested new backing"));
    assert!(state.stop_tunnel(A).await.unwrap());
    assert!(!ticket.is_current());
    assert!(!crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &snapshot.version,
        &ticket.desired().version
    ));
    assert!(matches!(ticket.desired().desired, intent::Desired::Stopped));
    assert!(!state.stop_tunnel(A).await.unwrap());
    drop(hook);
    assert_eq!(source.count(A), original_calls);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    drop(held);
    state.notify_shutdown();
    state.clear().await;
    drop(snapshot);
    drop(ticket);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn http_stop_paid_frame_preserves_original_authorization_revocation_while_coordinator_waits()
{
    let source = FakeSource::new();
    let fixture = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::ManageServer, Permission::ManageTunnels],
    )]);
    let access = AccessControl::from_file(fixture.path.clone())
        .await
        .unwrap();
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        access.clone(),
        Upstreams::default(),
        source.clone(),
        LOCAL_BUDGET,
    );
    drop(start(state.clone(), DEVICE).await.unwrap());
    let publication = state.inner.tunnels.read().await[DEVICE].publication.clone();
    let ticket = state.inner.lifecycle.ticket(DEVICE).await.unwrap();
    let snapshot = ticket.desired();
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let principal = access.authenticate(&headers).unwrap();
    principal
        .require(Permission::ManageTunnels, Some(DEVICE), None)
        .unwrap_or_else(|_| panic!("fixture initial authority"));
    let bank = state.inner.global_metadata_bytes.clone();
    let frame = http_control::stop_bytes();
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - frame) as u32)
        .unwrap();
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    let mut work = Box::pin(http_control::prepare_stop(&state, &principal, DEVICE).unwrap());
    assert!(futures_util::poll!(&mut work).is_pending());
    assert_eq!(bank.available_permits(), 0);
    bank.close();
    fixture.replace(Vec::new());
    access.reload(&principal).await.unwrap();
    let response = tokio::time::timeout(LOCAL_BUDGET, work)
        .await
        .unwrap()
        .err()
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(bank.available_permits(), frame);
    assert!(ticket.is_current());
    assert!(crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &snapshot.version,
        &ticket.desired().version
    ));
    assert!(publication.ptr_eq(&state.inner.tunnels.read().await[DEVICE].publication));
    assert_eq!(source.count(DEVICE), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    drop(coordinator);
    drop(response);
    drop(held);
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    drop(snapshot);
    drop(ticket);
    drop(publication);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

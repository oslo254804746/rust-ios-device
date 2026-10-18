//! Borrowed manual selectors are admitted before any manager-owned key or route.
use super::*;
use crate::access::test_support::{grant, PolicyFixture, DEVICE, TOKEN};
use futures_util::FutureExt;

const FIELD_BYTES: usize = 4096;

#[tokio::test]
async fn source_future_constructor_last_byte_refusal_never_runs_source_or_consumes_retry() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    drop(state.inner.lifecycle.ticket(A).await.unwrap());
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let held = Arc::new(StdMutex::new(None));
    let captured = held.clone();
    let charged = bank.clone();
    let hook = future_storage::on_dynamic_layout(move |bytes| {
        assert!(bytes > 0);
        *captured.lock().unwrap() = Some(
            charged
                .clone()
                .try_acquire_many_owned((charged.available_permits() - bytes + 1) as u32)
                .unwrap(),
        );
    });
    let error = state.ensure_tunnel(A, None, None).await.err().unwrap();
    drop(hook);
    assert!(held.lock().unwrap().is_some());
    assert!(error.is::<ownership::Admission>());
    resource(&error);
    assert_eq!(source_counts(&source), (0, 0, 0));
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert!(state.inner.failed_attempts.read().await.is_empty());
    held.lock().unwrap().take();
    let backing = state.inner.tunnels.read().await.backing_bytes();
    assert_eq!(bank.available_permits(), before - backing);
    drop(error);
    recover(&state, &source).await;
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn native_source_admission_full_registry_default_start_never_browses_paired_fallback() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    drop(start(state.clone(), B).await.unwrap());
    let original = state.inner.tunnels.read().await[B].publication.clone();
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let stamp = ticket.ownership_stamp();
    let reserved = (0..retirement::MAX_NATIVE_OWNERS)
        .map(|_| state.inner.native.reserve(stamp.clone()).unwrap())
        .collect::<Vec<_>>();
    let before = source_counts(&source);
    let error = state.ensure_tunnel(A, None, None).await.err().unwrap();
    assert!(error.is::<ownership::Admission>());
    resource(&error);
    assert!(error
        .chain()
        .any(|cause| cause.to_string() == "native tunnel owner capacity exhausted"));
    let after = source_counts(&source);
    assert_eq!(source.count(A), 0);
    assert_eq!(
        state.inner.native.snapshot().reserved,
        retirement::MAX_NATIVE_OWNERS
    );
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert!(matches!(
        ticket.desired().desired,
        intent::Desired::Automatic
    ));
    assert!(original.ptr_eq(&state.inner.tunnels.read().await[B].publication));
    assert!(state.get_record(B).await.unwrap().is_some());
    drop(reserved);
    drop(original);
    drop(error);
    recover(&state, &source).await;
    assert_eq!(
        after, before,
        "pre-Source owner rejection must not trigger paired discovery"
    );
}

#[tokio::test]
async fn native_source_admission_publication_last_byte_refusal_never_browses_default_fallback() {
    for global in [false, true] {
        let source = FakeSource::new();
        let state = state(source.clone(), LOCAL_BUDGET);
        drop(state.inner.lifecycle.ticket(A).await.unwrap());
        let metadata = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
        let published = Arc::new(Semaphore::new(MAX_PUBLISHED_METADATA_BYTES));
        let mut probe = publication_table::Table::<ManagedTunnel>::new(&metadata, &published);
        let prepared = probe.prepare(A).unwrap();
        let destination = MAX_PUBLISHED_METADATA_BYTES - published.available_permits();
        drop(prepared);
        drop(probe);
        let bank = if global {
            state.inner.global_metadata_bytes.clone()
        } else {
            state.inner.metadata_bytes.clone()
        };
        let before = bank.available_permits();
        let held = Arc::new(StdMutex::new(None));
        let captured = held.clone();
        let charged = bank.clone();
        *state.inner.publication_before_prepare.lock().unwrap() = Some(Box::new(move || {
            *captured.lock().unwrap() = Some(
                charged
                    .clone()
                    .try_acquire_many_owned((charged.available_permits() - destination + 1) as u32)
                    .unwrap(),
            );
        }));
        let counts = source_counts(&source);
        let error = state.ensure_tunnel(A, None, None).await.err().unwrap();
        assert!(error.is::<ownership::Admission>());
        resource(&error);
        assert_eq!(source_counts(&source), counts);
        assert_eq!(state.inner.native.snapshot().occupied(), 0);
        assert!(state.inner.tunnels.read().await.is_empty());
        held.lock().unwrap().take();
        let backing = state.inner.tunnels.read().await.backing_bytes();
        assert_eq!(bank.available_permits(), before - backing);
        drop(error);
        recover(&state, &source).await;
        assert_eq!(bank.available_permits(), before);
    }
}

#[tokio::test]
async fn native_source_admission_original_source_io_still_uses_existing_paired_fallback() {
    let source = FakeSource::new();
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
        ],
    );
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: A.into(),
        host: "192.0.2.143".into(),
    });
    let state = state(source.clone(), LOCAL_BUDGET);
    drop(state.ensure_tunnel(A, None, None).await.unwrap());
    assert_eq!(source.count(A), 2);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    {
        let calls = source.calls.lock().unwrap();
        assert_eq!(calls[0].1, Route::Usbmux);
        assert_eq!(calls[1].1, Route::Mobdev2("192.0.2.143".into()));
    }
    recover(&state, &source).await;
}

fn source_counts(source: &FakeSource) -> (usize, usize, usize) {
    (
        source.calls.lock().unwrap().len(),
        source.usb_calls.load(Ordering::SeqCst),
        source.paired_calls.load(Ordering::SeqCst),
    )
}

fn resource(error: &anyhow::Error) {
    let rendered = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "tunnel-manual-selector",
        error,
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(value["error"]["class"], "resource_limit");
    assert!(!rendered.contains("synthetic-private-selector"));
}

async fn recover(state: &TunnelAgentState, source: &FakeSource) {
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );
    assert!(state.inner.tunnels.read().await.is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
}

#[tokio::test]
async fn admission_manual_oversized_selectors_preserve_old_rows_before_any_source_or_gate() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux))
        .await
        .unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Mobdev2("192.0.2.8".into()),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let previous = state.inner.failed_attempts.read().await[B].clone();
    let gates = state.inner.lifecycle.coordinator_count().await;
    let counts = source_counts(&source);
    let slots = state.inner.failure_slots.available_permits();
    let oversized = format!("synthetic-private-selector{}", "x".repeat(FIELD_BYTES));
    let utf8_oversized = format!("{}x", "é".repeat(FIELD_BYTES / 2));
    let mut errors = Vec::new();
    for route in [
        None,
        Some(StartTunnelConnectionType::Usbmux),
        Some(StartTunnelConnectionType::Usb),
        Some(StartTunnelConnectionType::Wifi),
    ] {
        for (udid, ip) in [
            (oversized.as_str(), None),
            (utf8_oversized.as_str(), None),
            (A, Some(oversized.as_str())),
            (A, Some(utf8_oversized.as_str())),
        ] {
            errors.push(state.ensure_tunnel(udid, ip, route).await.err());
        }
    }
    let retained = state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .is_some_and(|row| row.publication.ptr_eq(&publication));
    let unchanged = state
        .inner
        .failed_attempts
        .read()
        .await
        .get(B)
        .is_some_and(|row| {
            row.identity == previous.identity
                && row.last_attempt == previous.last_attempt
                && row.fail_count == previous.fail_count
                && row
                    .slot
                    .as_ref()
                    .zip(previous.slot.as_ref())
                    .is_some_and(|(actual, old)| Arc::ptr_eq(actual, old))
        });
    let after_counts = source_counts(&source);
    let after_gates = state.inner.lifecycle.coordinator_count().await;
    let after_slots = state.inner.failure_slots.available_permits();
    let not_ready = !state.is_ready().await;
    drop(previous);
    drop(ticket);
    recover(&state, &source).await;

    assert!(retained && unchanged && not_ready);
    assert_eq!(
        (after_counts, after_gates, after_slots),
        (counts, gates, slots)
    );
    for error in errors {
        resource(&error.expect("oversized manual selector was accepted"));
    }
}

#[tokio::test]
async fn admission_manual_ticket_rejects_utf8_tail_without_waiting_for_coordinator() {
    let lifecycle = Lifecycle::new();
    let oversized = format!("{}x", "é".repeat(FIELD_BYTES / 2));
    let held = lifecycle.hold_coordinator().await;
    let result = lifecycle.ticket(&oversized).now_or_never();
    drop(held);
    let gates = lifecycle.coordinator_count().await;

    resource(
        &result
            .expect("oversized key waited for the held coordinator")
            .err()
            .unwrap(),
    );
    assert_eq!(gates, 0);
    assert_eq!(
        lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
}

#[tokio::test]
async fn admission_manual_exact_utf8_fields_keep_all_explicit_route_handoffs() {
    let udid = "é".repeat(FIELD_BYTES / 2);
    let host = format!("{}x", "界".repeat((FIELD_BYTES - 1) / 3));
    for route in [
        None,
        Some(StartTunnelConnectionType::Usbmux),
        Some(StartTunnelConnectionType::Usb),
        Some(StartTunnelConnectionType::Wifi),
    ] {
        let source = FakeSource::new();
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let result = state.ensure_tunnel(&udid, Some(&host), route).await;
        let actual = source.calls.lock().unwrap().clone();
        let gates = state.inner.lifecycle.coordinator_count().await;
        let paired = source.paired_calls.load(Ordering::SeqCst);
        recover(&state, &source).await;

        assert_eq!(result.unwrap().udid, udid);
        assert_eq!(gates, 1);
        assert_eq!(paired, 0);
        let expected = match route {
            Some(StartTunnelConnectionType::Usb) => Route::Usb(Some(host.clone())),
            Some(StartTunnelConnectionType::Wifi) => Route::Wifi(Some(host.clone())),
            Some(StartTunnelConnectionType::Usbmux) | None => Route::Usbmux,
        };
        assert_eq!(actual, vec![(udid.clone(), expected)]);
    }
}

#[tokio::test]
async fn admission_manual_exact_fields_keep_default_paired_fallback_matching() {
    let udid = "u".repeat(FIELD_BYTES);
    let host = "h".repeat(FIELD_BYTES);
    let source = FakeSource::new();
    source.script(&udid, [Behavior::Failure(std::io::ErrorKind::BrokenPipe)]);
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: udid.clone(),
        host: host.clone(),
    });
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let result = state.ensure_tunnel(&udid, Some(&host), None).await;
    let actual = source.calls.lock().unwrap().clone();
    let paired = source.paired_calls.load(Ordering::SeqCst);
    let gates = state.inner.lifecycle.coordinator_count().await;
    recover(&state, &source).await;

    assert_eq!(result.unwrap().udid, udid);
    assert_eq!(
        actual,
        vec![(udid.clone(), Route::Usbmux), (udid, Route::Mobdev2(host))]
    );
    assert_eq!((paired, gates), (1, 1));
}

#[tokio::test]
async fn admission_manual_small_empty_and_nul_selectors_keep_existing_acceptance() {
    for (udid, host) in [("", ""), ("synthetic\0selector", "synthetic\0host")] {
        let source = FakeSource::new();
        let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
        let result = state
            .ensure_tunnel(udid, Some(host), Some(StartTunnelConnectionType::Wifi))
            .await;
        let actual = source.calls.lock().unwrap().clone();
        recover(&state, &source).await;

        assert_eq!(result.unwrap().udid, udid);
        assert_eq!(actual, vec![(udid.into(), Route::Wifi(Some(host.into())))]);
    }
}

#[tokio::test]
async fn admission_manual_exact_gate_count_keeps_16mib_keys_and_original_pruning() {
    let lifecycle = Lifecycle::new();
    let suffix = "x".repeat(FIELD_BYTES - 4);
    let mut tickets = Vec::new();
    for index in 0..4096 {
        let udid = format!("{index:04}{suffix}");
        tickets.push(lifecycle.ticket(&udid).await.unwrap());
    }
    let full = lifecycle.coordinator_count().await;
    let error = lifecycle.ticket("next-key").await.err().unwrap();
    drop(tickets.pop());
    let replacement = lifecycle.ticket("replacement-key").await;
    let after = lifecycle.coordinator_count().await;
    let first_current = tickets[0].is_current();
    drop(tickets);

    resource(&error);
    assert_eq!(full, 4096);
    assert!(replacement.is_ok() && first_current);
    assert_eq!(after, 4096);
}

#[tokio::test]
async fn admission_manual_authorized_http_start_rejects_before_source_and_redacts_selector() {
    let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::ManageTunnels])]);
    let source = FakeSource::new();
    let state = TunnelAgentState::with_source(
        TunMode::Userspace,
        AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap(),
        Upstreams::default(),
        source.clone(),
        TUNNEL_CONNECT_TIMEOUT,
    );
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let oversized = format!("synthetic-private-selector{}", "x".repeat(FIELD_BYTES));
    let response = client()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .bearer_auth(TOKEN)
        .json(&json!({"udid": DEVICE, "ip": oversized, "connection_type": "wifi"}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let class = response.headers()["x-ios-error-class"]
        .to_str()
        .unwrap()
        .to_owned();
    let body = response.text().await.unwrap();
    let counts = source_counts(&source);
    let gates = state.inner.lifecycle.coordinator_count().await;
    server.abort();
    let stopped = server.await.is_err_and(|error| error.is_cancelled());
    recover(&state, &source).await;

    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(class, "resource_limit");
    assert!(!body.contains("synthetic-private-selector") && !body.contains(TOKEN));
    assert_eq!(counts, (0, 0, 0));
    assert_eq!(gates, 0);
    assert!(stopped);
}

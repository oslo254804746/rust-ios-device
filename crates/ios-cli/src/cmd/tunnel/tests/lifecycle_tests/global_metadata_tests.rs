use super::*;

fn paired_source_frame_bytes(source: &dyn Source) -> usize {
    let bank = Arc::new(Semaphore::new(MAX_GLOBAL_KNOWN_METADATA_BYTES));
    let frame = source.paired_devices_owned(&bank).unwrap();
    let bytes = MAX_GLOBAL_KNOWN_METADATA_BYTES - bank.available_permits();
    drop(frame); // Measure the actual constructor, never poll discovery.
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    bytes
}

#[tokio::test]
async fn manual_success_preflight_one_byte_refusal_is_before_all_source_and_keeps_namespace() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let bytes = scan::capsule_reservation_bytes() + manual_success::Preflight::bytes() - 1;
    let structural = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes();
    state.set_global_metadata_limit(structural + bytes);
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(A), 0);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert!(!state.is_ready().await);
    drop(error);
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert_eq!(bank.available_permits(), structural + bytes);
}

#[tokio::test]
async fn failure_window_roots_one_byte_refusal_precedes_source_with_prepared_single_key() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let observed_bank = Arc::clone(&bank);
    let target =
        bookkeeping::WriteBatch::backing_bytes(1) - failure_table::Destination::minimum_bytes();
    // Seen and all candidate nodes remain paid through reconciliation/window.
    // The callback leaves exactly one byte less than the new window backing.
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        let leave = target - 1;
        *retained.lock().unwrap() = Some(
            observed_bank
                .clone()
                .try_acquire_many_owned((observed_bank.available_permits() - leave) as u32)
                .unwrap(),
        );
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.count(A), 0);
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert!(!state.is_ready().await);
    drop(error);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn paired_failure_node_last_byte_refusal_keeps_usb_and_original_mobdev2_without_ticket_prefix(
) {
    let source = FakeSource::new();
    let old_host = "original-mobdev2-host";
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: old_host.into(),
    });
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    let publication = state.inner.tunnels.read().await[B].publication.clone();
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Mobdev2(old_host.into()),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let original = state.inner.failed_attempts.read().await[B].clone();
    source.usb.lock().unwrap().push(A.into());
    let targets = vec![
        PairedMobdev2Device {
            udid: B.into(),
            host: old_host.into(),
        },
        PairedMobdev2Device {
            udid: "fresh-paired-one".into(),
            host: "h".repeat(4096),
        },
        PairedMobdev2Device {
            udid: "fresh-paired-two".into(),
            host: "i".repeat(4096),
        },
        PairedMobdev2Device {
            udid: "fresh-paired-three".into(),
            host: "j".repeat(4096),
        },
    ];
    let raw = targets.len() * std::mem::size_of::<PairedMobdev2Device>()
        + targets
            .iter()
            .map(|target| target.udid.len() + target.host.len())
            .sum::<usize>();
    let plans = 2 * targets.len() * std::mem::size_of::<usize>();
    let inputs = targets.len()
        * std::mem::size_of::<(
            candidate_input::Owner,
            Option<Ticket>,
            failure_table::OwnedNode,
        )>();
    let owners = targets
        .iter()
        .map(|target| {
            let route = Route::Mobdev2(target.host.clone());
            candidate_input::Input::bytes(&target.udid, &route)
                + failure_table::Node::input_bytes(&route)
        })
        .sum::<usize>();
    let leave = raw + plans + inputs + owners - 1;
    *source.paired.lock().unwrap() = targets;
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let observed_bank = Arc::clone(&bank);
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        *retained.lock().unwrap() = Some(
            observed_bank
                .clone()
                .try_acquire_many_owned((observed_bank.available_permits() - leave) as u32)
                .unwrap(),
        );
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    for uid in ["fresh-paired-one", "fresh-paired-two", "fresh-paired-three"] {
        assert_eq!(source.count(uid), 0);
    }
    assert_eq!(
        state.inner.lifecycle.coordinator_count().await,
        2,
        "rejected paired nodes created an automatic ticket prefix"
    );
    assert!(state.get_record(A).await.unwrap().is_some());
    assert!(publication.ptr_eq(&state.inner.tunnels.read().await[B].publication));
    {
        let failed = state.inner.failed_attempts.read().await;
        assert!(Arc::ptr_eq(&original.version, &failed[B].version));
        assert_eq!(original.identity, failed[B].identity);
        assert_eq!(original.last_attempt, failed[B].last_attempt);
        assert_eq!(original.fail_count, failed[B].fail_count);
        assert!(Arc::ptr_eq(
            original.slot.as_ref().unwrap(),
            failed[B].slot.as_ref().unwrap()
        ));
    }
    assert!(
        state.is_ready().await,
        "incomplete paired view changed established readiness"
    );
    drop(error);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    drop(original);
    drop(ticket);
    drop(publication);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn seen_coordinator_byte_refusal_does_not_retry_same_usb_uid_after_paired_pool_recovers() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    source
        .paired
        .lock()
        .unwrap()
        .extend([A, B].map(|udid| PairedMobdev2Device {
            udid: udid.into(),
            host: "192.0.2.88".into(),
        }));
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let before = bank.available_permits();
    let remaining = scan::capsule_reservation_bytes()
        + manual_success::Preflight::bytes()
        + std::mem::size_of::<source::UsbDevice>()
        + A.len()
        + std::mem::size_of::<source::UsbDevice>()
        + std::mem::size_of::<usize>()
        + candidate_input::Input::body_bytes()
        + A.len()
        + scan_seen::Seen::first_bytes()
        + failure_table::Node::input_bytes(&Route::Usbmux)
        + Lifecycle::additional_metadata_bytes(A)
        - 1;
    let held = Arc::new(StdMutex::new(Some(
        bank.clone()
            .try_acquire_many_owned((before - remaining) as u32)
            .unwrap(),
    )));
    let retained = Arc::clone(&held);
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        drop(retained.lock().unwrap().take());
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        source.paired_calls.load(Ordering::SeqCst),
        1,
        "{error:?}; remaining={remaining}; available={}",
        bank.available_permits()
    );
    assert_eq!(source.count(A), 0);
    assert_eq!(source.count(B), 1);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 1);
    assert!(state.get_record(A).await.unwrap().is_none());
    assert!(state.get_record(B).await.unwrap().is_some());
    drop(error);
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
}

#[tokio::test]
async fn observed_last_byte_refusal_preserves_old_paired_publication_and_publishes_usb_window() {
    let source = FakeSource::new();
    let old_host = "192.0.2.87";
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: old_host.into(),
    });
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.ensure_tunnel(B, Some(old_host), None).await.unwrap();
    let old = state.inner.tunnels.read().await[B].publication.clone();
    source.usb.lock().unwrap().push(A.into());
    // A complete missing-host view would prune the original B publication.
    *source.paired.lock().unwrap() = vec![PairedMobdev2Device {
        udid: "new-paired".into(),
        host: "h".repeat(2048),
    }];
    let targets = source.paired.lock().unwrap().clone();
    let raw = targets.capacity() * std::mem::size_of::<PairedMobdev2Device>()
        + targets
            .iter()
            .map(|target| target.udid.capacity() + target.host.capacity())
            .sum::<usize>();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        // Raw, candidate plan and all but the last observed-index byte fit.
        let leave = raw + 2 * std::mem::size_of::<usize>() - 1;
        *retained.lock().unwrap() = Some(
            bank.clone()
                .try_acquire_many_owned((bank.available_permits() - leave) as u32)
                .unwrap(),
        );
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(old.ptr_eq(&state.inner.tunnels.read().await[B].publication));
    assert!(state.get_record(A).await.unwrap().is_some());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.count(B), 1);
    assert_eq!(source.count("new-paired"), 0);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 2);
    assert!(!state.is_ready().await);
    drop(error);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
}

async fn manual_paired_fixture() -> (Arc<FakeSource>, TunnelAgentState, Ticket) {
    let source = FakeSource::new();
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
            Behavior::Success,
        ],
    );
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: A.into(),
        host: "192.0.2.50".into(),
    });
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .json(&json!({"udid":A,"ip":"192.0.2.50"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    server.abort();
    let _ = server.await;
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    assert!(
        matches!(&ticket.desired().desired, intent::Desired::Manual { route, .. } if route.route == intent::RouteIntent::Mobdev2(Some("192.0.2.50".into())))
    );
    source.usb_calls.store(0, Ordering::SeqCst);
    source.paired_calls.store(0, Ordering::SeqCst);
    (source, state, ticket)
}

#[tokio::test]
async fn manual_paired_pending_one_byte_refusal_preserves_intent_and_publication_before_browse() {
    let (source, state, ticket) = manual_paired_fixture().await;
    let original = ticket.desired();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let before = bank.available_permits();
    let desired = state.inner.lifecycle.desired().await.unwrap();
    let listing_bytes = before - bank.available_permits();
    let mut manual = scan_desired::Manual::new(&bank);
    manual.insert(&desired.as_slice()[0]).unwrap();
    let manual_bytes = manual.storage_bytes();
    drop(manual);
    drop(desired);
    assert_eq!(bank.available_permits(), before);
    let pending = 2 * std::mem::size_of::<(&str, &Ticket, Option<&str>)>();
    let admitted = scan::capsule_reservation_bytes()
        + manual_success::Preflight::bytes()
        + listing_bytes
        + manual_bytes
        + scan_publications::InitialPublications::first_bytes(A)
        + pending
        - 1;
    // Earlier retain scratch now has its own temporary peak. Deplete at the
    // actual pending phase, after that scratch has been released, rather than
    // accidentally testing an upstream refusal with the old fixed allowance.
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let observed_bank = Arc::clone(&bank);
    *state.inner.usb_before_snapshot.lock().unwrap() = Some(Box::new(move || {
        *retained.lock().unwrap() = Some(
            observed_bank
                .clone()
                .try_acquire_many_owned((observed_bank.available_permits() - pending + 1) as u32)
                .unwrap(),
        );
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(A), 2);
    assert!(Arc::ptr_eq(&original.version, &ticket.desired().version));
    assert!(publication.ptr_eq(&state.inner.tunnels.read().await[A].publication));
    drop(error);
    assert_eq!(bank.available_permits(), admitted);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
}

#[tokio::test]
async fn manual_paired_plan_one_byte_refusal_is_incomplete_and_preserves_both_original_publications(
) {
    let (source, state, ticket) = manual_paired_fixture().await;
    source.usb.lock().unwrap().push(B.into());
    start(state.clone(), B).await.unwrap();
    source.usb_calls.store(0, Ordering::SeqCst);
    let original = ticket.desired();
    let (a, b) = {
        let tunnels = state.inner.tunnels.read().await;
        (
            tunnels[A].publication.clone(),
            tunnels[B].publication.clone(),
        )
    };
    let targets = source.paired.lock().unwrap().clone();
    let raw = targets.capacity() * std::mem::size_of::<PairedMobdev2Device>()
        + targets
            .iter()
            .map(|target| target.udid.capacity() + target.host.capacity())
            .sum::<usize>();
    let plan = 2 * std::mem::size_of::<(&str, &Ticket, usize)>();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        let leave = raw + plan - 1;
        *retained.lock().unwrap() = Some(
            bank.clone()
                .try_acquire_many_owned((bank.available_permits() - leave) as u32)
                .unwrap(),
        );
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.count(A), 2);
    assert_eq!(source.count(B), 1);
    assert!(Arc::ptr_eq(&original.version, &ticket.desired().version));
    {
        let tunnels = state.inner.tunnels.read().await;
        assert!(a.ptr_eq(&tunnels[A].publication));
        assert!(b.ptr_eq(&tunnels[B].publication));
    }
    drop(error);
    drop(held.lock().unwrap().take());
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
}

#[tokio::test]
async fn scan_desired_suppressed_index_quota_refusal_precedes_source_and_keeps_stop_epoch() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    state.inner.lifecycle.stop(A).await;
    let original = ticket.desired();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let before = bank.available_permits();
    let desired = state.inner.lifecycle.desired().await.unwrap();
    let listing_bytes = before - bank.available_permits();
    drop(desired);
    let admitted =
        scan::capsule_reservation_bytes() + listing_bytes + 2 * std::mem::size_of::<&str>() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((before - admitted) as u32)
        .unwrap();
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(&original.version, &ticket.desired().version));
    assert!(matches!(ticket.desired().desired, intent::Desired::Stopped));
    drop(error);
    drop(held);
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn scan_desired_manual_index_retains_original_snapshot_refs_and_exact_backing_owner() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .json(&json!({"udid":A,"connection_type":"wifi","ip":"192.0.2.45"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let desired = state.inner.lifecycle.desired().await.unwrap();
    let row = desired.as_slice().iter().find(|row| row.udid == A).unwrap();
    let intent::Desired::Manual { route, .. } = &row.snapshot.desired else {
        panic!("production manual desired row")
    };
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let before = bank.available_permits();
    let mut manual = scan_desired::Manual::new(&bank);
    manual.insert(row).unwrap();
    manual.sort();
    let charged = manual.storage_bytes();
    assert_eq!(bank.available_permits(), before - charged);
    let (same_ticket, same_route) = manual.get(A).unwrap();
    assert!(std::ptr::eq(same_ticket, &row.ticket));
    assert!(same_route.ptr_eq(route));
    assert!(std::ptr::eq(
        manual.iter().next().unwrap().0.as_ptr(),
        row.udid.as_ptr()
    ));
    assert!(!manual.contains_key(B));
    drop(manual);
    assert_eq!(bank.available_permits(), before);
    let held = bank
        .clone()
        .try_acquire_many_owned((before - charged + 1) as u32)
        .unwrap();
    let mut refused = scan_desired::Manual::new(&bank);
    assert!(refused.insert(row).is_err());
    assert!(refused.is_empty());
    assert_eq!(
        bank.available_permits() + refused.storage_bytes(),
        charged - 1
    );
    drop(refused);
    assert_eq!(bank.available_permits(), charged - 1);
    drop(held);
    drop(desired);
    server.abort();
    let _ = server.await;
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
}

#[tokio::test]
async fn scan_capsule_preflight_refusal_has_zero_source_calls_and_no_history_or_ready_change() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let bytes = scan::capsule_reservation_bytes() - 1;
    let native_bytes = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert_eq!(state.inner.scan_outcomes.payload_history().0, 0);
    assert!(state.inner.scan_outcomes.history().is_none());
    assert!(!state.is_ready().await);
    assert_eq!(bank.available_permits(), bytes);
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

#[tokio::test]
async fn desired_listing_bytes_refusal_is_before_discovery_and_preserves_stop() {
    let source = FakeSource::new();
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let reserve = scan::capsule_reservation_bytes();
    let bytes = Lifecycle::initial_metadata_bytes(A) + reserve;
    let native_bytes = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    state.inner.lifecycle.stop(A).await;
    let before = ticket.desired();
    assert_eq!(bank.available_permits(), reserve);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert!(Arc::ptr_eq(&before.version, &ticket.desired().version));
    assert!(matches!(ticket.desired().desired, intent::Desired::Stopped));
    assert!(!state.is_ready().await);
    assert_eq!(bank.available_permits(), reserve);
    drop(ticket);
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

#[tokio::test]
async fn write_batch_bytes_refusal_happens_before_original_source_poll() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let raw = std::mem::size_of::<source::UsbDevice>() + A.len();
    let selection = std::mem::size_of::<source::UsbDevice>();
    let rows = candidate_index::Candidate::first_usb_bytes(A);
    let bytes = scan::capsule_reservation_bytes()
        + manual_success::Preflight::bytes()
        + raw
        + selection
        + rows
        + Lifecycle::initial_metadata_bytes(A)
        + TunnelAgentState::scan_window_bytes(1)
        + bookkeeping::WriteBatch::backing_bytes(1)
        - 1;
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let native_bytes = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes();
    state.set_global_metadata_limit(native_bytes + (bytes));
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert!(source.calls.lock().unwrap().is_empty());
    assert!(state.inner.failed_attempts.read().await.is_empty());
    assert_eq!(state.inner.retry.receipt_count(), 0);
    assert_eq!(state.inner.native.snapshot().occupied(), 0);
    assert!(!state.is_ready().await);
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    assert_eq!(bank.available_permits(), bytes + native_bytes);
}

#[tokio::test]
async fn candidate_index_bytes_refusal_preserves_original_backoff_before_source_contact() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let ticket = state.inner.lifecycle.ticket(B).await.unwrap();
    let slot = state.reserve_failure_slot(B).await.unwrap();
    state
        .record_failed_attempt(
            B,
            AttemptIdentity {
                route: Route::Mobdev2("original-host".into()),
                native_id: None,
            },
            slot,
            &ticket,
        )
        .await;
    let original = state.inner.failed_attempts.read().await[B].clone();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let raw = std::mem::size_of::<source::UsbDevice>() + A.len();
    let selection = std::mem::size_of::<source::UsbDevice>();
    let remaining = scan::capsule_reservation_bytes()
        + manual_success::Preflight::bytes()
        + raw
        + selection
        + Lifecycle::additional_metadata_bytes(A)
        + candidate_index::Candidate::first_usb_bytes(A)
        + TunnelAgentState::scan_window_bytes(1)
        + std::mem::size_of::<usize>()
        + failure_table::Destination::minimum_bytes()
        - 1;
    let temporary = paired_source_frame_bytes(&*source);
    let held = Arc::new(StdMutex::new(None));
    let retained = held.clone();
    let charged = bank.clone();
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        // The real paired frame has been destroyed. Occupy only its measured
        // temporary extra, restoring the original final-byte index refusal.
        *retained.lock().unwrap() = Some(
            charged
                .clone()
                .try_acquire_many_owned(temporary as u32)
                .unwrap(),
        );
    }));
    let occupied = bank
        .clone()
        .try_acquire_many_owned(
            u32::try_from(bank.available_permits() - remaining - temporary).unwrap(),
        )
        .unwrap();
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    let failed = state.inner.failed_attempts.read().await;
    let current = &failed[B];
    assert!(Arc::ptr_eq(&current.version, &original.version));
    assert_eq!(current.last_attempt, original.last_attempt);
    assert_eq!(current.fail_count, original.fail_count);
    assert_eq!(current.identity, original.identity);
    assert!(Arc::ptr_eq(
        current.slot.as_ref().unwrap(),
        original.slot.as_ref().unwrap()
    ));
    drop(failed);
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert!(!state.is_ready().await);
    assert_eq!(
        bank.available_permits(),
        remaining - Lifecycle::additional_metadata_bytes(A)
    );
    state.notify_shutdown();
    state.clear().await;
    drop(ticket);
    drop(original);
    drop(state);
    drop(occupied);
    held.lock().unwrap().take();
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn paired_plan_bytes_refusal_is_incomplete_and_still_publishes_admitted_usb() {
    let source = FakeSource::new();
    source.usb.lock().unwrap().push(A.into());
    let host = "h".repeat(2048);
    source.paired.lock().unwrap().push(PairedMobdev2Device {
        udid: B.into(),
        host: host.clone(),
    });
    let raw_paired = std::mem::size_of::<PairedMobdev2Device>() + B.len() + host.len();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let before = bank.available_permits();
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let observed_bank = Arc::clone(&bank);
    *state.inner.paired_after_first_check.lock().unwrap() = Some(Box::new(move || {
        // Reject the actual paired index's final byte, after the USB window is
        // prepared. Its incomplete view must survive later publication storage.
        let leave = raw_paired + std::mem::size_of::<usize>() - 1;
        *retained.lock().unwrap() = Some(
            observed_bank
                .clone()
                .try_acquire_many_owned((observed_bank.available_permits() - leave) as u32)
                .unwrap(),
        );
    }));
    let retained = Arc::clone(&held);
    *state.inner.publication_before_prepare.lock().unwrap() = Some(Box::new(move || {
        drop(retained.lock().unwrap().take());
    }));
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(
        source.count(B),
        0,
        "rejected paired plan must not create a connection"
    );
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 1);
    assert_eq!(state.list_records().await.unwrap().len(), 1);
    assert!(!state.is_ready().await);
    state.clear().await;
    assert_eq!(
        bank.available_permits(),
        before - Lifecycle::initial_metadata_bytes(A)
    );
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    state.notify_shutdown();
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn global_metadata_scan_retains_usb_charge_across_paired_wait_and_drop_releases_it() {
    let source = FakeSource::new();
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: 7,
        usb: true,
    }]);
    *source.paired_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let mut state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let charge = std::mem::size_of::<source::UsbDevice>() + A.len();
    let selection = std::mem::size_of::<source::UsbDevice>();
    let candidate = candidate_index::Candidate::first_usb_bytes(A);
    let paired_frame = paired_source_frame_bytes(&*source);
    let native_bytes = state.inner.native.structural_bytes()
        + reconnect::Ledger::structural_bytes()
        + scan::Outcomes::body_bytes()
        + TunnelAgentOwner::body_bytes();
    state.set_global_metadata_limit(
        native_bytes
            + (scan::capsule_reservation_bytes()
                + manual_success::Preflight::bytes()
                + charge
                + selection
                + candidate
                + Lifecycle::initial_metadata_bytes(A)
                + TunnelAgentState::scan_window_bytes(1)
                + failure_table::Destination::minimum_bytes()
                + paired_frame),
    );
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.global_metadata_bytes.available_permits(), 0);
    assert!(state.list_records().await.unwrap().as_slice().is_empty());
    drop(scan);
    assert_eq!(
        state.inner.global_metadata_bytes.available_permits(),
        scan::capsule_reservation_bytes()
            + manual_success::Preflight::bytes()
            + charge
            + selection
            + candidate
            + TunnelAgentState::scan_window_bytes(1)
            + failure_table::Destination::minimum_bytes()
            + paired_frame
    );
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_exited.load(Ordering::SeqCst), 1);
    assert!(source.calls.lock().unwrap().is_empty());
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn initial_publication_snapshot_refusal_is_before_discovery_and_keeps_exact_publication() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Wifi))
        .await
        .unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let before = bank.available_permits();
    let desired = state.inner.lifecycle.desired().await.unwrap();
    let desired_bytes = before - bank.available_permits();
    drop(desired);
    assert_eq!(bank.available_permits(), before);
    let admitted = scan::capsule_reservation_bytes()
        + desired_bytes
        + publication_retain::first_bytes::<ManagedTunnel>()
        + scan_publications::InitialPublications::first_bytes(A)
        - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((before - admitted) as u32)
        .unwrap();
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.count(A), 1, "only the original explicit connection");
    assert!(state.inner.tunnels.read().await[A]
        .publication
        .ptr_eq(&publication));
    drop(error);
    drop(held);
    state.notify_shutdown();
    state.clear().await;
}

#[tokio::test]
async fn global_metadata_rejected_usb_snapshot_keeps_old_publication_and_no_extra_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state
        .ensure_tunnel(A, None, Some(StartTunnelConnectionType::Wifi))
        .await
        .unwrap();
    let publication = state.inner.tunnels.read().await[A].publication.clone();
    let bank = Arc::clone(&state.inner.global_metadata_bytes);
    let held = Arc::new(StdMutex::new(None));
    let retained = Arc::clone(&held);
    let observed_bank = Arc::clone(&bank);
    // All upstream publication/manual storage is already charged. Refuse the
    // actual USB snapshot identity backing by exactly one byte at its own phase.
    let raw = std::mem::size_of::<source::UsbDevice>() + A.len();
    *state.inner.usb_before_snapshot.lock().unwrap() = Some(Box::new(move || {
        let leave = raw + std::mem::size_of::<usize>() - 1;
        *retained.lock().unwrap() = Some(
            observed_bank
                .clone()
                .try_acquire_many_owned((observed_bank.available_permits() - leave) as u32)
                .unwrap(),
        );
    }));
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: 7,
        usb: true,
    }]);
    let result = state.refresh_once().await;
    assert_eq!(
        crate::output::operation_error_class(result.unwrap_err().as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(state.inner.tunnels.read().await[A]
        .publication
        .ptr_eq(&publication));
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        source.usb_calls.load(Ordering::SeqCst),
        1,
        "snapshot refusal was intercepted by upstream preflight"
    );
    assert_eq!(source.count(A), 1);
    drop(held.lock().unwrap().take());
    state.clear().await;
    state.notify_shutdown();
    drop(publication);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

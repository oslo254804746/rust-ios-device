use super::*;

fn no_source(source: &FakeSource) {
    assert_eq!(source.count(A), 0);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn ensure_frame_one_byte_refusal_precedes_coordinator_for_all_routes_and_zero_deadline() {
    for zero in [false, true] {
        for route in [
            None,
            Some(StartTunnelConnectionType::Usbmux),
            Some(StartTunnelConnectionType::Usb),
            Some(StartTunnelConnectionType::Wifi),
        ] {
            let source = FakeSource::new();
            let state = state(
                source.clone(),
                if zero { Duration::ZERO } else { LOCAL_BUDGET },
            );
            let bank = state.inner.global_metadata_bytes.clone();
            let before = bank.available_permits();
            let leave = state.ensure_frame_bytes() - 1;
            let held = bank
                .clone()
                .try_acquire_many_owned((before - leave) as u32)
                .unwrap();
            let coordinator = state.inner.lifecycle.hold_coordinator().await;
            let hook = metadata_vec::on_allocation(|_, _| panic!("refused ensure F backing"));
            let error = tokio::time::timeout(LOCAL_BUDGET, state.ensure_tunnel(A, None, route))
                .await
                .expect("refusal before coordinator wait")
                .unwrap_err();
            drop(hook);
            assert_eq!(
                crate::output::operation_error_class(error.as_ref()),
                ios_core::ErrorClass::ResourceLimit
            );
            no_source(&source);
            assert_eq!(state.inner.native.snapshot().occupied(), 0);
            assert_eq!(state.inner.retry.receipt_count(), 0);
            assert_eq!(bank.available_permits(), leave);
            drop(coordinator);
            assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
            drop(held);
            drop(state);
            assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
        }
    }
}

#[tokio::test]
async fn ensure_frame_original_input_and_deadline_prefixes_precede_closed_pool_refusal() {
    for case in 0..4 {
        let source = FakeSource::new();
        let state = state(
            source.clone(),
            if case == 0 {
                Duration::MAX
            } else {
                LOCAL_BUDGET
            },
        );
        let too_long = "x".repeat(4097);
        let udid = match case {
            1 => "",
            2 => &too_long,
            _ => A,
        };
        let manual = if case == 1 {
            Some(intent::ManualRequest {
                authority: state
                    .inner
                    .access
                    .authenticate(&axum::http::HeaderMap::new())
                    .unwrap()
                    .device_authority(Permission::ManageTunnels, A)
                    .unwrap_or_else(|_| panic!("fixture manage-tunnels authority")),
                host_filter: None,
            })
        } else {
            None
        };
        let bank = state.inner.global_metadata_bytes.clone();
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        bank.close();
        let hook = metadata_vec::on_allocation(|_, _| panic!("prefix/refused frame backing"));
        let error = state
            .ensure_tunnel_with_intent(udid, None, None, manual)
            .await
            .unwrap_err();
        drop(hook);
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            if case < 2 {
                ios_core::ErrorClass::InvalidInput
            } else {
                ios_core::ErrorClass::ResourceLimit
            }
        );
        // These are our own fixed prefix decisions, not a formatted native or
        // device-provided cause. Keep the exact earlier validation observable.
        let text = error.to_string();
        if case == 0 {
            assert!(text.contains("deadline overflow"));
        }
        if case == 1 {
            assert!(text.contains("exact device identifier"));
        }
        if case == 2 {
            assert!(text.contains("selector metadata limit"));
        }
        no_source(&source);
        assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
        assert_eq!(bank.available_permits(), 0);
        drop(held);
        drop(state);
        assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    }
}

#[tokio::test]
async fn ensure_frame_distinct_precoordinator_epochs_keep_each_exact_fee_through_actual_drop() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let frame = state.ensure_frame_bytes();
    let before = bank.available_permits();
    let held = bank
        .clone()
        .try_acquire_many_owned((before - 2 * frame) as u32)
        .unwrap();
    let probe = state.inner.lifecycle.scan_control(LOCAL_BUDGET).unwrap();
    let old = crate::cmd::tunnel::ControlIdentity::downgrade(&probe.epoch.generation);
    drop(probe);
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    let pointers = Arc::new(StdMutex::new(Vec::new()));
    let observed = pointers.clone();
    let observed_bank = bank.clone();
    let hook = metadata_vec::on_allocation(move |layout, pointer| {
        let mut pointers = observed.lock().unwrap();
        assert_eq!(layout.size(), frame);
        assert_eq!(
            observed_bank.available_permits(),
            (1 - pointers.len()) * frame
        );
        pointers.push(pointer);
    });
    let mut first = Box::pin(state.ensure_tunnel(A, None, None));
    assert!(futures_util::poll!(&mut first).is_pending());
    state.inner.lifecycle.clear();
    let probe = state.inner.lifecycle.scan_control(LOCAL_BUDGET).unwrap();
    let next = crate::cmd::tunnel::ControlIdentity::downgrade(&probe.epoch.generation);
    drop(probe);
    assert!(!crate::cmd::tunnel::ControlIdentity::ptr_eq(
        &old.upgrade().unwrap(),
        &next.upgrade().unwrap()
    ));
    let mut second = Box::pin(state.ensure_tunnel(A, None, None));
    assert!(futures_util::poll!(&mut second).is_pending());
    drop(hook);
    {
        let pointers = pointers.lock().unwrap();
        assert_eq!(pointers.len(), 2);
        assert_ne!(pointers[0], pointers[1]);
    }
    bank.close();
    state.inner.lifecycle.clear();
    no_source(&source);
    assert!(old.upgrade().is_some());
    assert!(next.upgrade().is_some());
    drop(first);
    assert_eq!(bank.available_permits(), frame);
    assert!(old.upgrade().is_none());
    assert!(next.upgrade().is_some());
    drop(second);
    assert_eq!(bank.available_permits(), 2 * frame);
    assert!(next.upgrade().is_none());
    drop(coordinator);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    drop(held);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn ensure_frame_allocation_delay_consumes_the_original_connection_deadline() {
    let source = FakeSource::new();
    let state = state(source.clone(), Duration::from_millis(20));
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let frame = state.ensure_frame_bytes();
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let hook = metadata_vec::on_allocation(move |layout, _| {
        assert_eq!(layout.size(), frame);
        assert_eq!(observed.fetch_add(1, Ordering::SeqCst), 0);
        std::thread::sleep(Duration::from_millis(60));
    });
    let error = state.ensure_tunnel(A, None, None).await.unwrap_err();
    drop(hook);
    assert!(error.is::<tokio::time::error::Elapsed>());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    no_source(&source);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    assert_eq!(bank.available_permits(), before);
}

#[tokio::test]
async fn ensure_frame_active_source_drop_keeps_preexisting_gate_and_refunds_actual_frame() {
    let source = FakeSource::new();
    source.script(A, [Behavior::Pending]);
    let state = state(source.clone(), LOCAL_BUDGET);
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let destination = state.inner.tunnels.write().await.prepare(A).unwrap();
    drop(destination);
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let frame = state.ensure_frame_bytes();
    let observed_bank = bank.clone();
    let observed = Arc::new(AtomicBool::new(false));
    let captured = observed.clone();
    let hook = metadata_vec::on_allocation(move |layout, _| {
        if !captured.swap(true, Ordering::SeqCst) {
            assert_eq!(layout.size(), frame);
            assert_eq!(observed_bank.available_permits(), before - frame);
        }
    });
    let mut work = Box::pin(state.ensure_tunnel(A, None, Some(StartTunnelConnectionType::Usbmux)));
    assert!(futures_util::poll!(&mut work).is_pending());
    drop(hook);
    assert!(observed.load(Ordering::SeqCst));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.active.load(Ordering::SeqCst), 1);
    bank.close();
    drop(work);
    assert_eq!(source.active.load(Ordering::SeqCst), 0);
    assert_eq!(source.exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(ticket.is_current());
    assert_eq!(bank.available_permits(), before);
    state.notify_shutdown();
    state.clear().await;
    state.drain_native(LOCAL_BUDGET).await.unwrap();
    drop(ticket);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn ensure_frame_actual_http_one_byte_refusal_precedes_coordinator_and_default_fallback() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let leave = 2048 + state.ensure_frame_bytes() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((before - leave) as u32)
        .unwrap();
    let coordinator = state.inner.lifecycle.hold_coordinator().await;
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/start-tunnel"))
        .json(&json!({"udid":A}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("failed to create tunnel"));
    no_source(&source);
    drop(coordinator);
    assert_eq!(state.inner.lifecycle.coordinator_count().await, 0);
    drop(held);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while bank.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

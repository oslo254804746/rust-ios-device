use super::*;

fn no_source(source: &FakeSource, state: &TunnelAgentState) {
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert!(source.calls.lock().unwrap().is_empty());
    assert_eq!(state.inner.scan_outcomes.payload_history().0, 0);
    assert!(state.inner.scan_outcomes.history().is_none());
    assert!(!futures_util::FutureExt::now_or_never(state.is_ready()).unwrap());
}

#[tokio::test]
async fn scan_frame_admission_preserves_prefix_and_refuses_before_capture_even_at_zero_whole() {
    // Whole overflow, terminal state and discovery overflow are the original
    // allocation-free prefix. A newly refused frame has not selected a timer.
    for case in 0..6 {
        let source = FakeSource::new();
        let mut state = state(
            source.clone(),
            if case == 2 {
                Duration::MAX
            } else {
                LOCAL_BUDGET
            },
        );
        if case == 0 {
            state.inner.get_mut().unwrap().scan_timeout = Duration::MAX;
        }
        if case == 3 {
            state.inner.get_mut().unwrap().scan_timeout = Duration::ZERO;
        }
        if case == 1 {
            state.notify_shutdown();
        }
        let bank = state.inner.global_metadata_bytes.clone();
        let before = bank.available_permits();
        let leave = if case < 3 || case == 5 {
            0
        } else {
            state.scan_frame_bytes() - 1
        };
        let held = bank
            .clone()
            .try_acquire_many_owned((before - leave) as u32)
            .unwrap();
        if case < 3 || case == 5 {
            bank.close();
        }
        let captures = Arc::new(AtomicUsize::new(0));
        let observed = captures.clone();
        state.inner.lifecycle.on_next_scan_capture(move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
        });
        let guard = metadata_vec::on_allocation(|_, _| panic!("refused frame allocated backing"));
        let error = state.refresh_once().await.unwrap_err();
        drop(guard);
        let expected = match case {
            0 | 2 => ios_core::ErrorClass::InvalidInput,
            1 => ios_core::ErrorClass::Cancelled,
            _ => ios_core::ErrorClass::ResourceLimit,
        };
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            expected
        );
        if case == 1 {
            assert!(error.is::<ios_core::OperationCancelled>());
        }
        assert_eq!(captures.load(Ordering::SeqCst), 0);
        no_source(&source, &state);
        assert!(state.inner.scan_lock.try_lock().is_ok());
        assert_eq!(bank.available_permits(), leave);
        drop(error);
        drop(held);
        drop(state);
        assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    }
}

#[tokio::test]
async fn scan_frame_prelock_old_markers_and_exact_backings_live_until_each_actual_drop() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let frame = state.scan_frame_bytes();
    let before = bank.available_permits();
    let held = bank
        .clone()
        .try_acquire_many_owned((before - 2 * frame) as u32)
        .unwrap();
    let control = state.inner.lifecycle.scan_control(LOCAL_BUDGET).unwrap();
    let scan = Arc::downgrade(&control.generation);
    let global = Arc::downgrade(&control.epoch.generation);
    drop(control);
    let lock = state.inner.scan_lock.lock().await;
    let allocations = Arc::new(StdMutex::new(Vec::new()));
    let observed = allocations.clone();
    let observed_bank = bank.clone();
    let guard = metadata_vec::on_allocation(move |layout, pointer| {
        let mut observed = observed.lock().unwrap();
        assert_eq!(layout.size(), frame);
        assert_eq!(
            observed_bank.available_permits(),
            (1 - observed.len()) * frame
        );
        observed.push(pointer);
    });
    let mut first = Box::pin(state.refresh_once());
    let mut second = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut first).is_pending());
    assert_eq!(bank.available_permits(), frame);
    // Keep the first stale frame unpolled while a second scan captures a
    // DIFFERENT actual global/scan pair. Cancellation is not destruction.
    state.inner.lifecycle.clear();
    let control = state.inner.lifecycle.scan_control(LOCAL_BUDGET).unwrap();
    let next_scan = Arc::downgrade(&control.generation);
    let next_global = Arc::downgrade(&control.epoch.generation);
    drop(control);
    assert!(!Arc::ptr_eq(
        &global.upgrade().unwrap(),
        &next_global.upgrade().unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &scan.upgrade().unwrap(),
        &next_scan.upgrade().unwrap()
    ));
    assert!(futures_util::poll!(&mut second).is_pending());
    assert_eq!(bank.available_permits(), 0);
    drop(guard);
    {
        let addresses = allocations.lock().unwrap();
        assert_eq!(addresses.len(), 2);
        assert_ne!(addresses[0], addresses[1]);
    }
    bank.close();
    state.inner.lifecycle.clear();
    assert!(scan.upgrade().is_some());
    assert!(global.upgrade().is_some());
    assert!(next_scan.upgrade().is_some());
    assert!(next_global.upgrade().is_some());
    no_source(&source, &state);
    drop(first);
    assert_eq!(bank.available_permits(), frame);
    assert!(scan.upgrade().is_none());
    assert!(global.upgrade().is_none());
    assert!(next_scan.upgrade().is_some());
    assert!(next_global.upgrade().is_some());
    drop(second);
    assert_eq!(bank.available_permits(), 2 * frame);
    assert!(scan.upgrade().is_none());
    assert!(global.upgrade().is_none());
    assert!(next_scan.upgrade().is_none());
    assert!(next_global.upgrade().is_none());
    drop(lock);
    drop(held);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn scan_frame_admission_delay_does_not_restart_whole_or_discovery_absolute_clock() {
    for whole_first in [false, true] {
        let source = FakeSource::new();
        let mut state = state(
            source.clone(),
            if whole_first {
                LOCAL_BUDGET
            } else {
                Duration::from_millis(20)
            },
        );
        state.inner.get_mut().unwrap().scan_timeout = if whole_first {
            Duration::from_millis(20)
        } else {
            LOCAL_BUDGET
        };
        let bank = state.inner.global_metadata_bytes.clone();
        let before = bank.available_permits();
        let frame = state.scan_frame_bytes();
        let observed = Arc::new(AtomicUsize::new(0));
        let captured = observed.clone();
        let guard = metadata_vec::on_allocation(move |layout, _| {
            assert_eq!(layout.size(), frame);
            assert_eq!(captured.fetch_add(1, Ordering::SeqCst), 0);
            std::thread::sleep(Duration::from_millis(60));
        });
        let error = state.refresh_once().await.unwrap_err();
        drop(guard);
        assert!(error.is::<tokio::time::error::Elapsed>());
        assert_eq!(observed.load(Ordering::SeqCst), 1);
        no_source(&source, &state);
        assert_eq!(bank.available_permits(), before);
        assert!(state.inner.scan_lock.try_lock().is_ok());
    }
}

#[tokio::test]
async fn scan_frame_active_source_caller_drop_releases_source_session_and_original_frame() {
    let source = FakeSource::new();
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let frame = state.scan_frame_bytes();
    let observed_bank = bank.clone();
    let observed = Arc::new(AtomicBool::new(false));
    let captured = observed.clone();
    let guard = metadata_vec::on_allocation(move |layout, _| {
        if !captured.swap(true, Ordering::SeqCst) {
            assert_eq!(layout.size(), frame);
            assert_eq!(observed_bank.available_permits(), before - frame);
        }
    });
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    drop(guard);
    assert!(observed.load(Ordering::SeqCst));
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.scan_outcomes.payload_history().0, 1);
    assert!(bank.available_permits() < before - frame);
    bank.close();
    drop(scan);
    assert_eq!(source.usb_active.load(Ordering::SeqCst), 0);
    assert_eq!(source.usb_exited.load(Ordering::SeqCst), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(state.inner.scan_outcomes.payload_history().0, 0);
    assert!(state.inner.scan_outcomes.history().is_none());
    assert!(state.inner.scan_lock.try_lock().is_ok());
    assert_eq!(bank.available_permits(), before);
    drop(state);
    assert_eq!(bank.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

#[tokio::test]
async fn scan_frame_actual_http_refusal_precedes_held_scan_lock_and_all_source_work() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    let bank = state.inner.global_metadata_bytes.clone();
    let before = bank.available_permits();
    let replies = response_json::Replies::new(&bank).unwrap();
    let response = replies.operation("refresh").unwrap();
    let response_bytes = before - bank.available_permits();
    drop(response);
    drop(replies);
    assert_eq!(bank.available_permits(), before);
    let leave = response_bytes + state.scan_frame_bytes() - 1;
    let held = bank
        .clone()
        .try_acquire_many_owned((before - leave) as u32)
        .unwrap();
    let lock = state.inner.scan_lock.lock().await;
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .post(format!("http://127.0.0.1:{port}/refresh"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("tunnel agent refresh failed"));
    no_source(&source, &state);
    drop(lock);
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

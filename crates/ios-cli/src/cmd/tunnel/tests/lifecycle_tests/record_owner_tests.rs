use super::*;

#[tokio::test]
async fn record_copy_refusal_keeps_live_device_and_never_starts_another_source() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    drop(start(state.clone(), A).await.unwrap());
    let bank = state.inner.global_metadata_bytes.clone();
    let mut held = bank
        .clone()
        .try_acquire_many_owned(bank.available_permits() as u32)
        .unwrap();
    let error = state.get_record(A).await.unwrap_err();
    assert!(error.is::<record_owner::CopyAdmission>());
    let error = state.list_records().await.err().unwrap();
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    // Keep the direct copy/list zero-quota checks above. The live ensure must
    // first own its actual F, then reach the SAME record-copy refusal at zero.
    drop(held.split(state.ensure_frame_bytes()).unwrap());
    let error = state.ensure_tunnel(A, None, None).await.unwrap_err();
    assert!(error.is::<record_owner::CopyAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert!(state.inner.tunnels.read().await.get(A).unwrap().is_alive());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    drop(held);
    assert!(state.get_record(A).await.unwrap().is_some());
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn record_copy_post_source_refusal_does_not_browse_default_fallback() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    let bank = state.inner.global_metadata_bytes.clone();
    let pressure = Arc::new(StdMutex::new(None));
    let captured = pressure.clone();
    let hook = record_owner::on_before_admission(move || {
        let held = bank
            .clone()
            .try_acquire_many_owned(bank.available_permits() as u32)
            .unwrap();
        *captured.lock().unwrap() = Some(held);
    });
    let error = state.ensure_tunnel(A, None, None).await.unwrap_err();
    drop(hook);
    assert!(error.is::<record_owner::CopyAdmission>());
    assert_eq!(
        crate::output::operation_error_class(error.as_ref()),
        ios_core::ErrorClass::ResourceLimit
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), 0);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(state.inner.tunnels.read().await.is_empty());
    drop(pressure.lock().unwrap().take());
    drop(start(state.clone(), A).await.unwrap());
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
    assert_eq!(source.count(A), 2);
}

#[tokio::test]
async fn record_automatic_alive_check_does_not_copy_output_fields() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    drop(start(state.clone(), A).await.unwrap());
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    let hook = record_owner::on_text_allocated(|_, _| panic!("alive scan copied output record"));
    state.refresh_connect(A, Route::Usbmux, &ticket, None).await;
    drop(hook);
    assert!(state.inner.tunnels.read().await.get(A).unwrap().is_alive());
    assert_eq!(source.count(A), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

#[tokio::test]
async fn record_http_quota_refusal_is_service_unavailable_instead_of_missing() {
    let source = FakeSource::new();
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    let mut control_input_0 = state.prepare_cutover::<2>().unwrap();

    drop(start(state.clone(), A).await.unwrap());
    let bank = state.inner.global_metadata_bytes.clone();
    let held = bank
        .clone()
        .try_acquire_many_owned((bank.available_permits() - 2048) as u32)
        .unwrap();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let response = client()
        .get(format!("http://127.0.0.1:{port}/tunnel/{A}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
    assert!(response.text().await.unwrap().contains("record"));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    drop(held);
    state.clear(&mut control_input_0).await;
    drop(control_input_0);
}

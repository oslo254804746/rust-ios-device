use super::*;

#[tokio::test]
async fn connection_ticket_actual_lookup_refusal_refunds_original_record_in_both_banks() {
    for global_short in [true, false] {
        let source = FakeSource::new();
        let state = state(source.clone(), LOCAL_BUDGET);
        drop(start(state.clone(), A).await.unwrap());
        let usb = source.usb_calls.load(Ordering::SeqCst);
        let paired = source.paired_calls.load(Ordering::SeqCst);
        let global = state.inner.global_metadata_bytes.clone();
        let published = state.inner.metadata_bytes.clone();
        let before_g = global.available_permits();
        let record = state.get_record(A).await.unwrap().unwrap();
        let record_bytes = before_g - global.available_permits();
        drop(record);
        assert_eq!(global.available_permits(), before_g);
        let bank = if global_short { &global } else { &published };
        let leave = publication::TicketOwner::body_bytes() - 1
            + if global_short { record_bytes } else { 0 };
        let held = bank
            .clone()
            .try_acquire_many_owned((bank.available_permits() - leave) as u32)
            .unwrap();
        let before = (global.available_permits(), published.available_permits());
        let error = state.get_local_target(A).await.err().unwrap();
        assert!(error.is::<publication::Admission>());
        assert_eq!(
            crate::output::operation_error_class(error.as_ref()),
            ios_core::ErrorClass::ResourceLimit
        );
        assert_eq!(
            (global.available_permits(), published.available_permits()),
            before
        );
        assert_eq!(source.count(A), 1);
        assert_eq!(source.usb_calls.load(Ordering::SeqCst), usb);
        assert_eq!(source.paired_calls.load(Ordering::SeqCst), paired);
        assert_eq!(source.drops.load(Ordering::SeqCst), 0);
        assert_eq!(state.inner.tunnels.read().await.len(), 1);
        drop(held);
        state.notify_shutdown();
        state.clear().await;
        drop(state);
        assert_eq!(global.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
        assert_eq!(published.available_permits(), MAX_PUBLISHED_METADATA_BYTES);
    }
}

#[tokio::test]
async fn connection_ticket_actual_target_moves_same_root_without_extending_row_or_state() {
    let source = FakeSource::new();
    let state = state(source.clone(), LOCAL_BUDGET);
    drop(start(state.clone(), A).await.unwrap());
    let global = state.inner.global_metadata_bytes.clone();
    let published = state.inner.metadata_bytes.clone();
    let LocalTarget {
        ticket,
        record,
        publication,
    } = state.get_local_target(A).await.unwrap().unwrap();
    let pointer = &*ticket as *const Ticket;
    let key = ticket.udid().as_ptr();
    assert!(ticket.is_current() && publication.check().is_ok());
    let held_g = global
        .clone()
        .try_acquire_many_owned(global.available_permits() as u32)
        .unwrap();
    let held_p = published
        .clone()
        .try_acquire_many_owned(published.available_permits() as u32)
        .unwrap();
    global.close();
    published.close();
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(publication.check().is_err() && !ticket.is_current());
    assert_eq!(&*ticket as *const Ticket, pointer);
    assert_eq!(ticket.udid().as_ptr(), key);
    state.notify_shutdown();
    drop((state, record, publication, held_g, held_p));
    assert_eq!(
        published.available_permits(),
        MAX_PUBLISHED_METADATA_BYTES - publication::TicketOwner::body_bytes()
    );
    assert!(global.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(ticket);
    assert_eq!(global.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
    assert_eq!(published.available_permits(), MAX_PUBLISHED_METADATA_BYTES);
}

#[tokio::test]
async fn connection_ticket_http_refusal_keeps_original_503_before_local_tcp_dial() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let source = FakeSource::new();
    source.proxy_port.store(
        listener.local_addr().unwrap().port() as usize,
        Ordering::SeqCst,
    );
    let state = state(source.clone(), LOCAL_BUDGET);
    drop(start(state.clone(), A).await.unwrap());
    let usb = source.usb_calls.load(Ordering::SeqCst);
    let paired = source.paired_calls.load(Ordering::SeqCst);
    let global = state.inner.global_metadata_bytes.clone();
    let published = state.inner.metadata_bytes.clone();
    let held = published
        .clone()
        .try_acquire_many_owned(
            (published.available_permits() - publication::TicketOwner::body_bytes() + 1) as u32,
        )
        .unwrap();
    let (port, server) = spawn_test_router(agent_router(state.clone())).await;
    let error = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{port}/connect?udid={A}&port=12345"
    ))
    .await
    .unwrap_err();
    match error {
        tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(response.headers()["x-ios-error-class"], "resource_limit");
            assert!(std::str::from_utf8(response.body().as_ref().unwrap())
                .unwrap()
                .contains("tunnel record copy failed"));
        }
        other => panic!("original lookup error expected: {other}"),
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    assert_eq!(source.count(A), 1);
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), usb);
    assert_eq!(source.paired_calls.load(Ordering::SeqCst), paired);
    drop(held);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    state.notify_shutdown();
    state.clear().await;
    drop(state);
    tokio::time::timeout(LOCAL_BUDGET, async {
        while global.available_permits() != MAX_GLOBAL_KNOWN_METADATA_BYTES {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(published.available_permits(), MAX_PUBLISHED_METADATA_BYTES);
}

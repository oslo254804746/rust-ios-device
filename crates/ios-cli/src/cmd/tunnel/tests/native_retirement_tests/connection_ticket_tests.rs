use super::*;

#[tokio::test]
async fn connection_ticket_native_root_survives_row_clear_without_delaying_original_joins_and_eof()
{
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    drop(publish(&state, Route::Wifi(None)).await.unwrap());
    let global = state.inner.global_metadata_bytes.clone();
    let published = state.inner.metadata_bytes.clone();
    let LocalTarget {
        ticket,
        record,
        publication,
    } = state.get_local_target(DEVICE).await.unwrap().unwrap();
    let pointer = &*ticket as *const Ticket;
    let key = ticket.udid().as_ptr();
    let native = source.handle();
    state.clear().await;
    assert!(publication.check().is_err() && !ticket.is_current());
    assert_eq!(state.inner.native.snapshot().retired, 1);
    state.notify_shutdown();
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert!(native.shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "original native joins, peer Ok(0) and task baseline"
    );
    assert_eq!(&*ticket as *const Ticket, pointer);
    assert_eq!(ticket.udid().as_ptr(), key);
    state.notify_shutdown();
    drop((state, source, record, publication));
    assert_eq!(
        published.available_permits(),
        MAX_PUBLISHED_METADATA_BYTES - publication::TicketOwner::body_bytes()
    );
    assert!(global.available_permits() < MAX_GLOBAL_KNOWN_METADATA_BYTES);
    drop(ticket);
    assert_eq!(published.available_permits(), MAX_PUBLISHED_METADATA_BYTES);
    assert_eq!(global.available_permits(), MAX_GLOBAL_KNOWN_METADATA_BYTES);
}

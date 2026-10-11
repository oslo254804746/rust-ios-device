//! The shared known-capacity permit follows actual native join ownership and
//! table metadata independently. These are real smoltcp reader/writer fixtures.
use super::*;

fn configured(source: Arc<LocalSource>) -> (TunnelAgentState, usize) {
    let mut state = state(source);
    let charged =
        std::mem::size_of::<ManagedDevice>() + std::mem::size_of::<ManagedTunnel>() + DEVICE.len();
    Arc::get_mut(&mut state.inner).unwrap().metadata_bytes = Arc::new(Semaphore::new(charged));
    (state, charged)
}

#[tokio::test]
async fn native_metadata_charge_survives_table_removal_until_original_joins_and_eof() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged) = configured(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    state.clear().await;
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    state.drain_native(OBSERVE).await.unwrap();
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
    assert!(source.handle().shutdown_status().is_ok());
    assert!(
        recovered(&source, expected).await,
        "native peer did not reach EOF or task baseline"
    );
    state.notify_shutdown();
}

#[tokio::test]
async fn native_metadata_charge_survives_certified_joins_while_table_still_owns_metadata() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged) = configured(source.clone());
    publish(&state, Route::Wifi(None)).await.unwrap();
    state.notify_shutdown();
    state.drain_native(OBSERVE).await.unwrap();
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(state.inner.tunnels.read().await.len(), 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.clear().await;
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
}

#[tokio::test]
async fn native_metadata_charge_survives_cancelled_publication_query_until_original_joins() {
    let expected = baseline();
    let source = LocalSource::new();
    let (state, charged) = configured(source.clone());
    *source.usb_gate.lock().unwrap() = Some(Arc::new(Semaphore::new(0)));
    let mut connection = Box::pin(publish(&state, Route::Usbmux));
    assert!(futures_util::poll!(&mut connection).is_pending());
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    drop(connection);
    assert_eq!(state.inner.tunnels.read().await.len(), 0);
    assert_eq!(state.inner.native.snapshot().retired, 1);
    assert_eq!(state.inner.metadata_bytes.available_permits(), 0);
    state.drain_native(OBSERVE).await.unwrap();
    assert_eq!(state.inner.metadata_bytes.available_permits(), charged);
    assert!(source.handle().shutdown_status().is_ok());
    assert!(recovered(&source, expected).await);
    state.notify_shutdown();
}

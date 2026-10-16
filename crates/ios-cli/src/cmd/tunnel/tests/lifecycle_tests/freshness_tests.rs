//! Complete paired observations must invalidate an alive old address, while
//! incomplete or older discovery cannot certify detachment of a fresh owner.
use super::*;

fn paired(source: &FakeSource, host: Option<&str>) {
    *source.paired.lock().unwrap() = host
        .map(|host| {
            vec![PairedMobdev2Device {
                udid: A.into(),
                host: host.into(),
            }]
        })
        .unwrap_or_default();
}

async fn observed(state: &TunnelAgentState) -> Option<Route> {
    state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .map(|tunnel| tunnel.observed_route.clone())
}

#[tokio::test]
async fn freshness_alive_paired_address_change_rebuilds_and_complete_detach_releases_charge() {
    let source = FakeSource::new();
    paired(&source, Some("192.0.2.70"));
    let state = state(source.clone(), LOCAL_BUDGET);
    state.refresh_once().await.unwrap();
    assert!(observed(&state).await == Some(Route::Mobdev2("192.0.2.70".into())));
    let old = state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .unwrap()
        .publication
        .clone();
    paired(&source, Some("192.0.2.71"));
    state.refresh_once().await.unwrap();
    assert!(observed(&state).await == Some(Route::Mobdev2("192.0.2.71".into())));
    assert!(!Arc::ptr_eq(
        &old,
        &state.inner.tunnels.read().await.get(A).unwrap().publication
    ));
    assert_eq!(source.count(A), 2);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    assert!(
        source.alive.load(Ordering::SeqCst),
        "fixture never faked runtime death"
    );
    paired(&source, None);
    state.refresh_once().await.unwrap();
    assert!(state.get_record(A).await.unwrap().is_none());
    assert_eq!(source.count(A), 2);
    assert_eq!(source.drops.load(Ordering::SeqCst), 2);
    let table = state.inner.tunnels.read().await;
    assert!(table.is_empty());
    let retained_backing = table.backing_bytes();
    drop(table);
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        MAX_PUBLISHED_METADATA_BYTES - retained_backing
    );
    state.clear().await;
    assert_eq!(
        state.inner.metadata_bytes.available_permits(),
        MAX_PUBLISHED_METADATA_BYTES
    );
}

#[tokio::test]
async fn freshness_failed_paired_browse_keeps_original_alive_publication_and_typed_primary() {
    let source = FakeSource::new();
    paired(&source, Some("192.0.2.70"));
    let state = state(source.clone(), LOCAL_BUDGET);
    state.refresh_once().await.unwrap();
    let old = state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .unwrap()
        .publication
        .clone();
    source.paired_failure.store(true, Ordering::SeqCst);
    paired(&source, None);
    let error = state.refresh_once().await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(Arc::ptr_eq(
        &old,
        &state.inner.tunnels.read().await.get(A).unwrap().publication
    ));
    assert_eq!(source.count(A), 1);
    assert_eq!(source.drops.load(Ordering::SeqCst), 0);
    state.clear().await;
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn freshness_old_paired_browse_cannot_retire_concurrent_replacement_publication() {
    let source = FakeSource::new();
    paired(&source, Some("192.0.2.70"));
    let state = state(source.clone(), LOCAL_BUDGET);
    state.refresh_once().await.unwrap();
    let gate = Arc::new(Semaphore::new(0));
    *source.paired_gate.lock().unwrap() = Some(gate.clone());
    let mut scan = Box::pin(state.refresh_once());
    assert!(futures_util::poll!(&mut scan).is_pending());
    assert_eq!(source.paired_active.load(Ordering::SeqCst), 1);
    state.inner.tunnels.write().await.remove(A);
    paired(&source, Some("192.0.2.71"));
    let ticket = state.inner.lifecycle.ticket(A).await.unwrap();
    state
        .connect_and_store(A, Route::Mobdev2("192.0.2.71".into()), &ticket, None, None)
        .await
        .unwrap();
    let replacement = state
        .inner
        .tunnels
        .read()
        .await
        .get(A)
        .unwrap()
        .publication
        .clone();
    gate.add_permits(1);
    scan.await.unwrap();
    assert!(Arc::ptr_eq(
        &replacement,
        &state.inner.tunnels.read().await.get(A).unwrap().publication
    ));
    assert!(observed(&state).await == Some(Route::Mobdev2("192.0.2.71".into())));
    assert_eq!(source.count(A), 2);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
    state.clear().await;
}

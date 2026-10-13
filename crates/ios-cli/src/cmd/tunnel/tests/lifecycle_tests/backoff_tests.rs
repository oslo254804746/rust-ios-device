use super::*;

fn usb(source: &FakeSource, id: u32) {
    *source.usb_details.lock().unwrap() = Some(vec![source::UsbDevice {
        udid: A.into(),
        device_id: id,
        usb: true,
    }]);
    source.connected_ids.lock().unwrap().insert(A.into(), id);
}

fn paired(source: &FakeSource, host: &str) {
    *source.usb_details.lock().unwrap() = Some(Vec::new());
    *source.paired.lock().unwrap() = vec![PairedMobdev2Device {
        udid: A.into(),
        host: host.into(),
    }];
}

#[tokio::test]
async fn lifecycle_tunnel_backoff_new_native_id_does_not_inherit_old_failure() {
    let source = FakeSource::new();
    usb(&source, 71);
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::PermissionDenied),
            Behavior::Success,
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    usb(&source, 72);
    state.refresh_once().await.unwrap();
    let published = state.get_record(A).await.is_some();
    let failed = state.inner.failed_attempts.read().await.contains_key(A);
    let calls = source.count(A);
    state.clear().await;
    assert!(
        published,
        "new native ID inherited previous attachment backoff"
    );
    assert!(!failed);
    assert_eq!(calls, 2);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_backoff_changed_paired_host_retries_without_new_pairing() {
    let source = FakeSource::new();
    paired(&source, "192.0.2.1");
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Success,
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 1);
    paired(&source, "192.0.2.2");
    state.refresh_once().await.unwrap();
    let published = state.get_record(A).await.is_some();
    let calls = source.calls.lock().unwrap().clone();
    state.clear().await;
    assert!(
        published,
        "known paired host change inherited old host backoff"
    );
    assert_eq!(
        calls,
        vec![
            (A.into(), Route::Mobdev2("192.0.2.1".into())),
            (A.into(), Route::Mobdev2("192.0.2.2".into()))
        ]
    );
    assert_eq!(source.usb_calls.load(Ordering::SeqCst), 3);
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lifecycle_tunnel_backoff_changed_failure_resets_count_and_same_id_keeps_schedule() {
    let source = FakeSource::new();
    usb(&source, 71);
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::PermissionDenied),
            Behavior::Failure(std::io::ErrorKind::ConnectionReset),
            Behavior::Failure(std::io::ErrorKind::BrokenPipe),
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    usb(&source, 72);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2, "changed identity did not reach source");
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 1);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 2);
    state
        .inner
        .failed_attempts
        .write()
        .await
        .get_mut(A)
        .unwrap()
        .last_attempt -= Duration::from_secs(31);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 3);
    assert_eq!(state.inner.failed_attempts.read().await[A].fail_count, 2);
    state.refresh_once().await.unwrap();
    assert_eq!(source.count(A), 3);
    assert!(state.list_records().await.is_empty());
}

#[tokio::test]
async fn lifecycle_tunnel_backoff_known_route_change_does_not_inherit_usb_failure() {
    let source = FakeSource::new();
    usb(&source, 71);
    source.script(
        A,
        [
            Behavior::Failure(std::io::ErrorKind::PermissionDenied),
            Behavior::Success,
        ],
    );
    let state = state(source.clone(), TUNNEL_CONNECT_TIMEOUT);
    state.refresh_once().await.unwrap();
    paired(&source, "192.0.2.3");
    state.refresh_once().await.unwrap();
    let published = state.get_record(A).await.is_some();
    let calls = source.calls.lock().unwrap().clone();
    state.clear().await;
    assert!(published, "known route change inherited USB backoff");
    assert_eq!(
        calls,
        vec![
            (A.into(), Route::Usbmux),
            (A.into(), Route::Mobdev2("192.0.2.3".into()))
        ]
    );
    assert_eq!(source.drops.load(Ordering::SeqCst), 1);
}

#[test]
fn lifecycle_tunnel_backoff_debug_does_not_expose_retained_route_host() {
    let secret = "synthetic-private-host-identity";
    let attempt = FailedTunnelAttempt {
        version: Arc::new(()),
        last_attempt: Instant::now(),
        fail_count: 2,
        identity: failure_table::Identity::owned(
            AttemptIdentity {
                route: Route::Mobdev2(secret.into()),
                native_id: None,
            },
            &Arc::new(Semaphore::new(1024)),
        )
        .unwrap(),
        slot: None,
    };
    let rendered = format!("{attempt:?}");
    assert!(!rendered.contains(secret));
    assert!(!rendered.contains("Mobdev2"));
}

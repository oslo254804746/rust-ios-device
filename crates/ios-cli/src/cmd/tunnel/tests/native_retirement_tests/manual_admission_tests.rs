//! Selector refusal preserves an already published original native owner.
use super::*;

#[tokio::test]
async fn native_manual_selector_rejection_keeps_original_arc_then_recovers_joins_and_eof() {
    let expected = baseline();
    let source = LocalSource::new();
    let state = state(source.clone());
    let mut terminal_inputs = PreparedTerminal::new(&state);
    state
        .ensure_tunnel(DEVICE, None, Some(StartTunnelConnectionType::Usbmux))
        .await
        .unwrap();
    let original = source.handle();
    let publication = state.inner.tunnels.read().await[DEVICE].publication.clone();
    let gates = state.inner.lifecycle.coordinator_count().await;
    let calls = (
        source.connect_calls.load(Ordering::SeqCst),
        source.usb_calls.load(Ordering::SeqCst),
    );
    let oversized = format!("synthetic-private-selector{}", "x".repeat(4096));
    let host_error = state
        .ensure_tunnel(
            DEVICE,
            Some(&oversized),
            Some(StartTunnelConnectionType::Wifi),
        )
        .await
        .err();
    let udid_error = state.ensure_tunnel(&oversized, None, None).await.err();
    let retained = state
        .inner
        .tunnels
        .read()
        .await
        .get(DEVICE)
        .is_some_and(|row| {
            row.publication.ptr_eq(&publication)
                && row
                    .device
                    .native_handle()
                    .is_some_and(|handle| Arc::ptr_eq(handle, &original))
        });
    let alive = original.is_alive();
    let snapshot = state.inner.native.snapshot();
    let after_gates = state.inner.lifecycle.coordinator_count().await;
    let after_calls = (
        source.connect_calls.load(Ordering::SeqCst),
        source.usb_calls.load(Ordering::SeqCst),
    );
    let mut proxy = TcpStream::connect(("127.0.0.1", original.userspace_port.unwrap()))
        .await
        .unwrap();
    proxy.write_all(&[0]).await.unwrap();
    let recovery = finish(&state, &source, expected, &mut terminal_inputs).await;
    drop(terminal_inputs);
    let eof = tokio::time::timeout(OBSERVE, proxy.read_u8()).await;
    // First certify original joins, registry release, duplex/loopback EOF and
    // task/permit recovery; only then evaluate the selector-refusal regression.
    assert!(recovery && original.shutdown_status().is_ok());
    assert!(state.inner.native.snapshot().closed_and_empty());
    assert_eq!(baseline(), expected);
    assert!(eof.unwrap().is_err_and(|error| matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset
    )));
    assert_eq!(
        state.inner.lifecycle.connects.available_permits(),
        lifecycle::MAX_CONNECTS
    );
    assert_eq!(
        state.inner.failure_slots.available_permits(),
        MAX_FAILURE_RECORDS
    );

    for error in [host_error, udid_error] {
        let error = error.expect("oversized selector was accepted beside an original native owner");
        let rendered = crate::output::operation_failure_message(
            crate::output::ErrorFormat::Json,
            "tunnel-manual-selector",
            &error,
        )
        .unwrap();
        let body: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(body["error"]["class"], "resource_limit");
        assert!(!crate::output::error_message(&error).contains("synthetic-private-selector"));
    }
    assert!(retained && alive);
    assert_eq!((after_gates, after_calls), (gates, calls));
    assert_eq!(
        (
            snapshot.active,
            snapshot.native_handles,
            snapshot.occupied()
        ),
        (1, 1, 1)
    );
}

//! External consumer of terminal stop and immutable native shutdown evidence.

use ios_core::{ErrorClass, TunnelHandle, TunnelInfo, TunnelShutdownFailure};
use std::{error::Error, time::Duration};

fn legacy() -> (TunnelHandle, tokio::sync::watch::Receiver<()>) {
    TunnelHandle::new(
        "public-shutdown-fixture".into(),
        TunnelInfo {
            server_address: "::1".into(),
            server_rsd_port: 58783,
            client_address: "::1".into(),
            client_mtu: 1280,
        },
        None,
    )
}

#[tokio::test]
async fn native_public_shutdown_stops_before_future_poll_and_overflow_leaves_owner_live() {
    let (handle, receiver) = legacy();
    assert!(handle.is_alive());
    let error = handle.shutdown(Duration::MAX).await.unwrap_err();
    assert_eq!(ios_core::classify_error(&error), ErrorClass::InvalidInput);
    assert!(handle.is_alive());
    assert!(matches!(receiver.has_changed(), Ok(false)));

    fn send<F: std::future::Future + Send>(future: F) -> F {
        future
    }
    let cleanup = send(handle.shutdown(Duration::from_secs(1)));
    let stopped_without_poll = !handle.is_alive();
    let signalled_without_poll = receiver.has_changed();
    drop(cleanup);
    // This legacy constructor owns no native worker; the external receiver is
    // deliberately retained, and shutdown cannot join any task it represents.
    handle.shutdown(Duration::from_secs(1)).await.unwrap();
    drop(handle);
    assert!(stopped_without_poll);
    assert!(matches!(signalled_without_poll, Ok(true)));
    assert!(receiver.has_changed().is_err());
}

#[tokio::test]
async fn native_public_zero_budget_exposes_native_elapsed_and_immutable_snapshot() {
    let (handle, receiver) = legacy();
    let error = handle.shutdown(Duration::ZERO).await.unwrap_err();
    let mut source: &(dyn Error + 'static) = &error;
    let report = loop {
        if let Some(report) = source.downcast_ref::<TunnelShutdownFailure>() {
            break report;
        }
        source = source
            .source()
            .expect("public native shutdown evidence missing");
    };
    let stopped = !handle.is_alive();
    let retry = handle.shutdown(Duration::from_secs(1)).await;
    drop(handle);
    drop(receiver);
    retry.unwrap();
    assert!(stopped);
    assert_eq!(ios_core::classify_error(&error), ErrorClass::Timeout);
    assert!(report.primary().is::<tokio::time::error::Elapsed>());
    assert_eq!(report.pending_joins(), 0);
    assert!(report.joins_complete());
    assert_eq!(report.cleanup().count(), 0);
    assert_eq!(report.additional_unretained_outcomes(), 0);
    assert!(!format!("{report:?}").contains("public-shutdown-fixture"));
}

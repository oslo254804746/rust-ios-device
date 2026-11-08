use super::*;
use ios_core::{classify_error, ErrorClass, OperationCancelled};
use std::sync::atomic::{AtomicUsize, Ordering};

fn pending_signal() -> impl Future<Output = std::io::Result<()>> {
    std::future::pending()
}

#[tokio::test]
async fn lifecycle_oneshot_admission_preserves_native_input_and_zero_timeout() {
    for seconds in [f64::NAN, f64::INFINITY, -1.0, f64::MAX] {
        let error = finite_duration(seconds).unwrap_err();
        assert_eq!(classify_error(error.as_ref()), ErrorClass::InvalidInput);
        assert!(error.downcast_ref::<std::io::Error>().is_some());
    }
    assert_eq!(finite_duration(0.001).unwrap(), Duration::from_millis(100));
    assert_eq!(finite_duration(0.0).unwrap(), Duration::ZERO);
    let zero = Budget::new(Duration::ZERO, "eval").err().unwrap();
    assert_eq!(classify_error(zero.as_ref()), ErrorClass::Timeout);
    assert!(zero.downcast_ref::<WebInspectorError>().is_some());
    let overflow = Budget::new(Duration::MAX, "eval").err().unwrap();
    assert_eq!(classify_error(overflow.as_ref()), ErrorClass::InvalidInput);
    assert!(overflow.downcast_ref::<std::io::Error>().is_some());
}

fn due() -> Budget {
    Budget {
        deadline: Instant::now() - Duration::from_millis(1),
        duration: Duration::from_millis(17),
        phase: "eval",
    }
}

#[tokio::test]
async fn lifecycle_oneshot_due_timer_suppresses_ready_work_and_keeps_elapsed() {
    let polls = AtomicUsize::new(0);
    let error = due()
        .drive(
            async {
                polls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            pending_signal(),
        )
        .await
        .unwrap_err();
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some());
    assert!(error.to_string().contains("17ms"));
}

#[tokio::test]
async fn lifecycle_oneshot_ready_signal_wins_due_timer_and_ready_work() {
    let polls = AtomicUsize::new(0);
    let error = due()
        .drive(
            async {
                polls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            async { Ok(()) },
        )
        .await
        .unwrap_err();
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Cancelled);
    assert!(error.downcast_ref::<OperationCancelled>().is_some());
    let error = due()
        .drive(async { Ok(()) }, async {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .await
        .unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert!(error.downcast_ref::<OperationCancelled>().is_none());
}

#[tokio::test]
async fn lifecycle_oneshot_one_pinned_signal_survives_stage_boundaries() {
    let polls = AtomicUsize::new(0);
    let signal = std::future::poll_fn(|_| {
        if polls.fetch_add(1, Ordering::SeqCst) == 2 {
            std::task::Poll::Ready(Ok(()))
        } else {
            std::task::Poll::Pending
        }
    });
    tokio::pin!(signal);
    let budget = Budget::new(Duration::from_secs(1), "eval").unwrap();
    assert_eq!(
        budget
            .drive(async { Ok(1) }, signal.as_mut())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        budget
            .drive(async { Ok(2) }, signal.as_mut())
            .await
            .unwrap(),
        2
    );
    let error = budget
        .drive(async { Ok(3) }, signal.as_mut())
        .await
        .unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Cancelled);
    assert_eq!(polls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn lifecycle_oneshot_stages_share_total_and_drop_pending_work() {
    struct DropFlag<'a>(&'a AtomicUsize);
    impl Drop for DropFlag<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = AtomicUsize::new(0);
    let budget = Budget::new(Duration::from_millis(100), "eval").unwrap();
    budget
        .drive(
            async {
                tokio::time::sleep(Duration::from_millis(60)).await;
                Ok(())
            },
            pending_signal(),
        )
        .await
        .unwrap();
    let flag = DropFlag(&drops);
    let error = budget
        .drive(
            async move {
                let _flag = flag;
                tokio::time::sleep(Duration::from_millis(60)).await;
                Ok(())
            },
            pending_signal(),
        )
        .await
        .unwrap_err();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some());
    assert!(error.to_string().contains("100ms"));
}

#[test]
fn lifecycle_oneshot_dual_failure_keeps_primary_class_and_safe_wrapper() {
    let primary = anyhow::Error::new(OperationCancelled::new()).context("owned-private-primary");
    let cleanup = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "owned-private-cleanup",
    ));
    let error = finish::<()>(Err(primary), Err(cleanup)).unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Cancelled);
    let dual = error.downcast_ref::<CleanupFailure>().unwrap();
    assert!(dual.primary.downcast_ref::<OperationCancelled>().is_some());
    assert!(dual.cleanup.downcast_ref::<std::io::Error>().is_some());
    assert!(!format!("{dual:?} {dual}").contains("owned-private"));
    let error = finish(
        Ok(42),
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()),
    )
    .unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
    assert_eq!(finish(Ok(42), Ok(())).unwrap(), 42);
}

#[tokio::test]
async fn lifecycle_oneshot_late_ready_success_expires_but_native_failure_stays_primary() {
    for failed in [false, true] {
        let polls = AtomicUsize::new(0);
        let budget = Budget::new(Duration::from_millis(20), "eval").unwrap();
        let error = budget
            .drive(
                async {
                    polls.fetch_add(1, Ordering::SeqCst);
                    // A synchronous source/output poll is cooperative. Its completed
                    // success still must not escape the already elapsed total.
                    std::thread::sleep(
                        budget.deadline.saturating_duration_since(Instant::now())
                            + Duration::from_millis(2),
                    );
                    if failed {
                        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into())
                    } else {
                        Ok(42)
                    }
                },
                pending_signal(),
            )
            .await
            .unwrap_err();
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        if failed {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        } else {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
            assert!(error
                .downcast_ref::<tokio::time::error::Elapsed>()
                .is_some());
        }
    }
}

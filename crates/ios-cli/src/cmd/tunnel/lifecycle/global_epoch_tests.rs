use super::*;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

fn poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(
        futures_util::task::noop_waker_ref(),
    ))
}

#[tokio::test]
async fn remote_epoch_readonly_snapshot_creates_no_device_and_rejects_old_work_after_clear() {
    let lifecycle = Lifecycle::new();
    let bank = lifecycle.metadata_bytes.clone();
    let before = bank.available_permits();
    let epoch = lifecycle.global_epoch().unwrap();
    assert_eq!(lifecycle.coordinator_count().await, 0);
    assert_eq!(bank.available_permits(), before);
    lifecycle.stop("not-a-local-row").await;
    assert!(epoch.check().is_ok());
    assert_eq!(lifecycle.coordinator_count().await, 0);
    lifecycle.clear();
    let polls = AtomicUsize::new(0);
    let error = epoch
        .run(async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    let next = lifecycle.global_epoch().unwrap();
    assert!(next.run(async { Ok(()) }).await.is_ok());
    lifecycle.close();
    assert!(next.check().is_err());
    assert!(lifecycle.global_epoch().is_err());
    assert_eq!(bank.available_permits(), before);
}

#[tokio::test]
async fn remote_epoch_pending_work_drops_on_clear_close_or_lifecycle_owner_drop() {
    struct Held(Arc<AtomicUsize>);
    impl Drop for Held {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    for action in 0..3 {
        let lifecycle = Lifecycle::new();
        let epoch = lifecycle.global_epoch().unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let mut operation = Box::pin(epoch.run(async {
            let _held = Held(drops.clone());
            std::future::pending::<Result<()>>().await
        }));
        assert!(poll(operation.as_mut()).is_pending());
        match action {
            0 => lifecycle.clear(),
            1 => lifecycle.close(),
            _ => {}
        }
        drop(lifecycle);
        assert!(operation
            .await
            .unwrap_err()
            .is::<ios_core::OperationCancelled>());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert!(epoch.check().is_err());
        let calls = AtomicUsize::new(0);
        assert!(epoch
            .commit(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn remote_epoch_ready_original_error_crossing_clear_keeps_typed_pointer() {
    let lifecycle = Lifecycle::new();
    let epoch = lifecycle.global_epoch().unwrap();
    let original = anyhow::Error::new(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "original-remote-IO",
    ));
    let pointer = original.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize;
    let error = epoch
        .run(async {
            lifecycle.clear();
            Err::<(), _>(original)
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap() as *const _ as usize,
        pointer
    );
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::BrokenPipe
    );
}

#[tokio::test]
async fn remote_epoch_ready_success_crossing_clear_cannot_commit_a_retired_handoff() {
    struct Stream(Arc<AtomicUsize>);
    impl Drop for Stream {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let lifecycle = Lifecycle::new();
    let epoch = lifecycle.global_epoch().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let ready = epoch
        .run(async {
            lifecycle.clear();
            Ok(Stream(drops.clone()))
        })
        .await
        .unwrap();
    let commits = AtomicUsize::new(0);
    let error = epoch
        .commit(|| {
            commits.fetch_add(1, Ordering::SeqCst);
            Ok(ready)
        })
        .err()
        .unwrap();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(commits.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

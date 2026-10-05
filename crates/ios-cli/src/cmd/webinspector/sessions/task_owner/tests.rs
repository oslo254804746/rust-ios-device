//! Owned Tokio actor proofs; these fixtures do not represent Apple cleanup.
use super::*;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Notify;

struct Released(Arc<AtomicBool>);
impl Drop for Released {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(1), operation)
        .await
        .expect("owned watcher fixture stalled")
}

async fn pending() -> (TaskOwner, Arc<AtomicBool>, Arc<Notify>) {
    let started = Arc::new(Notify::new());
    let active = started.clone();
    let released = Arc::new(AtomicBool::new(false));
    let guard = Released(released.clone());
    let allow = Arc::new(Notify::new());
    let finish = allow.clone();
    let owner = TaskOwner::new(tokio::spawn(async move {
        let _guard = guard;
        active.notify_one();
        finish.notified().await;
    }));
    bounded(started.notified()).await;
    (owner, released, allow)
}

#[tokio::test]
async fn lifecycle_session_tasks_normal_completion_and_repeated_join_release_actor() {
    let (owner, released, allow) = pending().await;
    allow.notify_one();
    bounded(owner.join()).await.unwrap();
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(owner.join.lock().await.completed, Some(Completion::Normal));
    assert!(!owner.requested_abort.load(Ordering::SeqCst));
    bounded(owner.join()).await.unwrap();
    bounded(owner.join_if_finished()).await.unwrap().unwrap();
}

#[tokio::test]
async fn lifecycle_session_tasks_explicit_abort_joins_and_releases_pending_actor() {
    let (owner, released, _) = pending().await;
    owner.abort();
    bounded(owner.join()).await.unwrap();
    assert!(released.load(Ordering::SeqCst));
    assert!(owner.requested_abort.load(Ordering::SeqCst));
    assert_eq!(
        owner.join.lock().await.completed,
        Some(Completion::RequestedAbort)
    );
    bounded(owner.join()).await.unwrap();
}

#[tokio::test]
async fn lifecycle_session_tasks_unexpected_panic_retains_original_join_error() {
    let secret = "synthetic-watcher-panic-payload";
    let owner = TaskOwner::new(tokio::spawn(async move { panic!("{secret}") }));
    let error = bounded(owner.join()).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Unknown
    );
    assert!(error.chain().any(|cause| cause
        .downcast_ref::<tokio::task::JoinError>()
        .is_some_and(|native| native.is_panic())));
    assert!(!crate::output::error_message(&error).contains(secret));
    bounded(owner.join()).await.unwrap();
}

#[tokio::test]
async fn lifecycle_session_tasks_external_abort_retains_unexpected_native_cancellation() {
    let (owner, released, _) = pending().await;
    owner.abort.abort();
    let error = bounded(owner.join()).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ios_core::ErrorClass::Cancelled
    );
    assert!(error.chain().any(|cause| cause
        .downcast_ref::<tokio::task::JoinError>()
        .is_some_and(|native| native.is_cancelled())));
    assert!(released.load(Ordering::SeqCst));
    assert!(!owner.requested_abort.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_session_tasks_cancelled_join_retains_handle_and_can_resume() {
    let (owner, released, _) = pending().await;
    let held = owner.join.lock().await;
    {
        let joining = owner.join();
        tokio::pin!(joining);
        let pending = std::future::poll_fn(|cx| {
            assert!(joining.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        });
        bounded(pending).await;
        owner.abort();
        assert!(held.completed.is_none());
    }
    drop(held);
    bounded(owner.join()).await.unwrap();
    assert!(released.load(Ordering::SeqCst));
    bounded(owner.join()).await.unwrap();
}

#[tokio::test]
async fn lifecycle_session_tasks_finished_reap_never_waits_for_active_actor() {
    let (owner, released, allow) = pending().await;
    assert!(bounded(owner.join_if_finished()).await.is_none());
    assert!(!released.load(Ordering::SeqCst));
    allow.notify_one();
    bounded(async {
        loop {
            if let Some(result) = owner.join_if_finished().await {
                result.unwrap();
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(released.load(Ordering::SeqCst));
    assert_eq!(owner.join.lock().await.completed, Some(Completion::Normal));
}

#[tokio::test]
async fn lifecycle_session_tasks_drop_fallback_aborts_without_async_join_claim() {
    let (owner, released, _) = pending().await;
    drop(owner);
    bounded(async {
        while !released.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await;
}

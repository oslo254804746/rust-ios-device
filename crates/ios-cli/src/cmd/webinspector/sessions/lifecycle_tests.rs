//! Actual admitted registry operations with owned host resources, not Apple IO.
use super::*;
use crate::access::{
    test_support::{grant, PolicyFixture, DEVICE, TOKEN},
    AccessControl, Admission, HttpBoundaryArgs, Permission,
};
use axum::{routing::post, Extension, Router};
use std::{
    future::Future,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
    time::Duration,
};
use task_owner::Completion;
use tokio::sync::{oneshot, Notify};

#[derive(Default)]
struct Trace {
    next: AtomicUsize,
    drops: AtomicUsize,
    completed: AtomicUsize,
    started: Notify,
}
struct Resource {
    generation: usize,
    trace: Arc<Trace>,
}
impl Drop for Resource {
    fn drop(&mut self) {
        self.trace.drops.fetch_add(1, Ordering::SeqCst);
    }
}

struct Fixture {
    sessions: Sessions<Resource>,
    access: AccessControl,
    principal: Principal,
    _policy: PolicyFixture,
    trace: Arc<Trace>,
    base: String,
    shutdown: Option<oneshot::Sender<()>>,
    listener: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(listener) = &self.listener {
            listener.abort();
        }
    }
}
async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .expect("owned registry fixture stalled")
}
impl Fixture {
    async fn post(&self, id: &str) {
        let response = bounded(
            reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .post(format!("{}/session/{id}", self.base))
                .bearer_auth(TOKEN)
                .send(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
    async fn owner(&self, id: &str) -> Arc<TaskOwner> {
        self.sessions.entries.lock().await.entries[id]
            .revocation
            .clone()
    }
    async fn reload(&self) {
        self.access.reload(&self.principal).await.unwrap();
    }
    async fn stop_listener(&mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        bounded(self.listener.take().unwrap()).await.unwrap();
    }
}

async fn opened(delay: Option<Duration>) -> Fixture {
    let sessions = Sessions::default();
    let trace = Arc::new(Trace::default());
    let policy = PolicyFixture::new(vec![grant(
        TOKEN,
        vec![Permission::Debug, Permission::ManageServer],
    )]);
    let access = AccessControl::from_file(policy.path.clone()).await.unwrap();
    let principal = Arc::new(StdMutex::new(None));
    let capture = principal.clone();
    let insert = sessions.clone();
    let created = trace.clone();
    let router = Router::new()
        .route(
            "/session/:id",
            post(
                move |Path(id): Path<String>, Extension(owner): Extension<Principal>| {
                    let sessions = insert.clone();
                    let trace = created.clone();
                    let principal = capture.clone();
                    async move {
                        *principal.lock().unwrap() = Some(owner.clone());
                        let generation = trace.next.fetch_add(1, Ordering::SeqCst);
                        let cleanup = trace.clone();
                        sessions
                            .insert(
                                id,
                                Resource { generation, trace },
                                owner,
                                move |resource| async move {
                                    let _guard = resource.lock().await;
                                    cleanup.started.notify_one();
                                    match delay {
                                        Some(delay) => tokio::time::sleep(delay).await,
                                        None => std::future::pending::<()>().await,
                                    }
                                    cleanup.completed.fetch_add(1, Ordering::SeqCst);
                                },
                            )
                            .await;
                        StatusCode::OK
                    }
                },
            ),
        )
        .layer(axum::middleware::from_fn_with_state(
            Admission::debug(
                access.clone(),
                crate::http_diagnostics::ErrorShape::WebDriver,
                DEVICE.into(),
            ),
            crate::access::admission,
        ));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let router = HttpBoundaryArgs::default()
        .validate()
        .unwrap()
        .for_listener(address)
        .protect(router, crate::http_diagnostics::ErrorShape::WebDriver);
    let (shutdown, stopped) = oneshot::channel();
    let listener = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let base = format!("http://{address}");
    let response = bounded(
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("{base}/session/owned-session"))
            .bearer_auth(TOKEN)
            .send(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let principal = principal.lock().unwrap().take().unwrap();
    Fixture {
        sessions,
        access,
        principal,
        _policy: policy,
        trace,
        base,
        shutdown: Some(shutdown),
        listener: Some(listener),
    }
}

#[tokio::test]
async fn lifecycle_session_store_explicit_remove_joins_before_returning_owned_resource() {
    let mut fixture = opened(None).await;
    let owner = fixture.owner("owned-session").await;
    let resource = bounded(fixture.sessions.remove("owned-session"))
        .await
        .unwrap();
    assert_eq!(owner.completion().await, Some(Completion::RequestedAbort));
    assert!(fixture.sessions.entries.lock().await.retiring.is_empty());
    assert!(fixture.sessions.get("owned-session").await.is_none());
    assert_eq!(fixture.trace.drops.load(Ordering::SeqCst), 0);
    assert_eq!(resource.lock().await.generation, 0);
    drop(resource);
    assert_eq!(fixture.trace.drops.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.trace.completed.load(Ordering::SeqCst), 0);
    fixture.stop_listener().await;
}

#[tokio::test]
async fn lifecycle_session_store_cancelled_remove_keeps_aborted_owner_and_unrelated_admission() {
    let mut fixture = opened(None).await;
    let owner = fixture.owner("owned-session").await;
    let held = owner.prevent_join().await;
    {
        let removing = fixture.sessions.remove("owned-session");
        tokio::pin!(removing);
        bounded(std::future::poll_fn(|cx| {
            assert!(removing.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        }))
        .await;
        {
            let store = fixture.sessions.entries.lock().await;
            assert!(!store.entries.contains_key("owned-session"));
            assert_eq!(store.retiring.len(), 1);
            assert!(Arc::ptr_eq(&store.retiring[0], &owner));
        }
        // The join is deliberately held, yet the real listener can admit a
        // different resource. No global map lock spans the task wait.
        fixture.post("other-session").await;
    }
    drop(held);
    bounded(async {
        loop {
            assert!(fixture.sessions.get("owned-session").await.is_none());
            if owner.completion().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(owner.completion().await, Some(Completion::RequestedAbort));
    assert!(fixture.sessions.entries.lock().await.retiring.is_empty());
    let other = fixture.sessions.remove("other-session").await.unwrap();
    assert_eq!(other.lock().await.generation, 1);
    drop(other);
    assert_eq!(fixture.trace.drops.load(Ordering::SeqCst), 2);
    fixture.stop_listener().await;
}

async fn revoked(delay: Option<Duration>) {
    let mut fixture = opened(delay).await;
    let owner = fixture.owner("owned-session").await;
    let began = tokio::time::Instant::now();
    fixture.reload().await;
    bounded(fixture.trace.started.notified()).await;
    {
        let store = fixture.sessions.entries.lock().await;
        assert!(store.entries.is_empty());
        assert_eq!(store.retiring.len(), 1);
        assert!(Arc::ptr_eq(&store.retiring[0], &owner));
    }
    bounded(async {
        loop {
            assert!(fixture.sessions.get("owned-session").await.is_none());
            if owner.completion().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(
        owner.completion().await,
        Some(Completion::Normal),
        "actor must never abort/join itself"
    );
    assert!(fixture.sessions.entries.lock().await.retiring.is_empty());
    assert_eq!(fixture.trace.drops.load(Ordering::SeqCst), 1);
    if let Some(delay) = delay {
        assert!(began.elapsed() >= delay);
        assert_eq!(fixture.trace.completed.load(Ordering::SeqCst), 1);
    } else {
        assert!(began.elapsed() >= super::super::SESSION_CLEANUP_MAX);
        assert_eq!(fixture.trace.completed.load(Ordering::SeqCst), 0);
    }
    fixture.stop_listener().await;
}

#[tokio::test]
async fn lifecycle_session_store_reload_cleanup_finishes_then_registry_reaps_normal_join() {
    revoked(Some(Duration::from_millis(80))).await;
}

#[tokio::test]
async fn lifecycle_session_store_reload_real_default_expiry_keeps_normal_actor_join() {
    revoked(None).await;
}

#[tokio::test]
async fn lifecycle_session_store_replacement_joins_old_watcher_and_keeps_new_resource() {
    let mut fixture = opened(None).await;
    let old = fixture.owner("owned-session").await;
    fixture.post("owned-session").await;
    assert_eq!(old.completion().await, Some(Completion::RequestedAbort));
    assert_eq!(fixture.trace.drops.load(Ordering::SeqCst), 1);
    assert!(fixture.sessions.entries.lock().await.retiring.is_empty());
    let new = fixture.sessions.remove("owned-session").await.unwrap();
    assert_eq!(new.lock().await.generation, 1);
    drop(new);
    assert_eq!(fixture.trace.drops.load(Ordering::SeqCst), 2);
    fixture.stop_listener().await;
}

#[tokio::test]
async fn lifecycle_session_store_drop_during_cleanup_has_no_strong_store_cycle() {
    let mut fixture = opened(None).await;
    let weak = Arc::downgrade(&fixture.sessions.entries);
    let trace = fixture.trace.clone();
    fixture.reload().await;
    bounded(trace.started.notified()).await;
    let actor = fixture.sessions.entries.lock().await.retiring[0].abort_handle();
    fixture.stop_listener().await;
    assert!(
        !actor.is_finished(),
        "the actual cleanup actor is still pending at store Drop"
    );
    assert_eq!(trace.drops.load(Ordering::SeqCst), 0);
    drop(fixture);
    assert!(
        weak.upgrade().is_none(),
        "cleanup cannot keep the store alive"
    );
    bounded(async {
        while trace.drops.load(Ordering::SeqCst) == 0 || !actor.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(trace.drops.load(Ordering::SeqCst), 1);
    assert_eq!(trace.completed.load(Ordering::SeqCst), 0);
    // Drop's abort/release is observed; no asynchronous product join claimed.
}

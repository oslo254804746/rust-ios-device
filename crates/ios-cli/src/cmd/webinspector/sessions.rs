//! Host authorization leases for stored automation resources.
//!
//! This owns cancellation and bounded cleanup, not the Apple protocol backend.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use axum::extract::{FromRequestParts, Path, Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use tokio::sync::Mutex;

use super::tasks::control::Budget;
use crate::access::Principal;
use anyhow::Result;

mod task_owner;
use task_owner::TaskOwner;

struct Entry<T> {
    resource: Arc<Mutex<T>>,
    principal: Principal,
    revocation: Arc<TaskOwner>,
}

struct Store<T> {
    entries: HashMap<String, Entry<T>>,
    retiring: Vec<Arc<TaskOwner>>,
}

pub(super) struct Sessions<T> {
    entries: Arc<Mutex<Store<T>>>,
}

impl<T> Clone for Sessions<T> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<T> Default for Sessions<T> {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(Store {
                entries: HashMap::new(),
                retiring: Vec::new(),
            })),
        }
    }
}

impl<T: Send + 'static> Sessions<T> {
    #[cfg(test)]
    pub(super) async fn insert<F, C>(
        &self,
        id: String,
        resource: T,
        principal: Principal,
        cleanup: C,
    ) where
        F: Future<Output = ()> + Send + 'static,
        C: FnOnce(Arc<Mutex<T>>) -> F + Send + 'static,
    {
        self.insert_with_budget(
            id,
            resource,
            principal,
            move |resource, budget| async move {
                budget
                    .drive(
                        async {
                            cleanup(resource).await;
                            Ok(())
                        },
                        std::future::pending::<std::io::Result<()>>(),
                    )
                    .await
            },
        )
        .await;
    }

    pub(super) async fn insert_with_budget<F, C>(
        &self,
        id: String,
        resource: T,
        principal: Principal,
        cleanup: C,
    ) where
        F: Future<Output = Result<()>> + Send + 'static,
        C: FnOnce(Arc<Mutex<T>>, Budget) -> F + Send + 'static,
    {
        self.reap_finished().await;
        let weak = Arc::downgrade(&self.entries);
        let lease = principal.clone();
        let resource = Arc::new(Mutex::new(resource));
        let watched_resource = Arc::downgrade(&resource);
        // Keep publication serialized with the revocation task. A lease can
        // already be revoked here, but cannot leave an orphan entry behind.
        let mut store = self.entries.lock().await;
        let task_id = id.clone();
        let revocation = tokio::spawn(async move {
            lease.revoked().await;
            let resource = match weak.upgrade() {
                Some(entries) => {
                    let mut store = entries.lock().await;
                    let same = store.entries.get(&task_id).is_some_and(|entry| {
                        watched_resource.ptr_eq(&Arc::downgrade(&entry.resource))
                    });
                    if same {
                        store.entries.remove(&task_id).map(|entry| {
                            // The store keeps this actor's handle; the actor must
                            // neither abort nor join itself. No strong store owner
                            // or global lock survives into protocol cleanup.
                            store.retiring.push(entry.revocation);
                            entry.resource
                        })
                    } else {
                        None
                    }
                }
                None => None,
            };
            if let Some(resource) = resource {
                // Remove admission first. Cleanup cannot keep an expired
                // resource addressable or hold its map lock across I/O.
                let budget =
                    Budget::new(super::SESSION_CLEANUP_MAX, "WebDriver revocation cleanup")?;
                // The callback stages own this same total and preserve known
                // primary/secondary errors. A competing outer timer could
                // discard them while the forwarding close remains pending.
                cleanup(resource, budget).await
            } else {
                Ok(())
            }
        });
        let replaced = store.entries.insert(
            id,
            Entry {
                resource,
                principal,
                revocation: Arc::new(TaskOwner::new(revocation)),
            },
        );
        let replaced = replaced.map(|entry| {
            entry.revocation.abort();
            store.retiring.push(entry.revocation.clone());
            entry.revocation
        });
        drop(store);
        if let Some(owner) = replaced {
            self.join_retired(owner).await;
        }
    }

    pub(super) async fn get(&self, id: &str) -> Option<Arc<Mutex<T>>> {
        self.reap_finished().await;
        self.entries
            .lock()
            .await
            .entries
            .get(id)
            .filter(|entry| entry.principal.current())
            .map(|entry| entry.resource.clone())
    }

    pub(super) async fn remove(&self, id: &str) -> Option<Arc<Mutex<T>>> {
        self.reap_finished().await;
        let entry = {
            let mut store = self.entries.lock().await;
            store.entries.remove(id).inspect(|entry| {
                entry.revocation.abort();
                store.retiring.push(entry.revocation.clone());
            })
        };
        if let Some(entry) = entry {
            self.join_retired(entry.revocation).await;
            Some(entry.resource)
        } else {
            None
        }
    }

    async fn principal(&self, id: &str) -> Option<Principal> {
        self.reap_finished().await;
        self.entries
            .lock()
            .await
            .entries
            .get(id)
            .map(|entry| entry.principal.clone())
    }

    async fn reap_finished(&self) {
        let owners = self.entries.lock().await.retiring.clone();
        for owner in owners {
            if let Some(result) = owner.join_if_finished().await {
                task_owner::report(result);
                self.forget_joined(&owner).await;
            }
        }
    }

    async fn join_retired(&self, owner: Arc<TaskOwner>) {
        task_owner::report(owner.join().await);
        self.forget_joined(&owner).await;
    }

    async fn forget_joined(&self, owner: &Arc<TaskOwner>) {
        self.entries
            .lock()
            .await
            .retiring
            .retain(|other| !Arc::ptr_eq(owner, other));
    }

    #[cfg(test)]
    pub(super) async fn observe(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<()>> + Send + 'static {
        let owner = self.entries.lock().await.entries[id].revocation.clone();
        async move { owner.join().await }
    }
}

#[cfg(test)]
mod lifecycle_tests;

/// Guard operations with the stored session's original lease as well as the
/// request's debug grant. A second valid debug credential cannot revive a
/// session whose creator expired. Use Axum's decoded route parameter, so an
/// escaped session ID cannot bypass this guard.
pub(super) async fn admission<T: Send + 'static>(
    State(sessions): State<Sessions<T>>,
    request: Request,
    next: Next,
) -> Response {
    let head = request.method() == Method::HEAD;
    let (mut parts, body) = request.into_parts();
    let parameters = Path::<HashMap<String, String>>::from_request_parts(&mut parts, &())
        .await
        .ok();
    let principal =
        match parameters.and_then(|Path(mut parameters)| parameters.remove("session_id")) {
            Some(id) => sessions.principal(&id).await,
            None => None,
        };
    let request = Request::from_parts(parts, body);
    match principal {
        Some(principal) => principal.run(next.run(request)).await.unwrap_or_else(|_| {
            crate::access::denied_for(
                crate::http_diagnostics::ErrorShape::WebDriver,
                StatusCode::FORBIDDEN,
                head,
            )
        }),
        None => next.run(request).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::test_support::{grant, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};
    use crate::access::{AccessControl, Admission, Permission};
    use axum::routing::{get, post};
    use axum::{Extension, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;

    struct Resource {
        drops: Arc<AtomicUsize>,
        dropped: Arc<Notify>,
    }

    impl Drop for Resource {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            self.dropped.notify_one();
        }
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn stored_owner_expiry_or_reload_cancels_other_credentials_and_bounds_cleanup() {
        for expiry in [true, false] {
            let owner = grant(TOKEN, vec![Permission::Debug, Permission::ManageServer]);
            let fixture = PolicyFixture::new(vec![
                if expiry {
                    owner.expires_after(3)
                } else {
                    owner
                },
                grant(
                    REPLACEMENT,
                    vec![Permission::Debug, Permission::ManageServer],
                ),
            ]);
            let access = AccessControl::from_file(fixture.path.clone())
                .await
                .unwrap();
            let sessions = Sessions::<Resource>::default();
            let drops = Arc::new(AtomicUsize::new(0));
            let dropped = Arc::new(Notify::new());
            let cleanup_started = Arc::new(Notify::new());
            let request_started = Arc::new(Notify::new());
            let insert = sessions.clone();
            let resource_drops = drops.clone();
            let resource_dropped = dropped.clone();
            let cleanup = cleanup_started.clone();
            let get_session = sessions.clone();
            let request = request_started.clone();
            let reload = access.clone();
            let router = Router::new()
                .route(
                    "/session",
                    post(
                        move |Extension(principal): Extension<Principal>| async move {
                            insert
                                .insert(
                                    "synthetic-session".into(),
                                    Resource {
                                        drops: resource_drops,
                                        dropped: resource_dropped,
                                    },
                                    principal,
                                    move |resource| async move {
                                        let _guard = resource.lock().await;
                                        cleanup.notify_one();
                                        // A deliberately stalled host cleanup, not an Apple
                                        // backend. Production uses the same timeout/ownership.
                                        std::future::pending::<()>().await;
                                    },
                                )
                                .await;
                            StatusCode::OK
                        },
                    ),
                )
                .route(
                    "/session/:session_id/url",
                    get(move |Path(id): Path<String>| async move {
                        let resource = get_session.get(&id).await.unwrap();
                        let _guard = resource.lock().await;
                        request.notify_one();
                        std::future::pending::<StatusCode>().await
                    }),
                )
                .route(
                    "/auth/reload",
                    post(
                        move |Extension(principal): Extension<Principal>| async move {
                            reload.reload(&principal).await.unwrap();
                            StatusCode::OK
                        },
                    ),
                )
                .layer(axum::middleware::from_fn_with_state(
                    sessions.clone(),
                    admission::<Resource>,
                ))
                .layer(axum::middleware::from_fn_with_state(
                    Admission::debug(
                        access,
                        crate::http_diagnostics::ErrorShape::WebDriver,
                        DEVICE.into(),
                    ),
                    crate::access::admission,
                ));
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            assert_eq!(
                client()
                    .post(format!("{base}/session"))
                    .bearer_auth(TOKEN)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
            let url = format!("{base}/session/synthetic%2dsession/url");
            let operation = tokio::spawn(async move {
                client()
                    .get(url)
                    .bearer_auth(REPLACEMENT)
                    .send()
                    .await
                    .unwrap()
            });
            tokio::time::timeout(Duration::from_secs(1), request_started.notified())
                .await
                .unwrap();
            if !expiry {
                fixture.replace(vec![grant(
                    REPLACEMENT,
                    vec![Permission::Debug, Permission::ManageServer],
                )]);
                assert_eq!(
                    client()
                        .post(format!("{base}/auth/reload"))
                        .bearer_auth(TOKEN)
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::OK
                );
            }
            let response = tokio::time::timeout(Duration::from_secs(4), operation)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let original = response.text().await.unwrap();
            assert!(original.contains("debug scope denied"));
            for secret in [TOKEN, REPLACEMENT, DEVICE, "synthetic-session"] {
                assert!(!original.contains(secret));
            }
            tokio::time::timeout(Duration::from_secs(1), cleanup_started.notified())
                .await
                .unwrap();
            assert!(sessions.get("synthetic-session").await.is_none());
            tokio::time::timeout(Duration::from_secs(1), dropped.notified())
                .await
                .unwrap();
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            server.abort();
            let _ = server.await;
        }
    }

    #[tokio::test]
    async fn dropping_a_store_cancels_its_watchers_and_releases_resources() {
        let sessions = Sessions::<Resource>::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(Notify::new());
        let access = AccessControl::trusted_test_local();
        let store = sessions.clone();
        let resource_drops = drops.clone();
        let resource_dropped = dropped.clone();
        let router = Router::new()
            .route(
                "/session",
                post(
                    move |Extension(principal): Extension<Principal>| async move {
                        store
                            .insert(
                                "synthetic-session".into(),
                                Resource {
                                    drops: resource_drops,
                                    dropped: resource_dropped,
                                },
                                principal,
                                |_| async { panic!("local trust must not expire") },
                            )
                            .await;
                        StatusCode::OK
                    },
                ),
            )
            .layer(axum::middleware::from_fn_with_state(
                Admission::debug(
                    access,
                    crate::http_diagnostics::ErrorShape::WebDriver,
                    DEVICE.into(),
                ),
                crate::access::admission,
            ));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        assert_eq!(
            client()
                .post(format!("{base}/session"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        server.abort();
        let _ = server.await;
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(sessions);
        tokio::time::timeout(Duration::from_secs(1), dropped.notified())
            .await
            .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

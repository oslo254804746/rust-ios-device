//! Request admission and immutable, atomically replaceable policy snapshots.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::Json;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::{watch, Mutex};

use super::{unix_seconds, ClientToken, Permission, Policy};

#[derive(clap::Args)]
pub(crate) struct ServerAccessArgs {
    /// Private version-1 scoped policy; required unless local trust is explicit
    #[arg(long, conflicts_with = "local_unauthenticated")]
    auth_config: Option<PathBuf>,
    /// Trust all local processes; accepted only for a loopback listener
    #[arg(long, conflicts_with = "auth_config")]
    local_unauthenticated: bool,
}

pub(crate) fn loopback_address(host: &str) -> Result<IpAddr> {
    let address = if host.eq_ignore_ascii_case("localhost") {
        IpAddr::from([127, 0, 0, 1])
    } else {
        host.trim_matches(['[', ']'])
            .parse::<IpAddr>()
            .context("local service host must be a literal loopback IP or localhost")?
    };
    anyhow::ensure!(
        address.is_loopback(),
        "plaintext local service access requires loopback; use a protected host transport"
    );
    Ok(address)
}

impl ServerAccessArgs {
    /// Validate and read credentials before opening a listener or scanning devices.
    pub(crate) async fn load(self, host: &str) -> Result<(IpAddr, AccessControl)> {
        let address = loopback_address(host)?;
        let access = match self.auth_config {
            Some(path) => AccessControl::from_file(path).await?,
            None if self.local_unauthenticated => AccessControl::local(),
            None => {
                anyhow::bail!("manager requires --auth-config or explicit --local-unauthenticated")
            }
        };
        Ok((address, access))
    }
}

#[derive(clap::Args)]
pub(crate) struct ClientAccessArgs {
    /// Read the bearer credential from a private file, never from argv or env
    #[arg(long)]
    token_file: Option<PathBuf>,
}

impl ClientAccessArgs {
    pub(crate) async fn header(self, url: &reqwest::Url) -> Result<Option<HeaderValue>> {
        anyhow::ensure!(
            url.scheme() == "http"
                && url.username().is_empty()
                && url.password().is_none()
                && url
                    .host_str()
                    .is_some_and(|host| loopback_address(host).is_ok()),
            "plaintext manager client requires a loopback URL without userinfo"
        );
        let Some(path) = self.token_file else {
            return Ok(None);
        };
        let token = tokio::task::spawn_blocking(move || ClientToken::from_file(&path))
            .await
            .context("client credential reader failed")??;
        let mut value = zeroize::Zeroizing::new(String::from("Bearer "));
        value.push_str(&token.0);
        let mut header =
            HeaderValue::from_str(&value).context("invalid client credential header")?;
        header.set_sensitive(true);
        Ok(Some(header))
    }
}

struct Snapshot {
    revision: u64,
    policy: Policy,
}

struct ConfiguredAccess {
    path: PathBuf,
    snapshot: watch::Sender<Arc<Snapshot>>,
    reload: Mutex<()>,
}

#[derive(Clone)]
pub(crate) struct AccessControl(Option<Arc<ConfiguredAccess>>);

impl std::fmt::Debug for AccessControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessControl")
            .field("authenticated", &self.0.is_some())
            .finish_non_exhaustive()
    }
}

impl AccessControl {
    fn local() -> Self {
        Self(None)
    }

    #[cfg(test)]
    pub(crate) fn trusted_test_local() -> Self {
        Self::local()
    }

    pub(crate) async fn from_file(path: PathBuf) -> Result<Self> {
        let read_path = path.clone();
        let policy = tokio::task::spawn_blocking(move || Policy::load(&read_path, unix_seconds()?))
            .await
            .context("manager policy reader failed")??;
        let (snapshot, _) = watch::channel(Arc::new(Snapshot {
            revision: 0,
            policy,
        }));
        Ok(Self(Some(Arc::new(ConfiguredAccess {
            path,
            snapshot,
            reload: Mutex::new(()),
        }))))
    }

    pub(crate) async fn reload(&self, principal: &Principal) -> Result<()> {
        let configured = self
            .0
            .as_ref()
            .context("local trust mode has no policy to reload")?;
        let _guard = configured.reload.lock().await;
        anyhow::ensure!(
            principal.is_for(configured) && principal.allows(Permission::ManageServer, None, None),
            "manager policy reload authorization revoked"
        );
        let path = configured.path.clone();
        let policy = tokio::task::spawn_blocking(move || Policy::load(&path, unix_seconds()?))
            .await
            .context("manager policy reader failed")??;
        anyhow::ensure!(
            principal.is_for(configured) && principal.allows(Permission::ManageServer, None, None),
            "manager policy reload authorization revoked"
        );
        let revision = configured
            .snapshot
            .borrow()
            .revision
            .checked_add(1)
            .context("manager policy revision exhausted")?;
        configured
            .snapshot
            .send_replace(Arc::new(Snapshot { revision, policy }));
        // Every successful replacement revokes admitted streams, including
        // unchanged credentials. Failed reads leave the old snapshot untouched.
        Ok(())
    }

    fn authenticate(&self, headers: &HeaderMap) -> Option<Principal> {
        let Some(configured) = &self.0 else {
            return Some(Principal(None));
        };
        let mut values = headers.get_all(header::AUTHORIZATION).iter();
        let value = values.next()?.as_bytes();
        if values.next().is_some()
            || value.len() != 71
            || !value[..7].eq_ignore_ascii_case(b"Bearer ")
            || !value[7..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return None;
        }
        let now = unix_seconds().ok()?;
        let updates = configured.snapshot.subscribe();
        let snapshot = updates.borrow().clone();
        let index = snapshot
            .policy
            .authenticate_digest(&Sha256::digest(&value[7..]).into(), now)?;
        let lifetime = Duration::from_secs(snapshot.policy.grants[index].scope.expires_unix)
            .checked_sub(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?,
            )?;
        Some(Principal(Some(AuthenticatedPrincipal {
            snapshot,
            index,
            updates,
            deadline: tokio::time::Instant::now() + lifetime,
        })))
    }

    /// The automatic monitor may only establish devices covered by an active
    /// device grant. Server administration alone grants no device access.
    pub(crate) fn permits_device(&self, device: &str) -> bool {
        let Some(configured) = &self.0 else {
            return true;
        };
        let Ok(now) = unix_seconds() else {
            return false;
        };
        configured
            .snapshot
            .borrow()
            .policy
            .grants
            .iter()
            .any(|grant| {
                grant.scope.expires_unix > now
                    && grant
                        .scope
                        .permissions
                        .iter()
                        .any(|role| matches!(role, Permission::Connect | Permission::ManageTunnels))
                    && (grant.scope.all_devices
                        || grant.scope.devices.iter().any(|id| id == device))
            })
    }

    pub(crate) fn permits_any_device(&self) -> bool {
        let Some(configured) = &self.0 else {
            return true;
        };
        let Ok(now) = unix_seconds() else {
            return false;
        };
        configured
            .snapshot
            .borrow()
            .policy
            .grants
            .iter()
            .any(|grant| {
                grant.scope.expires_unix > now
                    && grant
                        .scope
                        .permissions
                        .iter()
                        .any(|role| matches!(role, Permission::Connect | Permission::ManageTunnels))
            })
    }
}

#[derive(Clone)]
struct AuthenticatedPrincipal {
    snapshot: Arc<Snapshot>,
    index: usize,
    updates: watch::Receiver<Arc<Snapshot>>,
    deadline: tokio::time::Instant,
}

#[derive(Clone)]
pub(crate) struct Principal(Option<AuthenticatedPrincipal>);

pub(crate) struct ScopeDenied;

impl From<ScopeDenied> for Response {
    fn from(_: ScopeDenied) -> Self {
        denied(StatusCode::FORBIDDEN, false)
    }
}

impl std::fmt::Debug for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Principal").finish_non_exhaustive()
    }
}

impl Principal {
    fn is_for(&self, configured: &ConfiguredAccess) -> bool {
        self.0.as_ref().is_some_and(|principal| {
            Arc::ptr_eq(&configured.snapshot.borrow(), &principal.snapshot)
        })
    }

    fn current(&self) -> bool {
        self.0.as_ref().map_or(true, |principal| {
            principal.updates.borrow().revision == principal.snapshot.revision
                && tokio::time::Instant::now() < principal.deadline
                && unix_seconds().is_ok_and(|now| {
                    now < principal.snapshot.policy.grants[principal.index]
                        .scope
                        .expires_unix
                })
        })
    }

    pub(crate) fn has_permission(&self, permission: Permission) -> bool {
        self.current()
            && self.0.as_ref().map_or(true, |principal| {
                principal.snapshot.policy.grants[principal.index]
                    .scope
                    .permissions
                    .contains(&permission)
            })
    }

    pub(crate) fn allows(
        &self,
        permission: Permission,
        device: Option<&str>,
        port: Option<u16>,
    ) -> bool {
        self.current()
            && self.0.as_ref().map_or(true, |principal| {
                principal.snapshot.policy.grants[principal.index].allows(permission, device, port)
            })
    }

    pub(crate) fn require(
        &self,
        permission: Permission,
        device: Option<&str>,
        port: Option<u16>,
    ) -> Result<(), ScopeDenied> {
        if self.allows(permission, device, port) {
            Ok(())
        } else {
            Err(ScopeDenied)
        }
    }

    pub(crate) fn require_permission(&self, permission: Permission) -> Result<(), ScopeDenied> {
        if self.has_permission(permission) {
            Ok(())
        } else {
            Err(ScopeDenied)
        }
    }

    pub(crate) async fn run<T>(
        &self,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, ScopeDenied> {
        tokio::select! {
            biased;
            _ = self.clone().revoked() => Err(ScopeDenied),
            output = future => Ok(output),
        }
    }

    /// A revision check before waiting closes the authenticate/subscribe race.
    /// Clock changes are also checked once a second; rollback cannot extend the
    /// monotonic deadline admitted at authentication.
    pub(crate) async fn revoked(mut self) {
        let Some(principal) = &mut self.0 else {
            std::future::pending::<()>().await;
            return;
        };
        loop {
            if principal.updates.borrow().revision != principal.snapshot.revision
                || tokio::time::Instant::now() >= principal.deadline
                || unix_seconds().map_or(true, |now| {
                    now >= principal.snapshot.policy.grants[principal.index]
                        .scope
                        .expires_unix
                })
            {
                return;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(principal.deadline) => return,
                changed = principal.updates.changed() => {
                    if changed.is_err() { return; }
                }
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
        }
    }
}

fn denied(status: StatusCode, head: bool) -> Response {
    let message = if status == StatusCode::UNAUTHORIZED {
        "manager authentication required"
    } else {
        "manager scope denied"
    };
    let mut response =
        crate::http_diagnostics::reviewed((status, Json(json!({"message": message}))));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if status == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"ios-manager\""),
        );
    }
    if head {
        *response.body_mut() = axum::body::Body::empty();
    }
    response
}

pub(crate) async fn admission(
    State(access): State<AccessControl>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(principal) = access.authenticate(request.headers()) else {
        return denied(StatusCode::UNAUTHORIZED, request.method() == Method::HEAD);
    };
    request.extensions_mut().insert(principal);
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::test_support::{grant, PolicyFixture, DEVICE, REPLACEMENT, TOKEN};

    fn headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    #[tokio::test]
    async fn startup_requires_explicit_access_and_loopback_before_any_listener() {
        let unspecified = ServerAccessArgs {
            auth_config: None,
            local_unauthenticated: false,
        };
        assert!(unspecified.load("127.0.0.1").await.is_err());
        for host in [
            "0.0.0.0",
            "::",
            "192.0.2.1",
            "example.invalid",
            "127.0.0.1@evil.invalid",
        ] {
            let local = ServerAccessArgs {
                auth_config: None,
                local_unauthenticated: true,
            };
            assert!(local.load(host).await.is_err());
        }
        let local = ServerAccessArgs {
            auth_config: None,
            local_unauthenticated: true,
        };
        assert!(local.load("localhost").await.unwrap().0.is_loopback());
        let missing = ServerAccessArgs {
            auth_config: Some(PathBuf::from("nonexistent-synthetic-policy")),
            local_unauthenticated: false,
        };
        assert!(missing.load("::1").await.is_err());
    }

    #[tokio::test]
    async fn admission_rejects_duplicate_expired_and_malformed_headers_without_debug_secrets() {
        let fixture = PolicyFixture::new(vec![
            grant(TOKEN, vec![Permission::List, Permission::Connect]),
            grant(REPLACEMENT, vec![Permission::List]).expires_at(unix_seconds().unwrap()),
        ]);
        let access = AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap();
        assert!(access.authenticate(&HeaderMap::new()).is_none());
        assert!(access.authenticate(&headers(REPLACEMENT)).is_none());
        for text in [
            "Basic synthetic-credential",
            "Bearer short",
            &format!("Bearer {}", "A".repeat(64)),
        ] {
            let mut input = HeaderMap::new();
            input.insert(header::AUTHORIZATION, text.parse().unwrap());
            assert!(access.authenticate(&input).is_none());
        }
        let mut duplicate = headers(TOKEN);
        duplicate.append(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        assert!(access.authenticate(&duplicate).is_none());
        let principal = access.authenticate(&headers(TOKEN)).unwrap();
        assert!(principal.allows(Permission::Connect, Some(DEVICE), Some(12345)));
        assert!(!principal.allows(Permission::Connect, Some(DEVICE), Some(22)));
        assert!(!principal.allows(Permission::List, Some("synthetic-other-device"), None));
        assert!(!principal.has_permission(Permission::ManageServer));
        let diagnostic = format!("{access:?} {principal:#?}");
        for secret in [TOKEN, DEVICE, fixture.path.to_str().unwrap()] {
            assert!(!diagnostic.contains(secret));
        }
        assert!(access.permits_device(DEVICE));
        assert!(!access.permits_device("synthetic-other-device"));
    }

    #[tokio::test]
    async fn failed_reload_preserves_policy_and_success_revokes_even_unpolled_sessions() {
        let fixture = PolicyFixture::new(vec![grant(
            TOKEN,
            vec![Permission::List, Permission::ManageServer],
        )]);
        let access = AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap();
        let principal = access.authenticate(&headers(TOKEN)).unwrap();
        assert!(!access.permits_any_device());
        assert!(!access.permits_device(DEVICE));
        fixture.malformed();
        assert!(access.reload(&principal).await.is_err());
        assert!(principal.allows(Permission::List, Some(DEVICE), None));
        assert!(access.authenticate(&headers(TOKEN)).is_some());
        fixture.replace(vec![grant(REPLACEMENT, vec![Permission::ManageServer])]);
        access.reload(&principal).await.unwrap();
        assert!(!principal.has_permission(Permission::List));
        assert!(access.reload(&principal).await.is_err());
        tokio::time::timeout(Duration::from_secs(1), principal.revoked())
            .await
            .unwrap();
        assert!(access.authenticate(&headers(TOKEN)).is_none());
        assert!(access.authenticate(&headers(REPLACEMENT)).is_some());
        assert!(!access.permits_any_device());
    }

    #[tokio::test]
    async fn credentials_expire_on_http_and_existing_session_deadlines() {
        let fixture = PolicyFixture::new(vec![
            grant(TOKEN, vec![Permission::Connect]).expires_at(unix_seconds().unwrap() + 3)
        ]);
        let access = AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap();
        let principal = access.authenticate(&headers(TOKEN)).unwrap();
        tokio::time::timeout(Duration::from_secs(4), principal.clone().revoked())
            .await
            .unwrap();
        assert!(!principal.allows(Permission::Connect, Some(DEVICE), Some(12345)));
        assert!(access.authenticate(&headers(TOKEN)).is_none());
    }

    #[tokio::test]
    async fn concurrent_reload_rejects_stale_and_foreign_administrators() {
        let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::ManageServer])]);
        let access = AccessControl::from_file(fixture.path.clone())
            .await
            .unwrap();
        let administrator = access.authenticate(&headers(TOKEN)).unwrap();
        let foreign_fixture =
            PolicyFixture::new(vec![grant(TOKEN, vec![Permission::ManageServer])]);
        let foreign = AccessControl::from_file(foreign_fixture.path.clone())
            .await
            .unwrap();
        assert!(access
            .reload(&foreign.authenticate(&headers(TOKEN)).unwrap())
            .await
            .is_err());
        fixture.replace(vec![grant(REPLACEMENT, vec![Permission::ManageServer])]);
        let (first, second) =
            tokio::join!(access.reload(&administrator), access.reload(&administrator));
        assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
        assert!(access.authenticate(&headers(TOKEN)).is_none());
        assert!(access.authenticate(&headers(REPLACEMENT)).is_some());
    }

    #[tokio::test]
    async fn client_headers_are_sensitive_and_never_sent_to_plaintext_remote_destinations() {
        let fixture = PolicyFixture::new(vec![grant(TOKEN, vec![Permission::List])]);
        let options = || ClientAccessArgs {
            token_file: Some(fixture.token_path.clone()),
        };
        let header = options()
            .header(&reqwest::Url::parse("http://127.0.0.1:49151/tunnels").unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(header.is_sensitive());
        assert!(!format!("{header:?}").contains(TOKEN));
        for url in [
            "http://192.0.2.1/tunnels",
            "http://example.invalid/tunnels",
            "http://user:secret@127.0.0.1/tunnels",
        ] {
            assert!(options()
                .header(&reqwest::Url::parse(url).unwrap())
                .await
                .is_err());
        }
    }
}

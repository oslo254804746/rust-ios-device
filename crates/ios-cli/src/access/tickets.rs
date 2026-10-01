//! Bounded, single-use browser handshakes. Only ticket digests are retained.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{header, Method, StatusCode};
use axum::response::Response;
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use super::http_boundary::OriginKey;
use super::{Admission, ClientToken, HttpBoundary, Permission, Principal};
use crate::http_diagnostics::ErrorShape;

const LIFETIME: Duration = Duration::from_secs(30);
const CAPACITY: usize = 512;
const MAX_TARGET_BYTES: usize = 2048;

#[derive(Clone, Default)]
pub(super) struct Tickets(Arc<Mutex<HashMap<[u8; 32], Entry>>>);

struct Entry {
    origin: OriginKey,
    target: String,
    deadline: tokio::time::Instant,
    principal: Principal,
}

impl Entry {
    fn current(&self) -> bool {
        tokio::time::Instant::now() < self.deadline && self.principal.current()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issue {
    origin: String,
    target: Target,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Target {
    Tunnel {
        udid: String,
        port: u16,
        #[serde(default)]
        interface: Option<String>,
        #[serde(default)]
        upstream: Option<String>,
        #[serde(default)]
        hop: u8,
    },
    CdpPage {
        page_id: String,
    },
    CdpBrowser,
}

impl Target {
    fn uri(&self, admission: &Admission, principal: &Principal) -> Result<String, StatusCode> {
        let bounded = |value: &str| !value.is_empty() && value.len() <= 256;
        let mut url = reqwest::Url::parse("http://localhost").unwrap();
        match (self, admission.shape) {
            (
                Self::Tunnel {
                    udid,
                    port,
                    interface,
                    upstream,
                    hop,
                },
                ErrorShape::Manager,
            ) => {
                if !bounded(udid)
                    || *port == 0
                    || *hop > 3
                    || [interface, upstream].iter().any(|value| {
                        value
                            .as_ref()
                            .is_some_and(|value| value.is_empty() || value.len() > 512)
                    })
                {
                    return Err(StatusCode::BAD_REQUEST);
                }
                principal
                    .require(Permission::Connect, Some(udid), Some(*port))
                    .map_err(|_| StatusCode::FORBIDDEN)?;
                url.set_path("/connect");
                let mut pairs = url.query_pairs_mut();
                pairs
                    .append_pair("udid", udid)
                    .append_pair("port", &port.to_string());
                if let Some(value) = interface {
                    pairs.append_pair("interface", value);
                }
                if let Some(value) = upstream {
                    pairs.append_pair("upstream", value);
                }
                pairs.append_pair("hop", &hop.to_string());
            }
            (Self::CdpPage { page_id }, ErrorShape::Cdp)
                if bounded(page_id) && page_id != "." && page_id != ".." =>
            {
                url.path_segments_mut()
                    .unwrap()
                    .extend(["devtools", "page", page_id]);
            }
            (Self::CdpBrowser, ErrorShape::Cdp) => {
                url.set_path("/devtools/browser/ios-cli");
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }
        if let Some(device) = &admission.debug_device {
            principal
                .require(Permission::Debug, Some(device), None)
                .map_err(|_| StatusCode::FORBIDDEN)?;
        }
        let mut uri = url.path().to_owned();
        if let Some(query) = url.query() {
            uri.push('?');
            uri.push_str(query);
        }
        if uri.len() > MAX_TARGET_BYTES {
            return Err(StatusCode::BAD_REQUEST);
        }
        Ok(uri)
    }
}

pub(super) fn routes(admission: Admission) -> Router {
    Router::new()
        .route("/auth/ticket", axum::routing::post(issue))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(admission)
}

fn error(shape: ErrorShape, status: StatusCode) -> Response {
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return super::denied_for(shape, status, false);
    }
    let message = match status {
        StatusCode::TOO_MANY_REQUESTS => "browser handshake capacity exhausted",
        StatusCode::SERVICE_UNAVAILABLE => "browser handshake entropy unavailable",
        _ => "invalid browser handshake request",
    };
    let body = match shape {
        ErrorShape::Manager => json!({"message": message}),
        ErrorShape::Cdp => json!({"error": {"code": -32600, "message": message}}),
        ErrorShape::WebDriver => json!({"value": {"error": "invalid request", "message": message}}),
    };
    crate::http_diagnostics::reviewed((status, Json(body)))
}

async fn issue(
    State(admission): State<Admission>,
    Extension(principal): Extension<Principal>,
    Extension(boundary): Extension<HttpBoundary>,
    headers: axum::http::HeaderMap,
    Json(input): Json<Issue>,
) -> Response {
    let Some(origin) = boundary.approved_origin(&input.origin) else {
        return error(admission.shape, StatusCode::FORBIDDEN);
    };
    // Native bootstrap may select an approved Origin. Browser bootstrap must
    // select its own Origin, never a different approved UI's Origin.
    if let Some(actual) = headers.get(header::ORIGIN) {
        if actual.to_str().ok().and_then(OriginKey::parse).as_ref() != Some(&origin) {
            return error(admission.shape, StatusCode::FORBIDDEN);
        }
    }
    let target = match input.target.uri(&admission, &principal) {
        Ok(target) => target,
        Err(status) => return error(admission.shape, status),
    };
    let minted = principal
        .run(async {
            let mut entries = admission.tickets.0.lock().await;
            entries.retain(|_, entry| entry.current());
            if entries.len() >= CAPACITY {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
            // OS entropy failure/collision is explicit, never an empty success.
            let ticket = ClientToken::generate().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            let digest = ticket.digest();
            if entries.contains_key(&digest) || !principal.current() {
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            entries.insert(
                digest,
                Entry {
                    origin,
                    target: target.clone(),
                    deadline: tokio::time::Instant::now() + LIFETIME,
                    principal: principal.clone(),
                },
            );
            let separator = if target.contains('?') { '&' } else { '?' };
            Ok(crate::http_diagnostics::reviewed(Json(json!({
                "ticket": ticket.0.as_str(),
                "expires_in": LIFETIME.as_secs(),
                "websocket_path": format!("{target}{separator}ticket={}", ticket.0.as_str()),
            }))))
        })
        .await;
    match minted {
        Ok(Ok(response)) => response,
        Ok(Err(status)) => error(admission.shape, status),
        Err(_) => error(admission.shape, StatusCode::FORBIDDEN),
    }
}

impl Tickets {
    /// Some means a ticket parameter was supplied, including a malformed one.
    /// A supplied ticket never falls back to header authentication/local trust.
    pub(super) async fn authenticate(
        &self,
        request: &mut Request,
    ) -> Option<Result<Principal, ()>> {
        let query = request.uri().query()?;
        if query.len() > MAX_TARGET_BYTES + 128 {
            return Some(Err(()));
        }
        let url = reqwest::Url::parse(&format!("http://localhost/?{query}")).ok()?;
        if !url.query_pairs().any(|(name, _)| name == "ticket") {
            return None;
        }
        if request.method() != Method::GET
            || request.headers().contains_key(header::AUTHORIZATION)
            || !request
                .headers()
                .get(header::UPGRADE)
                .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
        {
            return Some(Err(()));
        }
        let mut ticket = None;
        let mut retained = Vec::new();
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("ticket=") {
                if ticket.replace(value).is_some() {
                    return Some(Err(()));
                }
            } else {
                retained.push(pair);
            }
        }
        let Some(ticket) = ticket else {
            return Some(Err(()));
        };
        if ticket.len() != 64
            || !ticket
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Some(Err(()));
        }
        // Catch encoded aliases alongside the canonical ticket parameter.
        if url
            .query_pairs()
            .filter(|(name, _)| name == "ticket")
            .count()
            != 1
        {
            return Some(Err(()));
        }
        let Some(origin) = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .and_then(OriginKey::parse)
        else {
            return Some(Err(()));
        };
        let mut target = request.uri().path().to_owned();
        if !retained.is_empty() {
            target.push('?');
            target.push_str(&retained.join("&"));
        }
        let digest: [u8; 32] = Sha256::digest(ticket.as_bytes()).into();
        let mut entries = self.0.lock().await;
        entries.retain(|_, entry| entry.current());
        // Remove under one lock: concurrent handshakes cannot both succeed.
        // An attempted redemption with a different approved origin/target burns
        // the ticket; malformed/unapproved requests never reach this point.
        let Some(entry) = entries.remove(&digest) else {
            return Some(Err(()));
        };
        if entry.origin != origin || entry.target != target || !entry.current() {
            return Some(Err(()));
        }
        *request.uri_mut() = match target.parse() {
            Ok(uri) => uri,
            Err(_) => return Some(Err(())),
        };
        Some(Ok(entry.principal))
    }
}

#[cfg(test)]
mod tests;

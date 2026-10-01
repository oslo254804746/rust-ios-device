//! Listener authority and browser-origin checks, independent of credentials.

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};

use anyhow::Result;
use axum::extract::{Request, State};
use axum::http::{header, uri::Authority, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::{Json, Router};
use serde_json::json;

use crate::http_diagnostics::ErrorShape;

const MAX_EXTRA_ENTRIES: usize = 32;
const MAX_AUTHORITY_BYTES: usize = 512;

#[derive(clap::Args, Default)]
pub(crate) struct HttpBoundaryArgs {
    /// Additional exact Host authority for a protected host forward (HOST:PORT)
    #[arg(long, value_name = "HOST:PORT")]
    allow_host: Vec<String>,
    /// Additional exact browser origin (http[s]://HOST[:PORT]); no wildcards
    #[arg(long, value_name = "ORIGIN")]
    allow_origin: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct AuthorityKey {
    host: String,
    port: u16,
}

impl AuthorityKey {
    fn parse(value: &str, default_port: u16) -> Option<Self> {
        if value.is_empty()
            || value.len() > MAX_AUTHORITY_BYTES
            || !value.is_ascii()
            || value
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || b"@/?#\\,*%".contains(&byte))
        {
            return None;
        }
        let authority: Authority = value.parse().ok()?;
        // Authority::port() returns None for an out-of-range supplied port.
        // Distinguish that from an omitted port instead of accepting a default.
        let supplied_port = if value.starts_with('[') {
            let (_, suffix) = value.split_once(']')?;
            if suffix.is_empty() {
                None
            } else {
                Some(suffix.strip_prefix(':')?)
            }
        } else {
            value.split_once(':').map(|(_, port)| port)
        };
        let port = match supplied_port {
            Some(port) if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) => {
                port.parse::<u16>().ok()?
            }
            Some(_) => return None,
            None => default_port,
        };
        if port == 0 {
            return None;
        }
        let host = authority.host();
        let host = if host.starts_with('[') {
            let address: IpAddr = host.strip_prefix('[')?.strip_suffix(']')?.parse().ok()?;
            if !address.is_ipv6() {
                return None;
            }
            format!("[{address}]")
        } else if let Ok(address) = host.parse::<IpAddr>() {
            address.to_string()
        } else {
            if host.is_empty()
                || host.len() > 253
                || !host
                    .strip_suffix('.')
                    .unwrap_or(host)
                    .split('.')
                    .all(|label| {
                        !label.is_empty()
                            && label.len() <= 63
                            && label
                                .as_bytes()
                                .first()
                                .is_some_and(u8::is_ascii_alphanumeric)
                            && label
                                .as_bytes()
                                .last()
                                .is_some_and(u8::is_ascii_alphanumeric)
                            && label
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                    })
            {
                return None;
            }
            host.to_ascii_lowercase()
        };
        Some(Self { host, port })
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct OriginKey {
    https: bool,
    authority: AuthorityKey,
}

impl OriginKey {
    fn parse(value: &str) -> Option<Self> {
        let (scheme, authority) = value.split_once("://")?;
        let (https, default_port) = match scheme {
            "http" => (false, 80),
            "https" => (true, 443),
            _ => return None,
        };
        Some(Self {
            https,
            authority: AuthorityKey::parse(authority, default_port)?,
        })
    }
}

pub(crate) struct ValidatedBoundaryArgs {
    hosts: HashSet<AuthorityKey>,
    origins: HashSet<OriginKey>,
}

impl HttpBoundaryArgs {
    /// Compile before binding; never echo rejected hosts or origins.
    pub(crate) fn validate(self) -> Result<ValidatedBoundaryArgs> {
        anyhow::ensure!(
            self.allow_host.len() <= MAX_EXTRA_ENTRIES
                && self.allow_origin.len() <= MAX_EXTRA_ENTRIES,
            "HTTP boundary entry limit exceeded"
        );
        let mut hosts = HashSet::new();
        for value in self.allow_host {
            let authority = AuthorityKey::parse(&value, 80)
                .ok_or_else(|| anyhow::anyhow!("invalid allowed HTTP authority"))?;
            anyhow::ensure!(
                value
                    .parse::<Authority>()
                    .is_ok_and(|value| value.port_u16().is_some()),
                "allowed HTTP authority requires an explicit port"
            );
            anyhow::ensure!(hosts.insert(authority), "duplicate allowed HTTP authority");
        }
        let mut origins = HashSet::new();
        for value in self.allow_origin {
            let origin = OriginKey::parse(&value)
                .ok_or_else(|| anyhow::anyhow!("invalid allowed browser origin"))?;
            anyhow::ensure!(origins.insert(origin), "duplicate allowed browser origin");
        }
        Ok(ValidatedBoundaryArgs { hosts, origins })
    }
}

impl ValidatedBoundaryArgs {
    pub(crate) fn for_listener(mut self, listener: SocketAddr) -> HttpBoundary {
        for value in [
            listener.to_string(),
            format!("localhost:{}", listener.port()),
        ] {
            let authority = AuthorityKey::parse(&value, 80).expect("bound listener authority");
            self.hosts.insert(authority.clone());
            self.origins.insert(OriginKey {
                https: false,
                authority,
            });
        }
        HttpBoundary {
            hosts: self.hosts,
            origins: self.origins,
        }
    }
}

#[derive(Clone)]
pub(crate) struct HttpBoundary {
    hosts: HashSet<AuthorityKey>,
    origins: HashSet<OriginKey>,
}

#[derive(Clone)]
struct BoundaryAdmission {
    boundary: HttpBoundary,
    shape: ErrorShape,
}

fn single<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    Some(value)
}

impl HttpBoundary {
    pub(crate) fn protect(self, router: Router, shape: ErrorShape) -> Router {
        router.layer(axum::middleware::from_fn_with_state(
            BoundaryAdmission {
                boundary: self,
                shape,
            },
            admission,
        ))
    }

    fn accepts(&self, request: &Request) -> bool {
        let Some(authority) =
            single(request.headers(), "host").and_then(|value| AuthorityKey::parse(value, 80))
        else {
            return false;
        };
        if !self.hosts.contains(&authority) {
            return false;
        }
        // Direct listeners never trust proxy headers to rewrite authority or
        // Origin. Protected forwarders must preserve/strip them deliberately.
        if [
            "forwarded",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-port",
        ]
        .iter()
        .any(|name| request.headers().contains_key(*name))
        {
            return false;
        }
        if let Some(target) = request.uri().authority() {
            if AuthorityKey::parse(target.as_str(), 80).as_ref() != Some(&authority)
                || request.uri().scheme_str() != Some("http")
            {
                return false;
            }
        }
        if request.headers().contains_key(header::ORIGIN) {
            let Some(origin) = single(request.headers(), "origin").and_then(OriginKey::parse)
            else {
                return false;
            };
            if !self.origins.contains(&origin) {
                return false;
            }
        }
        true
    }
}

async fn admission(
    State(admission): State<BoundaryAdmission>,
    request: Request,
    next: Next,
) -> Response {
    if admission.boundary.accepts(&request) {
        return next.run(request).await;
    }
    let message = "HTTP host/origin boundary denied";
    let body = match admission.shape {
        ErrorShape::Manager => json!({"message": message}),
        ErrorShape::Cdp => json!({"error": {"code": -32600, "message": message}}),
        ErrorShape::WebDriver => json!({"value": {"error": "invalid request", "message": message}}),
    };
    let mut response = crate::http_diagnostics::reviewed((StatusCode::FORBIDDEN, Json(body)));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    if request.method() == Method::HEAD {
        *response.body_mut() = axum::body::Body::empty();
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::WebSocketUpgrade;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};

    #[test]
    fn boundary_configuration_rejects_wildcards_paths_credentials_duplicates_and_excess() {
        for origin in [
            "null",
            "*",
            "https://*.invalid",
            "https://user:secret@example.invalid",
            "https://example.invalid/",
            "https://example.invalid/path",
            "https://example.invalid?token=secret",
            "https://example.invalid#secret",
            "https://example..invalid",
            "file://example.invalid",
            "https://example.invalid:0",
            "https://example.invalid:99999",
        ] {
            assert!(HttpBoundaryArgs {
                allow_host: vec![],
                allow_origin: vec![origin.into()]
            }
            .validate()
            .is_err());
        }
        for host in [
            "*",
            "example.invalid",
            "user:secret@example.invalid:443",
            "example.invalid:443/path",
            "example.invalid:0",
            "example.invalid:99999",
            "https://example.invalid:443",
        ] {
            assert!(HttpBoundaryArgs {
                allow_host: vec![host.into()],
                allow_origin: vec![]
            }
            .validate()
            .is_err());
        }
        assert!(HttpBoundaryArgs {
            allow_host: vec![],
            allow_origin: vec![
                "https://example.invalid".into(),
                "https://EXAMPLE.invalid:443".into()
            ]
        }
        .validate()
        .is_err());
        assert!(HttpBoundaryArgs {
            allow_host: vec!["example.invalid:443".into(), "EXAMPLE.invalid:443".into()],
            allow_origin: vec![]
        }
        .validate()
        .is_err());
        assert!(HttpBoundaryArgs {
            allow_host: vec![],
            allow_origin: (0..33)
                .map(|index| format!("https://host{index}.invalid"))
                .collect()
        }
        .validate()
        .is_err());
    }

    async fn raw(port: u16, request: &str) -> String {
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut bytes = vec![0; 4096];
        let size = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8(bytes[..size].to_vec()).unwrap()
    }

    #[tokio::test]
    async fn actual_http_boundary_rejects_rebinding_origins_authority_mismatch_and_proxy_spoofing()
    {
        for shape in [ErrorShape::Manager, ErrorShape::Cdp, ErrorShape::WebDriver] {
            let calls = Arc::new(AtomicUsize::new(0));
            let called = calls.clone();
            let router = Router::new().route(
                "/",
                get(move || async move {
                    called.fetch_add(1, Ordering::SeqCst);
                    "accepted"
                }),
            );
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let address = listener.local_addr().unwrap();
            let router = HttpBoundaryArgs {
                allow_host: vec!["protected.example.invalid:443".into()],
                allow_origin: vec!["https://viewer.example.invalid".into()],
            }
            .validate()
            .unwrap()
            .for_listener(address)
            .protect(router, shape);
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let base = format!("http://{address}/");
            for (host, origin) in [
                (address.to_string(), None),
                (
                    format!("localhost:{}", address.port()),
                    Some(format!("http://localhost:{}", address.port())),
                ),
                (
                    address.to_string(),
                    Some("https://viewer.example.invalid:443".into()),
                ),
                (
                    "protected.example.invalid:443".into(),
                    Some("https://viewer.example.invalid".into()),
                ),
            ] {
                let mut request = client.get(&base).header("host", host);
                if let Some(origin) = origin {
                    request = request.header("origin", origin);
                }
                assert_eq!(request.send().await.unwrap().status(), StatusCode::OK);
            }
            let accepted = calls.load(Ordering::SeqCst);
            for (name, value) in [
                ("host", "synthetic-secret.evil.invalid:12345"),
                ("host", "127.0.0.1:1"),
                ("origin", "https://synthetic-secret.evil.invalid"),
                ("origin", "null"),
                ("origin", "*"),
                ("origin", "http://localhost:1"),
                ("origin", "https://viewer.example.invalid/path"),
                ("forwarded", "host=synthetic-secret.evil.invalid"),
                ("x-forwarded-host", "synthetic-secret.evil.invalid"),
                ("x-forwarded-proto", "https"),
                ("x-forwarded-port", "443"),
            ] {
                let response = client.get(&base).header(name, value).send().await.unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
                assert_eq!(response.headers()["cache-control"], "no-store");
                let original = response.text().await.unwrap();
                assert!(!original.contains("synthetic-secret"));
                assert!(original.contains("HTTP host/origin boundary denied"));
                let body: serde_json::Value = serde_json::from_str(&original).unwrap();
                match shape {
                    ErrorShape::Manager => assert!(body["message"].is_string()),
                    ErrorShape::Cdp => assert_eq!(body["error"]["code"], -32600),
                    ErrorShape::WebDriver => assert_eq!(body["value"]["error"], "invalid request"),
                }
            }
            for request in [
                format!("GET http://synthetic-secret.evil.invalid/ HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"),
                "GET / HTTP/1.0\r\nConnection: close\r\n\r\n".into(),
                format!("GET / HTTP/1.1\r\nHost: {address}\r\nHost: synthetic-secret.evil.invalid\r\nConnection: close\r\n\r\n"),
                format!("GET / HTTP/1.1\r\nHost: {address}\r\nOrigin: https://viewer.example.invalid\r\nOrigin: https://synthetic-secret.evil.invalid\r\nConnection: close\r\n\r\n"),
            ] {
                let original = raw(address.port(), &request).await;
                assert!(original.starts_with("HTTP/1.1 403") || original.starts_with("HTTP/1.1 400") || original.starts_with("HTTP/1.0 403"));
                assert!(!original.contains("synthetic-secret"));
            }
            let response = client
                .head(&base)
                .header("origin", "null")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(response.bytes().await.unwrap().is_empty());
            assert_eq!(calls.load(Ordering::SeqCst), accepted);
            server.abort();
            let _ = server.await;
        }
    }

    #[tokio::test]
    async fn ipv6_http_authorities_and_origins_use_the_actual_bound_port() {
        let listener = tokio::net::TcpListener::bind(("::1", 0)).await.unwrap();
        let bound = listener.local_addr().unwrap();
        let router = HttpBoundaryArgs::default()
            .validate()
            .unwrap()
            .for_listener(bound)
            .protect(
                Router::new().route("/", get(|| async { "accepted" })),
                ErrorShape::Manager,
            );
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let base = format!("http://{bound}/");
        assert_eq!(
            client
                .get(&base)
                .header(
                    "origin",
                    format!("http://[0:0:0:0:0:0:0:1]:{}", bound.port())
                )
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            client
                .get(&base)
                .header("origin", "http://[::1]:1")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            client
                .get(&base)
                .header("host", format!("[::2]:{}", bound.port()))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn actual_websocket_boundary_rejects_browser_origin_and_host_before_upgrade() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let router = Router::new().route(
            "/ws",
            get(move |ws: WebSocketUpgrade| async move {
                called.fetch_add(1, Ordering::SeqCst);
                ws.on_upgrade(|socket| async move {
                    let _ = socket.close().await;
                })
                .into_response()
            }),
        );
        let router = HttpBoundaryArgs::default()
            .validate()
            .unwrap()
            .for_listener(address)
            .protect(router, ErrorShape::Cdp);
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("ws://{address}/ws");
        for (name, value) in [
            ("origin", "https://evil.invalid"),
            ("host", "evil.invalid:443"),
            ("origin", "null"),
        ] {
            let mut request = url.clone().into_client_request().unwrap();
            request.headers_mut().insert(name, value.parse().unwrap());
            assert!(
                matches!(tokio_tungstenite::connect_async(request).await.unwrap_err(), tungstenite::Error::Http(response) if response.status() == StatusCode::FORBIDDEN)
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for origin in [None, Some(format!("http://{address}"))] {
            let mut request = url.clone().into_client_request().unwrap();
            if let Some(origin) = origin {
                request
                    .headers_mut()
                    .insert("origin", origin.parse().unwrap());
            }
            let (_, response) = tokio_tungstenite::connect_async(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        server.abort();
        let _ = server.await;
    }
}

//! Per-upstream private credentials and verified HTTP/WebSocket transport.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::http::{header, HeaderValue};
use serde::Deserialize;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use super::ClientToken;

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_CA_BYTES: u64 = 64 * 1024;
const MAX_UPSTREAMS: usize = 32;

#[derive(clap::Args, Default)]
pub(crate) struct UpstreamArgs {
    /// Private version-1 upstream URL/token/CA configuration, loaded before bind
    #[arg(long, value_name = "PRIVATE_FILE")]
    upstream_config: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u8,
    upstreams: Vec<Configured>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configured {
    url: String,
    token_file: PathBuf,
    #[serde(default)]
    ca_file: Option<PathBuf>,
}

/// Local plaintext URLs must use literal loopback, never DNS hostnames. Remote
/// access uses verified HTTPS or an independently protected loopback forward.
pub(crate) fn normalize_upstream_url(raw: &str) -> Result<String> {
    anyhow::ensure!(raw.len() <= 2048, "upstream URL limit exceeded");
    let url = reqwest::Url::parse(raw).context("invalid upstream URL")?;
    anyhow::ensure!(
        matches!(url.scheme(), "http" | "https"),
        "upstream requires HTTP(S)"
    );
    anyhow::ensure!(url.host().is_some(), "upstream URL has no host");
    anyhow::ensure!(
        url.port_or_known_default().is_some_and(|port| port != 0),
        "upstream requires nonzero port"
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "upstream URL must not contain credentials"
    );
    anyhow::ensure!(
        url.query().is_none() && url.fragment().is_none(),
        "upstream URL must not contain query or fragment"
    );
    anyhow::ensure!(
        url.path() == "/",
        "upstream URL must point to the manager root"
    );
    if url.scheme() == "http" {
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|address| address.is_loopback());
        anyhow::ensure!(
            loopback,
            "plaintext upstream requires literal loopback or a protected host forward"
        );
    }
    Ok(url.to_string())
}

fn tls_config(ca: Option<&Path>) -> Result<Arc<rustls::ClientConfig>> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Some(path) = ca {
        let bytes = ios_core::secret_file::read_secret(path, MAX_CA_BYTES)
            .context("private upstream CA read failed")?;
        let mut cursor = std::io::Cursor::new(bytes.as_slice());
        let mut certificates = rustls_pemfile::certs(&mut cursor);
        let mut count = 0;
        for certificate in &mut certificates {
            count += 1;
            anyhow::ensure!(count <= 16, "upstream CA certificate limit exceeded");
            roots
                .add(certificate.context("invalid upstream CA PEM")?)
                .context("invalid upstream CA certificate")?;
        }
        anyhow::ensure!(count > 0, "upstream CA file contains no certificate");
    }
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .context("upstream TLS protocol configuration failed")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

#[derive(Clone)]
pub(crate) struct UpstreamClient {
    root: String,
    token: Option<Arc<ClientToken>>,
    http: reqwest::Client,
    tls: Arc<rustls::ClientConfig>,
}

impl std::fmt::Debug for UpstreamClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamClient")
            .field("authenticated", &self.token.is_some())
            .finish_non_exhaustive()
    }
}

impl UpstreamClient {
    fn new(root: String, token: Option<ClientToken>, ca: Option<&Path>) -> Result<Self> {
        let tls = tls_config(ca)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .use_preconfigured_tls(tls.as_ref().clone())
            .build()
            .context("upstream HTTP client configuration failed")?;
        Ok(Self {
            root,
            token: token.map(Arc::new),
            http,
            tls,
        })
    }

    fn header(&self, url: &reqwest::Url) -> Result<Option<HeaderValue>> {
        let mut root = url.clone();
        let scheme = match url.scheme() {
            "http" | "ws" => "http",
            "https" | "wss" => "https",
            _ => anyhow::bail!("unsupported upstream request scheme"),
        };
        root.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("invalid upstream request scheme"))?;
        root.set_path("/");
        root.set_query(None);
        root.set_fragment(None);
        anyhow::ensure!(
            normalize_upstream_url(root.as_str())? == self.root,
            "upstream credential target mismatch"
        );
        let Some(token) = &self.token else {
            return Ok(None);
        };
        let mut value = zeroize::Zeroizing::new(String::from("Bearer "));
        value.push_str(&token.0);
        let mut header =
            HeaderValue::from_str(&value).context("invalid upstream credential header")?;
        header.set_sensitive(true);
        Ok(Some(header))
    }

    pub(crate) async fn listing(&self, hop: u8, limit: usize) -> Result<serde_json::Value> {
        let mut url = reqwest::Url::parse(&self.root)?;
        url.query_pairs_mut().append_pair("hop", &hop.to_string());
        let header = self.header(&url)?;
        let mut request = self.http.get(url);
        if let Some(header) = header {
            request = request.header(header::AUTHORIZATION, header);
        }
        let mut response = request
            .send()
            .await
            .context("upstream HTTP transport failed")?;
        // reqwest::error_for_status doesn't reject redirects. Never follow or
        // parse a redirect as a successful empty listing.
        anyhow::ensure!(
            response.status().is_success(),
            "upstream HTTP status refused"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .context("upstream HTTP body failed")?
        {
            anyhow::ensure!(
                bytes.len().saturating_add(chunk.len()) <= limit,
                "upstream listing byte limit exceeded"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).context("invalid upstream listing JSON")
    }

    pub(crate) fn websocket_request(
        &self,
        url: &reqwest::Url,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
        let header = self.header(url)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .context("invalid upstream WebSocket request")?;
        if let Some(header) = header {
            request.headers_mut().insert(header::AUTHORIZATION, header);
        }
        Ok(request)
    }

    pub(crate) fn connector(&self) -> tokio_tungstenite::Connector {
        tokio_tungstenite::Connector::Rustls(self.tls.clone())
    }
}

#[derive(Clone, Default)]
pub(crate) struct Upstreams(Arc<BTreeMap<String, UpstreamClient>>);

impl Upstreams {
    pub(crate) fn urls(&self) -> impl Iterator<Item = &String> {
        self.0.keys()
    }

    pub(crate) fn client(&self, url: &str) -> Result<UpstreamClient> {
        let root = normalize_upstream_url(url)?;
        self.0
            .get(&root)
            .cloned()
            .map_or_else(|| UpstreamClient::new(root, None, None), Ok)
    }

    fn read(path: &Path) -> Result<Self> {
        let bytes = ios_core::secret_file::read_secret(path, MAX_CONFIG_BYTES)
            .context("private upstream configuration read failed")?;
        let file: File =
            serde_json::from_slice(&bytes).context("invalid upstream configuration")?;
        anyhow::ensure!(
            file.version == 1 && file.upstreams.len() <= MAX_UPSTREAMS,
            "unsupported upstream configuration version or count"
        );
        let base = path.parent().unwrap_or(Path::new("."));
        let resolve = |path: PathBuf| {
            if path.is_absolute() {
                path
            } else {
                base.join(path)
            }
        };
        let mut clients = BTreeMap::new();
        for entry in file.upstreams {
            let root = normalize_upstream_url(&entry.url)?;
            anyhow::ensure!(
                !clients.contains_key(&root),
                "duplicate configured upstream"
            );
            anyhow::ensure!(
                entry.token_file.as_os_str().len() <= 4096
                    && entry
                        .ca_file
                        .as_ref()
                        .map_or(true, |path| path.as_os_str().len() <= 4096),
                "upstream private path limit exceeded"
            );
            let token = ClientToken::from_file(&resolve(entry.token_file))?;
            let ca = entry.ca_file.map(resolve);
            anyhow::ensure!(
                ca.is_none() || root.starts_with("https:"),
                "upstream CA requires HTTPS"
            );
            clients.insert(
                root.clone(),
                UpstreamClient::new(root, Some(token), ca.as_deref())?,
            );
        }
        Ok(Self(Arc::new(clients)))
    }
}

impl UpstreamArgs {
    pub(crate) async fn load(self) -> Result<Upstreams> {
        let Some(path) = self.upstream_config else {
            return Ok(Upstreams::default());
        };
        tokio::task::spawn_blocking(move || Upstreams::read(&path))
            .await
            .context("upstream configuration reader failed")?
    }
}

#[cfg(test)]
mod tests;

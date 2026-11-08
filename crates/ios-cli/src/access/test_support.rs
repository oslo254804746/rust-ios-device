//! Owned private fixtures; no production credential paths or devices.

use std::path::PathBuf;

use super::{unix_seconds, GrantFile, Permission, PolicyFile};

pub(crate) const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(crate) const REPLACEMENT: &str =
    "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
pub(crate) const DEVICE: &str = "synthetic-manager-device";

/// Keep test JSON requests independent of reqwest's optional JSON feature.
pub(crate) trait JsonRequest {
    fn json(self, value: &serde_json::Value) -> Self;
}

impl JsonRequest for reqwest::RequestBuilder {
    fn json(self, value: &serde_json::Value) -> Self {
        self.header("content-type", "application/json")
            .body(value.to_string())
    }
}

pub(crate) struct TestGrant(GrantFile);

impl TestGrant {
    pub(crate) fn expires_after(self, seconds: u64) -> Self {
        self.expires_at(unix_seconds().unwrap() + seconds)
    }

    pub(crate) fn expires_at(mut self, expiry: u64) -> Self {
        self.0.expires_unix = expiry;
        self
    }
}

pub(crate) fn grant(token: &str, permissions: Vec<Permission>) -> TestGrant {
    use sha2::{Digest, Sha256};
    TestGrant(GrantFile {
        token_sha256: hex::encode(Sha256::digest(token.as_bytes())),
        expires_unix: unix_seconds().unwrap() + 3600,
        permissions,
        devices: vec![DEVICE.into()],
        all_devices: false,
        ports: vec![12345],
        all_ports: false,
    })
}

pub(crate) struct PolicyFixture {
    directory: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) token_path: PathBuf,
}

impl PolicyFixture {
    pub(crate) fn new(grants: Vec<TestGrant>) -> Self {
        let directory =
            std::env::temp_dir().join(format!("ios-manager-policy-{}", uuid::Uuid::new_v4()));
        let fixture = Self {
            path: directory.join("policy.json"),
            token_path: directory.join("client-token"),
            directory,
        };
        ios_core::secret_file::write_secret_new(
            &fixture.path,
            &serde_json::to_vec(&PolicyFile {
                version: 1,
                grants: grants.into_iter().map(|grant| grant.0).collect(),
            })
            .unwrap(),
        )
        .unwrap();
        ios_core::secret_file::write_secret_new(&fixture.token_path, TOKEN.as_bytes()).unwrap();
        fixture
    }

    pub(crate) fn replace(&self, grants: Vec<TestGrant>) {
        ios_core::secret_file::write_secret(
            &self.path,
            &serde_json::to_vec(&PolicyFile {
                version: 1,
                grants: grants.into_iter().map(|grant| grant.0).collect(),
            })
            .unwrap(),
        )
        .unwrap();
    }

    pub(crate) fn malformed(&self) {
        ios_core::secret_file::write_secret(&self.path, b"invalid synthetic policy").unwrap();
    }
}

impl Drop for PolicyFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Owned TLS terminator for actual HTTPS/WSS fixtures, forwarding only to an
/// owned literal loopback HTTP router. Aborting the parent aborts all children.
pub(crate) async fn tls_forward(
    port: u16,
    certificate: &[u8],
) -> (u16, tokio::task::JoinHandle<()>) {
    use std::sync::Arc;
    let mut certs = std::io::Cursor::new(certificate);
    let chain = rustls_pemfile::certs(&mut certs)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut key = std::io::Cursor::new(super::test_tls_material::KEY);
    let key = rustls_pemfile::private_key(&mut key).unwrap().unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(chain, key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let bound = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (stream,_) = accepted.unwrap();
                    let acceptor = acceptor.clone();
                    children.spawn(async move {
                        let Ok(Ok(mut tls)) = tokio::time::timeout(std::time::Duration::from_secs(2),acceptor.accept(stream)).await else { return; };
                        let Ok(mut http) = tokio::net::TcpStream::connect(("127.0.0.1",port)).await else { return; };
                        let _ = tokio::io::copy_bidirectional(&mut tls,&mut http).await;
                    });
                }
                _ = children.join_next(), if !children.is_empty() => {}
            }
        }
    });
    (bound, task)
}

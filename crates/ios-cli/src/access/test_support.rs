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

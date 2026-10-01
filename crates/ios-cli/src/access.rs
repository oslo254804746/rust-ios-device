//! Private manager credential configuration and least-privilege checks.
//!
//! Server admission is wired separately; creating a policy never deploys it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

mod http_boundary;
pub(crate) use http_boundary::{HttpBoundary, HttpBoundaryArgs};
mod runtime;
pub(crate) use runtime::{
    admission, denied_for, loopback_address, AccessControl, Admission, ClientAccessArgs, Principal,
    ServerAccessArgs,
};

#[cfg(test)]
pub(crate) mod test_support;

const MAX_POLICY_BYTES: u64 = 64 * 1024;
const MAX_GRANTS: usize = 64;
const MAX_DEVICES: usize = 128;
const MAX_PORTS: usize = 256;
const MAX_LIFETIME_SECONDS: u64 = 365 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Permission {
    List,
    Connect,
    ManageTunnels,
    ManageServer,
    Debug,
}

impl Permission {
    fn needs_device(self) -> bool {
        self != Self::ManageServer
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    version: u8,
    grants: Vec<GrantFile>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantFile {
    token_sha256: String,
    expires_unix: u64,
    permissions: Vec<Permission>,
    #[serde(default)]
    devices: Vec<String>,
    #[serde(default)]
    all_devices: bool,
    #[serde(default)]
    ports: Vec<u16>,
    #[serde(default)]
    all_ports: bool,
}

pub(crate) struct ClientToken(Zeroizing<String>);

impl std::fmt::Debug for ClientToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientToken").finish_non_exhaustive()
    }
}

impl ClientToken {
    fn generate() -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng
            .try_fill_bytes(bytes.as_mut())
            .context("manager credential entropy unavailable")?;
        Ok(Self(Zeroizing::new(hex::encode(bytes.as_slice()))))
    }

    fn from_file(path: &Path) -> Result<Self> {
        let bytes = ios_core::secret_file::read_secret(path, 66)
            .context("private client credential read failed")?;
        let value = std::str::from_utf8(&bytes)
            .context("client credential must be UTF-8")?
            .trim_end_matches(['\r', '\n']);
        anyhow::ensure!(
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "client credential must contain exactly 64 lowercase hexadecimal characters"
        );
        Ok(Self(Zeroizing::new(value.into())))
    }

    fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

#[derive(Clone)]
struct Grant {
    digest: [u8; 32],
    scope: GrantFile,
}

pub(crate) struct Policy {
    grants: Vec<Grant>,
}

impl std::fmt::Debug for Policy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Policy")
            .field("grant_count", &self.grants.len())
            .finish_non_exhaustive()
    }
}

impl Policy {
    fn compile(file: PolicyFile, now: u64) -> Result<Self> {
        anyhow::ensure!(file.version == 1, "unsupported manager policy version");
        anyhow::ensure!(
            file.grants.len() <= MAX_GRANTS,
            "manager policy grant limit exceeded"
        );
        let mut grants: Vec<Grant> = Vec::new();
        for scope in file.grants {
            let mut digest = [0u8; 32];
            hex::decode_to_slice(&scope.token_sha256, &mut digest)
                .context("manager policy credential digest must be SHA-256 hexadecimal")?;
            anyhow::ensure!(
                !grants
                    .iter()
                    .any(|grant| bool::from(grant.digest.ct_eq(&digest))),
                "duplicate manager policy credential"
            );
            anyhow::ensure!(
                scope.expires_unix.saturating_sub(now) <= MAX_LIFETIME_SECONDS,
                "manager credential expiry exceeds one year"
            );
            anyhow::ensure!(
                !scope.permissions.is_empty(),
                "manager grant requires explicit permissions"
            );
            anyhow::ensure!(
                !scope
                    .permissions
                    .iter()
                    .enumerate()
                    .any(|(index, permission)| scope.permissions[..index].contains(permission)),
                "duplicate manager permission"
            );
            anyhow::ensure!(
                scope.devices.len() <= MAX_DEVICES && scope.ports.len() <= MAX_PORTS,
                "manager scope limit exceeded"
            );
            anyhow::ensure!(
                scope.devices.iter().all(|device| !device.is_empty()
                    && device.len() <= 256
                    && !device.contains('\0')),
                "invalid manager device selector"
            );
            anyhow::ensure!(
                !scope.all_devices || scope.devices.is_empty(),
                "use all_devices or explicit device selectors, not both"
            );
            anyhow::ensure!(
                !scope.all_ports || scope.ports.is_empty(),
                "use all_ports or explicit ports, not both"
            );
            anyhow::ensure!(
                !scope.ports.contains(&0),
                "manager grant ports must be nonzero"
            );
            anyhow::ensure!(
                scope.all_devices
                    || !scope.devices.is_empty()
                    || !scope
                        .permissions
                        .iter()
                        .any(|permission| permission.needs_device()),
                "device-scoped permission requires explicit devices or all_devices"
            );
            anyhow::ensure!(
                !scope.permissions.contains(&Permission::Connect)
                    || scope.all_ports
                    || !scope.ports.is_empty(),
                "connect permission requires explicit ports or all_ports"
            );
            grants.push(Grant { digest, scope });
        }
        Ok(Self { grants })
    }

    fn load(path: &Path, now: u64) -> Result<Self> {
        let bytes = ios_core::secret_file::read_secret(path, MAX_POLICY_BYTES)
            .context("private manager policy read failed")?;
        Self::compile(
            serde_json::from_slice(&bytes).context("invalid manager policy JSON")?,
            now,
        )
    }

    fn authenticate(&self, token: &ClientToken, now: u64) -> Option<&Grant> {
        self.authenticate_digest(&token.digest(), now)
            .map(|index| &self.grants[index])
    }

    fn authenticate_digest(&self, digest: &[u8; 32], now: u64) -> Option<usize> {
        let mut selected = None;
        // Compare every fixed-size digest; never compare the original token
        // text or reveal which credential/expiry failed in diagnostic errors.
        for (index, grant) in self.grants.iter().enumerate() {
            if bool::from(grant.digest.ct_eq(digest)) && grant.scope.expires_unix > now {
                selected = Some(index);
            }
        }
        selected
    }
}

impl Grant {
    fn allows(&self, permission: Permission, device: Option<&str>, port: Option<u16>) -> bool {
        if !self.scope.permissions.contains(&permission) {
            return false;
        }
        if permission.needs_device()
            && !device.is_some_and(|device| {
                self.scope.all_devices || self.scope.devices.iter().any(|allowed| allowed == device)
            })
        {
            return false;
        }
        permission != Permission::Connect
            || port.is_some_and(|port| {
                port != 0 && (self.scope.all_ports || self.scope.ports.contains(&port))
            })
    }
}

fn unix_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("host clock precedes credential epoch")?
        .as_secs())
}

#[derive(clap::Args)]
pub(crate) struct AuthCmd {
    #[command(subcommand)]
    command: AuthSub,
}

#[derive(clap::Subcommand)]
enum AuthSub {
    /// Create private scoped credentials; this does not start or secure a server
    Create {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long, required = true)]
        permission: Vec<Permission>,
        #[arg(long, conflicts_with = "all_devices")]
        device: Vec<String>,
        #[arg(long)]
        all_devices: bool,
        #[arg(long, conflicts_with = "all_ports")]
        port: Vec<u16>,
        #[arg(long)]
        all_ports: bool,
        #[arg(long, default_value_t = 24 * 60 * 60)]
        expires_in: u64,
    },
    /// Check private credentials and a precise scope without contacting a device
    Check {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        token_file: PathBuf,
        #[arg(long)]
        permission: Permission,
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        port: Option<u16>,
    },
}

impl AuthCmd {
    pub(crate) async fn run(self) -> Result<()> {
        let receipt = tokio::task::spawn_blocking(move || self.run_blocking())
            .await
            .context("manager credential worker failed")??;
        println!("{}", serde_json::to_string(&receipt)?);
        Ok(())
    }

    fn run_blocking(self) -> Result<serde_json::Value> {
        let now = unix_seconds()?;
        match self.command {
            AuthSub::Create {
                config,
                token_file,
                permission,
                device,
                all_devices,
                port,
                all_ports,
                expires_in,
            } => {
                anyhow::ensure!(
                    (1..=MAX_LIFETIME_SECONDS).contains(&expires_in),
                    "manager credential lifetime must be between one second and one year"
                );
                let expires_unix = now
                    .checked_add(expires_in)
                    .context("credential expiry overflow")?;
                let token = ClientToken::generate()?;
                let file = PolicyFile {
                    version: 1,
                    grants: vec![GrantFile {
                        token_sha256: hex::encode(token.digest()),
                        expires_unix,
                        permissions: permission,
                        devices: device,
                        all_devices,
                        ports: port,
                        all_ports,
                    }],
                };
                Policy::compile(file.clone(), now)?;
                let bytes = Zeroizing::new(serde_json::to_vec_pretty(&file)?);
                ios_core::secret_file::write_secret_new(&config, &bytes)
                    .context("manager policy creation failed")?;
                if let Err(error) =
                    ios_core::secret_file::write_secret_new(&token_file, token.0.as_bytes())
                {
                    // This is a concrete partial publication, not a multi-file
                    // transaction. Preserve the new policy and every existing
                    // file instead of unlinking by a path another writer could
                    // have changed. The operator can inspect their two paths.
                    eprintln!("Manager policy was created but client credential creation failed; inspect both requested paths before retrying.");
                    return Err(error)
                        .context("client credential creation failed after policy publication");
                }
                Ok(
                    serde_json::json!({"operation": "credential_create", "expires_unix": expires_unix, "data": true}),
                )
            }
            AuthSub::Check {
                config,
                token_file,
                permission,
                device,
                port,
            } => {
                let token = ClientToken::from_file(&token_file)?;
                let policy = Policy::load(&config, now)?;
                let grant = policy
                    .authenticate(&token, now)
                    .context("manager credential rejected")?;
                anyhow::ensure!(
                    grant.allows(permission, device.as_deref(), port),
                    "manager permission rejected"
                );
                Ok(
                    serde_json::json!({"operation": "credential_check", "permission": permission, "data": true}),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(token: &ClientToken) -> GrantFile {
        GrantFile {
            token_sha256: hex::encode(token.digest()),
            expires_unix: 200,
            permissions: vec![Permission::List, Permission::Connect],
            devices: vec!["synthetic-device-a".into()],
            all_devices: false,
            ports: vec![58783],
            all_ports: false,
        }
    }

    #[test]
    fn credentials_and_policy_debug_hide_material_and_device_selectors() {
        let token = ClientToken(Zeroizing::new("synthetic-private-secret-material".into()));
        let policy = Policy::compile(
            PolicyFile {
                version: 1,
                grants: vec![scope(&token)],
            },
            100,
        )
        .unwrap();
        let output = format!("{token:?} {policy:?} {policy:#?}");
        for secret in [
            token.0.as_str(),
            "synthetic-device-a",
            &hex::encode(token.digest()),
        ] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("grant_count"));
    }

    #[test]
    fn exact_scope_expiry_and_invalid_credentials_deny_without_broadening() {
        let token = ClientToken::generate().unwrap();
        let invalid = ClientToken::generate().unwrap();
        let policy = Policy::compile(
            PolicyFile {
                version: 1,
                grants: vec![scope(&token)],
            },
            100,
        )
        .unwrap();
        let grant = policy.authenticate(&token, 199).unwrap();
        assert!(grant.allows(Permission::List, Some("synthetic-device-a"), None));
        assert!(grant.allows(Permission::Connect, Some("synthetic-device-a"), Some(58783)));
        assert!(!grant.allows(Permission::Connect, Some("synthetic-device-b"), Some(58783)));
        assert!(!grant.allows(Permission::Connect, Some("synthetic-device-a"), Some(22)));
        assert!(!grant.allows(Permission::Connect, Some("synthetic-device-a"), Some(0)));
        assert!(!grant.allows(Permission::ManageServer, None, None));
        assert!(!grant.allows(Permission::Debug, Some("synthetic-device-a"), None));
        assert!(policy.authenticate(&token, 200).is_none());
        assert!(policy.authenticate(&invalid, 100).is_none());
        let empty = Policy::compile(
            PolicyFile {
                version: 1,
                grants: vec![],
            },
            100,
        )
        .unwrap();
        assert!(empty.authenticate(&token, 100).is_none());
        let mut explicit = scope(&token);
        explicit.all_devices = true;
        explicit.devices.clear();
        explicit.all_ports = true;
        explicit.ports.clear();
        let broad = Policy::compile(
            PolicyFile {
                version: 1,
                grants: vec![explicit],
            },
            100,
        )
        .unwrap();
        let grant = broad.authenticate(&token, 100).unwrap();
        assert!(grant.allows(Permission::Connect, Some("synthetic-device-b"), Some(22)));
        assert!(!grant.allows(Permission::Connect, Some("synthetic-device-b"), Some(0)));
        assert!(!grant.allows(Permission::ManageServer, None, None));
    }

    #[test]
    fn invalid_policy_version_duplicates_scope_and_unknown_fields_fail() {
        let token = ClientToken::generate().unwrap();
        let good = scope(&token);
        assert!(Policy::compile(
            PolicyFile {
                version: 2,
                grants: vec![good.clone()]
            },
            100
        )
        .is_err());
        assert!(Policy::compile(
            PolicyFile {
                version: 1,
                grants: vec![good.clone(), good.clone()]
            },
            100
        )
        .is_err());
        for change in [0, 1, 2, 3, 4] {
            let mut grant = good.clone();
            match change {
                0 => grant.all_devices = true,
                1 => grant.ports = vec![0],
                2 => grant.ports.clear(),
                3 => grant.devices.clear(),
                _ => grant.expires_unix = 100 + MAX_LIFETIME_SECONDS + 1,
            }
            assert!(Policy::compile(
                PolicyFile {
                    version: 1,
                    grants: vec![grant]
                },
                100
            )
            .is_err());
        }
        let mut unknown = serde_json::to_value(PolicyFile {
            version: 1,
            grants: vec![good],
        })
        .unwrap();
        unknown["grants"][0]["all_device"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PolicyFile>(unknown).is_err());
    }

    #[test]
    fn private_file_commands_create_check_and_preserve_existing_outputs() {
        let directory =
            std::env::temp_dir().join(format!("ios-auth-fixture-{}", uuid::Uuid::new_v4()));
        let config = directory.join("policy.json");
        let token_file = directory.join("client-token");
        let create = || AuthCmd {
            command: AuthSub::Create {
                config: config.clone(),
                token_file: token_file.clone(),
                permission: vec![Permission::Connect],
                device: vec!["synthetic-device-a".into()],
                all_devices: false,
                port: vec![58783],
                all_ports: false,
                expires_in: 3600,
            },
        };
        assert_eq!(create().run_blocking().unwrap()["data"], true);
        let original_policy = std::fs::read(&config).unwrap();
        let original_token = ios_core::secret_file::read_secret(&token_file, 66).unwrap();
        assert!(create().run_blocking().is_err());
        assert_eq!(std::fs::read(&config).unwrap(), original_policy);
        assert_eq!(
            ios_core::secret_file::read_secret(&token_file, 66)
                .unwrap()
                .as_slice(),
            original_token.as_slice()
        );
        let check = |port| AuthCmd {
            command: AuthSub::Check {
                config: config.clone(),
                token_file: token_file.clone(),
                permission: Permission::Connect,
                device: Some("synthetic-device-a".into()),
                port: Some(port),
            },
        };
        assert_eq!(check(58783).run_blocking().unwrap()["data"], true);
        assert!(check(22).run_blocking().is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn partial_creation_preserves_existing_token_and_reports_failure() {
        let directory =
            std::env::temp_dir().join(format!("ios-auth-partial-{}", uuid::Uuid::new_v4()));
        let config = directory.join("policy.json");
        let token_file = directory.join("client-token");
        ios_core::secret_file::write_secret_new(&token_file, b"synthetic-existing-token").unwrap();
        let create = AuthCmd {
            command: AuthSub::Create {
                config: config.clone(),
                token_file: token_file.clone(),
                permission: vec![Permission::ManageServer],
                device: vec![],
                all_devices: false,
                port: vec![],
                all_ports: false,
                expires_in: 3600,
            },
        };
        assert!(create.run_blocking().is_err());
        assert_eq!(
            std::fs::read(&token_file).unwrap(),
            b"synthetic-existing-token"
        );
        assert!(config.is_file());
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

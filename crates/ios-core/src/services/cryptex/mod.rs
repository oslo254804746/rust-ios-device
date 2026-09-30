//! Cryptex DDI installation over `com.apple.security.cryptexd.remote`.
//!
//! Each routine gets a fresh connection. The iOS 27 DDI is personalized with
//! a Cryptex1 ticket and the nonce-domain **handle** from its build identity.

mod assets;
mod signing;

use std::path::Path;
use std::time::Duration;

use indexmap::IndexMap;

use crate::xpc::message::{XpcMessage, XpcValue};
use crate::xpc::{XpcClient, XpcError};
use crate::ConnectedDevice;

pub use assets::{CryptexDdiAssets, LATEST_CRYPTEX_DDI_BUILD_ID};
pub use signing::build_cryptex_tss_request;

pub const SERVICE_NAME: &str = "com.apple.security.cryptexd.remote";
pub const DDI_IDENTIFIER: &str = "com.apple.MobileAsset.DDI";
pub const FEATURE_INSTALL: &str = "CryptexInstall";
pub const FEATURE_IDENTIFIERS: &str = "ReadIdentifiers";
const ROUTINE_TIMEOUT: Duration = Duration::from_secs(30);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

type Result<T> = std::result::Result<T, CryptexError>;

#[derive(Debug, thiserror::Error)]
pub enum CryptexError {
    #[error("Cryptex XPC: {0}")]
    Xpc(#[from] XpcError),
    #[error("Cryptex I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cryptex plist: {0}")]
    Plist(#[from] plist::Error),
    #[error("Cryptex protocol: {0}")]
    Protocol(String),
    #[error("Cryptex device error: {0}")]
    Device(String),
    #[error("Cryptex download: {0}")]
    Download(String),
    #[error("Cryptex routine timed out: {0}")]
    Timeout(&'static str),
    #[error("Cryptex capability not supported: {0}")]
    Unsupported(&'static str),
    #[error("DDI mount conflict: {0}")]
    Conflict(String),
    #[error("Cryptex device connection: {0}")]
    Connection(#[from] crate::CoreError),
    #[error("Cryptex personalization or image mounter: {0}")]
    ImageMounter(#[from] crate::imagemounter::protocol::ImageMounterError),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct InstalledCryptex {
    pub identifier: String,
    pub version: String,
}

/// The daemon distinguishes a nonce table index from a build identity's handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceDomain {
    Index(u64),
    Handle(u64),
}

impl Default for NonceDomain {
    fn default() -> Self {
        Self::Index(2)
    }
}

impl NonceDomain {
    fn arguments(self) -> IndexMap<String, XpcValue> {
        let (key, number) = match self {
            Self::Index(number) => ("nonce-domain", number),
            Self::Handle(number) => ("nonce-domain-handle", number),
        };
        IndexMap::from([(key.into(), XpcValue::Uint64(number))])
    }
}

/// Service client that reconnects before every routine after its first call.
pub struct CryptexClient {
    xpc: XpcClient,
    features: Vec<String>,
    used: bool,
}

impl CryptexClient {
    pub fn new(xpc: XpcClient, features: Vec<String>) -> Self {
        Self {
            xpc,
            features,
            used: false,
        }
    }

    pub async fn connect(device: &ConnectedDevice) -> Result<Self> {
        let (xpc, metadata) = tokio::time::timeout(
            ROUTINE_TIMEOUT,
            device.connect_xpc_service_with_metadata(SERVICE_NAME),
        )
        .await
        .map_err(|_| CryptexError::Timeout("connect"))??;
        Ok(Self::new(xpc, metadata.features))
    }

    fn require_feature(&self, name: &'static str) -> Result<()> {
        if !self.features.is_empty() && !self.features.iter().any(|feature| feature == name) {
            return Err(CryptexError::Unsupported(name));
        }
        Ok(())
    }

    async fn connection(&mut self) -> Result<&mut XpcClient> {
        if self.used {
            self.xpc = self.xpc.reconnect().await?;
        }
        self.used = true;
        Ok(&mut self.xpc)
    }

    async fn invoke(
        &mut self,
        routine: &'static str,
        arguments: IndexMap<String, XpcValue>,
    ) -> Result<IndexMap<String, XpcValue>> {
        tokio::time::timeout(ROUTINE_TIMEOUT, async {
            let xpc = self.connection().await?;
            xpc.send_request(request(routine, arguments)).await?;
            unwrap_response(routine, receive_response(xpc).await?)
        })
        .await
        .map_err(|_| CryptexError::Timeout(routine))?
    }

    pub async fn copy_installed(&mut self) -> Result<Vec<InstalledCryptex>> {
        let result = self.invoke("copy-installed", IndexMap::new()).await?;
        parse_installed(&result)
    }

    pub async fn installed_ddi(&mut self) -> Result<Option<InstalledCryptex>> {
        Ok(self
            .copy_installed()
            .await?
            .into_iter()
            .find(|entry| entry.identifier == DDI_IDENTIFIER))
    }

    pub async fn read_personalization_identifiers(&mut self) -> Result<plist::Dictionary> {
        self.require_feature(FEATURE_IDENTIFIERS)?;
        let result = self
            .invoke("read-personalization-id", IndexMap::new())
            .await?;
        result
            .into_iter()
            .map(|(key, value)| {
                let value = match value {
                    XpcValue::Uint64(value) => plist::Value::Integer(value.into()),
                    XpcValue::Int64(value) => plist::Value::Integer(value.into()),
                    XpcValue::Bool(value) => plist::Value::Boolean(value),
                    XpcValue::Data(value) => plist::Value::Data(value.to_vec()),
                    XpcValue::String(value) => plist::Value::String(value),
                    _ => {
                        return Err(CryptexError::Protocol(
                            "unsupported personalization identifier type".into(),
                        ))
                    }
                };
                Ok((key, value))
            })
            .collect()
    }

    /// Read and validate the wrapped nonce; callers receive only the nonce bytes.
    pub async fn nonce(&mut self, domain: NonceDomain) -> Result<Vec<u8>> {
        let result = self.invoke("get-nonce", domain.arguments()).await?;
        match result.get("nonce") {
            Some(XpcValue::Data(data)) => unwrap_nonce(data),
            _ => Err(CryptexError::Protocol("get-nonce has no nonce data".into())),
        }
    }

    pub async fn uninstall(&mut self, identifier: &str, version: Option<&str>) -> Result<()> {
        self.require_feature(FEATURE_INSTALL)?;
        if identifier.is_empty()
            || identifier.len() > 1024
            || version.is_some_and(|value| value.is_empty() || value.len() > 1024)
        {
            return Err(CryptexError::Protocol(
                "invalid Cryptex identifier or version".into(),
            ));
        }
        let mut arguments = IndexMap::from([(
            "remote-cryptex-identifier".into(),
            XpcValue::String(identifier.into()),
        )]);
        if let Some(version) = version {
            arguments.insert(
                "remote-cryptex-version".into(),
                XpcValue::String(version.into()),
            );
        }
        self.invoke("uninstall", arguments).await?;
        Ok(())
    }

    /// Install a signed DDI, sending five announced transfers on odd H2 streams.
    /// The caller must obtain a Cryptex1 ticket using this identity's nonce handle.
    pub async fn install(&mut self, assets: &CryptexDdiAssets, ticket: &[u8]) -> Result<()> {
        self.require_feature(FEATURE_INSTALL)?;
        if ticket.is_empty() || ticket.len() as u64 > assets::MAX_METADATA_BYTES {
            return Err(CryptexError::Protocol(
                "Cryptex ticket is empty or oversized".into(),
            ));
        }
        let mut files = Vec::with_capacity(4);
        let mut sizes = Vec::with_capacity(4);
        for (index, path) in assets.paths.iter().enumerate() {
            let limit = if index == 0 {
                assets::MAX_IMAGE_BYTES
            } else {
                assets::MAX_METADATA_BYTES
            };
            let file = tokio::fs::File::open(path).await?;
            let metadata = file.metadata().await?;
            if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
                return Err(CryptexError::Protocol(
                    "Cryptex payload is empty, oversized, or not a regular file".into(),
                ));
            }
            sizes.push(metadata.len());
            files.push(file);
        }
        let lengths = [sizes[0], sizes[1], ticket.len() as u64, sizes[2], sizes[3]];
        let arguments = install_arguments(assets.properties()?, lengths);
        tokio::time::timeout(INSTALL_TIMEOUT, async {
            let xpc = self.connection().await?;
            xpc.send_request(request("install", arguments)).await?;
            xpc.send_file_transfer(1, lengths[0], &mut files[0]).await?;
            xpc.send_file_transfer(2, lengths[1], &mut files[1]).await?;
            xpc.send_file_transfer(3, lengths[2], &mut std::io::Cursor::new(ticket))
                .await?;
            xpc.send_file_transfer(4, lengths[3], &mut files[2]).await?;
            xpc.send_file_transfer(5, lengths[4], &mut files[3]).await?;
            unwrap_response("install", receive_response(xpc).await?)?;
            Ok(())
        })
        .await
        .map_err(|_| CryptexError::Timeout("install"))?
    }
}

/// Personalize, install, and verify the iOS 27 DDI. Existing mounter images are
/// an explicit conflict, never evidence that the Cryptex is ready.
pub async fn auto_install_ddi(
    device: &ConnectedDevice,
    restore_dir: Option<&Path>,
    cache_dir: Option<&Path>,
) -> Result<InstalledCryptex> {
    if device.product_version().await?.major < 27 {
        return Err(CryptexError::Unsupported(
            "DDI Cryptex installation requires iOS 27+",
        ));
    }
    let mut client = CryptexClient::connect(device).await?;
    client.require_feature(FEATURE_INSTALL)?;
    client.require_feature(FEATURE_IDENTIFIERS)?;
    if let Some(installed) = client.installed_ddi().await? {
        return Err(CryptexError::Conflict(format!(
            "{} {} is already installed",
            installed.identifier, installed.version
        )));
    }
    tokio::time::timeout(ROUTINE_TIMEOUT, async {
        let stream = device.connect_rsd_service(crate::imagemounter::protocol::SERVICE_NAME).await?;
        let mut mounter = crate::imagemounter::ImageMounterClient::new(stream);
        if !mounter.query_developer_mode_status().await? {
            return Err(CryptexError::Protocol("Developer Mode is disabled".into()));
        }
        if !mounter.lookup_image_signatures("Personalized").await?.is_empty() {
            return Err(CryptexError::Conflict("a Personalized image owns /System/Developer; unmount it before installing a Cryptex DDI".into()));
        }
        Ok(())
    }).await.map_err(|_| CryptexError::Timeout("image mounter preflight"))??;
    let assets = match restore_dir {
        Some(path) => CryptexDdiAssets::load(path).await?,
        None => CryptexDdiAssets::download(cache_dir).await?,
    };
    let identifiers = client.read_personalization_identifiers().await?;
    let nonce = client
        .nonce(NonceDomain::Handle(assets.nonce_domain_handle()?))
        .await?;
    let request = build_cryptex_tss_request(&assets.build_identity, &identifiers, &nonce)?;
    let ticket = crate::imagemounter::tss::get_cryptex_tss_ticket(&request).await?;
    client.install(&assets, &ticket).await?;
    client.installed_ddi().await?.ok_or_else(|| {
        CryptexError::Protocol("install reported success but the DDI Cryptex is absent".into())
    })
}

fn request(routine: &str, arguments: IndexMap<String, XpcValue>) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        ("routine".into(), XpcValue::String(routine.into())),
        ("argv".into(), XpcValue::Dictionary(arguments)),
    ]))
}

fn install_arguments(properties: XpcValue, sizes: [u64; 5]) -> IndexMap<String, XpcValue> {
    let mut arguments = IndexMap::from([
        ("auth".into(), XpcValue::Uint64(0)),
        ("client-version".into(), XpcValue::Uint64(3)),
        ("image-type-index".into(), XpcValue::Int64(10)),
        ("persistence".into(), XpcValue::Uint64(2)),
        ("nonce-persistence".into(), XpcValue::Uint64(1)),
        ("cryptex1-properties".into(), properties),
    ]);
    for (index, key) in ["image", "trustcache", "im4m", "info", "volumehash"]
        .iter()
        .enumerate()
    {
        arguments.insert(
            (*key).into(),
            XpcValue::FileTransfer {
                msg_id: index as u64 + 1,
                data: Box::new(XpcValue::Dictionary(IndexMap::from([(
                    "s".into(),
                    XpcValue::Uint64(sizes[index]),
                )]))),
            },
        );
    }
    arguments
}

async fn receive_response(xpc: &mut XpcClient) -> Result<XpcMessage> {
    // Transfer acknowledgements are bodyless XPC preambles. Bound even those
    // so an invalid daemon cannot spin forever inside the operation timeout.
    for _ in 0..32 {
        let response = xpc.recv_any().await?;
        if response.body.is_some() {
            return Ok(response);
        }
    }
    Err(CryptexError::Protocol(
        "too many empty Cryptex responses".into(),
    ))
}

fn unwrap_response(routine: &str, response: XpcMessage) -> Result<IndexMap<String, XpcValue>> {
    let Some(XpcValue::Dictionary(mut response)) = response.body else {
        return Err(CryptexError::Protocol(
            "Cryptex reply is not a dictionary".into(),
        ));
    };
    if let Some(cferr) = response.get("cferr") {
        let description = cferr
            .as_dict()
            .and_then(|error| error.get("cferr_userinfo"))
            .and_then(XpcValue::as_dict)
            .and_then(|info| info.get("NSLocalizedDescription"))
            .and_then(XpcValue::as_str)
            .unwrap_or("device returned a CFError");
        return Err(CryptexError::Device(format!(
            "{routine}: {}",
            description.chars().take(512).collect::<String>()
        )));
    }
    match response.get("error") {
        Some(XpcValue::Int64(0) | XpcValue::Uint64(0)) => {}
        Some(XpcValue::Int64(code)) => {
            return Err(CryptexError::Device(format!(
                "{routine}: Darwin errno {code}"
            )))
        }
        Some(XpcValue::Uint64(code)) => {
            return Err(CryptexError::Device(format!(
                "{routine}: Darwin errno {code}"
            )))
        }
        None if routine == "read-personalization-id" => {}
        _ => {
            return Err(CryptexError::Protocol(format!(
                "{routine}: missing or invalid error status"
            )))
        }
    }
    match response.swap_remove("argv") {
        Some(XpcValue::Dictionary(arguments)) => Ok(arguments),
        None | Some(XpcValue::Null) if routine != "read-personalization-id" => Ok(IndexMap::new()),
        _ => Err(CryptexError::Protocol(format!("{routine}: invalid argv"))),
    }
}

fn parse_installed(arguments: &IndexMap<String, XpcValue>) -> Result<Vec<InstalledCryptex>> {
    let entries = match arguments.get("remote-cryptex-array") {
        None => return Ok(Vec::new()),
        Some(XpcValue::Array(entries)) if entries.len() <= 1024 => entries,
        _ => {
            return Err(CryptexError::Protocol(
                "invalid installed Cryptex array".into(),
            ))
        }
    };
    entries
        .iter()
        .map(|entry| {
            let entry = entry
                .as_dict()
                .ok_or_else(|| CryptexError::Protocol("invalid installed Cryptex entry".into()))?;
            let field = |key| {
                entry
                    .get(key)
                    .and_then(XpcValue::as_str)
                    .filter(|value| !value.is_empty() && value.len() <= 1024)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        CryptexError::Protocol(
                            "missing installed Cryptex identifier or version".into(),
                        )
                    })
            };
            Ok(InstalledCryptex {
                identifier: field("remote-cryptex-identifier")?,
                version: field("remote-cryptex-version")?,
            })
        })
        .collect()
}

fn unwrap_nonce(blob: &[u8]) -> Result<Vec<u8>> {
    if !(7..=1024).contains(&blob.len()) {
        return Err(CryptexError::Protocol(
            "invalid Cryptex nonce structure length".into(),
        ));
    }
    let length = u32::from_le_bytes(
        blob[blob.len() - 4..]
            .try_into()
            .expect("four-byte nonce length"),
    ) as usize;
    if length == 0 || length > blob.len() - 6 || length > 64 {
        return Err(CryptexError::Protocol(
            "invalid Cryptex nonce length".into(),
        ));
    }
    Ok(blob[2..2 + length].to_vec())
}

#[cfg(test)]
mod tests;

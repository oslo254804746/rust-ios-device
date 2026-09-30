//! Manifest-directed Cryptex DDI assets. Large images remain on disk.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use indexmap::IndexMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{CryptexError, Result};
use crate::xpc::message::XpcValue;

pub const LATEST_CRYPTEX_DDI_BUILD_ID: &str = "27A5228h";
const REPOSITORY: &str = "https://raw.githubusercontent.com/doronz88/DeveloperDiskImage/main/PersonalizedImages/Xcode_iOS_DDI_Cryptex";
pub(super) const MAX_METADATA_BYTES: u64 = 16 * 1024 * 1024;
pub(super) const MAX_IMAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const PAYLOADS: [(&str, &str); 4] = [
    ("Cryptex1,GenericDmg", "Image.dmg"),
    ("Cryptex1,GenericTrustCache", "Image.dmg.trustcache"),
    ("Cryptex1,CryptexInfoPlist", "Image.dmg.cryptex_info"),
    ("Cryptex1,GenericVolume", "Image.dmg.root_hash"),
];

/// A validated unpacked DDI Restore directory. Payloads are streamed on install.
pub struct CryptexDdiAssets {
    pub build_identity: plist::Dictionary,
    pub build_id: String,
    pub(super) paths: [PathBuf; 4],
}

impl CryptexDdiAssets {
    /// Load either Xcode's Restore directory or a downloaded normalized bundle.
    pub async fn load(restore_dir: &Path) -> Result<Self> {
        let root = tokio::fs::canonicalize(restore_dir).await?;
        let manifest_path = safe_payload_path(&root, "BuildManifest.plist").await?;
        let data = read_metadata(&manifest_path).await?;
        let (build_id, identity) = parse_identity(&data)?;
        let mut paths = Vec::with_capacity(4);
        for (index, (key, _)) in PAYLOADS.iter().enumerate() {
            let path = safe_payload_path(&root, payload_name(&identity, key)?).await?;
            validate_file(
                &path,
                if index == 0 {
                    MAX_IMAGE_BYTES
                } else {
                    MAX_METADATA_BYTES
                },
            )
            .await?;
            paths.push(path);
        }
        let assets = Self {
            build_identity: identity,
            build_id,
            paths: paths
                .try_into()
                .map_err(|_| CryptexError::Protocol("invalid asset count".into()))?,
        };
        // Validate the install arguments before contacting a device or TSS.
        assets.properties()?;
        Ok(assets)
    }

    /// Download and atomically cache the published Cryptex bundle.
    pub async fn download(cache_dir: Option<&Path>) -> Result<Self> {
        let base = cache_dir.map(Path::to_path_buf).unwrap_or_else(|| {
            dirs_next::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ios-rs/ddi")
        });
        let destination = base.join(format!("cryptex-{LATEST_CRYPTEX_DDI_BUILD_ID}"));
        if let Ok(assets) = Self::load(&destination).await {
            if assets.build_id == LATEST_CRYPTEX_DDI_BUILD_ID {
                return Ok(assets);
            }
        }
        tokio::fs::create_dir_all(&base).await?;
        let stage = base.join(format!(".cryptex-download-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&stage).await?;
        let _cleanup = DownloadStage(stage.clone());
        let result = download_into(&stage).await;
        if let Err(error) = result {
            let _ = tokio::fs::remove_dir_all(&stage).await;
            return Err(error);
        }
        // Never replace a possibly in-use cache. Incomplete existing directories
        // are reported explicitly so a concurrent installation cannot lose files.
        if let Err(error) = tokio::fs::rename(&stage, &destination).await {
            let _ = tokio::fs::remove_dir_all(&stage).await;
            if let Ok(assets) = Self::load(&destination).await {
                if assets.build_id == LATEST_CRYPTEX_DDI_BUILD_ID {
                    return Ok(assets);
                }
            }
            return Err(CryptexError::Protocol(format!(
                "cannot publish Cryptex cache; remove the incomplete {} directory: {error}",
                destination.display()
            )));
        }
        Self::load(&destination).await
    }

    pub fn nonce_domain_handle(&self) -> Result<u64> {
        integer(&self.build_identity, "Cryptex1,NonceDomain")
    }

    pub(super) fn properties(&self) -> Result<XpcValue> {
        let identity = &self.build_identity;
        let mut properties = IndexMap::new();
        properties.insert("MountedCryptex".to_string(), XpcValue::Bool(false));
        properties.insert(
            "Cryptex1,UseProductClass".to_string(),
            XpcValue::Bool(boolean(identity, "Cryptex1,UseProductClass")?),
        );
        for key in ["Cryptex1,SubType", "Cryptex1,NonceDomain"] {
            properties.insert(key.into(), XpcValue::Uint64(integer(identity, key)?));
        }
        properties.insert(
            "Cryptex1,Version".to_string(),
            XpcValue::String(string(identity, "Cryptex1,Version")?.into()),
        );
        properties.insert(
            "Cryptex1,PreauthVersion".to_string(),
            XpcValue::String(string(identity, "Cryptex1,PreauthorizationVersion")?.into()),
        );
        Ok(XpcValue::Dictionary(properties))
    }
}

// A cancelled download must not retain a partial multi-gigabyte image. This
// private directory contains only our five files, so cleanup remains small.
struct DownloadStage(PathBuf);

impl Drop for DownloadStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn download_into(stage: &Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| CryptexError::Download(e.to_string()))?;
    fetch(
        &client,
        "BuildManifest.plist",
        &stage.join("BuildManifest.plist"),
        MAX_METADATA_BYTES,
    )
    .await?;
    let (build, identity) =
        parse_identity(&read_metadata(&stage.join("BuildManifest.plist")).await?)?;
    if build != LATEST_CRYPTEX_DDI_BUILD_ID {
        return Err(CryptexError::Download(format!("published Cryptex build {build} differs from expected {LATEST_CRYPTEX_DDI_BUILD_ID}; use ddi mount --path with a matching Restore directory")));
    }
    for (index, (key, published)) in PAYLOADS.iter().enumerate() {
        let name = payload_name(&identity, key)?;
        validate_relative_path(name)?;
        let path = stage.join(name);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        fetch(
            &client,
            published,
            &path,
            if index == 0 {
                MAX_IMAGE_BYTES
            } else {
                MAX_METADATA_BYTES
            },
        )
        .await?;
    }
    CryptexDdiAssets::load(stage).await?;
    Ok(())
}

async fn fetch(client: &reqwest::Client, name: &str, destination: &Path, limit: u64) -> Result<()> {
    let mut response = client
        .get(format!("{REPOSITORY}/{name}"))
        .send()
        .await
        .map_err(|e| CryptexError::Download(e.to_string()))?;
    if !response.status().is_success() || response.content_length().is_some_and(|size| size > limit)
    {
        return Err(CryptexError::Download(format!(
            "invalid {name} download status or size ({})",
            response.status()
        )));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .await?;
    let mut received = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| CryptexError::Download(e.to_string()))?
    {
        received += chunk.len() as u64;
        if received > limit {
            return Err(CryptexError::Download(format!(
                "{name} exceeds download limit"
            )));
        }
        file.write_all(&chunk).await?;
    }
    if received == 0 {
        return Err(CryptexError::Download(format!("{name} is empty")));
    }
    file.flush().await?;
    Ok(())
}

fn parse_identity(data: &[u8]) -> Result<(String, plist::Dictionary)> {
    let value: plist::Value = plist::from_bytes(data)?;
    let root = value
        .as_dictionary()
        .ok_or_else(|| CryptexError::Protocol("BuildManifest is not a dictionary".into()))?;
    let build = string(root, "ProductBuildVersion")?.to_string();
    let identities = root
        .get("BuildIdentities")
        .and_then(plist::Value::as_array)
        .ok_or_else(|| CryptexError::Protocol("missing BuildIdentities".into()))?;
    let candidates: Vec<_> = identities
        .iter()
        .filter_map(plist::Value::as_dictionary)
        .filter(|identity| {
            identity
                .get("Info")
                .and_then(plist::Value::as_dictionary)
                .and_then(|info| info.get("Variant"))
                .and_then(plist::Value::as_string)
                .is_some_and(|variant| variant.ends_with("Developer Disk Image Cryptex"))
        })
        .collect();
    if candidates.len() != 1 {
        return Err(CryptexError::Protocol(
            "manifest must have exactly one Developer Disk Image Cryptex identity".into(),
        ));
    }
    Ok((build, candidates[0].clone()))
}

fn payload_name<'a>(identity: &'a plist::Dictionary, key: &str) -> Result<&'a str> {
    identity
        .get("Manifest")
        .and_then(plist::Value::as_dictionary)
        .and_then(|entries| entries.get(key))
        .and_then(plist::Value::as_dictionary)
        .and_then(|entry| entry.get("Info"))
        .and_then(plist::Value::as_dictionary)
        .and_then(|info| info.get("Path"))
        .and_then(plist::Value::as_string)
        .ok_or_else(|| CryptexError::Protocol(format!("missing {key} payload path")))
}

fn validate_relative_path(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains(['\\', ':'])
        || Path::new(name)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(CryptexError::Protocol("unsafe Cryptex payload path".into()));
    }
    Ok(())
}

async fn safe_payload_path(root: &Path, name: &str) -> Result<PathBuf> {
    validate_relative_path(name)?;
    let path = tokio::fs::canonicalize(root.join(name)).await?;
    if !path.starts_with(root) {
        return Err(CryptexError::Protocol(
            "Cryptex payload escapes Restore directory".into(),
        ));
    }
    Ok(path)
}

pub(super) async fn validate_file(path: &Path, limit: u64) -> Result<u64> {
    let meta = tokio::fs::metadata(path).await?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > limit {
        return Err(CryptexError::Protocol(
            "Cryptex payload is empty, oversized, or not a regular file".into(),
        ));
    }
    Ok(meta.len())
}

async fn read_metadata(path: &Path) -> Result<Vec<u8>> {
    validate_file(path, MAX_METADATA_BYTES).await?;
    let mut data = Vec::new();
    tokio::fs::File::open(path)
        .await?
        .take(MAX_METADATA_BYTES + 1)
        .read_to_end(&mut data)
        .await?;
    if data.len() as u64 > MAX_METADATA_BYTES {
        return Err(CryptexError::Protocol(
            "Cryptex metadata grew past size limit".into(),
        ));
    }
    Ok(data)
}

pub(super) fn integer(dictionary: &plist::Dictionary, key: &str) -> Result<u64> {
    let value = match dictionary.get(key) {
        Some(plist::Value::Integer(value)) => value.as_unsigned(),
        Some(plist::Value::String(value)) => value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .map(|value| u64::from_str_radix(value, 16).ok())
            .unwrap_or_else(|| value.parse().ok()),
        _ => None,
    };
    value.ok_or_else(|| CryptexError::Protocol(format!("missing or invalid unsigned {key}")))
}

pub(super) fn string<'a>(dictionary: &'a plist::Dictionary, key: &str) -> Result<&'a str> {
    dictionary
        .get(key)
        .and_then(plist::Value::as_string)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CryptexError::Protocol(format!("missing or invalid {key}")))
}

pub(super) fn boolean(dictionary: &plist::Dictionary, key: &str) -> Result<bool> {
    dictionary
        .get(key)
        .and_then(plist::Value::as_boolean)
        .ok_or_else(|| CryptexError::Protocol(format!("missing or invalid boolean {key}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_payload_traversal_on_all_hosts() {
        for name in [
            "",
            "../image",
            "/tmp/image",
            "Firmware/../../image",
            "C:\\image",
            "Firmware\\image",
        ] {
            assert!(validate_relative_path(name).is_err(), "{name}");
        }
        assert!(validate_relative_path("Firmware/Image.dmg.trustcache").is_ok());
    }

    #[test]
    fn rejects_missing_or_ambiguous_cryptex_identity() {
        let identity = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Info".to_string(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Variant".to_string(),
                plist::Value::String("iOS Customer Developer Disk Image Cryptex".into()),
            )])),
        )]));
        for identities in [vec![], vec![identity.clone(), identity.clone()]] {
            let mut data = Vec::new();
            plist::to_writer_xml(
                &mut data,
                &plist::Dictionary::from_iter([
                    ("ProductBuildVersion".to_string(), "fixture".into()),
                    (
                        "BuildIdentities".to_string(),
                        plist::Value::Array(identities),
                    ),
                ]),
            )
            .unwrap();
            assert!(parse_identity(&data).is_err());
        }
    }

    #[tokio::test]
    async fn rejects_oversized_or_nonregular_payloads_before_reading() {
        let root = std::env::temp_dir().join(format!("ios-cryptex-size-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&root).await.unwrap();
        let path = root.join("image");
        let file = tokio::fs::File::create(&path).await.unwrap();
        assert!(validate_file(&path, MAX_IMAGE_BYTES).await.is_err());
        file.set_len(MAX_IMAGE_BYTES + 1).await.unwrap();
        assert!(validate_file(&path, MAX_IMAGE_BYTES).await.is_err());
        assert!(validate_file(&root, MAX_IMAGE_BYTES).await.is_err());
        drop(file);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn discarded_download_stage_removes_partial_payloads() {
        let stage =
            std::env::temp_dir().join(format!("ios-cryptex-cancel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&stage).unwrap();
        std::fs::write(stage.join("Image.dmg"), b"partial").unwrap();
        drop(DownloadStage(stage.clone()));
        assert!(!stage.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinks_escaping_restore_directory() {
        let root = std::env::temp_dir().join(format!("ios-cryptex-link-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(root.join("Restore"))
            .await
            .unwrap();
        tokio::fs::write(root.join("outside"), b"fixture")
            .await
            .unwrap();
        std::os::unix::fs::symlink(root.join("outside"), root.join("Restore/image")).unwrap();
        let restore = tokio::fs::canonicalize(root.join("Restore")).await.unwrap();
        assert!(safe_payload_path(&restore, "image").await.is_err());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

//! DDI download and caching.
//!
//! - iOS 17+: personalized DDI from `https://deviceboxhq.com/ddi-15F31d.zip`
//! - iOS <17: standard DDI from GitHub `mspvirajpatel/Xcode_Developer_Disk_Images`
//!
//! Cache directory: `~/.ios-rs/ddi/`
//!
//! Reference: go-ios/ios/imagemounter/imagedownloader.go

use std::path::{Path, PathBuf};

use super::protocol::ImageMounterError;

const DDI_PERSONALIZED_URL: &str = "https://deviceboxhq.com/ddi-15F31d.zip";
const DDI_GITHUB_RELEASES: &str =
    "https://github.com/mspvirajpatel/Xcode_Developer_Disk_Images/releases/download";

/// DDI downloader with local caching.
pub struct DdiDownloader {
    cache_dir: PathBuf,
}

/// Downloaded DDI contents for standard (pre-iOS 17) mounting.
pub struct StandardDdi {
    pub image: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Downloaded DDI contents for personalized (iOS 17+) mounting.
pub struct PersonalizedDdi {
    pub image: Vec<u8>,
    pub trustcache: Vec<u8>,
    pub build_manifest: Vec<u8>,
}

impl DdiDownloader {
    pub fn new(cache_dir: Option<PathBuf>) -> Self {
        let cache_dir = cache_dir.unwrap_or_else(|| {
            dirs_next::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".ios-rs")
                .join("ddi")
        });
        Self { cache_dir }
    }

    /// Download a standard DDI for iOS <17.
    ///
    /// Version matching: tries exact match first (e.g. "16.4"), then major (e.g. "16.0").
    pub async fn download_standard(
        &self,
        version: &semver::Version,
    ) -> Result<StandardDdi, ImageMounterError> {
        let exact = format!("{}.{}", version.major, version.minor);
        let fallback = format!("{}.0", version.major);

        // Check cache first
        for ver_str in &[&exact, &fallback] {
            let dir = self.cache_dir.join("standard").join(ver_str);
            let img_path = dir.join("DeveloperDiskImage.dmg");
            let sig_path = dir.join("DeveloperDiskImage.dmg.signature");
            if img_path.exists() && sig_path.exists() {
                tracing::info!("Using cached DDI from {}", dir.display());
                let image = tokio::fs::read(&img_path)
                    .await
                    .map_err(|e| ImageMounterError::Download(format!("read cached image: {e}")))?;
                let signature = tokio::fs::read(&sig_path).await.map_err(|e| {
                    ImageMounterError::Download(format!("read cached signature: {e}"))
                })?;
                return Ok(StandardDdi { image, signature });
            }
        }

        // Download from GitHub Releases as zip
        for ver_str in &[&exact, &fallback] {
            let zip_url = format!("{DDI_GITHUB_RELEASES}/{ver_str}/{ver_str}.zip");

            let client = build_client()?;
            tracing::info!("Downloading DDI from {zip_url}");
            let resp = client
                .get(&zip_url)
                .send()
                .await
                .map_err(|e| ImageMounterError::Download(format!("fetch DDI zip: {e}")))?;

            if !resp.status().is_success() {
                tracing::debug!("DDI not found at {zip_url} (HTTP {})", resp.status());
                continue;
            }

            let zip_bytes = resp
                .bytes()
                .await
                .map_err(|e| ImageMounterError::Download(format!("read DDI zip: {e}")))?;

            // Extract DMG and signature from zip
            let (image, signature) = extract_standard_ddi(&zip_bytes, ver_str)?;

            // Cache
            let dir = self.cache_dir.join("standard").join(ver_str);
            if let Err(e) = cache_files(
                &dir,
                &[
                    ("DeveloperDiskImage.dmg", &image),
                    ("DeveloperDiskImage.dmg.signature", &signature),
                ],
            )
            .await
            {
                tracing::warn!("Failed to cache DDI: {e}");
            }

            return Ok(StandardDdi { image, signature });
        }

        Err(ImageMounterError::Download(format!(
            "DDI not found for iOS {exact} or {fallback}"
        )))
    }

    /// Download a personalized DDI for iOS 17+.
    pub async fn download_personalized(&self) -> Result<PersonalizedDdi, ImageMounterError> {
        let dir = self.cache_dir.join("personalized");

        // Check cache
        let img_path = dir.join("Restore").join("PersonalizedDMG.dmg");
        let tc_path = dir.join("Restore").join("PersonalizedDMG.trustcache");
        let bm_path = dir.join("Restore").join("BuildManifest.plist");

        if img_path.exists() && tc_path.exists() && bm_path.exists() {
            tracing::info!("Using cached personalized DDI from {}", dir.display());
            return load_personalized_from_dir(&dir).await;
        }

        // Download zip
        eprintln!("Downloading personalized DDI...");
        let client = build_client()?;
        let resp = client
            .get(DDI_PERSONALIZED_URL)
            .send()
            .await
            .map_err(|e| ImageMounterError::Download(format!("fetch DDI zip: {e}")))?;

        if !resp.status().is_success() {
            return Err(ImageMounterError::Download(format!(
                "DDI download failed: HTTP {}",
                resp.status()
            )));
        }

        tokio::fs::create_dir_all(&dir).await.map_err(|e| {
            ImageMounterError::Download(format!("create personalized DDI cache dir: {e}"))
        })?;
        let zip_path = dir.join("ddi.zip.tmp");
        download_response_to_file(resp, &zip_path).await?;

        // Extract zip (blocking zip decompression + filesystem writes)
        let dir_clone = dir.clone();
        let zip_path_clone = zip_path.clone();
        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&zip_path_clone)
                .map_err(|e| ImageMounterError::Download(format!("open DDI zip: {e}")))?;
            extract_zip_from_reader(std::io::BufReader::new(file), &dir_clone)
        })
        .await
        .map_err(|e| ImageMounterError::Download(format!("join error: {e}")))??;
        let _ = tokio::fs::remove_file(&zip_path).await;

        load_personalized_from_dir(&dir).await
    }
}

async fn download_response_to_file(
    mut response: reqwest::Response,
    path: &Path,
) -> Result<(), ImageMounterError> {
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|e| ImageMounterError::Download(format!("create DDI zip cache: {e}")))?;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ImageMounterError::Download(format!("read DDI zip chunk: {e}")))?
    {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| ImageMounterError::Download(format!("write DDI zip chunk: {e}")))?;
    }
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| ImageMounterError::Download(format!("flush DDI zip: {e}")))?;
    Ok(())
}

async fn load_personalized_from_dir(dir: &Path) -> Result<PersonalizedDdi, ImageMounterError> {
    let restore = dir.join("Restore");

    let image = tokio::fs::read(restore.join("PersonalizedDMG.dmg"))
        .await
        .map_err(|e| ImageMounterError::Download(format!("read PersonalizedDMG.dmg: {e}")))?;
    let trustcache = tokio::fs::read(restore.join("PersonalizedDMG.trustcache"))
        .await
        .map_err(|e| ImageMounterError::Download(format!("read trustcache: {e}")))?;
    let build_manifest = tokio::fs::read(restore.join("BuildManifest.plist"))
        .await
        .map_err(|e| ImageMounterError::Download(format!("read BuildManifest.plist: {e}")))?;

    Ok(PersonalizedDdi {
        image,
        trustcache,
        build_manifest,
    })
}

#[cfg(test)]
fn extract_zip(data: &[u8], dest: &Path) -> Result<(), ImageMounterError> {
    extract_zip_from_reader(std::io::Cursor::new(data), dest)
}

fn extract_zip_from_reader<R>(reader: R, dest: &Path) -> Result<(), ImageMounterError>
where
    R: std::io::Read + std::io::Seek,
{
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|e| ImageMounterError::Download(format!("open zip: {e}")))?;
    extract_zip_archive(&mut archive, dest)
}

fn extract_zip_archive<R>(
    archive: &mut zip::ZipArchive<R>,
    dest: &Path,
) -> Result<(), ImageMounterError>
where
    R: std::io::Read + std::io::Seek,
{
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ImageMounterError::Download(format!("read zip entry: {e}")))?;

        let entry_name = file.name().to_string();
        // Skip directories and __MACOSX entries
        if file.is_dir() || entry_name.starts_with("__MACOSX") {
            continue;
        }

        if file.enclosed_name().is_none() {
            tracing::warn!(entry = %entry_name, "skipping unsafe DDI zip entry");
            continue;
        }
        let normalized_name = entry_name.replace('\\', "/");
        let raw_components = normalized_name
            .split('/')
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>();
        let Some(restore_pos) = raw_components
            .iter()
            .position(|component| component.eq_ignore_ascii_case("Restore"))
        else {
            continue;
        };
        let restore_tail = &raw_components[restore_pos + 1..];
        if restore_tail.is_empty()
            || restore_tail
                .iter()
                .any(|component| matches!(*component, "." | ".."))
        {
            tracing::warn!(entry = %entry_name, "skipping unsafe DDI Restore entry");
            continue;
        }
        let mut rel_path = PathBuf::from("Restore");
        rel_path.extend(restore_tail);
        let out_path = dest.join(rel_path);

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ImageMounterError::Download(format!("create dir: {e}")))?;
        }

        let mut out_file = std::fs::File::create(&out_path).map_err(|e| {
            ImageMounterError::Download(format!("create file {}: {e}", out_path.display()))
        })?;
        std::io::copy(&mut file, &mut out_file)
            .map_err(|e| ImageMounterError::Download(format!("extract {}: {e}", entry_name)))?;
    }

    Ok(())
}

async fn cache_files(dir: &Path, files: &[(&str, &[u8])]) -> Result<(), ImageMounterError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| ImageMounterError::Download(format!("create cache dir: {e}")))?;
    for (name, data) in files {
        tokio::fs::write(dir.join(name), data)
            .await
            .map_err(|e| ImageMounterError::Download(format!("write {name}: {e}")))?;
    }
    Ok(())
}

/// Build an HTTP client, respecting HTTP_PROXY / HTTPS_PROXY env vars.
fn build_client() -> Result<reqwest::Client, ImageMounterError> {
    let mut builder = reqwest::Client::builder();
    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("HTTP_PROXY")) {
        let proxy = reqwest::Proxy::all(&proxy_url)
            .map_err(|e| ImageMounterError::Download(format!("invalid proxy URL: {e}")))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|e| ImageMounterError::Download(format!("build client: {e}")))
}

/// Extract DeveloperDiskImage.dmg and .signature from a GitHub Releases zip.
///
/// The zip contains a directory like `15.5/DeveloperDiskImage.dmg` and
/// `15.5/DeveloperDiskImage.dmg.signature`.
fn extract_standard_ddi(data: &[u8], _ver: &str) -> Result<(Vec<u8>, Vec<u8>), ImageMounterError> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| ImageMounterError::Download(format!("open zip: {e}")))?;

    let mut image: Option<Vec<u8>> = None;
    let mut signature: Option<Vec<u8>> = None;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ImageMounterError::Download(format!("read zip entry: {e}")))?;
        if file.is_dir() {
            continue;
        }
        let name = file.name().to_string();
        let base = name.split('/').next_back().unwrap_or("");
        if base == "DeveloperDiskImage.dmg" {
            let mut buf = Vec::new();
            std::io::copy(&mut file, &mut buf)
                .map_err(|e| ImageMounterError::Download(format!("extract dmg: {e}")))?;
            image = Some(buf);
        } else if base == "DeveloperDiskImage.dmg.signature" {
            let mut buf = Vec::new();
            std::io::copy(&mut file, &mut buf)
                .map_err(|e| ImageMounterError::Download(format!("extract sig: {e}")))?;
            signature = Some(buf);
        }
    }

    let image = image.ok_or_else(|| {
        ImageMounterError::Download("DeveloperDiskImage.dmg not found in zip".into())
    })?;
    let signature = signature.ok_or_else(|| {
        ImageMounterError::Download("DeveloperDiskImage.dmg.signature not found in zip".into())
    })?;
    Ok((image, signature))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut bytes);
            let mut zip = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            for (name, data) in entries {
                zip.start_file(*name, options).unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        bytes
    }

    #[test]
    fn extract_zip_skips_restore_entries_that_escape_destination() {
        let root = std::env::temp_dir().join(format!("ios-rs-ddi-escape-{}", std::process::id()));
        let dest = root.join("cache");
        let outside = root.join("outside");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&dest).unwrap();

        let zip = zip_with_entries(&[
            ("ddi/Restore/../../outside", b"escape"),
            ("ddi/Restore/PersonalizedDMG.trustcache", b"safe"),
        ]);

        extract_zip(&zip, &dest).unwrap();

        assert!(!outside.exists());
        assert_eq!(
            std::fs::read(dest.join("Restore").join("PersonalizedDMG.trustcache")).unwrap(),
            b"safe"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

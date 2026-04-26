use std::path::PathBuf;

use anyhow::Result;
use plist::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestResult {
    personalized_image_type: String,
    image_signature: Vec<u8>,
    manifest: Vec<u8>,
}

#[derive(clap::Args)]
pub struct DdiCmd {
    #[command(subcommand)]
    sub: DdiSub,
}

#[derive(clap::Subcommand)]
enum DdiSub {
    /// Detect the device version, download a matching DDI, and mount it
    Auto {
        /// Cache directory for downloaded DDIs
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Check if a developer disk image is mounted
    Status,
    /// List mounted developer disk image signatures
    List,
    /// List raw mounted image entries reported by mobile_image_mounter
    Devices,
    /// Query mounted signatures for a specific image type
    Lookup {
        /// Image type, such as Developer or Personalized
        image_type: String,
    },
    /// Query personalization manifests for mounted personalized image entries
    PersonalizationManifests,
    /// Query whether developer mode is enabled
    DevmodeStatus,
    /// Query the personalized image nonce used for iOS 17+ DDI personalization
    Nonce {
        /// Personalized image type sent with QueryNonce
        #[arg(long, default_value = "DeveloperDiskImage")]
        image_type: String,
    },
    /// Query personalization identifiers used for iOS 17+ DDI personalization
    PersonalizationIdentifiers {
        /// Personalized image type sent with QueryPersonalizationIdentifiers
        #[arg(long, default_value = "DeveloperDiskImage")]
        image_type: String,
    },
    /// Unmount the currently mounted developer disk image
    Unmount,
    /// Download and mount a developer disk image
    Mount {
        /// Path to a local DDI (skips download)
        #[arg(long)]
        path: Option<PathBuf>,
        /// Cache directory for downloaded DDIs
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

impl DdiCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for ddi"))?;

        match self.sub {
            DdiSub::Auto { cache_dir } => run_mount(udid, None, cache_dir).await,
            DdiSub::Status => run_status(udid, json).await,
            DdiSub::List => run_list(udid, json).await,
            DdiSub::Devices => run_devices(udid, json).await,
            DdiSub::Lookup { image_type } => run_lookup(udid, json, image_type).await,
            DdiSub::PersonalizationManifests => run_personalization_manifests(udid, json).await,
            DdiSub::DevmodeStatus => run_devmode_status(udid, json).await,
            DdiSub::Nonce { image_type } => run_nonce(udid, json, image_type).await,
            DdiSub::PersonalizationIdentifiers { image_type } => {
                run_personalization_identifiers(udid, json, image_type).await
            }
            DdiSub::Unmount => run_unmount(udid).await,
            DdiSub::Mount { path, cache_dir } => run_mount(udid, path, cache_dir).await,
        }
    }
}

async fn run_status(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let mounted = client
        .is_image_mounted()
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "mounted": mounted }))?
        );
    } else if mounted {
        println!("Developer disk image is mounted.");
    } else {
        println!("No developer disk image mounted.");
    }
    Ok(())
}

async fn run_list(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let developer = client
        .lookup_image_signatures("Developer")
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;
    let personalized = client
        .lookup_image_signatures("Personalized")
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer": signatures_to_hex(&developer),
                "personalized": signatures_to_hex(&personalized),
            }))?
        );
        return Ok(());
    }

    if developer.is_empty() && personalized.is_empty() {
        println!("No developer disk images mounted.");
        return Ok(());
    }

    print_image_signatures("Developer", &developer);
    print_image_signatures("Personalized", &personalized);
    Ok(())
}

async fn run_lookup(udid: String, json: bool, image_type: String) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let signatures = client
        .lookup_image_signatures(&image_type)
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&lookup_to_json(&image_type, &signatures))?
        );
        return Ok(());
    }

    if signatures.is_empty() {
        println!("No mounted image signatures found for {image_type}.");
        return Ok(());
    }

    print_image_signatures(&image_type, &signatures);
    Ok(())
}

async fn run_devices(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let entries = client
        .copy_devices()
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&devices_to_json(&entries))?
        );
        return Ok(());
    }

    if entries.is_empty() {
        println!("No mounted image entries reported.");
        return Ok(());
    }

    print_devices(&entries);
    Ok(())
}

async fn run_personalization_manifests(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let entries = client
        .copy_devices()
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    let queries = personalization_manifest_queries(&entries);
    let mut manifests = Vec::with_capacity(queries.len());
    for (personalized_image_type, image_signature) in queries {
        let manifest = client
            .query_personalization_manifest(&personalized_image_type, &image_signature)
            .await
            .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;
        manifests.push(ManifestResult {
            personalized_image_type,
            image_signature,
            manifest,
        });
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&personalization_manifests_to_json(&manifests))?
        );
        return Ok(());
    }

    if manifests.is_empty() {
        println!("No mounted personalized image entries reported.");
        return Ok(());
    }

    print_personalization_manifests(&manifests);
    Ok(())
}

async fn run_devmode_status(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let enabled = client
        .query_developer_mode_status()
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer_mode": enabled,
            }))?
        );
    } else {
        println!(
            "Developer mode is {}.",
            if enabled { "enabled" } else { "disabled" }
        );
    }
    Ok(())
}

async fn run_nonce(udid: String, json: bool, image_type: String) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let nonce = client
        .query_nonce_with_type(&image_type)
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&nonce_to_json(&nonce))?);
    } else {
        println!("{}", hex::encode(&nonce));
    }
    Ok(())
}

async fn run_personalization_identifiers(
    udid: String,
    json: bool,
    image_type: String,
) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let identifiers = client
        .query_personalization_identifiers_with_type(&image_type)
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&personalization_identifiers_to_json(&identifiers))?
        );
    } else {
        print_personalization_identifiers(&identifiers);
    }
    Ok(())
}

async fn run_unmount(udid: String) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let version = device.product_version().await?;
    let mount_path = if version.major >= 17 {
        "/System/Developer"
    } else {
        "/Developer"
    };
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;
    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let developer = client
        .lookup_image_signatures("Developer")
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;
    let personalized = client
        .lookup_image_signatures("Personalized")
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;
    if developer.is_empty() && personalized.is_empty() {
        println!("No developer disk images mounted.");
        return Ok(());
    }

    client
        .unmount_image(mount_path)
        .await
        .map_err(|e| anyhow::anyhow!("image mounter: {e}"))?;

    println!("Unmounted developer disk image from {mount_path}.");
    Ok(())
}

async fn run_mount(
    udid: String,
    local_path: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
) -> Result<()> {
    use ios_core::imagemounter::ImageMounterClient;

    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;

    // Check if already mounted
    {
        let mut stream = device
            .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
            .await?;
        let mut client = ImageMounterClient::new(&mut *stream);
        if client.is_image_mounted().await.unwrap_or(false) {
            eprintln!("Developer disk image is already mounted.");
            return Ok(());
        }
    }

    let version = device.product_version().await?;
    eprintln!("Device iOS version: {version}");

    if version.major >= 17 {
        // Personalized DDI path
        mount_personalized(&device, cache_dir).await
    } else {
        // Standard DDI path
        mount_standard(&device, &version, local_path, cache_dir).await
    }
}

async fn mount_standard(
    device: &ios_core::ConnectedDevice,
    version: &semver::Version,
    local_path: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
) -> Result<()> {
    use ios_core::imagemounter::{DdiDownloader, ImageMounterClient};

    let (image, signature) = if let Some(path) = local_path {
        let image = tokio::fs::read(path.join("DeveloperDiskImage.dmg")).await?;
        let signature = tokio::fs::read(path.join("DeveloperDiskImage.dmg.signature")).await?;
        (image, signature)
    } else {
        eprintln!(
            "Downloading DDI for iOS {}.{}...",
            version.major, version.minor
        );
        let downloader = DdiDownloader::new(cache_dir);
        let ddi = downloader
            .download_standard(version)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        (ddi.image, ddi.signature)
    };

    eprintln!("Uploading and mounting DDI ({} bytes)...", image.len());
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;
    let mut client = ImageMounterClient::new(&mut *stream);
    client
        .mount_standard(&image, &signature)
        .await
        .map_err(|e| anyhow::anyhow!("mount failed: {e}"))?;

    eprintln!("Developer disk image mounted successfully.");
    Ok(())
}

async fn mount_personalized(
    device: &ios_core::ConnectedDevice,
    cache_dir: Option<PathBuf>,
) -> Result<()> {
    use ios_core::imagemounter::tss;
    use ios_core::imagemounter::{BuildManifest, DdiDownloader, ImageMounterClient};

    let downloader = DdiDownloader::new(cache_dir);
    let ddi = downloader
        .download_personalized()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    eprintln!("Parsing BuildManifest...");
    let manifest = BuildManifest::parse(&ddi.build_manifest).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Get personalization identifiers from device
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;
    let mut client = ImageMounterClient::new(&mut *stream);

    let ids = client
        .query_personalization_identifiers()
        .await
        .map_err(|e| anyhow::anyhow!("query identifiers: {e}"))?;

    let board_id = extract_id(&ids, "BoardId")?;
    let chip_id = extract_id(&ids, "ChipID")?;

    let identity = manifest
        .find_identity(board_id, chip_id)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let nonce = client
        .query_nonce()
        .await
        .map_err(|e| anyhow::anyhow!("query nonce: {e}"))?;

    eprintln!("Requesting TSS ticket from Apple...");
    let tss_request = tss::build_tss_request(&ids, &nonce, identity);
    let ticket = tss::get_tss_ticket(&tss_request)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    eprintln!("Mounting personalized DDI ({} bytes)...", ddi.image.len());
    client
        .mount_personalized(&ddi.trustcache, &ddi.build_manifest, &ddi.image, &ticket)
        .await
        .map_err(|e| anyhow::anyhow!("mount failed: {e}"))?;

    eprintln!("Personalized developer disk image mounted successfully.");
    Ok(())
}

fn extract_id(ids: &std::collections::HashMap<String, plist::Value>, key: &str) -> Result<u64> {
    let val = ids
        .get(key)
        .ok_or_else(|| anyhow::anyhow!("missing {key} in personalization identifiers"))?;
    match val {
        plist::Value::Integer(n) => n
            .as_unsigned()
            .ok_or_else(|| anyhow::anyhow!("{key} is not an unsigned integer")),
        plist::Value::String(s) => {
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).map_err(|e| anyhow::anyhow!("parse {key} hex: {e}"))
            } else {
                s.parse().map_err(|e| anyhow::anyhow!("parse {key}: {e}"))
            }
        }
        _ => Err(anyhow::anyhow!("{key} has unexpected type")),
    }
}

fn print_image_signatures(image_type: &str, signatures: &[Vec<u8>]) {
    for signature in signatures {
        println!("{image_type}: {}", hex::encode(signature));
    }
}

fn signatures_to_hex(signatures: &[Vec<u8>]) -> Vec<String> {
    signatures.iter().map(hex::encode).collect()
}

fn nonce_to_json(nonce: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "nonce": hex::encode(nonce),
        "bytes": nonce.len(),
    })
}

fn lookup_to_json(image_type: &str, signatures: &[Vec<u8>]) -> serde_json::Value {
    serde_json::json!({
        "image_type": image_type,
        "signatures": signatures_to_hex(signatures),
    })
}

fn devices_to_json(entries: &[plist::Dictionary]) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .iter()
            .map(|entry| plist_to_json(&Value::Dictionary(entry.clone())))
            .collect(),
    )
}

fn personalization_manifest_queries(entries: &[plist::Dictionary]) -> Vec<(String, Vec<u8>)> {
    entries
        .iter()
        .filter_map(|entry| {
            let personalized_image_type = entry.get("PersonalizedImageType")?.as_string()?;
            let image_signature = entry.get("ImageSignature")?.as_data()?;
            Some((
                personalized_image_type.to_string(),
                image_signature.to_vec(),
            ))
        })
        .collect()
}

fn personalization_manifests_to_json(results: &[ManifestResult]) -> serde_json::Value {
    serde_json::Value::Array(
        results
            .iter()
            .map(|result| {
                serde_json::json!({
                    "personalized_image_type": result.personalized_image_type,
                    "image_signature": hex::encode(&result.image_signature),
                    "manifest": hex::encode(&result.manifest),
                })
            })
            .collect(),
    )
}

fn personalization_identifiers_to_json(
    identifiers: &std::collections::HashMap<String, Value>,
) -> serde_json::Value {
    let mut entries: Vec<_> = identifiers.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut obj = serde_json::Map::with_capacity(entries.len());
    for (key, value) in entries {
        obj.insert(key.clone(), plist_to_json(value));
    }
    serde_json::Value::Object(obj)
}

fn print_personalization_identifiers(identifiers: &std::collections::HashMap<String, Value>) {
    let mut entries: Vec<_> = identifiers.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in entries {
        println!("{key}: {}", format_plist_value(value));
    }
}

fn print_devices(entries: &[plist::Dictionary]) {
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            println!();
        }

        let mut fields: Vec<_> = entry.iter().collect();
        fields.sort_by(|a, b| a.0.cmp(b.0));
        for (key, value) in fields {
            println!("{key}: {}", format_plist_value(value));
        }
    }
}

fn print_personalization_manifests(results: &[ManifestResult]) {
    for (index, result) in results.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("PersonalizedImageType: {}", result.personalized_image_type);
        println!("ImageSignature: {}", hex::encode(&result.image_signature));
        println!("Manifest: {}", hex::encode(&result.manifest));
    }
}

fn format_plist_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Boolean(value) => value.to_string(),
        Value::Integer(value) => value
            .as_signed()
            .map(|value| value.to_string())
            .or_else(|| value.as_unsigned().map(|value| value.to_string()))
            .unwrap_or_default(),
        Value::Real(value) => value.to_string(),
        Value::Data(value) => hex::encode(value),
        other => serde_json::to_string(&plist_to_json(other)).unwrap_or_else(|_| "null".into()),
    }
}

fn plist_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Array(items) => serde_json::Value::Array(items.iter().map(plist_to_json).collect()),
        Value::Boolean(value) => serde_json::Value::Bool(*value),
        Value::Data(value) => serde_json::Value::String(hex::encode(value)),
        Value::Date(value) => serde_json::Value::String(value.to_xml_format()),
        Value::Dictionary(dict) => serde_json::Value::Object(
            dict.iter()
                .map(|(key, value)| (key.clone(), plist_to_json(value)))
                .collect(),
        ),
        Value::Integer(value) => value
            .as_signed()
            .map(serde_json::Value::from)
            .or_else(|| value.as_unsigned().map(serde_json::Value::from))
            .unwrap_or(serde_json::Value::Null),
        Value::Real(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(value) => serde_json::Value::String(value.clone()),
        Value::Uid(value) => serde_json::Value::from(value.get()),
        _ => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: DdiSub,
    }

    #[test]
    fn parses_devmode_status_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "devmode-status"]);
        match cmd.command {
            DdiSub::DevmodeStatus => {}
            _ => panic!("expected devmode-status subcommand"),
        }
    }

    #[test]
    fn parses_list_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "list"]);
        match cmd.command {
            DdiSub::List => {}
            _ => panic!("expected list subcommand"),
        }
    }

    #[test]
    fn parses_devices_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "devices"]);
        match cmd.command {
            DdiSub::Devices => {}
            _ => panic!("expected devices subcommand"),
        }
    }

    #[test]
    fn parses_personalization_manifests_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "personalization-manifests"]);
        match cmd.command {
            DdiSub::PersonalizationManifests => {}
            _ => panic!("expected personalization-manifests subcommand"),
        }
    }

    #[test]
    fn parses_unmount_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "unmount"]);
        match cmd.command {
            DdiSub::Unmount => {}
            _ => panic!("expected unmount subcommand"),
        }
    }

    #[test]
    fn parses_nonce_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "nonce"]);
        match cmd.command {
            DdiSub::Nonce { image_type } => assert_eq!(image_type, "DeveloperDiskImage"),
            _ => panic!("expected nonce subcommand"),
        }
    }

    #[test]
    fn parses_nonce_subcommand_with_custom_image_type() {
        let cmd = TestCli::parse_from(["ddi", "nonce", "--image-type", "Cryptex"]);
        match cmd.command {
            DdiSub::Nonce { image_type } => assert_eq!(image_type, "Cryptex"),
            _ => panic!("expected nonce subcommand"),
        }
    }

    #[test]
    fn parses_personalization_identifiers_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "personalization-identifiers"]);
        match cmd.command {
            DdiSub::PersonalizationIdentifiers { image_type } => {
                assert_eq!(image_type, "DeveloperDiskImage")
            }
            _ => panic!("expected personalization-identifiers subcommand"),
        }
    }

    #[test]
    fn parses_personalization_identifiers_subcommand_with_custom_image_type() {
        let cmd = TestCli::parse_from([
            "ddi",
            "personalization-identifiers",
            "--image-type",
            "Cryptex",
        ]);
        match cmd.command {
            DdiSub::PersonalizationIdentifiers { image_type } => {
                assert_eq!(image_type, "Cryptex")
            }
            _ => panic!("expected personalization-identifiers subcommand"),
        }
    }

    #[test]
    fn parses_lookup_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "lookup", "Developer"]);
        match cmd.command {
            DdiSub::Lookup { image_type } => assert_eq!(image_type, "Developer"),
            _ => panic!("expected lookup subcommand"),
        }
    }

    #[test]
    fn parses_auto_subcommand() {
        let cmd = TestCli::parse_from(["ddi", "auto", "--cache-dir", "ios-rs-tmp/ddi-cache"]);
        match cmd.command {
            DdiSub::Auto { cache_dir } => {
                assert_eq!(cache_dir, Some(PathBuf::from("ios-rs-tmp/ddi-cache")));
            }
            _ => panic!("expected auto subcommand"),
        }
    }

    #[test]
    fn nonce_json_includes_hex_and_byte_count() {
        let value = nonce_to_json(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            value,
            serde_json::json!({
                "nonce": "deadbeef",
                "bytes": 4,
            })
        );
    }

    #[test]
    fn personalization_identifiers_json_sorts_and_converts_values() {
        let value = personalization_identifiers_to_json(&std::collections::HashMap::from([
            ("BoardId".to_string(), Value::Integer(12.into())),
            ("ApNonce".to_string(), Value::Data(vec![0xaa, 0xbb])),
        ]));

        assert_eq!(
            value,
            serde_json::json!({
                "ApNonce": "aabb",
                "BoardId": 12,
            })
        );
    }

    #[test]
    fn lookup_json_includes_image_type_and_hex_signatures() {
        let value = lookup_to_json("Personalized", &[vec![0xde, 0xad], vec![0xbe, 0xef]]);

        assert_eq!(
            value,
            serde_json::json!({
                "image_type": "Personalized",
                "signatures": ["dead", "beef"],
            })
        );
    }

    #[test]
    fn devices_json_hex_encodes_entry_signatures() {
        let value = devices_to_json(&[plist::Dictionary::from_iter([
            ("ImageType".to_string(), Value::String("Developer".into())),
            (
                "ImageSignature".to_string(),
                Value::Data(vec![0xde, 0xad, 0xbe, 0xef]),
            ),
        ])]);

        assert_eq!(
            value,
            serde_json::json!([{
                "ImageType": "Developer",
                "ImageSignature": "deadbeef",
            }])
        );
    }

    #[test]
    fn personalization_manifest_queries_filter_and_extract_personalized_entries() {
        let queries = personalization_manifest_queries(&[
            plist::Dictionary::from_iter([
                (
                    "PersonalizedImageType".to_string(),
                    Value::String("DeveloperDiskImage".into()),
                ),
                ("ImageSignature".to_string(), Value::Data(vec![0xaa, 0xbb])),
            ]),
            plist::Dictionary::from_iter([(
                "ImageType".to_string(),
                Value::String("Developer".into()),
            )]),
        ]);

        assert_eq!(
            queries,
            vec![("DeveloperDiskImage".to_string(), vec![0xaa, 0xbb])]
        );
    }

    #[test]
    fn personalization_manifests_json_hex_encodes_manifest_payloads() {
        let value = personalization_manifests_to_json(&[ManifestResult {
            personalized_image_type: "DeveloperDiskImage".to_string(),
            image_signature: vec![0xaa, 0xbb],
            manifest: vec![0xfa, 0xce],
        }]);

        assert_eq!(
            value,
            serde_json::json!([{
                "personalized_image_type": "DeveloperDiskImage",
                "image_signature": "aabb",
                "manifest": "face",
            }])
        );
    }
}

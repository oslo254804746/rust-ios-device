use ios_core::plist_json::{self, DataEncoding, PlistJsonPolicy};
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
    /// Manage Cryptex developer images over an RSD tunnel
    Cryptex {
        #[command(subcommand)]
        command: CryptexSub,
    },
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
        /// Local DDI directory; iOS 17–26 accepts Restore or its parent
        #[arg(long)]
        path: Option<PathBuf>,
        /// Cache directory for downloaded DDIs
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

#[derive(clap::Subcommand)]
enum CryptexSub {
    /// List installed Cryptex images
    List,
    /// Read AppleImage4 personalization identifiers
    PersonalizationIdentifiers,
    /// Read the unwrapped nonce using a domain index or handle
    Nonce {
        #[arg(long, conflicts_with = "nonce_domain_handle")]
        nonce_domain: Option<u64>,
        #[arg(long, conflicts_with = "nonce_domain")]
        nonce_domain_handle: Option<u64>,
    },
    /// Personalize and install the DDI Cryptex
    AutoInstall {
        #[arg(long)]
        restore_dir: Option<PathBuf>,
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Uninstall a Cryptex image by identifier
    Uninstall {
        identifier: String,
        #[arg(long)]
        version: Option<String>,
    },
}

impl DdiCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for ddi"))?;

        match self.sub {
            DdiSub::Cryptex { command } => run_cryptex(udid, json, command).await,
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
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    if uses_cryptex(&device.product_version().await?) {
        let device = connect_cryptex_device(&udid).await?;
        let mut client = ios_core::cryptex::CryptexClient::connect(&device).await?;
        let installed = client.installed_ddi().await?;
        if json {
            crate::output::outln!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"mounted": installed.is_some(), "backend": "cryptex", "installed": installed})
                )?
            );
        } else if let Some(installed) = installed {
            crate::output::outln!(
                "DDI Cryptex {} {} is installed.",
                installed.identifier,
                installed.version
            );
        } else {
            crate::output::outln!("No DDI Cryptex installed.");
        }
        return Ok(());
    }
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;

    let mut client = ios_core::imagemounter::ImageMounterClient::new(&mut *stream);
    let mounted = client
        .is_image_mounted()
        .await
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "mounted": mounted }))?
        );
    } else if mounted {
        crate::output::outln!("Developer disk image is mounted.");
    } else {
        crate::output::outln!("No developer disk image mounted.");
    }
    Ok(())
}

async fn run_list(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;
    let personalized = client
        .lookup_image_signatures("Personalized")
        .await
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer": signatures_to_hex(&developer),
                "personalized": signatures_to_hex(&personalized),
            }))?
        );
        return Ok(());
    }

    if developer.is_empty() && personalized.is_empty() {
        crate::output::outln!("No developer disk images mounted.");
        return Ok(());
    }

    print_image_signatures("Developer", &developer)?;
    print_image_signatures("Personalized", &personalized)?;
    Ok(())
}

async fn run_lookup(udid: String, json: bool, image_type: String) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&lookup_to_json(&image_type, &signatures))?
        );
        return Ok(());
    }

    if signatures.is_empty() {
        crate::output::outln!("No mounted image signatures found for {image_type}.");
        return Ok(());
    }

    print_image_signatures(&image_type, &signatures)?;
    Ok(())
}

async fn run_devices(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&devices_to_json(&entries))?
        );
        return Ok(());
    }

    if entries.is_empty() {
        crate::output::outln!("No mounted image entries reported.");
        return Ok(());
    }

    print_devices(&entries)?;
    Ok(())
}

async fn run_personalization_manifests(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    let queries = personalization_manifest_queries(&entries);
    let mut manifests = Vec::with_capacity(queries.len());
    for (personalized_image_type, image_signature) in queries {
        let manifest = client
            .query_personalization_manifest(&personalized_image_type, &image_signature)
            .await
            .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;
        manifests.push(ManifestResult {
            personalized_image_type,
            image_signature,
            manifest,
        });
    }

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&personalization_manifests_to_json(&manifests))?
        );
        return Ok(());
    }

    if manifests.is_empty() {
        crate::output::outln!("No mounted personalized image entries reported.");
        return Ok(());
    }

    print_personalization_manifests(&manifests)?;
    Ok(())
}

async fn run_devmode_status(udid: String, json: bool) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "developer_mode": enabled,
            }))?
        );
    } else {
        crate::output::outln!(
            "Developer mode is {}.",
            if enabled { "enabled" } else { "disabled" }
        );
    }
    Ok(())
}

async fn run_nonce(udid: String, json: bool, image_type: String) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!("{}", serde_json::to_string_pretty(&nonce_to_json(&nonce))?);
    } else {
        crate::output::outln!("{}", hex::encode(&nonce));
    }
    Ok(())
}

async fn run_personalization_identifiers(
    udid: String,
    json: bool,
    image_type: String,
) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    if json {
        crate::output::outln!(
            "{}",
            serde_json::to_string_pretty(&personalization_identifiers_to_json(&identifiers))?
        );
    } else {
        print_personalization_identifiers(&identifiers)?;
    }
    Ok(())
}

async fn run_unmount(udid: String) -> Result<()> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;
    let version = device.product_version().await?;
    if uses_cryptex(&version) {
        let tunnel_device = connect_cryptex_device(&udid).await?;
        let mut client = ios_core::cryptex::CryptexClient::connect(&tunnel_device).await?;
        if let Some(installed) = client.installed_ddi().await? {
            client
                .uninstall(&installed.identifier, Some(&installed.version))
                .await?;
            crate::output::outln!("Uninstalled DDI Cryptex {}.", installed.version);
            return Ok(());
        }
    }
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
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;
    let personalized = client
        .lookup_image_signatures("Personalized")
        .await
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;
    if developer.is_empty() && personalized.is_empty() {
        crate::output::outln!("No developer disk images mounted.");
        return Ok(());
    }

    client
        .unmount_image(mount_path)
        .await
        .map_err(|e| anyhow::Error::new(e).context("image mounter"))?;

    crate::output::outln!("Unmounted developer disk image from {mount_path}.");
    Ok(())
}

async fn run_mount(
    udid: String,
    local_path: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
) -> Result<()> {
    use ios_core::imagemounter::ImageMounterClient;

    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(&udid, opts).await?;

    let version = device.product_version().await?;
    crate::output::write_error_line(format_args!("Device iOS version: {version}"))?;
    if uses_cryptex(&version) {
        let device = connect_cryptex_device(&udid).await?;
        let installed = ios_core::cryptex::auto_install_ddi(
            &device,
            local_path.as_deref(),
            cache_dir.as_deref(),
        )
        .await?;
        crate::output::write_error_line(format_args!(
            "Installed DDI Cryptex {} {}.",
            ios_core::redaction::diagnostic_id(&installed.identifier),
            ios_core::redaction::diagnostic_id(&installed.version)
        ))?;
        return Ok(());
    }

    // Check if already mounted
    {
        let mut stream = device
            .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
            .await?;
        let mut client = ImageMounterClient::new(&mut *stream);
        if client.is_image_mounted().await? {
            crate::output::write_error_line(format_args!(
                "Developer disk image is already mounted."
            ))?;
            return Ok(());
        }
    }

    if version.major >= 17 {
        // Personalized DDI path
        mount_personalized(&device, local_path, cache_dir).await
    } else {
        // Standard DDI path
        mount_standard(&device, &version, local_path, cache_dir).await
    }
}

fn uses_cryptex(version: &semver::Version) -> bool {
    version.major >= 27
}

async fn connect_cryptex_device(udid: &str) -> Result<ios_core::ConnectedDevice> {
    Ok(ios_core::connect(
        udid,
        ios_core::device::ConnectOptions {
            tun_mode: ios_core::TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        },
    )
    .await?)
}

async fn run_cryptex(udid: String, json: bool, command: CryptexSub) -> Result<()> {
    use ios_core::cryptex::{CryptexClient, NonceDomain};
    let device = connect_cryptex_device(&udid).await?;
    if let CryptexSub::AutoInstall {
        restore_dir,
        cache_dir,
    } = command
    {
        let installed = ios_core::cryptex::auto_install_ddi(
            &device,
            restore_dir.as_deref(),
            cache_dir.as_deref(),
        )
        .await?;
        if json {
            crate::output::outln!("{}", serde_json::to_string_pretty(&installed)?);
        } else {
            crate::output::outln!("Installed {} {}.", installed.identifier, installed.version);
        }
        return Ok(());
    }
    let mut client = CryptexClient::connect(&device).await?;
    match command {
        CryptexSub::List => {
            let installed = client.copy_installed().await?;
            if json {
                crate::output::outln!("{}", serde_json::to_string_pretty(&installed)?);
            } else if installed.is_empty() {
                crate::output::outln!("No Cryptex images installed.");
            } else {
                for item in installed {
                    crate::output::outln!("{} {}", item.identifier, item.version);
                }
            }
        }
        CryptexSub::PersonalizationIdentifiers => {
            let identifiers = client.read_personalization_identifiers().await?;
            if json {
                crate::output::outln!(
                    "{}",
                    serde_json::to_string_pretty(&plist_to_json(&Value::Dictionary(identifiers)))?
                );
            } else {
                for (key, value) in identifiers {
                    crate::output::outln!("{key}: {}", format_plist_value(&value));
                }
            }
        }
        CryptexSub::Nonce {
            nonce_domain,
            nonce_domain_handle,
        } => {
            let domain = nonce_domain_handle
                .map(NonceDomain::Handle)
                .or_else(|| nonce_domain.map(NonceDomain::Index))
                .unwrap_or_default();
            let nonce = client.nonce(domain).await?;
            if json {
                crate::output::outln!("{}", serde_json::to_string_pretty(&nonce_to_json(&nonce))?);
            } else {
                crate::output::outln!("{}", hex::encode(nonce));
            }
        }
        CryptexSub::Uninstall {
            identifier,
            version,
        } => {
            client.uninstall(&identifier, version.as_deref()).await?;
            if json {
                crate::output::outln!("{}", serde_json::json!({"uninstalled": identifier}));
            } else {
                crate::output::outln!("Uninstalled {identifier}.");
            }
        }
        CryptexSub::AutoInstall { .. } => unreachable!("handled before connecting client"),
    }
    Ok(())
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
        crate::output::write_error_line(format_args!(
            "Downloading DDI for iOS {}.{}...",
            version.major, version.minor
        ))?;
        let downloader = DdiDownloader::new(cache_dir);
        let ddi = downloader
            .download_standard(version)
            .await
            .map_err(anyhow::Error::new)?;
        (ddi.image, ddi.signature)
    };

    crate::output::write_error_line(format_args!(
        "Uploading and mounting DDI ({} bytes)...",
        image.len()
    ))?;
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;
    let mut client = ImageMounterClient::new(&mut *stream);
    client
        .mount_standard(&image, &signature)
        .await
        .map_err(|e| anyhow::Error::new(e).context("mount failed"))?;

    crate::output::write_error_line(format_args!("Developer disk image mounted successfully."))?;
    Ok(())
}

async fn mount_personalized(
    device: &ios_core::ConnectedDevice,
    local_path: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
) -> Result<()> {
    use ios_core::imagemounter::tss;
    use ios_core::imagemounter::{BuildManifest, DdiDownloader, ImageMounterClient};

    let ddi = personalized_material(local_path.as_deref(), cache_dir, |cache_dir| async move {
        DdiDownloader::new(cache_dir).download_personalized().await
    })
    .await?;

    crate::output::write_error_line(format_args!("Parsing BuildManifest..."))?;
    let manifest = BuildManifest::parse(&ddi.build_manifest).map_err(anyhow::Error::new)?;
    let ecid = extract_identifier_value(&device.lockdown_get_value(Some("UniqueChipID")).await?)?;

    // Get personalization identifiers from device
    let mut stream = device
        .connect_service(ios_core::imagemounter::protocol::SERVICE_NAME)
        .await?;
    let mut client = ImageMounterClient::new(&mut *stream);

    let ids = client
        .query_personalization_identifiers()
        .await
        .map_err(|e| anyhow::Error::new(e).context("query identifiers"))?;

    let board_id = extract_id(&ids, "BoardId")?;
    let chip_id = extract_id(&ids, "ChipID")?;

    let identity = manifest
        .find_identity(board_id, chip_id)
        .map_err(anyhow::Error::new)?;

    let nonce = client
        .query_nonce()
        .await
        .map_err(|e| anyhow::Error::new(e).context("query nonce"))?;

    crate::output::write_error_line(format_args!("Requesting TSS ticket from Apple..."))?;
    let tss_request = tss::build_tss_request(&ids, board_id, chip_id, ecid, &nonce, identity);
    let ticket = tss::get_tss_ticket(&tss_request)
        .await
        .map_err(anyhow::Error::new)?;

    crate::output::write_error_line(format_args!(
        "Mounting personalized DDI ({} bytes)...",
        ddi.image.len()
    ))?;
    client
        .mount_personalized(&ddi.trustcache, &ddi.build_manifest, &ddi.image, &ticket)
        .await
        .map_err(|e| anyhow::Error::new(e).context("mount failed"))?;

    crate::output::write_error_line(format_args!(
        "Personalized developer disk image mounted successfully."
    ))?;
    Ok(())
}

async fn personalized_material<F, Fut>(
    local_path: Option<&std::path::Path>,
    cache_dir: Option<PathBuf>,
    fetch: F,
) -> Result<ios_core::imagemounter::download::PersonalizedDdi>
where
    F: FnOnce(Option<PathBuf>) -> Fut,
    Fut: std::future::Future<
        Output = std::result::Result<
            ios_core::imagemounter::download::PersonalizedDdi,
            ios_core::imagemounter::ImageMounterError,
        >,
    >,
{
    if let Some(path) = local_path {
        let nested = path.join("Restore");
        let restore = if nested.is_dir() {
            nested
        } else {
            path.to_path_buf()
        };
        return Ok(
            ios_core::imagemounter::download::load_personalized_from_restore_dir(&restore).await?,
        );
    }
    Ok(fetch(cache_dir).await?)
}

fn extract_id(ids: &std::collections::HashMap<String, plist::Value>, key: &str) -> Result<u64> {
    let val = ids
        .get(key)
        .ok_or_else(|| crate::output::received_decision("missing personalization identifier"))?;
    extract_identifier_value(val)
}

fn extract_identifier_value(val: &plist::Value) -> Result<u64> {
    match val {
        plist::Value::Integer(n) => n.as_unsigned().ok_or_else(|| {
            crate::output::received_decision(
                "personalization identifier is not an unsigned integer",
            )
        }),
        plist::Value::String(s) => {
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).map_err(|e| {
                    crate::output::received_error("parse personalization identifier hex", e)
                })
            } else {
                s.parse::<u64>().map_err(|e| {
                    crate::output::received_error("parse personalization identifier", e)
                })
            }
        }
        _ => Err(crate::output::received_decision(
            "personalization identifier has unexpected type",
        )),
    }
}

fn print_image_signatures(image_type: &str, signatures: &[Vec<u8>]) -> Result<()> {
    for signature in signatures {
        crate::output::outln!("{image_type}: {}", hex::encode(signature));
    }
    Ok(())
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

fn print_personalization_identifiers(
    identifiers: &std::collections::HashMap<String, Value>,
) -> Result<()> {
    let mut entries: Vec<_> = identifiers.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (key, value) in entries {
        crate::output::outln!("{key}: {}", format_plist_value(value));
    }
    Ok(())
}

fn print_devices(entries: &[plist::Dictionary]) -> Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            crate::output::outln!();
        }

        let mut fields: Vec<_> = entry.iter().collect();
        fields.sort_by(|a, b| a.0.cmp(b.0));
        for (key, value) in fields {
            crate::output::outln!("{key}: {}", format_plist_value(value));
        }
    }
    Ok(())
}

fn print_personalization_manifests(results: &[ManifestResult]) -> Result<()> {
    for (index, result) in results.iter().enumerate() {
        if index > 0 {
            crate::output::outln!();
        }
        crate::output::outln!("PersonalizedImageType: {}", result.personalized_image_type);
        crate::output::outln!("ImageSignature: {}", hex::encode(&result.image_signature));
        crate::output::outln!("Manifest: {}", hex::encode(&result.manifest));
    }
    Ok(())
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
    plist_json::to_json(value, PlistJsonPolicy::new(DataEncoding::HexString))
}

#[cfg(test)]
mod tests {
    #[test]
    fn cli_native_sources_personalization_identifier_retains_received_parse_error() {
        for value in ["0xowned-private-identifier", "18446744073709551616"] {
            let ids = std::collections::HashMap::from([(
                "ApChipID".to_owned(),
                plist::Value::String(value.to_owned()),
            )]);
            let error = super::extract_id(&ids, "ApChipID").unwrap_err();
            assert_eq!(
                ios_core::classify_error(error.as_ref()),
                ios_core::ErrorClass::Protocol
            );
            assert!(error
                .chain()
                .any(|source| source.is::<std::num::ParseIntError>()));
            let json = crate::output::operation_failure_message(
                crate::output::ErrorFormat::Json,
                "ddi",
                &error,
            )
            .unwrap();
            assert!(!json.contains("owned-private"));
        }
    }
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: DdiSub,
    }

    #[tokio::test]
    async fn personalized_path_selects_local_material_without_fetch_or_fallback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ios-ddi-{}-{unique}", std::process::id()));
        let restore = root.join("Restore");
        tokio::fs::create_dir_all(&restore).await.unwrap();
        for (name, bytes) in [
            ("PersonalizedDMG.dmg", b"image".as_slice()),
            ("PersonalizedDMG.trustcache", b"trust".as_slice()),
            ("BuildManifest.plist", b"manifest".as_slice()),
        ] {
            tokio::fs::write(restore.join(name), bytes).await.unwrap();
        }
        let calls = AtomicUsize::new(0);
        for path in [&root, &restore] {
            let ddi = personalized_material(Some(path), Some(root.join("unused-cache")), |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                async {
                    Ok(ios_core::imagemounter::download::PersonalizedDdi {
                        image: b"wrong source".to_vec(),
                        trustcache: Vec::new(),
                        build_manifest: Vec::new(),
                    })
                }
            })
            .await
            .unwrap();
            assert_eq!(ddi.image, b"image");
            assert_eq!(ddi.trustcache, b"trust");
            assert_eq!(ddi.build_manifest, b"manifest");
        }
        tokio::fs::remove_file(restore.join("PersonalizedDMG.trustcache"))
            .await
            .unwrap();
        let error = personalized_material(Some(&root), None, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            async {
                Ok(ios_core::imagemounter::download::PersonalizedDdi {
                    image: Vec::new(),
                    trustcache: Vec::new(),
                    build_manifest: Vec::new(),
                })
            }
        })
        .await;
        assert!(error.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        tokio::fs::remove_file(restore.join("PersonalizedDMG.dmg"))
            .await
            .unwrap();
        tokio::fs::remove_file(restore.join("BuildManifest.plist"))
            .await
            .unwrap();
        tokio::fs::remove_dir(&restore).await.unwrap();
        tokio::fs::remove_dir(&root).await.unwrap();
    }

    #[tokio::test]
    async fn personalized_without_path_uses_requested_cache_source() {
        let cache = PathBuf::from("synthetic-cache");
        let ddi = personalized_material(None, Some(cache.clone()), |selected| async move {
            assert_eq!(selected, Some(cache));
            Ok(ios_core::imagemounter::download::PersonalizedDdi {
                image: b"from-cache-source".to_vec(),
                trustcache: Vec::new(),
                build_manifest: Vec::new(),
            })
        })
        .await
        .unwrap();
        assert_eq!(ddi.image, b"from-cache-source");
    }

    #[test]
    fn cryptex_selection_starts_at_ios_27() {
        for version in ["16.7.0", "17.0.0", "26.9.0"] {
            assert!(!uses_cryptex(&semver::Version::parse(version).unwrap()));
        }
        for version in ["27.0.0", "27.2.0", "28.0.0"] {
            assert!(uses_cryptex(&semver::Version::parse(version).unwrap()));
        }
    }

    #[test]
    fn cryptex_nonce_selectors_conflict_and_handle_parses() {
        assert!(TestCli::try_parse_from([
            "ddi",
            "cryptex",
            "nonce",
            "--nonce-domain",
            "2",
            "--nonce-domain-handle",
            "4"
        ])
        .is_err());
        let cli = TestCli::parse_from(["ddi", "cryptex", "nonce", "--nonce-domain-handle", "4"]);
        assert!(matches!(
            cli.command,
            DdiSub::Cryptex {
                command: CryptexSub::Nonce {
                    nonce_domain: None,
                    nonce_domain_handle: Some(4)
                }
            }
        ));
    }

    #[test]
    fn cryptex_auto_install_accepts_a_local_restore_directory() {
        let cli = TestCli::parse_from([
            "ddi",
            "cryptex",
            "auto-install",
            "--restore-dir",
            "/tmp/Restore",
        ]);
        assert!(
            matches!(cli.command, DdiSub::Cryptex { command: CryptexSub::AutoInstall { restore_dir: Some(path), cache_dir: None } } if path.as_path() == std::path::Path::new("/tmp/Restore"))
        );
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

#[cfg(test)]
#[test]
fn plist_json_legacy_contract() {
    crate::plist_contract::check(
        plist_to_json(&crate::plist_contract::value()),
        serde_json::json!("007fff"),
        serde_json::json!(""),
        true,
    );
}

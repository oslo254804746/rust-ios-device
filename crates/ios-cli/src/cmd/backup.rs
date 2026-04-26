use std::path::Path;

use anyhow::Result;
use ios_core::services::afc::{AfcClient, AfcError, AfcStatusCode};
use ios_core::services::apps::installation::InstallationProxy;
use ios_core::services::notificationproxy::NotificationProxyClient;
use ios_core::services::springboard::SpringboardClient;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};
use tokio::time::{sleep, Duration};

const BACKUP_DOMAIN: &str = "com.apple.mobile.backup";
const WILL_ENCRYPT_KEY: &str = "WillEncrypt";
const AFC_SERVICE_NAME: &str = "com.apple.afc";
const NOTIFICATION_PROXY_SERVICE_NAME: &str = "com.apple.mobile.notification_proxy";
const AFC_FILE_MODE_READ_WRITE: u64 = 0x00000002;
const AFC_LOCK_EXCLUSIVE: u64 = 2 | 4;
const AFC_LOCK_UNLOCK: u64 = 8 | 4;
const ITUNES_FILES: &[&str] = &[
    "ApertureAlbumPrefs",
    "IC-Info.sidb",
    "IC-Info.sidv",
    "PhotosFolderAlbums",
    "PhotosFolderName",
    "PhotosFolderPrefs",
    "VoiceMemos.plist",
    "iPhotoAlbumPrefs",
    "iTunesApplicationIDs",
    "iTunesPrefs",
    "iTunesPrefs.plist",
];
const NP_SYNC_WILL_START: &str = "com.apple.itunes-mobdev.syncWillStart";
const NP_SYNC_DID_START: &str = "com.apple.itunes-mobdev.syncDidStart";
const NP_SYNC_LOCK_REQUEST: &str = "com.apple.itunes-mobdev.syncLockRequest";
const NP_SYNC_DID_FINISH: &str = "com.apple.itunes-mobdev.syncDidFinish";
const ITUNES_SYNC_LOCK_PATH: &str = "/com.apple.itunes.lock_sync";
const BACKUP_APP_RETURN_ATTRIBUTES: &[&str] =
    &["CFBundleIdentifier", "ApplicationSINF", "iTunesMetadata"];

#[derive(clap::Args)]
pub struct BackupCmd {
    #[command(subcommand)]
    sub: BackupSubcommand,
}

#[derive(Debug, clap::Subcommand)]
enum BackupSubcommand {
    /// Query the mobilebackup2 protocol version handshake
    Version,
    /// Show whether future backups are configured to be encrypted
    EncryptionStatus,
    /// Create a MobileBackup2 backup in the given directory
    Create {
        output_dir: String,
        #[arg(long)]
        full: bool,
    },
    /// Query metadata for an existing MobileBackup2 backup
    Info {
        backup_directory: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// List the contents of an existing MobileBackup2 backup
    List {
        backup_directory: String,
        #[arg(long)]
        source: Option<String>,
    },
    /// Restore a MobileBackup2 backup to the connected device
    Restore {
        backup_directory: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        system: bool,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        reboot: bool,
        #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
        copy: bool,
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        settings: bool,
        #[arg(long)]
        remove: bool,
        #[arg(long)]
        skip_apps: bool,
    },
    /// Change or enable the backup password used by MobileBackup2
    ChangePassword {
        #[arg(long)]
        old_password: Option<String>,
        #[arg(long)]
        new_password: Option<String>,
        #[arg(long, default_value = ".")]
        backup_directory: String,
    },
}

impl BackupCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for backup"))?;

        match self.sub {
            BackupSubcommand::Version => run_version(&udid, json).await,
            BackupSubcommand::EncryptionStatus => run_encryption_status(&udid, json).await,
            BackupSubcommand::Create { output_dir, full } => {
                run_create(&udid, &output_dir, full, json).await
            }
            BackupSubcommand::Info {
                backup_directory,
                source,
            } => run_info(&udid, &backup_directory, source.as_deref(), json).await,
            BackupSubcommand::List {
                backup_directory,
                source,
            } => run_list(&udid, &backup_directory, source.as_deref(), json).await,
            BackupSubcommand::Restore {
                backup_directory,
                source,
                password,
                system,
                reboot,
                copy,
                settings,
                remove,
                skip_apps,
            } => {
                run_restore(
                    &udid,
                    &backup_directory,
                    source.as_deref(),
                    password.as_deref(),
                    system,
                    reboot,
                    copy,
                    settings,
                    remove,
                    skip_apps,
                    json,
                )
                .await
            }
            BackupSubcommand::ChangePassword {
                old_password,
                new_password,
                backup_directory,
            } => {
                run_change_password(
                    &udid,
                    old_password.as_deref(),
                    new_password.as_deref(),
                    &backup_directory,
                    json,
                )
                .await
            }
        }
    }
}

async fn run_version(udid: &str, json: bool) -> Result<()> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let stream = device
        .connect_service(ios_core::services::backup2::SERVICE_NAME)
        .await?;
    let mut client = ios_core::services::backup2::Mobilebackup2Client::new(stream);
    let version = client.version_exchange().await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "device_link_version": version.device_link_version,
                "protocol_version": version.protocol_version,
                "supported_protocol_versions": version.local_versions,
                "service_name": ios_core::services::backup2::SERVICE_NAME,
            }))?
        );
    } else {
        println!("Service: {}", ios_core::services::backup2::SERVICE_NAME);
        println!("DeviceLinkVersion: {}", version.device_link_version);
        println!("ProtocolVersion: {}", version.protocol_version);
        println!(
            "SupportedProtocolVersions: {}",
            version
                .local_versions
                .iter()
                .map(|version| format_protocol_version(*version))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(())
}

async fn run_encryption_status(udid: &str, json: bool) -> Result<()> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let will_encrypt = resolve_will_encrypt(
        device
            .lockdown_get_value_in_domain(Some(BACKUP_DOMAIN), Some(WILL_ENCRYPT_KEY))
            .await,
    )?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                WILL_ENCRYPT_KEY: will_encrypt,
            }))?
        );
    } else {
        println!("{will_encrypt}");
    }

    Ok(())
}

async fn run_create(udid: &str, output_dir: &str, full: bool, json: bool) -> Result<()> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;

    let info_plist = build_backup_info_plist(&device).await?;
    let (mut afc, mut notification_proxy, lock_handle) = acquire_backup_lock(&device).await?;
    let backup_result: Result<_> = async {
        let stream = device
            .connect_service(ios_core::services::backup2::SERVICE_NAME)
            .await?;
        let mut client = ios_core::services::backup2::Mobilebackup2Client::new(stream);
        client
            .backup(Path::new(output_dir), udid, full, &info_plist)
            .await
            .map_err(Into::into)
    }
    .await;
    let release_result = release_backup_lock(&mut afc, &mut notification_proxy, lock_handle).await;
    let result = backup_result?;
    release_result?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "device_directory": result.layout.device_directory,
                "device_link_version": result.device_link_version,
                "protocol_version": result.protocol_version,
                "full_backup": full,
            }))?
        );
    } else {
        println!(
            "Backup created at {}",
            result.layout.device_directory.display()
        );
        println!("DeviceLinkVersion: {}", result.device_link_version);
        println!(
            "ProtocolVersion: {}",
            format_protocol_version(result.protocol_version)
        );
    }

    Ok(())
}

async fn acquire_backup_lock(
    device: &ios_core::ConnectedDevice,
) -> Result<(
    AfcClient<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>,
    NotificationProxyClient<impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>,
    u64,
)> {
    let afc_stream = device.connect_service(AFC_SERVICE_NAME).await?;
    let notification_stream = device
        .connect_service(NOTIFICATION_PROXY_SERVICE_NAME)
        .await?;
    let mut afc = AfcClient::new(afc_stream);
    let mut notification_proxy = NotificationProxyClient::new(notification_stream);

    notification_proxy.post(NP_SYNC_WILL_START).await?;
    let lock_handle = afc
        .open_file(ITUNES_SYNC_LOCK_PATH, AFC_FILE_MODE_READ_WRITE)
        .await?;
    notification_proxy.post(NP_SYNC_LOCK_REQUEST).await?;

    for _ in 0..50 {
        match afc.lock_file(lock_handle, AFC_LOCK_EXCLUSIVE).await {
            Ok(()) => {
                notification_proxy.post(NP_SYNC_DID_START).await?;
                return Ok((afc, notification_proxy, lock_handle));
            }
            Err(AfcError::Status(AfcStatusCode::OpWouldBlock)) => {
                sleep(Duration::from_millis(200)).await;
            }
            Err(err) => {
                let _ = afc.close_file(lock_handle).await;
                return Err(err.into());
            }
        }
    }

    let _ = afc.close_file(lock_handle).await;
    Err(anyhow::anyhow!("failed to lock iTunes sync file"))
}

async fn release_backup_lock<A, N>(
    afc: &mut AfcClient<A>,
    notification_proxy: &mut NotificationProxyClient<N>,
    lock_handle: u64,
) -> Result<()>
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    N: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    afc.lock_file(lock_handle, AFC_LOCK_UNLOCK).await?;
    afc.close_file(lock_handle).await?;
    notification_proxy.post(NP_SYNC_DID_FINISH).await?;
    Ok(())
}

async fn write_restore_applications<A>(
    afc: &mut AfcClient<A>,
    applications: &plist::Value,
) -> Result<()>
where
    A: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    match afc.make_dir("/iTunesRestore").await {
        Ok(()) | Err(AfcError::Status(AfcStatusCode::ObjectExists)) => {}
        Err(err) => return Err(err.into()),
    }

    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, applications)?;
    afc.write_file("/iTunesRestore/RestoreApplications.plist", &payload)
        .await?;
    Ok(())
}

async fn run_info(
    udid: &str,
    backup_directory: &str,
    source: Option<&str>,
    json: bool,
) -> Result<()> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let stream = device
        .connect_service(ios_core::services::backup2::SERVICE_NAME)
        .await?;
    let mut client = ios_core::services::backup2::Mobilebackup2Client::new(stream);
    let result = client
        .info(Path::new(backup_directory), udid, source)
        .await?;
    print_process_message_content(result, json)?;
    Ok(())
}

async fn run_list(
    udid: &str,
    backup_directory: &str,
    source: Option<&str>,
    json: bool,
) -> Result<()> {
    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let stream = device
        .connect_service(ios_core::services::backup2::SERVICE_NAME)
        .await?;
    let mut client = ios_core::services::backup2::Mobilebackup2Client::new(stream);
    let result = client
        .list(Path::new(backup_directory), udid, source)
        .await?;
    print_process_message_content(result, json)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_restore(
    udid: &str,
    backup_directory: &str,
    source: Option<&str>,
    password: Option<&str>,
    system: bool,
    reboot: bool,
    copy: bool,
    settings: bool,
    remove: bool,
    skip_apps: bool,
    json: bool,
) -> Result<()> {
    let source_identifier = source.unwrap_or(udid);
    if ios_core::services::backup2::backup_is_encrypted(
        Path::new(backup_directory),
        source_identifier,
    )? && password.is_none()
    {
        return Err(anyhow::anyhow!(
            "backup restore requires --password for encrypted backups"
        ));
    }

    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;

    let (mut afc, mut notification_proxy, lock_handle) = acquire_backup_lock(&device).await?;
    let restore_result: Result<_> = async {
        if !skip_apps {
            if let Some(applications) = ios_core::services::backup2::load_backup_applications(
                Path::new(backup_directory),
                source_identifier,
            )? {
                write_restore_applications(&mut afc, &applications).await?;
            }
        }

        let stream = device
            .connect_service(ios_core::services::backup2::SERVICE_NAME)
            .await?;
        let mut client = ios_core::services::backup2::Mobilebackup2Client::new(stream);
        client
            .restore(
                Path::new(backup_directory),
                udid,
                ios_core::services::backup2::RestoreOptions {
                    system,
                    reboot,
                    copy,
                    settings,
                    remove,
                    password,
                    source_identifier: source,
                },
            )
            .await
            .map_err(Into::into)
    }
    .await;
    let release_result = release_backup_lock(&mut afc, &mut notification_proxy, lock_handle).await;
    let result = restore_result?;
    release_result?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "restored": true,
                "device_directory": result.layout.device_directory,
                "device_link_version": result.device_link_version,
                "protocol_version": result.protocol_version,
                "source_identifier": source_identifier,
                "system": system,
                "reboot": reboot,
                "copy": copy,
                "settings": settings,
                "remove": remove,
                "skip_apps": skip_apps,
            }))?
        );
    } else {
        println!(
            "Restore completed from {}",
            result.layout.device_directory.display()
        );
        println!("DeviceLinkVersion: {}", result.device_link_version);
        println!(
            "ProtocolVersion: {}",
            format_protocol_version(result.protocol_version)
        );
    }

    Ok(())
}

async fn run_change_password(
    udid: &str,
    old_password: Option<&str>,
    new_password: Option<&str>,
    backup_directory: &str,
    json: bool,
) -> Result<()> {
    if old_password.is_none() && new_password.is_none() {
        return Err(anyhow::anyhow!(
            "backup change-password requires at least one of --old-password or --new-password"
        ));
    }

    let device = connect(
        udid,
        ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: true,
        },
    )
    .await?;
    let stream = device
        .connect_service(ios_core::services::backup2::SERVICE_NAME)
        .await?;
    let mut client = ios_core::services::backup2::Mobilebackup2Client::new(stream);
    client
        .change_password(
            Path::new(backup_directory),
            udid,
            old_password,
            new_password,
        )
        .await?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "changed": true,
                "old_password_supplied": old_password.is_some(),
                "new_password_supplied": new_password.is_some(),
            }))?
        );
    } else {
        println!("Backup password change request completed.");
    }

    Ok(())
}

async fn build_backup_info_plist(device: &ios_core::ConnectedDevice) -> Result<plist::Dictionary> {
    let lockdown_values = device
        .lockdown_get_value(None)
        .await
        .map_err(|err| anyhow::anyhow!("read lockdown values for backup metadata: {err}"))?;
    let values = lockdown_values.as_dictionary().ok_or_else(|| {
        anyhow::anyhow!("lockdown GetValue(None, None) did not return a dictionary")
    })?;

    let min_itunes_version = values
        .get("com.apple.mobile.iTunes")
        .and_then(plist::Value::as_dictionary)
        .and_then(|dict| dict.get("MinITunesVersion"))
        .and_then(plist::Value::as_string)
        .filter(|value| !value.is_empty())
        .unwrap_or("10.0.1")
        .to_string();
    let unique_identifier = dict_get_string(values, "UniqueDeviceID")
        .or_else(|| dict_get_string(values, "UniqueChipID"))
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let device_name = dict_get_string(values, "DeviceName").unwrap_or_else(|| "iPhone".to_string());
    let mut installed_apps = Vec::new();
    let mut applications = plist::Dictionary::new();

    let springboard_stream = device
        .connect_service(ios_core::services::springboard::SERVICE_NAME)
        .await;
    let mut springboard = springboard_stream.ok().map(SpringboardClient::new);
    let install_stream = device
        .connect_service(ios_core::services::apps::installation::SERVICE_NAME)
        .await?;
    let mut install_proxy = InstallationProxy::new(install_stream);
    for app in install_proxy
        .list_user_apps_with_attributes(BACKUP_APP_RETURN_ATTRIBUTES)
        .await?
    {
        if app.bundle_id.is_empty() {
            continue;
        }
        installed_apps.push(app.bundle_id.clone());
        let application_sinf = app.extra.get("ApplicationSINF").cloned();
        let itunes_metadata = app.extra.get("iTunesMetadata").cloned();
        if let (Some(application_sinf), Some(itunes_metadata)) = (application_sinf, itunes_metadata)
        {
            let mut app_entry = plist::Dictionary::from_iter([
                ("ApplicationSINF".to_string(), application_sinf),
                ("iTunesMetadata".to_string(), itunes_metadata),
            ]);
            if let Some(client) = springboard.as_mut() {
                match client.get_icon_png_data(&app.bundle_id).await {
                    Ok(icon) => {
                        app_entry.insert("PlaceholderIcon".to_string(), plist::Value::Data(icon));
                    }
                    Err(err) => {
                        tracing::warn!(
                            "failed to read placeholder icon for {}: {}",
                            app.bundle_id,
                            err
                        );
                    }
                }
            }
            applications.insert(app.bundle_id.clone(), plist::Value::Dictionary(app_entry));
        }
    }
    let mut itunes_files = plist::Dictionary::new();
    if let Ok(stream) = device.connect_service(AFC_SERVICE_NAME).await {
        let mut afc = AfcClient::new(stream);
        for file_name in ITUNES_FILES {
            let path = format!("/iTunes_Control/iTunes/{file_name}");
            match afc.read_file(&path).await {
                Ok(bytes) => {
                    itunes_files
                        .insert((*file_name).to_string(), plist::Value::Data(bytes.to_vec()));
                }
                Err(AfcError::Status(AfcStatusCode::ObjectNotFound)) => {}
                Err(err) => {
                    tracing::warn!("failed to read backup metadata file {}: {}", path, err);
                }
            }
        }
    }

    let mut info = plist::Dictionary::from_iter([
        (
            "iTunes Version".to_string(),
            plist::Value::String(min_itunes_version.to_string()),
        ),
        (
            "Unique Identifier".to_string(),
            plist::Value::String(unique_identifier.to_uppercase()),
        ),
        (
            "Target Type".to_string(),
            plist::Value::String("Device".into()),
        ),
        (
            "Target Identifier".to_string(),
            plist::Value::String(unique_identifier.clone()),
        ),
        (
            "Serial Number".to_string(),
            plist::Value::String(dict_get_string(values, "SerialNumber").unwrap_or_default()),
        ),
        (
            "Product Version".to_string(),
            plist::Value::String(dict_get_string(values, "ProductVersion").unwrap_or_default()),
        ),
        (
            "Product Type".to_string(),
            plist::Value::String(dict_get_string(values, "ProductType").unwrap_or_default()),
        ),
        (
            "Installed Applications".to_string(),
            plist::Value::Array(
                installed_apps
                    .iter()
                    .cloned()
                    .map(plist::Value::String)
                    .collect(),
            ),
        ),
        (
            "GUID".to_string(),
            plist::Value::Data(build_backup_guid().to_vec()),
        ),
        (
            "iTunes Files".to_string(),
            plist::Value::Dictionary(itunes_files),
        ),
        (
            "Display Name".to_string(),
            plist::Value::String(device_name.clone()),
        ),
        ("Device Name".to_string(), plist::Value::String(device_name)),
        (
            "Build Version".to_string(),
            plist::Value::String(dict_get_string(values, "BuildVersion").unwrap_or_default()),
        ),
        (
            "Applications".to_string(),
            plist::Value::Dictionary(applications),
        ),
    ]);

    if let Some(value) = dict_get_string(values, "IntegratedCircuitCardIdentity") {
        info.insert("ICCID".to_string(), plist::Value::String(value));
    }
    if let Some(value) = dict_get_string(values, "InternationalMobileEquipmentIdentity") {
        info.insert("IMEI".to_string(), plist::Value::String(value));
    }
    if let Some(value) = dict_get_string(values, "MobileEquipmentIdentifier") {
        info.insert("MEID".to_string(), plist::Value::String(value));
    }
    if let Some(value) = dict_get_string(values, "PhoneNumber") {
        info.insert("Phone Number".to_string(), plist::Value::String(value));
    }
    if let Ok(itunes_settings) = device
        .lockdown_get_value_in_domain(Some("com.apple.iTunes"), None)
        .await
    {
        if itunes_settings
            .as_dictionary()
            .map(|dict| !dict.is_empty())
            .unwrap_or(false)
        {
            info.insert("iTunes Settings".to_string(), itunes_settings);
        }
    }
    if let Ok(stream) = device.connect_service(AFC_SERVICE_NAME).await {
        let mut afc = AfcClient::new(stream);
        match afc.read_file("/Books/iBooksData2.plist").await {
            Ok(bytes) => {
                info.insert(
                    "iBooks Data 2".to_string(),
                    plist::Value::Data(bytes.to_vec()),
                );
            }
            Err(AfcError::Status(AfcStatusCode::ObjectNotFound)) => {}
            Err(err) => {
                tracing::warn!("failed to read /Books/iBooksData2.plist: {}", err);
            }
        }
    }

    Ok(info)
}

fn dict_get_string(dict: &plist::Dictionary, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(plist::Value::as_string)
        .map(str::to_string)
}

fn build_backup_guid() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

fn format_protocol_version(version: f64) -> String {
    if version.fract() == 0.0 {
        format!("{version:.1}")
    } else {
        version.to_string()
    }
}

fn print_process_message_content(content: Option<plist::Value>, json: bool) -> Result<()> {
    let value = content.unwrap_or(plist::Value::Dictionary(plist::Dictionary::new()));
    if json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if let Some(text) = value.as_string() {
        println!("{text}");
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

fn plist_value_to_bool(value: &plist::Value) -> Option<bool> {
    match value {
        plist::Value::Boolean(value) => Some(*value),
        plist::Value::Integer(value) => value
            .as_signed()
            .map(|value| value != 0)
            .or_else(|| value.as_unsigned().map(|value| value != 0)),
        _ => None,
    }
}

fn resolve_will_encrypt<E>(value: std::result::Result<plist::Value, E>) -> Result<bool>
where
    E: std::fmt::Display,
{
    match value {
        Ok(value) => plist_value_to_bool(&value).ok_or_else(|| {
            anyhow::anyhow!("{WILL_ENCRYPT_KEY} was not a boolean-compatible value: {value:?}")
        }),
        Err(err) => {
            let err = err.to_string();
            if err.contains("MissingValue") {
                Ok(false)
            } else {
                Err(anyhow::anyhow!("failed to read {WILL_ENCRYPT_KEY}: {err}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ios_core::lockdown::LockdownError;

    use super::{
        build_backup_guid, format_protocol_version, plist_value_to_bool, resolve_will_encrypt,
    };
    use super::{BackupCmd, BackupSubcommand};

    #[derive(clap::Parser)]
    struct BackupTestCli {
        #[command(flatten)]
        backup: BackupCmd,
    }

    #[test]
    fn protocol_version_formatter_preserves_trailing_zero_for_whole_numbers() {
        assert_eq!(format_protocol_version(2.0), "2.0");
        assert_eq!(format_protocol_version(2.1), "2.1");
    }

    #[test]
    fn plist_value_to_bool_accepts_integer_backed_flags() {
        assert_eq!(
            plist_value_to_bool(&plist::Value::Integer(1.into())),
            Some(true)
        );
        assert_eq!(
            plist_value_to_bool(&plist::Value::Integer(0.into())),
            Some(false)
        );
    }

    #[test]
    fn resolve_will_encrypt_defaults_to_false_on_lockdown_errors() {
        let value = resolve_will_encrypt(Err(LockdownError::Protocol(
            "GetValue failed for domain=Some(\"com.apple.mobile.backup\") key=Some(\"WillEncrypt\"): MissingValue".into(),
        )))
        .expect("lockdown read failures should default to false");

        assert!(!value);
    }

    #[test]
    fn parses_backup_create_subcommand() {
        let parsed = BackupTestCli::try_parse_from(["backup", "create", "ios-rs-tmp/backup"])
            .expect("backup create command should parse");

        let BackupSubcommand::Create { output_dir, full } = parsed.backup.sub else {
            panic!("expected backup create subcommand");
        };
        assert_eq!(output_dir, "ios-rs-tmp/backup");
        assert!(!full, "backup create should default to incremental mode");
    }

    #[test]
    fn parses_backup_create_full_flag() {
        let parsed =
            BackupTestCli::try_parse_from(["backup", "create", "ios-rs-tmp/backup", "--full"])
                .expect("backup create --full command should parse");

        let BackupSubcommand::Create { output_dir, full } = parsed.backup.sub else {
            panic!("expected backup create subcommand");
        };
        assert_eq!(output_dir, "ios-rs-tmp/backup");
        assert!(full, "backup create should honor --full");
    }

    #[test]
    fn parses_backup_info_subcommand() {
        let parsed = BackupTestCli::try_parse_from([
            "backup",
            "info",
            "ios-rs-tmp/backup",
            "--source",
            "00008150-000A584C0E62401C",
        ]);
        assert!(parsed.is_ok(), "backup info command should parse");
    }

    #[test]
    fn parses_backup_list_subcommand() {
        let parsed = BackupTestCli::try_parse_from([
            "backup",
            "list",
            "ios-rs-tmp/backup",
            "--source",
            "00008150-000A584C0E62401C",
        ]);
        assert!(parsed.is_ok(), "backup list command should parse");
    }

    #[test]
    fn parses_backup_restore_subcommand() {
        let parsed = BackupTestCli::try_parse_from([
            "backup",
            "restore",
            "ios-rs-tmp/backup",
            "--source",
            "00008150-000A584C0E62401C",
            "--password",
            "example1234",
            "--skip-apps",
        ]);
        assert!(parsed.is_ok(), "backup restore command should parse");
    }

    #[test]
    fn parses_backup_change_password_subcommand() {
        let parsed = BackupTestCli::try_parse_from([
            "backup",
            "change-password",
            "--backup-directory",
            "ios-rs-tmp/backup",
            "--new-password",
            "example1234",
        ]);
        assert!(
            parsed.is_ok(),
            "backup change-password command should parse"
        );
    }

    #[test]
    fn resolve_will_encrypt_propagates_non_missing_value_errors() {
        let result = resolve_will_encrypt(Err(LockdownError::Protocol("connection reset".into())));
        assert!(result.is_err());
    }

    #[test]
    fn backup_guid_is_uuid_v4_bytes() {
        let guid = build_backup_guid();
        let parsed = uuid::Uuid::from_bytes(guid);

        assert_eq!(parsed.get_version_num(), 4);
    }
}

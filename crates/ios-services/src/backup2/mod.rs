use std::fs::{self, File};
use std::io::ErrorKind;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use std::time::SystemTime;

use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::warn;

use crate::device_link::{DeviceLinkClient, DeviceLinkError};

pub const SERVICE_NAME: &str = "com.apple.mobilebackup2";
pub const RSD_SERVICE_NAME: &str = "com.apple.mobilebackup2.shim.remote";
pub const SUPPORTED_PROTOCOL_VERSIONS: [f64; 2] = [2.0, 2.1];

const FILE_TRANSFER_CODE_SUCCESS: u8 = 0x00; // Transfer completed successfully
const FILE_TRANSFER_CODE_LOCAL_ERROR: u8 = 0x06; // Local (host) file I/O error
const FILE_TRANSFER_CODE_FILE_DATA: u8 = 0x0c; // Payload contains file data chunk
const FILE_TRANSFER_CODE_REMOTE_ERROR: u8 = 0x0b; // Remote (device) reported an error
const BULK_OPERATION_ERROR: i64 = -13;
const EMPTY_PARAMETER_STRING: &str = "___EmptyParameterString___";
const DOWNLOAD_CHUNK_SIZE: usize = 8 * 1024 * 1024;
// 978_307_200 seconds = 2001-01-01T00:00:00Z Unix timestamp
// This is the Apple Core Data / NSDate epoch offset (seconds between Unix epoch and Apple epoch)
const APPLE_EPOCH_OFFSET: Duration = Duration::from_secs(978_307_200);

#[derive(Debug, Clone, PartialEq)]
pub struct VersionExchange {
    pub device_link_version: u64,
    pub protocol_version: f64,
    pub local_versions: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupDirectoryLayout {
    pub root: PathBuf,
    pub device_directory: PathBuf,
    pub target_identifier: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BackupResult {
    pub layout: BackupDirectoryLayout,
    pub device_link_version: u64,
    pub protocol_version: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOptions<'a> {
    pub system: bool,
    pub reboot: bool,
    pub copy: bool,
    pub settings: bool,
    pub remove: bool,
    pub password: Option<&'a str>,
    pub source_identifier: Option<&'a str>,
}

impl Default for RestoreOptions<'_> {
    fn default() -> Self {
        Self {
            system: false,
            reboot: true,
            copy: false,
            settings: true,
            remove: false,
            password: None,
            source_identifier: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestoreResult {
    pub layout: BackupDirectoryLayout,
    pub device_link_version: u64,
    pub protocol_version: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum Mobilebackup2Error {
    #[error("device link error: {0}")]
    DeviceLink(#[from] DeviceLinkError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct Mobilebackup2Client<S> {
    device_link: DeviceLinkClient<S>,
}

impl<S> Mobilebackup2Client<S> {
    pub fn new(stream: S) -> Self {
        Self {
            device_link: DeviceLinkClient::new(stream),
        }
    }
}

impl<S> Mobilebackup2Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn version_exchange(&mut self) -> Result<VersionExchange, Mobilebackup2Error> {
        let device_link_version = self.device_link.version_exchange().await?;
        let local_versions = SUPPORTED_PROTOCOL_VERSIONS.to_vec();

        self.device_link
            .send_process_message(&HelloRequest {
                message_name: "Hello",
                supported_protocol_versions: local_versions.clone(),
            })
            .await?;

        let response = self.device_link.recv_process_message().await?;
        let error_code = response
            .get("ErrorCode")
            .and_then(plist_number_to_u64)
            .ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "backup2 hello response missing ErrorCode: {response:?}"
                ))
            })?;
        if error_code != 0 {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup2 hello returned ErrorCode={error_code}: {response:?}"
            )));
        }

        let protocol_version = response
            .get("ProtocolVersion")
            .and_then(plist_number_to_f64)
            .ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "backup2 hello response missing ProtocolVersion: {response:?}"
                ))
            })?;
        if !local_versions.contains(&protocol_version) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup2 negotiated unsupported protocol version {protocol_version}"
            )));
        }

        Ok(VersionExchange {
            device_link_version,
            protocol_version,
            local_versions,
        })
    }

    pub async fn backup(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        full: bool,
        info_plist: &plist::Dictionary,
    ) -> Result<BackupResult, Mobilebackup2Error> {
        let version = self.version_exchange().await?;
        let layout = initialize_backup_directory(backup_root, target_identifier, info_plist, full)?;

        self.device_link
            .send_process_message(&BackupRequest {
                message_name: "Backup",
                target_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let _ = self.finish_session(run_result).await?;

        Ok(BackupResult {
            layout,
            device_link_version: version.device_link_version,
            protocol_version: version.protocol_version,
        })
    }

    pub async fn change_password(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        old_password: Option<&str>,
        new_password: Option<&str>,
    ) -> Result<(), Mobilebackup2Error> {
        let _ = self.version_exchange().await?;
        let layout = create_runtime_layout(backup_root, target_identifier)?;

        self.device_link
            .send_process_message(&ChangePasswordRequest {
                message_name: "ChangePassword",
                target_identifier,
                old_password,
                new_password,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let _ = self.finish_session(run_result).await?;
        Ok(())
    }

    pub async fn restore(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        options: RestoreOptions<'_>,
    ) -> Result<RestoreResult, Mobilebackup2Error> {
        let source_identifier = options.source_identifier.unwrap_or(target_identifier);
        ensure_backup_directory(backup_root, source_identifier)?;
        let layout = create_runtime_layout(backup_root, source_identifier)?;
        let manifest = read_backup_dictionary(&layout.device_directory.join("Manifest.plist"))?;
        let password = if manifest
            .get("IsEncrypted")
            .and_then(plist_value_to_bool)
            .unwrap_or(false)
        {
            Some(options.password.ok_or_else(|| {
                Mobilebackup2Error::Protocol(
                    "backup is encrypted; restore requires a password".into(),
                )
            })?)
        } else {
            None
        };
        let version = self.version_exchange().await?;

        self.device_link
            .send_process_message(&RestoreRequest {
                message_name: "Restore",
                target_identifier,
                source_identifier,
                password,
                options: RestoreRequestOptions {
                    restore_should_reboot: options.reboot,
                    restore_dont_copy_backup: !options.copy,
                    restore_preserve_settings: options.settings,
                    restore_system_files: options.system,
                    remove_items_not_restored: options.remove,
                },
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let _ = self.finish_session(run_result).await?;
        Ok(RestoreResult {
            layout,
            device_link_version: version.device_link_version,
            protocol_version: version.protocol_version,
        })
    }

    pub async fn info(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        let _ = self.version_exchange().await?;
        let layout_identifier = source_identifier.unwrap_or(target_identifier);
        ensure_backup_directory(backup_root, layout_identifier)?;
        let layout = create_runtime_layout(backup_root, layout_identifier)?;

        self.device_link
            .send_process_message(&InfoRequest {
                message_name: "Info",
                target_identifier,
                source_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        self.finish_session(run_result).await
    }

    pub async fn list(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        let _ = self.version_exchange().await?;
        let source_identifier = source_identifier.unwrap_or(target_identifier);
        ensure_backup_directory(backup_root, source_identifier)?;
        let layout = create_runtime_layout(backup_root, source_identifier)?;

        self.device_link
            .send_process_message(&ListRequest {
                message_name: "List",
                target_identifier,
                source_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        self.finish_session(run_result).await
    }

    async fn disconnect_best_effort(&mut self) {
        if let Err(err) = self.device_link.disconnect().await {
            if !should_suppress_disconnect_error(&err) {
                warn!("backup2 disconnect failed: {err}");
            }
        }
    }

    async fn finish_session<T>(
        &mut self,
        result: Result<T, Mobilebackup2Error>,
    ) -> Result<T, Mobilebackup2Error> {
        self.disconnect_best_effort().await;
        result
    }

    async fn run_loop(
        &mut self,
        layout: &BackupDirectoryLayout,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        loop {
            let message = self.device_link.recv_message().await?;
            let parts = message.as_array().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "device link loop expected array message, got {message:?}"
                ))
            })?;

            let command = parts
                .first()
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "device link message missing command: {message:?}"
                    ))
                })?;
            match command {
                "DLMessageProcessMessage" => {
                    let payload = parts
                        .get(1)
                        .and_then(plist::Value::as_dictionary)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "process message missing dictionary payload: {message:?}"
                            ))
                        })?;
                    let error_code = payload.get("ErrorCode").and_then(plist_number_to_u64);
                    if let Some(code) = error_code {
                        if code != 0 {
                            return Err(Mobilebackup2Error::Protocol(format!(
                                "backup process returned ErrorCode={code}: {payload:?}"
                            )));
                        }
                    }
                    return Ok(payload.get("Content").cloned());
                }
                "DLMessageCreateDirectory" => {
                    let path = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "create directory missing path: {message:?}"
                            ))
                        })?;
                    fs::create_dir_all(resolve_relative_path(layout, path)?)?;
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessageUploadFiles" => {
                    self.receive_uploaded_files(layout).await?;
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessageDownloadFiles" => {
                    let files = parts
                        .get(1)
                        .and_then(plist::Value::as_array)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "download files missing array payload: {message:?}"
                            ))
                        })?;
                    let (status_code, status_message, status_payload) =
                        self.send_requested_files(layout, files).await?;
                    self.send_status_response(status_code, &status_message, status_payload)
                        .await?;
                }
                "DLMessageGetFreeDiskSpace" => {
                    let free_bytes = available_space(&layout.device_directory)?;
                    self.send_status_response(0, "", plist::Value::Integer(free_bytes.into()))
                        .await?;
                }
                "DLMessageMoveItems" | "DLMessageMoveFiles" => {
                    let items = parts
                        .get(1)
                        .and_then(plist::Value::as_dictionary)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "move items missing mapping payload: {message:?}"
                            ))
                        })?;
                    for (src, dst_value) in items {
                        let dst = dst_value.as_string().ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "move target for {src} was not a string: {message:?}"
                            ))
                        })?;
                        let src_path = resolve_relative_path(layout, src)?;
                        let dst_path = resolve_relative_path(layout, dst)?;
                        if let Some(parent) = dst_path.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        fs::rename(src_path, dst_path)?;
                    }
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessageRemoveItems" | "DLMessageRemoveFiles" => {
                    let items = parts
                        .get(1)
                        .and_then(plist::Value::as_array)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "remove items missing array payload: {message:?}"
                            ))
                        })?;
                    for item in items {
                        let rel = item.as_string().ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "remove item path was not a string: {message:?}"
                            ))
                        })?;
                        let target = resolve_relative_path(layout, rel)?;
                        if target.is_dir() {
                            fs::remove_dir_all(target)?;
                        } else if target.exists() {
                            fs::remove_file(target)?;
                        }
                    }
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLContentsOfDirectory" => {
                    let rel = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "contents-of-directory missing path: {message:?}"
                            ))
                        })?;
                    let path = resolve_relative_path(layout, rel)?;
                    let listing = contents_of_directory(&path)?;
                    self.send_status_response(0, "", plist::Value::Dictionary(listing))
                        .await?;
                }
                "DLMessageCopyItem" => {
                    let src = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "copy item missing source: {message:?}"
                            ))
                        })?;
                    let dst = parts
                        .get(2)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "copy item missing destination: {message:?}"
                            ))
                        })?;
                    copy_item(
                        &resolve_relative_path(layout, src)?,
                        &resolve_relative_path(layout, dst)?,
                    )?;
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessagePurgeDiskSpace" => {
                    return Err(Mobilebackup2Error::Protocol(
                        "backup host cannot purge disk space automatically".into(),
                    ));
                }
                other => {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "unsupported backup device-link command {other}: {message:?}"
                    )));
                }
            }
        }
    }

    async fn receive_uploaded_files(
        &mut self,
        layout: &BackupDirectoryLayout,
    ) -> Result<(), Mobilebackup2Error> {
        loop {
            let device_name = read_prefixed_string(self.device_link.stream_mut()).await?;
            if device_name.is_empty() {
                break;
            }

            let file_name = read_prefixed_string(self.device_link.stream_mut()).await?;
            let output_path = resolve_relative_path(layout, &file_name)?;
            if let Some(parent) = output_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(&output_path)?;

            loop {
                let frame_size = read_u32_be(self.device_link.stream_mut()).await?;
                let mut code = [0u8; 1];
                self.device_link.stream_mut().read_exact(&mut code).await?;
                let payload_len = frame_size.checked_sub(1).ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "backup file transfer frame too short for {file_name}"
                    ))
                })? as usize;
                let mut payload = vec![0u8; payload_len];
                self.device_link
                    .stream_mut()
                    .read_exact(&mut payload)
                    .await?;

                match code[0] {
                    FILE_TRANSFER_CODE_FILE_DATA => file.write_all(&payload)?,
                    FILE_TRANSFER_CODE_SUCCESS => break,
                    FILE_TRANSFER_CODE_REMOTE_ERROR => {
                        let message = String::from_utf8_lossy(&payload);
                        warn!(
                            "backup upload for device path '{}' to local file '{}' reported remote error: {}",
                            device_name,
                            file_name,
                            message
                        );
                        break;
                    }
                    other => {
                        return Err(Mobilebackup2Error::Protocol(format!(
                            "unknown backup file transfer code 0x{other:02x} for {file_name}"
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    async fn send_status_response(
        &mut self,
        status_code: i64,
        status_message: &str,
        status_payload: plist::Value,
    ) -> Result<(), Mobilebackup2Error> {
        self.device_link
            .send_message(&vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(status_code.into()),
                plist::Value::String(
                    if status_message.is_empty() {
                        EMPTY_PARAMETER_STRING
                    } else {
                        status_message
                    }
                    .into(),
                ),
                status_payload,
            ])
            .await?;
        Ok(())
    }

    async fn send_requested_files(
        &mut self,
        layout: &BackupDirectoryLayout,
        files: &[plist::Value],
    ) -> Result<(i64, String, plist::Value), Mobilebackup2Error> {
        let mut failures = plist::Dictionary::new();
        for file in files {
            let rel = file.as_string().ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "download file path was not a string: {file:?}"
                ))
            })?;
            let local_path = resolve_relative_path(layout, rel)?;
            write_prefixed_string(self.device_link.stream_mut(), rel).await?;

            match fs::read(&local_path) {
                Ok(contents) => {
                    let mut offset = 0usize;
                    while offset < contents.len() {
                        let end = (offset + DOWNLOAD_CHUNK_SIZE).min(contents.len());
                        write_transfer_frame(
                            self.device_link.stream_mut(),
                            FILE_TRANSFER_CODE_FILE_DATA,
                            &contents[offset..end],
                        )
                        .await?;
                        offset = end;
                    }
                    write_transfer_frame(
                        self.device_link.stream_mut(),
                        FILE_TRANSFER_CODE_SUCCESS,
                        &[],
                    )
                    .await?;
                }
                Err(err) => {
                    let mut failure = plist::Dictionary::from_iter([(
                        "DLFileErrorString".to_string(),
                        plist::Value::String(err.to_string()),
                    )]);
                    if let Some(code) = file_error_code_from_os_error(&err) {
                        failure.insert(
                            "DLFileErrorCode".to_string(),
                            plist::Value::Integer(code.into()),
                        );
                    }
                    failures.insert(rel.to_string(), plist::Value::Dictionary(failure));
                    write_transfer_frame(
                        self.device_link.stream_mut(),
                        FILE_TRANSFER_CODE_LOCAL_ERROR,
                        err.to_string().as_bytes(),
                    )
                    .await?;
                }
            }
        }

        self.device_link
            .stream_mut()
            .write_all(&0u32.to_be_bytes())
            .await?;
        self.device_link.stream_mut().flush().await?;
        if failures.is_empty() {
            Ok((
                0,
                String::new(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ))
        } else {
            Ok((
                BULK_OPERATION_ERROR,
                "Multi status".to_string(),
                plist::Value::Dictionary(failures),
            ))
        }
    }
}

pub fn initialize_backup_directory(
    backup_root: &Path,
    target_identifier: &str,
    info_plist: &plist::Dictionary,
    full: bool,
) -> Result<BackupDirectoryLayout, Mobilebackup2Error> {
    let root = backup_root.to_path_buf();
    let device_directory = root.join(target_identifier);
    fs::create_dir_all(&device_directory)?;

    let mut info_file = File::create(device_directory.join("Info.plist"))?;
    plist::to_writer_xml(
        &mut info_file,
        &plist::Value::Dictionary(info_plist.clone()),
    )
    .map_err(|e| Mobilebackup2Error::Plist(e.to_string()))?;

    let status = plist::Dictionary::from_iter([
        (
            "BackupState".to_string(),
            plist::Value::String("new".into()),
        ),
        (
            "Date".to_string(),
            plist::Value::Date(plist::Date::from(SystemTime::now())),
        ),
        ("IsFullBackup".to_string(), plist::Value::Boolean(full)),
        ("Version".to_string(), plist::Value::String("3.3".into())),
        (
            "SnapshotState".to_string(),
            plist::Value::String("finished".into()),
        ),
        (
            "UUID".to_string(),
            plist::Value::String(generate_backup_uuid()),
        ),
    ]);
    let mut status_file = File::create(device_directory.join("Status.plist"))?;
    plist::to_writer_binary(&mut status_file, &plist::Value::Dictionary(status))
        .map_err(|e| Mobilebackup2Error::Plist(e.to_string()))?;

    let manifest_path = device_directory.join("Manifest.plist");
    if full && manifest_path.exists() {
        fs::remove_file(&manifest_path)?;
    }
    let _ = File::create(&manifest_path)?;

    Ok(BackupDirectoryLayout {
        root,
        device_directory,
        target_identifier: target_identifier.to_string(),
    })
}

fn create_runtime_layout(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<BackupDirectoryLayout, Mobilebackup2Error> {
    let root = backup_root.to_path_buf();
    let device_directory = root.join(target_identifier);
    fs::create_dir_all(&device_directory)?;
    Ok(BackupDirectoryLayout {
        root,
        device_directory,
        target_identifier: target_identifier.to_string(),
    })
}

fn ensure_backup_directory(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<(), Mobilebackup2Error> {
    let device_directory = backup_root.join(target_identifier);
    for file_name in ["Info.plist", "Manifest.plist", "Status.plist"] {
        let path = device_directory.join(file_name);
        if !path.exists() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup directory missing required file {}",
                path.display()
            )));
        }
    }
    Ok(())
}

pub fn load_backup_applications(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<Option<plist::Value>, Mobilebackup2Error> {
    ensure_backup_directory(backup_root, target_identifier)?;
    let info = plist::Value::from_file(backup_root.join(target_identifier).join("Info.plist"))
        .map_err(|err| Mobilebackup2Error::Plist(err.to_string()))?;
    Ok(info
        .as_dictionary()
        .and_then(|dict| dict.get("Applications"))
        .cloned())
}

pub fn backup_is_encrypted(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<bool, Mobilebackup2Error> {
    ensure_backup_directory(backup_root, target_identifier)?;
    Ok(
        read_backup_dictionary(&backup_root.join(target_identifier).join("Manifest.plist"))?
            .get("IsEncrypted")
            .and_then(plist_value_to_bool)
            .unwrap_or(false),
    )
}

fn read_backup_dictionary(path: &Path) -> Result<plist::Dictionary, Mobilebackup2Error> {
    plist::Value::from_file(path)
        .map_err(|err| Mobilebackup2Error::Plist(err.to_string()))?
        .into_dictionary()
        .ok_or_else(|| {
            Mobilebackup2Error::Protocol(format!(
                "expected plist dictionary in backup metadata file {}",
                path.display()
            ))
        })
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct HelloRequest {
    message_name: &'static str,
    supported_protocol_versions: Vec<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct BackupRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct RestoreRequestOptions {
    restore_should_reboot: bool,
    restore_dont_copy_backup: bool,
    restore_preserve_settings: bool,
    restore_system_files: bool,
    remove_items_not_restored: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct RestoreRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    source_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<&'a str>,
    options: RestoreRequestOptions,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ChangePasswordRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_password: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_password: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct InfoRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_identifier: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ListRequest<'a> {
    message_name: &'static str,
    target_identifier: &'a str,
    source_identifier: &'a str,
}

// A fresh random UUID is generated for each backup session.
// Backup UUIDs are not required to be deterministic across sessions.
fn generate_backup_uuid() -> String {
    uuid::Uuid::new_v4().to_string().to_uppercase()
}

fn sanitize_relative_path(path: &str) -> Result<PathBuf, Mobilebackup2Error> {
    let mut clean = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path escapes backup root: {path}"
                )));
            }
        }
    }

    Ok(clean)
}

fn resolve_relative_path(
    layout: &BackupDirectoryLayout,
    rel: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let clean = sanitize_relative_path(rel)?;
    let prefixed_with_target = clean
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        == Some(layout.target_identifier.as_str());

    Ok(if prefixed_with_target {
        layout.root.join(clean)
    } else {
        layout.device_directory.join(clean)
    })
}

fn copy_item(src: &Path, dst: &Path) -> Result<(), Mobilebackup2Error> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }

    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_item(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        fs::copy(src, dst)?;
    }

    Ok(())
}

fn contents_of_directory(path: &Path) -> Result<plist::Dictionary, Mobilebackup2Error> {
    let mut entries = plist::Dictionary::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let file_type = if metadata.is_dir() {
            "DLFileTypeDirectory"
        } else if metadata.is_file() {
            "DLFileTypeRegular"
        } else {
            "DLFileTypeUnknown"
        };
        let modified = metadata.modified().unwrap_or_else(|err| {
            tracing::debug!("cannot read mtime for {}: {err}", entry.path().display());
            SystemTime::UNIX_EPOCH
        });
        entries.insert(
            entry.file_name().to_string_lossy().into_owned(),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "DLFileType".to_string(),
                    plist::Value::String(file_type.into()),
                ),
                (
                    "DLFileSize".to_string(),
                    plist::Value::Integer(metadata.len().into()),
                ),
                (
                    "DLFileModificationDate".to_string(),
                    plist::Value::Date(device_link_modification_date(modified)),
                ),
            ])),
        );
    }

    Ok(entries)
}

fn device_link_modification_date(modified: SystemTime) -> plist::Date {
    // pymobiledevice3 encodes directory mtimes as local wall-clock time relative to Apple's
    // 2001 epoch, then serializes that wall-clock timestamp as if it were UTC.
    let modified = device_link_local_wall_clock(modified);
    let shifted = modified
        .checked_sub(APPLE_EPOCH_OFFSET)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    plist::Date::from(shifted)
}

fn device_link_local_wall_clock(modified: SystemTime) -> SystemTime {
    let utc = OffsetDateTime::from(modified);
    let local_offset = UtcOffset::local_offset_at(utc).unwrap_or(UtcOffset::UTC);
    let local_wall_clock = utc.to_offset(local_offset).replace_offset(UtcOffset::UTC);
    local_wall_clock.into()
}

async fn read_prefixed_string<S>(stream: &mut S) -> Result<String, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let size = read_u32_be(stream).await? as usize;
    if size == 0 {
        return Ok(String::new());
    }

    let mut buf = vec![0u8; size];
    stream.read_exact(&mut buf).await?;
    String::from_utf8(buf)
        .map_err(|err| Mobilebackup2Error::Protocol(format!("backup path was not utf-8: {err}")))
}

async fn read_u32_be<S>(stream: &mut S) -> Result<u32, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let mut buf = [0u8; 4];
    stream.read_exact(&mut buf).await?;
    Ok(u32::from_be_bytes(buf))
}

async fn write_prefixed_string<S>(stream: &mut S, value: &str) -> Result<(), Mobilebackup2Error>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&(value.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(value.as_bytes()).await?;
    Ok(())
}

async fn write_transfer_frame<S>(
    stream: &mut S,
    code: u8,
    payload: &[u8],
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncWrite + Unpin,
{
    stream
        .write_all(&((payload.len() as u32) + 1).to_be_bytes())
        .await?;
    stream.write_all(&[code]).await?;
    if !payload.is_empty() {
        stream.write_all(payload).await?;
    }
    Ok(())
}

#[cfg(windows)]
fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let probe = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    let wide: Vec<u16> = OsStr::new(probe.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut available = 0u64;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(Mobilebackup2Error::Io(std::io::Error::last_os_error()));
    }
    Ok(available)
}

#[cfg(not(windows))]
fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
    let _ = path;
    Ok(0)
}

fn plist_number_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned(),
        plist::Value::Real(value) => Some(*value as u64),
        _ => None,
    }
}

fn plist_number_to_f64(value: &plist::Value) -> Option<f64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned().map(|value| value as f64),
        plist::Value::Real(value) => Some(*value),
        _ => None,
    }
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

fn file_error_code_from_os_error(error: &std::io::Error) -> Option<i64> {
    match error.raw_os_error()? {
        2 => Some(-6),
        17 => Some(-7),
        20 => Some(-8),
        21 => Some(-9),
        62 => Some(-10),
        5 => Some(-11),
        28 => Some(-15),
        _ => None,
    }
}

fn should_suppress_disconnect_error(error: &DeviceLinkError) -> bool {
    matches!(
        error,
        DeviceLinkError::Io(io_error)
            if matches!(
                io_error.kind(),
                ErrorKind::BrokenPipe
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::ConnectionReset
                    | ErrorKind::NotConnected
                    | ErrorKind::UnexpectedEof
            )
    )
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;

    #[test]
    fn initialize_backup_directory_creates_expected_seed_files() {
        let root =
            std::env::temp_dir().join(format!("ios-tunnel-backup2-layout-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();

        let info = plist::Dictionary::from_iter([(
            "Device Name".to_string(),
            plist::Value::String("Codex".into()),
        )]);
        let layout = initialize_backup_directory(&root, "device-id", &info, true).unwrap();

        assert_eq!(layout.device_directory, root.join("device-id"));
        assert!(layout.device_directory.join("Info.plist").exists());
        assert!(layout.device_directory.join("Status.plist").exists());
        assert!(layout.device_directory.join("Manifest.plist").exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_relative_path_accepts_plain_and_prefixed_paths() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };

        assert_eq!(
            resolve_relative_path(&layout, "Manifest.db").unwrap(),
            PathBuf::from("backup-root/device-id/Manifest.db")
        );
        assert_eq!(
            resolve_relative_path(&layout, "device-id/Manifest.db").unwrap(),
            PathBuf::from("backup-root/device-id/Manifest.db")
        );
    }

    #[test]
    fn resolve_relative_path_rejects_parent_escapes() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };

        let err = resolve_relative_path(&layout, "../outside").unwrap_err();
        assert!(err.to_string().contains("escapes"));
    }

    #[test]
    fn generated_backup_uuid_is_uppercase_v4() {
        let generated = generate_backup_uuid();
        let parsed = uuid::Uuid::parse_str(&generated).expect("status UUID should be parseable");

        assert_eq!(generated, generated.to_uppercase());
        assert_eq!(parsed.get_version_num(), 4);
    }

    #[test]
    fn backup_is_encrypted_reads_manifest_flag() {
        let root = std::env::temp_dir().join(format!(
            "ios-tunnel-backup2-encryption-{}",
            std::process::id()
        ));
        let device_dir = root.join("device-id");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&device_dir).unwrap();
        std::fs::write(device_dir.join("Info.plist"), b"info").unwrap();
        plist::to_file_xml(
            device_dir.join("Manifest.plist"),
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "IsEncrypted".to_string(),
                plist::Value::Boolean(true),
            )])),
        )
        .unwrap();
        std::fs::write(device_dir.join("Status.plist"), b"status").unwrap();

        assert!(backup_is_encrypted(&root, "device-id").unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn device_link_modification_date_preserves_subsecond_apple_epoch_timestamp() {
        let modified = SystemTime::UNIX_EPOCH
            + APPLE_EPOCH_OFFSET
            + Duration::from_secs(123)
            + Duration::from_millis(900);
        let encoded = device_link_modification_date(modified);
        let shifted: SystemTime = encoded.into();
        let expected = device_link_local_wall_clock(modified)
            .checked_sub(APPLE_EPOCH_OFFSET)
            .unwrap_or(SystemTime::UNIX_EPOCH);

        assert_eq!(shifted, expected);
    }

    #[test]
    fn suppresses_expected_disconnect_transport_errors() {
        for kind in [
            ErrorKind::BrokenPipe,
            ErrorKind::ConnectionAborted,
            ErrorKind::ConnectionReset,
            ErrorKind::NotConnected,
            ErrorKind::UnexpectedEof,
        ] {
            assert!(should_suppress_disconnect_error(&DeviceLinkError::Io(
                std::io::Error::from(kind),
            )));
        }
    }

    #[test]
    fn keeps_unexpected_disconnect_errors_visible() {
        assert!(!should_suppress_disconnect_error(&DeviceLinkError::Io(
            std::io::Error::from(ErrorKind::Other),
        )));
        assert!(!should_suppress_disconnect_error(
            &DeviceLinkError::Protocol("disconnect protocol mismatch".into(),)
        ));
    }
}

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use std::time::SystemTime;

use serde::Serialize;
use time::{OffsetDateTime, UtcOffset};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::warn;

use crate::services::device_link::{DeviceLinkClient, DeviceLinkError};

pub const SERVICE_NAME: &str = "com.apple.mobilebackup2";
pub const RSD_SERVICE_NAME: &str = "com.apple.mobilebackup2.shim.remote";
pub const SUPPORTED_PROTOCOL_VERSIONS: [f64; 2] = [2.0, 2.1];

const FILE_TRANSFER_CODE_SUCCESS: u8 = 0x00; // Transfer completed successfully
const FILE_TRANSFER_CODE_LOCAL_ERROR: u8 = 0x06; // Local (host) file I/O error
const FILE_TRANSFER_CODE_FILE_DATA: u8 = 0x0c; // Payload contains file data chunk
const FILE_TRANSFER_CODE_REMOTE_ERROR: u8 = 0x0b; // Remote (device) reported an error
const BULK_OPERATION_ERROR: i64 = -13;
const PURGE_DISK_SPACE_ERROR: i64 = -1;
const PURGE_DISK_SPACE_ERROR_STRING: &str = "DLPurgeDiskSpace failed to purge";
// BackupAgent2 adds this amount to every purge request beyond the actual free-space threshold.
const PURGE_REQUEST_OVERSHOOT: u64 = 0x8000_0000;
const MB_ERROR_INSUFFICIENT_DISK_SPACE: u64 = 105;
const EMPTY_PARAMETER_STRING: &str = "___EmptyParameterString___";
// BackupAgent2 buffers one complete transfer frame before writing it to disk. Keep frames small
// enough for large incremental Manifest.db files to avoid exhausting the device process.
const DOWNLOAD_CHUNK_SIZE: usize = 32 * 1024;
const MAX_TRANSFER_ERROR_PREVIEW_SIZE: usize = 64 * 1024;
const INCREMENTAL_BACKUP_REQUIRED_FILES: &[&str] =
    &["Manifest.plist", "Manifest.db", "Status.plist"];
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

service_error!(
    Mobilebackup2Error,
    before {
    #[error("device link error: {0}")]
    DeviceLink(#[from] DeviceLinkError),
    },
    after {},
);

pub struct Mobilebackup2Client<S> {
    device_link: DeviceLinkClient<S>,
    // DeviceLink reports free space before asking the host to purge. Keeping the last pair lets
    // us turn the device's follow-up MBErrorDomain/105 into a useful diagnostic.
    reported_free_space: Option<u64>,
    required_free_space: Option<u64>,
}

impl<S> Mobilebackup2Client<S> {
    pub fn new(stream: S) -> Self {
        Self {
            device_link: DeviceLinkClient::new(stream),
            reported_free_space: None,
            required_free_space: None,
        }
    }
}

impl<S> Mobilebackup2Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn version_exchange(&mut self) -> Result<VersionExchange, Mobilebackup2Error> {
        // A client can be reused for several operations in tests or by higher-level callers;
        // diagnostics from a previous DeviceLink session must not leak into the next one.
        self.reported_free_space = None;
        self.required_free_space = None;

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
        validate_backup_identifier(target_identifier)?;
        let version = self.version_exchange().await?;
        let layout = {
            let root = backup_root.to_path_buf();
            let id = target_identifier.to_owned();
            let info = info_plist.clone();
            tokio::task::spawn_blocking(move || {
                initialize_backup_directory(&root, &id, &info, full)
            })
            .await
            .map_err(|e| Mobilebackup2Error::Io(std::io::Error::other(e.to_string())))?
        }?;

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
        validate_backup_identifier(target_identifier)?;
        let _ = self.version_exchange().await?;
        let layout = {
            let root = backup_root.to_path_buf();
            let id = target_identifier.to_owned();
            run_blocking(move || create_runtime_layout(&root, &id)).await?
        };

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
        validate_backup_identifier(target_identifier)?;
        validate_backup_identifier(source_identifier)?;
        let (layout, manifest) = {
            let root = backup_root.to_path_buf();
            let id = source_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                let layout = create_runtime_layout(&root, &id)?;
                let manifest = read_backup_dictionary(&metadata_file_path(
                    &layout.root,
                    &layout.target_identifier,
                    "Manifest.plist",
                )?)?;
                Ok((layout, manifest))
            })
            .await?
        };
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
        validate_backup_identifier(target_identifier)?;
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
        let _ = self.version_exchange().await?;
        let layout_identifier = source_identifier.unwrap_or(target_identifier);
        let layout = {
            let root = backup_root.to_path_buf();
            let id = layout_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                create_runtime_layout(&root, &id)
            })
            .await?
        };

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
        validate_backup_identifier(target_identifier)?;
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
        let _ = self.version_exchange().await?;
        let source_identifier = source_identifier.unwrap_or(target_identifier);
        let layout = {
            let root = backup_root.to_path_buf();
            let id = source_identifier.to_owned();
            run_blocking(move || {
                ensure_backup_directory(&root, &id)?;
                create_runtime_layout(&root, &id)
            })
            .await?
        };

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
                            if code == MB_ERROR_INSUFFICIENT_DISK_SPACE {
                                return Err(Mobilebackup2Error::Protocol(
                                    insufficient_disk_space_message(
                                        payload,
                                        self.reported_free_space,
                                        self.required_free_space,
                                    ),
                                ));
                            }
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
                    let directory = resolve_relative_path(layout, path)?;
                    create_layout_directory(layout, &directory)?;
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
                    let device_directory = layout.device_directory.clone();
                    let free_bytes =
                        run_blocking(move || available_space(&device_directory)).await?;
                    tracing::debug!(
                        free_bytes,
                        path = %layout.device_directory.display(),
                        "reporting available backup disk space"
                    );
                    self.reported_free_space = Some(free_bytes);
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
                        create_layout_parent_directory(layout, &dst_path)?;
                        rename_layout_path(layout, &src_path, &dst_path).await?;
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
                        remove_layout_path(layout, &target).await?;
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
                    let root = layout.root.clone();
                    let listing =
                        tokio::task::spawn_blocking(move || contents_of_directory(&root, &path))
                            .await
                            .map_err(|e| {
                                Mobilebackup2Error::Io(std::io::Error::other(e.to_string()))
                            })??;
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
                    let src_path = resolve_relative_path(layout, src)?;
                    let dst_path = resolve_relative_path(layout, dst)?;
                    let root = layout.root.clone();
                    tokio::task::spawn_blocking(move || copy_item(&root, &src_path, &dst_path))
                        .await
                        .map_err(|e| {
                            Mobilebackup2Error::Io(std::io::Error::other(e.to_string()))
                        })??;
                    self.send_status_response(
                        0,
                        "",
                        plist::Value::Dictionary(plist::Dictionary::new()),
                    )
                    .await?;
                }
                "DLMessagePurgeDiskSpace" => {
                    let requested = parts.get(1).and_then(plist_number_to_u64);
                    let urgency = parts.get(2).and_then(plist_number_to_u64);
                    self.required_free_space =
                        derive_required_free_space(self.reported_free_space, requested);
                    if let Some(required) = self.required_free_space {
                        let reported = self
                            .reported_free_space
                            .expect("derived free-space requirement has a report");
                        let additional = required.saturating_add(1).saturating_sub(reported);
                        tracing::warn!(
                            required_free_space = required,
                            reported_free_space = reported,
                            additional_free_space = additional,
                            requested,
                            urgency,
                            "device requested host disk-space purge; no purge backend is available"
                        );
                    } else {
                        tracing::warn!(
                            requested,
                            urgency,
                            "device requested host disk-space purge; no purge backend is available"
                        );
                    }
                    // Purging is a request, not a terminal protocol error. Apple's host replies
                    // with the purge-failed status and lets the device report its own diagnosis.
                    self.send_status_response_value(
                        plist::Value::Integer(PURGE_DISK_SPACE_ERROR.into()),
                        PURGE_DISK_SPACE_ERROR_STRING,
                        plist::Value::Integer(0u64.into()),
                    )
                    .await?;
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
            create_layout_parent_directory(layout, &output_path)?;
            let mut file =
                tokio::fs::File::from_std(open_layout_file_for_write(layout, &output_path)?);

            loop {
                let frame_size = read_u32_be(self.device_link.stream_mut()).await?;
                let mut code = [0u8; 1];
                self.device_link.stream_mut().read_exact(&mut code).await?;
                let payload_len = frame_size.checked_sub(1).ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "backup file transfer frame too short for {file_name}"
                    ))
                })? as usize;

                match code[0] {
                    FILE_TRANSFER_CODE_FILE_DATA => {
                        // A device can advertise a very large frame size. Stream its payload in
                        // bounded pieces instead of allocating the complete frame up front.
                        copy_transfer_payload(
                            self.device_link.stream_mut(),
                            &mut file,
                            payload_len,
                        )
                        .await?;
                    }
                    FILE_TRANSFER_CODE_SUCCESS => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        break;
                    }
                    FILE_TRANSFER_CODE_REMOTE_ERROR => {
                        let message =
                            read_transfer_error_preview(self.device_link.stream_mut(), payload_len)
                                .await?;
                        warn!(
                            "backup upload for device path '{}' to local file '{}' reported remote error: {}",
                            device_name,
                            file_name,
                            message
                        );
                        break;
                    }
                    other => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        return Err(Mobilebackup2Error::Protocol(format!(
                            "unknown backup file transfer code 0x{other:02x} for {file_name}"
                        )));
                    }
                }
            }
            file.flush().await?;
        }

        Ok(())
    }

    async fn send_status_response(
        &mut self,
        status_code: i64,
        status_message: &str,
        status_payload: plist::Value,
    ) -> Result<(), Mobilebackup2Error> {
        self.send_status_response_value(
            plist::Value::Integer(status_code.into()),
            status_message,
            status_payload,
        )
        .await
    }

    async fn send_status_response_value(
        &mut self,
        status_code: plist::Value,
        status_message: &str,
        status_payload: plist::Value,
    ) -> Result<(), Mobilebackup2Error> {
        self.device_link
            .send_message(&vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                status_code,
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

            match open_layout_file_for_read(layout, &local_path) {
                Ok(file) => {
                    let mut file = tokio::fs::File::from_std(file);
                    let mut buf = vec![0u8; DOWNLOAD_CHUNK_SIZE];
                    let mut read_failed = false;
                    loop {
                        let n = match file.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(err) => {
                                insert_file_failure(&mut failures, rel, &err);
                                write_transfer_frame(
                                    self.device_link.stream_mut(),
                                    FILE_TRANSFER_CODE_LOCAL_ERROR,
                                    err.to_string().as_bytes(),
                                )
                                .await?;
                                read_failed = true;
                                break;
                            }
                        };
                        write_transfer_frame(
                            self.device_link.stream_mut(),
                            FILE_TRANSFER_CODE_FILE_DATA,
                            &buf[..n],
                        )
                        .await?;
                    }
                    if !read_failed {
                        write_transfer_frame(
                            self.device_link.stream_mut(),
                            FILE_TRANSFER_CODE_SUCCESS,
                            &[],
                        )
                        .await?;
                    }
                }
                Err(Mobilebackup2Error::Io(err)) => {
                    insert_file_failure(&mut failures, rel, &err);
                    write_transfer_frame(
                        self.device_link.stream_mut(),
                        FILE_TRANSFER_CODE_LOCAL_ERROR,
                        err.to_string().as_bytes(),
                    )
                    .await?;
                }
                Err(err) => return Err(err),
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
    validate_backup_identifier(target_identifier)?;
    reject_symlink_components(backup_root)?;
    fs::create_dir_all(backup_root)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = create_dir_all_no_symlink(&root, Path::new(target_identifier))?;
    for file_name in ["Info.plist", "Status.plist", "Manifest.plist"] {
        validate_seed_file_path(&device_directory.join(file_name))?;
    }
    let full = should_do_full_backup(full, &device_directory)?;

    let mut info_file = open_file_for_write(&device_directory.join("Info.plist"))?;
    plist::to_writer_xml(
        &mut info_file,
        &plist::Value::Dictionary(info_plist.clone()),
    )?;

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
    let mut status_file = open_file_for_write(&device_directory.join("Status.plist"))?;
    plist::to_writer_binary(&mut status_file, &plist::Value::Dictionary(status))?;

    let manifest_path = device_directory.join("Manifest.plist");
    match fs::symlink_metadata(&manifest_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(symlink_path_error(&manifest_path));
        }
        Ok(_) if full => {
            fs::remove_file(&manifest_path)?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let _ = open_file_for_write(&manifest_path)?;

    Ok(BackupDirectoryLayout {
        root,
        device_directory,
        target_identifier: target_identifier.to_string(),
    })
}

fn validate_seed_file_path(path: &Path) -> Result<(), Mobilebackup2Error> {
    reject_symlink_components(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(symlink_path_error(path)),
        Ok(metadata) if !metadata.is_file() => Err(Mobilebackup2Error::Protocol(format!(
            "backup seed path is not a regular file: {}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub fn should_do_full_backup(
    full: bool,
    device_directory: &Path,
) -> Result<bool, Mobilebackup2Error> {
    Ok(full || !has_incremental_backup_metadata(device_directory)?)
}

pub fn has_incremental_backup_metadata(
    device_directory: &Path,
) -> Result<bool, Mobilebackup2Error> {
    for filename in INCREMENTAL_BACKUP_REQUIRED_FILES {
        let path = device_directory.join(filename);
        reject_symlink_components(&path)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
            Err(err) => return Err(err.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&path));
        }
        if !metadata.is_file() || metadata.len() == 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn backup_status_is_full(device_directory: &Path) -> Result<bool, Mobilebackup2Error> {
    let status_path = safe_file_in_directory(device_directory, "Status.plist")?;
    let status = read_backup_dictionary(&status_path)?;
    Ok(status
        .get("IsFullBackup")
        .and_then(plist_value_to_bool)
        .unwrap_or(false))
}

/// Run a synchronous filesystem step off the runtime's worker threads.
///
/// The helpers below stat, create and parse files on disk; calling them
/// directly from an async fn blocks the reactor, which the `backup()` path
/// already avoids via `spawn_blocking`.
async fn run_blocking<T, F>(op: F) -> Result<T, Mobilebackup2Error>
where
    F: FnOnce() -> Result<T, Mobilebackup2Error> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| Mobilebackup2Error::Io(std::io::Error::other(err.to_string())))?
}

fn create_runtime_layout(
    backup_root: &Path,
    target_identifier: &str,
) -> Result<BackupDirectoryLayout, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    reject_symlink_components(backup_root)?;
    fs::create_dir_all(backup_root)?;
    let root = canonical_backup_root(backup_root)?;
    let device_directory = create_dir_all_no_symlink(&root, Path::new(target_identifier))?;
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
    let root = canonical_backup_root(backup_root)?;
    validate_backup_identifier(target_identifier)?;
    let device_directory = safe_path_from_root(&root, Path::new(target_identifier))?;
    for file_name in ["Info.plist", "Manifest.plist", "Status.plist"] {
        let path = device_directory.join(file_name);
        let relative = Path::new(target_identifier).join(file_name);
        ensure_no_symlink_components_at_root(&root, &relative)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup directory missing required file {}",
                    path.display()
                )));
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&path));
        }
        if !metadata.is_file() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup directory required path is not a file {}",
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
    let info_path = metadata_file_path(backup_root, target_identifier, "Info.plist")?;
    let info = plist::Value::from_reader(open_file_for_read(&info_path)?)?;
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
    let manifest_path = metadata_file_path(backup_root, target_identifier, "Manifest.plist")?;
    Ok(read_backup_dictionary(&manifest_path)?
        .get("IsEncrypted")
        .and_then(plist_value_to_bool)
        .unwrap_or(false))
}

fn read_backup_dictionary(path: &Path) -> Result<plist::Dictionary, Mobilebackup2Error> {
    plist::Value::from_reader(open_file_for_read(path)?)?
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

/// Validate an identifier before it is ever used as a child of the backup root.
///
/// Device and source identifiers identify one backup directory, not a path.
/// Treating them as a single normal component keeps the contract explicit on
/// Unix and Windows alike; the explicit separator checks are needed because a
/// Windows-style path is a normal filename component on Unix.
pub fn validate_backup_identifier(identifier: &str) -> Result<(), Mobilebackup2Error> {
    if identifier.is_empty() {
        return Err(Mobilebackup2Error::Protocol(
            "backup identifier must not be empty".into(),
        ));
    }
    if identifier.as_bytes().contains(&0) {
        return Err(Mobilebackup2Error::Protocol(
            "backup identifier must not contain NUL".into(),
        ));
    }
    if identifier.contains('/') || identifier.contains('\\') {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup identifier must be one path component: {identifier:?}"
        )));
    }
    if has_windows_drive_prefix(identifier) {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup identifier must not contain a platform path prefix: {identifier:?}"
        )));
    }

    let mut components = Path::new(identifier).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(Mobilebackup2Error::Protocol(format!(
            "backup identifier must be one normal path component: {identifier:?}"
        ))),
    }
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn sanitize_relative_path(path: &str) -> Result<PathBuf, Mobilebackup2Error> {
    if path.is_empty() {
        return Err(Mobilebackup2Error::Protocol(
            "backup relative path must not be empty".into(),
        ));
    }
    if path.as_bytes().contains(&0) {
        return Err(Mobilebackup2Error::Protocol(
            "backup relative path must not contain NUL".into(),
        ));
    }
    if path.contains('\\') || has_windows_drive_prefix(path) {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup relative path uses an unsupported platform separator or prefix: {path:?}"
        )));
    }

    let mut clean = PathBuf::new();
    let mut saw_component = false;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                clean.push(part);
                saw_component = true;
            }
            Component::CurDir => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup relative path contains a current-directory component: {path:?}"
                )));
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path escapes backup root: {path}"
                )));
            }
        }
    }

    if !saw_component {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup relative path must contain a normal component: {path:?}"
        )));
    }

    Ok(clean)
}

fn resolve_relative_path(
    layout: &BackupDirectoryLayout,
    rel: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let clean = sanitize_relative_path(rel)?;
    validate_backup_identifier(&layout.target_identifier)?;
    let prefixed_with_target = clean
        .components()
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        == Some(layout.target_identifier.as_str());

    let relative = if prefixed_with_target {
        clean
    } else {
        let mut relative = PathBuf::from(&layout.target_identifier);
        relative.push(clean);
        relative
    };
    let root = canonical_backup_root(&layout.root)?;
    ensure_no_symlink_components_at_root(&root, &relative)?;
    Ok(root.join(&relative))
}

fn canonical_backup_root(root: &Path) -> Result<PathBuf, Mobilebackup2Error> {
    reject_symlink_components(root)?;
    fs::canonicalize(root).map_err(Mobilebackup2Error::Io)
}

fn symlink_path_error(path: &Path) -> Mobilebackup2Error {
    Mobilebackup2Error::Protocol(format!(
        "backup path contains a symlink component: {}",
        path.display()
    ))
}

fn ensure_no_symlink_components_at_root(
    root: &Path,
    relative: &Path,
) -> Result<(), Mobilebackup2Error> {
    let mut current = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let part = match component {
            Component::Normal(part) => part,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path is not a safe relative path: {}",
                    relative.display()
                )));
            }
        };
        current.push(part);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() {
            return Err(symlink_path_error(&current));
        }
        if index + 1 < component_count && !metadata.is_dir() {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup path component is not a directory: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn safe_path_from_root(root: &Path, relative: &Path) -> Result<PathBuf, Mobilebackup2Error> {
    ensure_no_symlink_components_at_root(root, relative)?;
    Ok(root.join(relative))
}

fn create_dir_all_no_symlink(root: &Path, relative: &Path) -> Result<PathBuf, Mobilebackup2Error> {
    let root = canonical_backup_root(root)?;
    let mut current = root.clone();
    for component in relative.components() {
        let part = match component {
            Component::Normal(part) => part,
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup directory path is not safe: {}",
                    relative.display()
                )));
            }
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(symlink_path_error(&current));
                }
                if !metadata.is_dir() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "backup directory path is not a directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = fs::symlink_metadata(&current)?;
                if metadata.file_type().is_symlink() {
                    return Err(symlink_path_error(&current));
                }
                if !metadata.is_dir() {
                    return Err(Mobilebackup2Error::Protocol(format!(
                        "backup directory path is not a directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) => return Err(error.into()),
        }

        let canonical = fs::canonicalize(&current)?;
        if !canonical.starts_with(&root) {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup directory path escapes backup root: {}",
                current.display()
            )));
        }
    }
    Ok(current)
}

fn reject_symlink_components(path: &Path) -> Result<(), Mobilebackup2Error> {
    let mut current = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()?
    };
    for component in path.components() {
        match component {
            Component::Normal(part) => current.push(part),
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(Mobilebackup2Error::Protocol(format!(
                    "backup path is not safe: {}",
                    path.display()
                )));
            }
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(symlink_path_error(&current));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Open a backup file without following a final symlink. Unix uses the
/// kernel's `O_NOFOLLOW`; other platforms retain the component metadata check
/// above as the portable fallback.
fn open_file_for_read(path: &Path) -> Result<File, Mobilebackup2Error> {
    reject_symlink_components(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(Mobilebackup2Error::Io)
}

fn open_file_for_write(path: &Path) -> Result<File, Mobilebackup2Error> {
    reject_symlink_components(path)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(Mobilebackup2Error::Io)
}

fn metadata_file_path(
    backup_root: &Path,
    target_identifier: &str,
    file_name: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    validate_backup_identifier(target_identifier)?;
    let root = canonical_backup_root(backup_root)?;
    let relative = Path::new(target_identifier).join(file_name);
    let path = safe_path_from_root(&root, &relative)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_file() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup metadata path is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
}

fn layout_relative_path(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<PathBuf, Mobilebackup2Error> {
    path.strip_prefix(&layout.root)
        .map(PathBuf::from)
        .map_err(|_| {
            Mobilebackup2Error::Protocol(format!(
                "backup path is outside backup root: {}",
                path.display()
            ))
        })
}

fn create_layout_directory(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    create_dir_all_no_symlink(&layout.root, &relative)?;
    Ok(())
}

fn create_layout_parent_directory(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let parent = path.parent().ok_or_else(|| {
        Mobilebackup2Error::Protocol(format!(
            "backup path has no parent directory: {}",
            path.display()
        ))
    })?;
    let relative = layout_relative_path(layout, parent)?;
    create_dir_all_no_symlink(&layout.root, &relative)?;
    Ok(())
}

fn open_layout_file_for_write(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<File, Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    open_file_for_write(path)
}

fn open_layout_file_for_read(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<File, Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    open_file_for_read(path)
}

async fn rename_layout_path(
    layout: &BackupDirectoryLayout,
    src: &Path,
    dst: &Path,
) -> Result<(), Mobilebackup2Error> {
    let src_relative = layout_relative_path(layout, src)?;
    let dst_relative = layout_relative_path(layout, dst)?;
    ensure_no_symlink_components_at_root(&layout.root, &src_relative)?;
    ensure_no_symlink_components_at_root(&layout.root, &dst_relative)?;
    tokio::fs::rename(src, dst).await?;
    Ok(())
}

fn ensure_tree_has_no_symlinks(path: &Path) -> Result<(), Mobilebackup2Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(path));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            ensure_tree_has_no_symlinks(&entry?.path())?;
        }
    }
    Ok(())
}

async fn remove_layout_path(
    layout: &BackupDirectoryLayout,
    path: &Path,
) -> Result<(), Mobilebackup2Error> {
    let relative = layout_relative_path(layout, path)?;
    ensure_no_symlink_components_at_root(&layout.root, &relative)?;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(path));
    }
    if metadata.is_dir() {
        ensure_tree_has_no_symlinks(path)?;
        tokio::fs::remove_dir_all(path).await?;
    } else {
        tokio::fs::remove_file(path).await?;
    }
    Ok(())
}

fn copy_item(root: &Path, src: &Path, dst: &Path) -> Result<(), Mobilebackup2Error> {
    let src_relative = src.strip_prefix(root).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "backup copy source is outside backup root: {}",
            src.display()
        ))
    })?;
    let dst_relative = dst.strip_prefix(root).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "backup copy destination is outside backup root: {}",
            dst.display()
        ))
    })?;
    ensure_no_symlink_components_at_root(root, src_relative)?;
    ensure_no_symlink_components_at_root(root, dst_relative)?;
    if src == dst || dst.starts_with(src) {
        return Err(Mobilebackup2Error::Protocol(
            "cannot copy a directory into itself".into(),
        ));
    }

    let source_metadata = fs::symlink_metadata(src)?;
    if source_metadata.file_type().is_symlink() {
        return Err(symlink_path_error(src));
    }
    if source_metadata.is_dir() {
        create_dir_all_no_symlink(root, dst_relative)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_item(root, &entry.path(), &dst.join(entry.file_name()))?;
        }
    } else if source_metadata.is_file() {
        if let Some(parent) = dst.parent() {
            let parent_relative = parent.strip_prefix(root).map_err(|_| {
                Mobilebackup2Error::Protocol(format!(
                    "backup copy destination parent is outside backup root: {}",
                    parent.display()
                ))
            })?;
            create_dir_all_no_symlink(root, parent_relative)?;
        }
        let mut source = open_file_for_read(src)?;
        let mut destination = open_file_for_write(dst)?;
        std::io::copy(&mut source, &mut destination)?;
    } else {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup copy source is neither a file nor a directory: {}",
            src.display()
        )));
    }

    Ok(())
}

fn contents_of_directory(
    root: &Path,
    path: &Path,
) -> Result<plist::Dictionary, Mobilebackup2Error> {
    let relative = path.strip_prefix(root).map_err(|_| {
        Mobilebackup2Error::Protocol(format!(
            "backup directory is outside backup root: {}",
            path.display()
        ))
    })?;
    ensure_no_symlink_components_at_root(root, relative)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(path));
    }
    if !metadata.is_dir() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup directory path is not a directory: {}",
            path.display()
        )));
    }

    let mut entries = plist::Dictionary::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path)?;
        let file_type = if metadata.file_type().is_symlink() {
            "DLFileTypeUnknown"
        } else if metadata.is_dir() {
            "DLFileTypeDirectory"
        } else if metadata.is_file() {
            "DLFileTypeRegular"
        } else {
            "DLFileTypeUnknown"
        };
        let modified = metadata.modified().unwrap_or_else(|err| {
            tracing::debug!("cannot read mtime for {}: {err}", entry_path.display());
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

/*
 * Keep this helper adjacent to the path policy so every file operation above
 * goes through the same root and symlink checks. The protocol permits nested
 * relative paths, but never permits them to become an alternate root.
 */
fn safe_file_in_directory(
    directory: &Path,
    file_name: &str,
) -> Result<PathBuf, Mobilebackup2Error> {
    let path = directory.join(file_name);
    reject_symlink_components(&path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err(symlink_path_error(&path));
    }
    if !metadata.is_file() {
        return Err(Mobilebackup2Error::Protocol(format!(
            "backup metadata path is not a file: {}",
            path.display()
        )));
    }
    Ok(path)
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

/// Maximum size for a length-prefixed string (64 KiB). Device names and file
/// paths are never anywhere near this limit; the guard protects against
/// corrupted or malicious size fields causing unbounded allocation.
const MAX_PREFIXED_STRING_SIZE: usize = 64 * 1024;

async fn read_prefixed_string<S>(stream: &mut S) -> Result<String, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let size = read_u32_be(stream).await? as usize;
    if size == 0 {
        return Ok(String::new());
    }
    if size > MAX_PREFIXED_STRING_SIZE {
        return Err(Mobilebackup2Error::Protocol(format!(
            "prefixed string too large: {size} bytes (max {MAX_PREFIXED_STRING_SIZE})"
        )));
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

async fn copy_transfer_payload<S, W>(
    stream: &mut S,
    writer: &mut W,
    mut remaining: usize,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0u8; DOWNLOAD_CHUNK_SIZE];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len());
        stream.read_exact(&mut buffer[..chunk_len]).await?;
        writer.write_all(&buffer[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

async fn discard_transfer_payload<S>(
    stream: &mut S,
    mut remaining: usize,
) -> Result<(), Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let mut buffer = [0u8; DOWNLOAD_CHUNK_SIZE];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len());
        stream.read_exact(&mut buffer[..chunk_len]).await?;
        remaining -= chunk_len;
    }
    Ok(())
}

async fn read_transfer_error_preview<S>(
    stream: &mut S,
    payload_len: usize,
) -> Result<String, Mobilebackup2Error>
where
    S: AsyncRead + Unpin,
{
    let preview_len = payload_len.min(MAX_TRANSFER_ERROR_PREVIEW_SIZE);
    let mut preview = vec![0u8; preview_len];
    stream.read_exact(&mut preview).await?;
    discard_transfer_payload(stream, payload_len - preview_len).await?;
    Ok(String::from_utf8_lossy(&preview).into_owned())
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

#[cfg(unix)]
fn available_space(path: &Path) -> Result<u64, Mobilebackup2Error> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let probe = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };
    let c_path = CString::new(probe.as_os_str().as_bytes()).map_err(|e| {
        Mobilebackup2Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
    })?;

    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return Err(Mobilebackup2Error::Io(std::io::Error::last_os_error()));
        }
        // Available space = available blocks * fragment size
        Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
}

fn plist_number_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value.as_unsigned(),
        plist::Value::Real(value) if value.is_finite() && *value >= 0.0 => {
            // Rust's float-to-integer cast saturates, so an unchecked cast
            // would silently turn negative, fractional, NaN, or oversized
            // protocol values into a different error/space value.
            let upper_bound = (u64::MAX as f64) + 1.0;
            if *value < upper_bound && value.fract() == 0.0 {
                Some(*value as u64)
            } else {
                None
            }
        }
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

fn derive_required_free_space(reported: Option<u64>, requested: Option<u64>) -> Option<u64> {
    let reported = reported?;
    let requested = requested?;
    if requested < PURGE_REQUEST_OVERSHOOT {
        return None;
    }

    requested
        .checked_add(reported)
        .and_then(|value| value.checked_sub(PURGE_REQUEST_OVERSHOOT))
}

fn insufficient_disk_space_message(
    response: &plist::Dictionary,
    reported: Option<u64>,
    required: Option<u64>,
) -> String {
    let detail = "the device counts hardlink and clone references at full size because each uploaded path is stored separately";
    match (reported, required) {
        (Some(reported), Some(required)) => {
            let additional = required.saturating_add(1).saturating_sub(reported);
            format!(
                "backup process returned ErrorCode={MB_ERROR_INSUFFICIENT_DISK_SPACE}: device needs more than {required} bytes free, host reported {reported} bytes ({additional} more needed) (MBErrorDomain/{MB_ERROR_INSUFFICIENT_DISK_SPACE}); {detail}; response: {response:?}"
            )
        }
        _ => format!(
            "backup process returned ErrorCode={MB_ERROR_INSUFFICIENT_DISK_SPACE}: {response:?}; {detail}"
        ),
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

fn insert_file_failure(failures: &mut plist::Dictionary, rel: &str, err: &std::io::Error) {
    let mut failure = plist::Dictionary::from_iter([(
        "DLFileErrorString".to_string(),
        plist::Value::String(err.to_string()),
    )]);
    if let Some(code) = file_error_code_from_os_error(err) {
        failure.insert(
            "DLFileErrorCode".to_string(),
            plist::Value::Integer(code.into()),
        );
    }
    failures.insert(rel.to_string(), plist::Value::Dictionary(failure));
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

    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn encode_test_frame(value: &plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, value).expect("plist serialization");
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        frame
    }

    async fn read_test_frame(stream: &mut tokio::io::DuplexStream) -> plist::Value {
        let mut length = [0u8; 4];
        stream.read_exact(&mut length).await.expect("frame length");
        let length = u32::from_be_bytes(length) as usize;
        let mut payload = vec![0u8; length];
        stream
            .read_exact(&mut payload)
            .await
            .expect("frame payload");
        plist::from_bytes(&payload).expect("plist frame")
    }

    #[tokio::test]
    async fn purge_disk_space_is_answered_and_the_loop_continues() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.run_loop(&layout).await
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessagePurgeDiskSpace".into()),
                plist::Value::Integer(42u64.into()),
                plist::Value::Integer(4u64.into()),
            ])))
            .await
            .expect("write purge request");

        assert_eq!(
            read_test_frame(&mut server_stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(PURGE_DISK_SPACE_ERROR.into()),
                plist::Value::String(PURGE_DISK_SPACE_ERROR_STRING.into()),
                plist::Value::Integer(0u64.into()),
            ])
        );

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("ErrorCode".to_string(), plist::Value::Integer(0u64.into())),
                    (
                        "Content".to_string(),
                        plist::Value::String("backup finished".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write completion");

        assert_eq!(
            task.await.expect("run loop task").expect("backup loop"),
            Some(plist::Value::String("backup finished".into()))
        );
    }

    #[tokio::test]
    async fn insufficient_space_process_error_uses_the_last_purge_diagnostics() {
        let layout = BackupDirectoryLayout {
            root: PathBuf::from("backup-root"),
            device_directory: PathBuf::from("backup-root/device-id"),
            target_identifier: "device-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(4096);
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.reported_free_space = Some(1_000_000);
            client.run_loop(&layout).await
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessagePurgeDiskSpace".into()),
                plist::Value::Integer(2_335_105_975u64.into()),
                plist::Value::Integer(4u64.into()),
            ])))
            .await
            .expect("write purge request");
        let _ = read_test_frame(&mut server_stream).await;

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        "ErrorCode".to_string(),
                        plist::Value::Integer(105u64.into()),
                    ),
                    (
                        "ErrorDescription".to_string(),
                        plist::Value::String("Insufficient free disk space".into()),
                    ),
                ])),
            ])))
            .await
            .expect("write insufficient-space response");

        let error = task
            .await
            .expect("run loop task")
            .expect_err("insufficient space should fail the operation");
        let message = error.to_string();
        assert!(message.contains("needs more than 188622327 bytes free"));
        assert!(message.contains("host reported 1000000 bytes"));
        assert!(message.contains("MBErrorDomain/105"));
    }

    #[test]
    fn derives_required_free_space_from_purge_request() {
        let required = derive_required_free_space(Some(1_000_000), Some(2_335_105_975));
        assert_eq!(required, Some(188_622_327));
        assert_eq!(
            derive_required_free_space(Some(1_000_000), Some(PURGE_REQUEST_OVERSHOOT - 1)),
            None
        );
        assert_eq!(
            derive_required_free_space(None, Some(PURGE_REQUEST_OVERSHOOT)),
            None
        );
    }

    #[test]
    fn plist_u64_parser_rejects_non_integral_and_out_of_range_reals() {
        assert_eq!(plist_number_to_u64(&plist::Value::Real(42.0)), Some(42));
        for value in [-1.0, 42.5, f64::NAN, f64::INFINITY, u64::MAX as f64] {
            assert_eq!(
                plist_number_to_u64(&plist::Value::Real(value)),
                None,
                "real protocol value must be a finite non-negative integer: {value:?}"
            );
        }
    }

    #[test]
    fn insufficient_space_message_includes_requirement_and_host_report() {
        let response = plist::Dictionary::from_iter([(
            "ErrorDescription".to_string(),
            plist::Value::String("Insufficient free disk space".into()),
        )]);
        let message =
            insufficient_disk_space_message(&response, Some(1_000_000), Some(188_622_327));
        assert!(message.contains("needs more than 188622327 bytes free"));
        assert!(message.contains("host reported 1000000 bytes"));
        assert!(message.contains("187622328 more needed"));
        assert!(message.contains("MBErrorDomain/105"));
    }

    #[test]
    fn initialize_backup_directory_creates_expected_seed_files() {
        let root =
            std::env::temp_dir().join(format!("ios-core-backup2-layout-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();

        let info = plist::Dictionary::from_iter([(
            "Device Name".to_string(),
            plist::Value::String("Example".into()),
        )]);
        let layout = initialize_backup_directory(&root, "device-id", &info, true).unwrap();

        assert_eq!(layout.device_directory, root.join("device-id"));
        assert!(layout.device_directory.join("Info.plist").exists());
        assert!(layout.device_directory.join("Status.plist").exists());
        assert!(layout.device_directory.join("Manifest.plist").exists());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn initial_backup_becomes_full_when_incremental_metadata_is_missing() {
        let root = std::env::temp_dir().join(format!(
            "ios-core-backup2-initial-full-{}",
            std::process::id()
        ));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();

        let layout =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), false)
                .unwrap();

        let status = plist::Value::from_file(layout.device_directory.join("Status.plist"))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(status["IsFullBackup"].as_boolean(), Some(true));
        assert!(backup_status_is_full(&layout.device_directory).unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incremental_backup_is_preserved_when_required_metadata_exists() {
        let root = std::env::temp_dir().join(format!(
            "ios-core-backup2-incremental-{}",
            std::process::id()
        ));
        let device_dir = root.join("device-id");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&device_dir).unwrap();
        for filename in INCREMENTAL_BACKUP_REQUIRED_FILES {
            std::fs::write(device_dir.join(filename), b"data").unwrap();
        }

        let layout =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), false)
                .unwrap();
        assert!(!backup_status_is_full(&layout.device_directory).unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_incremental_metadata_forces_full_backup() {
        let root = std::env::temp_dir().join(format!(
            "ios-core-backup2-empty-metadata-{}",
            std::process::id()
        ));
        let device_dir = root.join("device-id");
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(&device_dir).unwrap();
        std::fs::write(device_dir.join("Manifest.plist"), b"").unwrap();
        std::fs::write(device_dir.join("Manifest.db"), b"data").unwrap();
        std::fs::write(device_dir.join("Status.plist"), b"data").unwrap();

        assert!(should_do_full_backup(false, &device_dir).unwrap());

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn resolve_relative_path_accepts_plain_and_prefixed_paths() {
        let root =
            std::env::temp_dir().join(format!("ios-core-backup2-resolve-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        std::fs::create_dir_all(root.join("device-id")).unwrap();
        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: root.join("device-id"),
            target_identifier: "device-id".into(),
        };

        assert_eq!(
            resolve_relative_path(&layout, "Manifest.db").unwrap(),
            root.join("device-id/Manifest.db")
        );
        assert_eq!(
            resolve_relative_path(&layout, "device-id/Manifest.db").unwrap(),
            root.join("device-id/Manifest.db")
        );

        std::fs::remove_dir_all(root).unwrap();
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
    fn backup_identifiers_are_single_normal_components() {
        for identifier in [
            "",
            ".",
            "..",
            "../outside",
            "/absolute",
            "nested/id",
            r"nested\id",
            "C:backup",
            r"C:\backup",
        ] {
            assert!(
                validate_backup_identifier(identifier).is_err(),
                "identifier should be rejected: {identifier:?}"
            );
        }
        for identifier in ["device-id", "设备-✅"] {
            assert!(
                validate_backup_identifier(identifier).is_ok(),
                "identifier should be accepted: {identifier:?}"
            );
        }
    }

    #[test]
    fn relative_paths_reject_absolute_parent_and_platform_forms() {
        for path in [
            "",
            ".",
            "..",
            "../outside",
            "/absolute",
            r"..\outside",
            r"nested\file",
            "C:relative",
            r"C:\absolute",
        ] {
            assert!(
                sanitize_relative_path(path).is_err(),
                "relative path should be rejected: {path:?}"
            );
        }

        assert_eq!(
            sanitize_relative_path("nested/目录/файл").unwrap(),
            PathBuf::from("nested/目录/файл")
        );
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_components_are_rejected_without_writing_outside_root() {
        use std::os::unix::fs::symlink;

        let base =
            std::env::temp_dir().join(format!("ios-core-backup2-symlink-{}", std::process::id()));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        let root = base.join("root");
        let device = root.join("device-id");
        let outside = base.join("outside");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"outside").unwrap();

        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: device.clone(),
            target_identifier: "device-id".into(),
        };

        symlink(&outside, device.join("middle")).unwrap();
        let err = resolve_relative_path(&layout, "middle/created").unwrap_err();
        assert!(err.to_string().contains("symlink"));

        symlink(outside.join("sentinel"), device.join("final")).unwrap();
        let err = resolve_relative_path(&layout, "final").unwrap_err();
        assert!(err.to_string().contains("symlink"));

        let err = open_layout_file_for_write(&layout, &device.join("final")).unwrap_err();
        assert!(err.to_string().contains("symlink"));
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"outside");

        std::fs::write(device.join("source"), b"inside").unwrap();
        let copy_target = device.join("copy");
        symlink(outside.join("sentinel"), copy_target.clone()).unwrap();
        let err = copy_item(&root, &device.join("source"), &copy_target).unwrap_err();
        assert!(err.to_string().contains("symlink"));
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"outside");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn seed_symlink_is_rejected_before_existing_metadata_is_overwritten() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "ios-core-backup2-seed-symlink-{}",
            std::process::id()
        ));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        let root = base.join("root");
        let device = root.join("device-id");
        let outside = base.join("outside");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(device.join("Info.plist"), b"old-info").unwrap();
        std::fs::write(device.join("Status.plist"), b"old-status").unwrap();
        std::fs::write(outside.join("sentinel"), b"outside").unwrap();
        symlink(outside.join("sentinel"), device.join("Manifest.plist")).unwrap();

        let result =
            initialize_backup_directory(&root, "device-id", &plist::Dictionary::new(), true);
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(device.join("Info.plist")).unwrap(),
            b"old-info"
        );
        assert_eq!(
            std::fs::read(device.join("Status.plist")).unwrap(),
            b"old-status"
        );
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"outside");

        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn backup_root_symlink_is_rejected_before_creating_outside_directory() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!(
            "ios-core-backup2-root-symlink-{}",
            std::process::id()
        ));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        std::fs::create_dir_all(&base).unwrap();
        let outside = base.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let linked_root = base.join("linked-root");
        symlink(&outside, &linked_root).unwrap();

        let result = initialize_backup_directory(
            &linked_root.join("new-device"),
            "device-id",
            &plist::Dictionary::new(),
            true,
        );
        assert!(result.is_err());
        assert!(!outside.join("new-device").exists());

        std::fs::remove_dir_all(base).unwrap();
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
            "ios-core-backup2-encryption-{}",
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

    #[tokio::test]
    async fn read_prefixed_string_rejects_oversized_allocation() {
        // Craft a size field that exceeds MAX_PREFIXED_STRING_SIZE
        let size = (MAX_PREFIXED_STRING_SIZE as u32) + 1;
        let data = size.to_be_bytes();
        let mut cursor = std::io::Cursor::new(data.to_vec());
        let err = read_prefixed_string(&mut cursor).await.unwrap_err();
        assert!(
            err.to_string().contains("too large"),
            "expected size guard error, got: {err}"
        );
    }

    #[tokio::test]
    async fn read_prefixed_string_accepts_normal_size() {
        let payload = b"hello";
        let size = (payload.len() as u32).to_be_bytes();
        let mut data = size.to_vec();
        data.extend_from_slice(payload);
        let mut cursor = std::io::Cursor::new(data);
        let result = read_prefixed_string(&mut cursor).await.unwrap();
        assert_eq!(result, "hello");
    }
}

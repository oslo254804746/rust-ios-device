//! MobileBackup2 session orchestration, transfer accounting and cleanup.
//!
//! Wire shapes and framing come from protocol; host operations use explicitly
//! named layout/path helpers. Public entry points remain on the facade client.

#[cfg(test)]
use super::host_fs::canonicalize_simplified;
use super::host_fs::{
    available_space, cleanup_discarded_files, commit_uploaded_file, copy_layout_item,
    create_layout_directory, create_layout_parent_directory, create_layout_staged_file,
    create_runtime_layout, ensure_backup_directory, initialize_backup_directory,
    list_layout_directory, manifest_relative_path_string, metadata_file_path, move_layout_item,
    open_layout_file_for_read, read_backup_dictionary, remove_layout_item, resolve_relative_path,
    run_blocking, sanitize_relative_path, validate_backup_identifier, with_layout,
};
use super::{
    BackupDirectoryLayout, BackupOptions, BackupResult, Mobilebackup2Client, Mobilebackup2Error,
    RestoreOptions, RestoreResult, VersionExchange, SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::services::device_link::DeviceLinkError;

use super::filter::should_preserve_backup_file;
#[cfg(feature = "backup2-manifest")]
use super::manifest::patch_backup_directory;
use super::protocol::{
    copy_transfer_payload, derive_required_free_space, discard_transfer_payload,
    insert_file_failure, insufficient_disk_space_message, non_empty_protocol_string,
    plist_number_to_f64, plist_number_to_u64, plist_value_to_bool, read_prefixed_string,
    read_u32_be, redacted_protocol_dictionary, redacted_protocol_value, write_prefixed_string,
    write_transfer_frame, BackupRequest, ChangePasswordRequest, ExtractRequest, HelloRequest,
    InfoRequest, ListRequest, RestoreRequest, RestoreRequestOptions, UnbackRequest,
    BULK_OPERATION_ERROR, DOWNLOAD_CHUNK_SIZE, EMPTY_PARAMETER_STRING,
    FILE_TRANSFER_CODE_FILE_DATA, FILE_TRANSFER_CODE_LOCAL_ERROR, FILE_TRANSFER_CODE_REMOTE_ERROR,
    FILE_TRANSFER_CODE_SUCCESS, MB_ERROR_INSUFFICIENT_DISK_SPACE, PURGE_DISK_SPACE_ERROR,
    PURGE_DISK_SPACE_ERROR_STRING,
};
#[cfg(test)]
use std::path::PathBuf;
use std::{io::ErrorKind, path::Path, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::warn;
#[cfg(feature = "backup2-manifest")]
use zeroize::Zeroizing;

const MAX_DEVICE_TRANSFER_FILES: u64 = 1_000_000;
const MAX_DEVICE_TRANSFER_BYTES: u64 = 512 * 1024 * 1024 * 1024;
// Device-side Unback/Extract are long-running DeviceLink operations. Keep one
// bounded lifetime for the complete exchange, including version negotiation,
// so a peer that stops sending cannot leave a caller waiting forever.
const DEVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(5 * 60);
impl<S> Mobilebackup2Client<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub async fn version_exchange(&mut self) -> Result<VersionExchange, Mobilebackup2Error> {
        // A client can be reused for several operations in tests or by higher-level callers;
        // diagnostics from a previous DeviceLink session must not leak into the next one.
        self.reported_free_space = None;
        self.required_free_space = None;
        self.transfer_files = 0;
        self.transfer_bytes = 0;

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
                    "backup2 hello response missing ErrorCode: {:?}",
                    redacted_protocol_dictionary(&response)
                ))
            })?;
        if error_code != 0 {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup2 hello returned ErrorCode={error_code}: {:?}",
                redacted_protocol_dictionary(&response)
            )));
        }

        let protocol_version = response
            .get("ProtocolVersion")
            .and_then(plist_number_to_f64)
            .ok_or_else(|| {
                Mobilebackup2Error::Protocol(format!(
                    "backup2 hello response missing ProtocolVersion: {:?}",
                    redacted_protocol_dictionary(&response)
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
        self.backup_with_options(
            backup_root,
            target_identifier,
            full,
            info_plist,
            BackupOptions::default(),
        )
        .await
    }

    /// Run the real MobileBackup2 `Backup` request with optional host-side selection policy.
    pub async fn backup_with_options(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        full: bool,
        info_plist: &plist::Dictionary,
        options: BackupOptions,
    ) -> Result<BackupResult, Mobilebackup2Error> {
        #[cfg(feature = "backup2-manifest")]
        let mut options = options;
        #[cfg(feature = "backup2-manifest")]
        let password = options.password.take().map(Zeroizing::new);
        validate_backup_identifier(target_identifier)?;
        if options.patch_manifest && options.filter.is_none() {
            return Err(Mobilebackup2Error::Protocol(
                "patching a backup manifest requires a file selection filter".into(),
            ));
        }
        #[cfg(not(feature = "backup2-manifest"))]
        if options.patch_manifest {
            return Err(Mobilebackup2Error::Protocol(
                "manifest patching requires the ios-core backup2-manifest feature".into(),
            ));
        }
        let version = self.version_exchange().await?;
        self.backup_filter = options.filter.clone();
        self.discarded_files.clear();
        let layout = {
            let root = backup_root.to_path_buf();
            let id = target_identifier.to_owned();
            let info = info_plist.clone();
            run_blocking(move || {
                initialize_backup_directory(&root, &id, &info, full || options.patch_manifest)
            })
            .await?
        };

        self.device_link
            .send_process_message(&BackupRequest {
                message_name: "Backup",
                target_identifier,
            })
            .await?;

        let run_result = self.run_loop(&layout).await;
        let session_result = self.finish_session(run_result).await;
        self.backup_filter = None;
        let discarded_files = std::mem::take(&mut self.discarded_files);
        let cleanup_result = with_layout(&layout, move |layout| {
            cleanup_discarded_files(&layout, &discarded_files)
        })
        .await;
        let _content = session_result?;
        cleanup_result?;

        #[cfg(feature = "backup2-manifest")]
        if options.patch_manifest {
            let root = layout.root.clone();
            let id = layout.target_identifier.clone();
            let filter = options.filter.clone().ok_or_else(|| {
                Mobilebackup2Error::Protocol("manifest patch filter unexpectedly missing".into())
            })?;
            run_blocking(move || {
                patch_backup_directory(&root, &id, &filter, password.as_deref().map(String::as_str))
            })
            .await?;
        }

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

    /// Ask the device to expand a completed backup with MobileBackup2's real
    /// `Unback` operation. This is distinct from the host-side Manifest.db
    /// helper exposed behind the `backup2-manifest` feature.
    pub async fn unback(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        let deadline = tokio::time::Instant::now() + DEVICE_OPERATION_TIMEOUT;
        let result = match tokio::time::timeout_at(
            deadline,
            self.run_device_unback(backup_root, target_identifier, source_identifier, password),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Unback exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        };
        match tokio::time::timeout_at(deadline, self.finish_session(result)).await {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Unback exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn run_device_unback(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        let source_identifier = non_empty_protocol_string(source_identifier);
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
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

        self.version_exchange().await?;
        self.device_link
            .send_process_message(&UnbackRequest {
                message_name: "Unback",
                target_identifier,
                source_identifier,
                password: non_empty_protocol_string(password),
            })
            .await?;
        self.run_loop(&layout).await
    }

    /// Ask the device to extract one domain/path from a completed backup with
    /// MobileBackup2's real `Extract` operation. `backup_root` is the local
    /// directory used by the DeviceLink file-transfer loop, not an output path.
    pub async fn extract(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        domain_name: &str,
        relative_path: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        let deadline = tokio::time::Instant::now() + DEVICE_OPERATION_TIMEOUT;
        let result = match tokio::time::timeout_at(
            deadline,
            self.run_device_extract(
                backup_root,
                target_identifier,
                domain_name,
                relative_path,
                source_identifier,
                password,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Extract exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        };
        match tokio::time::timeout_at(deadline, self.finish_session(result)).await {
            Ok(result) => result,
            Err(_) => Err(Mobilebackup2Error::Protocol(format!(
                "device-side Extract exceeded the total timeout of {} seconds",
                DEVICE_OPERATION_TIMEOUT.as_secs()
            ))),
        }
    }

    async fn run_device_extract(
        &mut self,
        backup_root: &Path,
        target_identifier: &str,
        domain_name: &str,
        relative_path: &str,
        source_identifier: Option<&str>,
        password: Option<&str>,
    ) -> Result<Option<plist::Value>, Mobilebackup2Error> {
        validate_backup_identifier(target_identifier)?;
        // Domain names are single backup-domain components (HomeDomain,
        // AppDomain-* and so on). The relative path uses the same traversal
        // and platform-separator checks as DeviceLink file transfers.
        validate_backup_identifier(domain_name)?;
        let relative_path = sanitize_relative_path(relative_path)?;
        let relative_path = manifest_relative_path_string(&relative_path);
        let source_identifier = non_empty_protocol_string(source_identifier);
        if let Some(source_identifier) = source_identifier {
            validate_backup_identifier(source_identifier)?;
        }
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

        self.version_exchange().await?;
        self.device_link
            .send_process_message(&ExtractRequest {
                message_name: "Extract",
                target_identifier,
                domain_name,
                relative_path: &relative_path,
                source_identifier,
                password: non_empty_protocol_string(password),
            })
            .await?;
        self.run_loop(&layout).await
    }

    async fn disconnect_best_effort(&mut self) {
        if let Err(err) = self.device_link.disconnect().await {
            if !should_suppress_disconnect_error(&err) {
                warn!(error = %crate::redaction::diagnostic_error(&err), "backup2 disconnect failed");
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
                    "device link loop expected array message, got {:?}",
                    redacted_protocol_value(&message)
                ))
            })?;

            let command = parts
                .first()
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    Mobilebackup2Error::Protocol(format!(
                        "device link message missing command: {:?}",
                        redacted_protocol_value(&message)
                    ))
                })?;
            match command {
                "DLMessageProcessMessage" => {
                    let payload = parts
                        .get(1)
                        .and_then(plist::Value::as_dictionary)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "process message missing dictionary payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let error_code = payload
                        .get("ErrorCode")
                        .and_then(plist_number_to_u64)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "backup process response missing numeric ErrorCode: {:?}",
                                redacted_protocol_dictionary(payload)
                            ))
                        })?;
                    if error_code != 0 {
                        let safe_payload = redacted_protocol_dictionary(payload);
                        if error_code == MB_ERROR_INSUFFICIENT_DISK_SPACE {
                            return Err(Mobilebackup2Error::Protocol(
                                insufficient_disk_space_message(
                                    &safe_payload,
                                    self.reported_free_space,
                                    self.required_free_space,
                                ),
                            ));
                        }
                        return Err(Mobilebackup2Error::Protocol(format!(
                            "backup process returned ErrorCode={error_code}: {safe_payload:?}"
                        )));
                    }
                    return Ok(payload.get("Content").cloned());
                }
                "DLMessageCreateDirectory" => {
                    let path = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "create directory missing path: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let path = path.to_owned();
                    with_layout(layout, move |layout| {
                        let directory = resolve_relative_path(&layout, &path)?;
                        create_layout_directory(&layout, &directory)
                    })
                    .await?;
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
                                "download files missing array payload: {:?}",
                                redacted_protocol_value(&message)
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
                        path = %crate::redaction::diagnostic_id(&layout.device_directory.to_string_lossy()),
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
                                "move items missing mapping payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    for (src, dst_value) in items {
                        let dst = dst_value.as_string().ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "move target for {src} was not a string: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                        let source = src.to_owned();
                        let destination = dst.to_owned();
                        let allow_missing = self.backup_filter.is_some();
                        if let Some(change) = with_layout(layout, move |layout| {
                            move_layout_item(&layout, &source, &destination, allow_missing)
                        })
                        .await?
                        {
                            self.relocate_discarded_files(
                                &change.source,
                                &change.destination,
                                change.is_directory,
                            );
                        }
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
                                "remove items missing array payload: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    for item in items {
                        let rel = item.as_string().ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "remove item path was not a string: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                        let relative = rel.to_owned();
                        let target = with_layout(layout, move |layout| {
                            remove_layout_item(&layout, &relative)
                        })
                        .await?;
                        self.forget_discarded_files(&target);
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
                                "contents-of-directory missing path: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let relative = rel.to_owned();
                    let listing = with_layout(layout, move |layout| {
                        list_layout_directory(&layout, &relative)
                    })
                    .await?;
                    self.send_status_response(0, "", plist::Value::Dictionary(listing))
                        .await?;
                }
                "DLMessageCopyItem" => {
                    let src = parts
                        .get(1)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "copy item missing source: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let dst = parts
                        .get(2)
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            Mobilebackup2Error::Protocol(format!(
                                "copy item missing destination: {:?}",
                                redacted_protocol_value(&message)
                            ))
                        })?;
                    let source = src.to_owned();
                    let destination = dst.to_owned();
                    let allow_missing = self.backup_filter.is_some();
                    if let Some(change) = with_layout(layout, move |layout| {
                        copy_layout_item(&layout, &source, &destination, allow_missing)
                    })
                    .await?
                    {
                        self.copy_discarded_files(
                            &change.source,
                            &change.destination,
                            change.is_directory,
                        );
                    }
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
                        "unsupported backup device-link command {other}: {:?}",
                        redacted_protocol_value(&message)
                    )));
                }
            }
        }
    }

    fn account_transfer_file(&mut self) -> Result<(), Mobilebackup2Error> {
        self.transfer_files = self.transfer_files.checked_add(1).ok_or_else(|| {
            Mobilebackup2Error::Protocol("backup transfer file count overflow".into())
        })?;
        if self.transfer_files > MAX_DEVICE_TRANSFER_FILES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup transfer has too many files (max {MAX_DEVICE_TRANSFER_FILES})"
            )));
        }
        Ok(())
    }

    fn account_transfer_bytes(&mut self, bytes: usize) -> Result<(), Mobilebackup2Error> {
        let bytes = u64::try_from(bytes).map_err(|_| {
            Mobilebackup2Error::Protocol("backup transfer byte count does not fit in u64".into())
        })?;
        self.transfer_bytes = self.transfer_bytes.checked_add(bytes).ok_or_else(|| {
            Mobilebackup2Error::Protocol("backup transfer byte count overflow".into())
        })?;
        if self.transfer_bytes > MAX_DEVICE_TRANSFER_BYTES {
            return Err(Mobilebackup2Error::Protocol(format!(
                "backup transfer exceeds byte budget (max {MAX_DEVICE_TRANSFER_BYTES} bytes)"
            )));
        }
        Ok(())
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

            self.account_transfer_file()?;

            let file_name = read_prefixed_string(self.device_link.stream_mut()).await?;
            let preserve = self.backup_filter.as_ref().map_or(true, |filter| {
                should_preserve_backup_file(&file_name, &device_name, filter)
            });
            let name = file_name.clone();
            let (output_path, file, pending) = with_layout(layout, move |layout| {
                let path = resolve_relative_path(&layout, &name)?;
                create_layout_parent_directory(&layout, &path)?;
                let (file, pending) = create_layout_staged_file(&layout, &path)?;
                Ok((path, file, pending))
            })
            .await?;
            // Keep the file ahead of the guard in this aggregate so cancellation
            // drops the handle before removing this operation's temporary.
            let mut staged = (tokio::fs::File::from_std(file), pending);

            let succeeded = loop {
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
                        self.account_transfer_bytes(payload_len)?;
                        // A device can advertise a very large frame size. Stream its payload in
                        // bounded pieces instead of allocating the complete frame up front.
                        if preserve {
                            copy_transfer_payload(
                                self.device_link.stream_mut(),
                                &mut staged.0,
                                payload_len,
                            )
                            .await?;
                        } else {
                            discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                                .await?;
                        }
                    }
                    FILE_TRANSFER_CODE_SUCCESS => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        break true;
                    }
                    FILE_TRANSFER_CODE_REMOTE_ERROR => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        warn!(
                            device_path = %crate::redaction::diagnostic_id(&device_name),
                            output_path = %crate::redaction::diagnostic_id(&file_name),
                            "backup file transfer failed on device; staged output discarded"
                        );
                        // DeviceLink permits per-file remote errors followed by
                        // more uploads. Keep that wire flow, but never publish
                        // a partial file or truncate its previous contents.
                        break false;
                    }
                    other => {
                        discard_transfer_payload(self.device_link.stream_mut(), payload_len)
                            .await?;
                        return Err(Mobilebackup2Error::Protocol(format!(
                            "unknown backup file transfer code 0x{other:02x} for {file_name}"
                        )));
                    }
                }
            };
            if succeeded {
                staged.0.flush().await?;
                staged.0.sync_all().await?;
                let (file, pending) = staged;
                drop(file);
                let commit_path = output_path.clone();
                with_layout(layout, move |layout| {
                    commit_uploaded_file(&layout, &commit_path, pending)
                })
                .await?;
                if !preserve {
                    // BackupAgent2 may Move/Copy a filtered file later. Commit
                    // an empty placeholder only after its transfer succeeds.
                    self.discarded_files.push(output_path);
                }
            }
        }

        Ok(())
    }

    fn relocate_discarded_files(&mut self, source: &Path, destination: &Path, is_directory: bool) {
        let mut relocated = Vec::with_capacity(self.discarded_files.len());
        for path in self.discarded_files.drain(..) {
            let should_relocate = path == source || (is_directory && path.starts_with(source));
            if should_relocate {
                let suffix = path.strip_prefix(source).unwrap_or(Path::new(""));
                relocated.push(if suffix.as_os_str().is_empty() {
                    destination.to_path_buf()
                } else {
                    destination.join(suffix)
                });
            } else {
                relocated.push(path);
            }
        }
        self.discarded_files = relocated;
    }

    fn copy_discarded_files(&mut self, source: &Path, destination: &Path, is_directory: bool) {
        let copied = self
            .discarded_files
            .iter()
            .filter(|path| *path == source || (is_directory && path.starts_with(source)))
            .map(|path| {
                let suffix = path.strip_prefix(source).unwrap_or(Path::new(""));
                if suffix.as_os_str().is_empty() {
                    destination.to_path_buf()
                } else {
                    destination.join(suffix)
                }
            })
            .collect::<Vec<_>>();
        self.discarded_files.extend(copied);
    }

    fn forget_discarded_files(&mut self, target: &Path) {
        self.discarded_files
            .retain(|path| path != target && !path.starts_with(target));
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
            let requested_path = rel.to_owned();
            let local_path = with_layout(layout, move |layout| {
                resolve_relative_path(&layout, &requested_path)
            })
            .await?;
            self.account_transfer_file()?;
            write_prefixed_string(self.device_link.stream_mut(), rel).await?;

            match with_layout(layout, move |layout| {
                open_layout_file_for_read(&layout, &local_path)
            })
            .await
            {
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
                        self.account_transfer_bytes(n)?;
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

    // The path policy rejects symlink components deliberately.  macOS exposes
    // its temporary directory through `/var`, which is a system symlink, and
    // Windows may return a verbatim or short-name spelling.  Use the same
    // canonical spelling as production code for fixtures so these tests cover
    // the path policy itself rather than host-specific aliases.
    fn test_temp_dir() -> PathBuf {
        super::canonicalize_simplified(&std::env::temp_dir()).expect("canonical temp directory")
    }

    fn test_backup_root(label: &str, identifier: &str) -> PathBuf {
        let root = test_temp_dir().join(format!(
            "ios-core-backup2-device-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let directory = root.join(identifier);
        std::fs::create_dir_all(&directory).expect("backup fixture directory");
        for name in ["Info.plist", "Manifest.plist", "Status.plist"] {
            std::fs::write(directory.join(name), b"fixture").expect("backup fixture metadata");
        }
        root
    }

    async fn complete_device_link_handshake(stream: &mut tokio::io::DuplexStream) {
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::Integer(300u64.into()),
            ])))
            .await
            .expect("write version exchange");
        assert_eq!(
            read_test_frame(stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageVersionExchange".into()),
                plist::Value::String("DLVersionsOk".into()),
                plist::Value::Integer(300u64.into()),
            ])
        );
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageDeviceReady".into()),
            ])))
            .await
            .expect("write device ready");
        let hello = read_test_frame(stream).await;
        let hello = hello.as_array().expect("hello frame");
        assert_eq!(hello[0].as_string(), Some("DLMessageProcessMessage"));
        let hello = hello[1].as_dictionary().expect("hello dictionary");
        assert_eq!(
            hello.get("MessageName").and_then(plist::Value::as_string),
            Some("Hello")
        );
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("ErrorCode".to_string(), plist::Value::Integer(0u64.into())),
                    ("ProtocolVersion".to_string(), plist::Value::Real(2.1)),
                ])),
            ])))
            .await
            .expect("write hello response");
    }

    async fn complete_process_message(
        stream: &mut tokio::io::DuplexStream,
        error_code: u64,
        content: Option<plist::Value>,
    ) {
        let mut response = plist::Dictionary::from_iter([(
            "ErrorCode".to_string(),
            plist::Value::Integer(error_code.into()),
        )]);
        if let Some(content) = content {
            response.insert("Content".into(), content);
        }
        stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageProcessMessage".into()),
                plist::Value::Dictionary(response),
            ])))
            .await
            .expect("write process response");
    }

    #[tokio::test]
    async fn device_unback_sends_exact_request_and_returns_content() {
        let root = test_backup_root("unback", "source-id");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .unback(
                    &client_root,
                    "target-device",
                    Some("source-id"),
                    Some("秘密🔐"),
                )
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let request = read_test_frame(&mut server_stream).await;
        let request = request.as_array().expect("Unback process frame");
        assert_eq!(request[0].as_string(), Some("DLMessageProcessMessage"));
        assert_eq!(
            request[1]
                .as_dictionary()
                .expect("Unback request dictionary"),
            &plist::Dictionary::from_iter([
                (
                    "MessageName".to_string(),
                    plist::Value::String("Unback".into()),
                ),
                (
                    "TargetIdentifier".to_string(),
                    plist::Value::String("target-device".into()),
                ),
                (
                    "SourceIdentifier".to_string(),
                    plist::Value::String("source-id".into()),
                ),
                (
                    "Password".to_string(),
                    plist::Value::String("秘密🔐".into()),
                ),
            ])
        );

        complete_process_message(
            &mut server_stream,
            0,
            Some(plist::Value::String("expanded".into())),
        )
        .await;
        assert_eq!(
            task.await
                .expect("Unback client task")
                .expect("Unback result"),
            Some(plist::Value::String("expanded".into()))
        );
        assert_eq!(
            read_test_frame(&mut server_stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageDisconnect".into()),
                plist::Value::String("___EmptyParameterString___".into()),
            ])
        );
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_extract_preserves_unicode_and_omits_empty_optional_fields() {
        let root = test_backup_root("extract", "target-device");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .extract(
                    &client_root,
                    "target-device",
                    "AppDomain-com.example.测试",
                    "Library/数据/файл.txt",
                    Some(""),
                    Some(""),
                )
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let request = read_test_frame(&mut server_stream).await;
        let request = request.as_array().expect("Extract process frame");
        let request = request[1].as_dictionary().expect("Extract dictionary");
        assert_eq!(
            request.get("MessageName").and_then(plist::Value::as_string),
            Some("Extract")
        );
        assert_eq!(
            request
                .get("TargetIdentifier")
                .and_then(plist::Value::as_string),
            Some("target-device")
        );
        assert_eq!(
            request.get("DomainName").and_then(plist::Value::as_string),
            Some("AppDomain-com.example.测试")
        );
        assert_eq!(
            request
                .get("RelativePath")
                .and_then(plist::Value::as_string),
            Some("Library/数据/файл.txt")
        );
        assert!(!request.contains_key("SourceIdentifier"));
        assert!(!request.contains_key("Password"));

        complete_process_message(&mut server_stream, 0, None).await;
        assert_eq!(
            task.await
                .expect("Extract client task")
                .expect("Extract result"),
            None
        );
        let _ = read_test_frame(&mut server_stream).await;
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_unback_error_is_redacted_and_disconnects() {
        let root = test_backup_root("error", "target-device");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .unback(
                    &client_root,
                    "target-device",
                    None,
                    Some("do-not-leak-this-password"),
                )
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let _ = read_test_frame(&mut server_stream).await;
        let response = plist::Value::Array(vec![
            plist::Value::String("DLMessageProcessMessage".into()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                ("ErrorCode".to_string(), plist::Value::Integer(77u64.into())),
                (
                    "Password".to_string(),
                    plist::Value::String("do-not-leak-this-password".into()),
                ),
            ])),
        ]);
        server_stream
            .write_all(&encode_test_frame(&response))
            .await
            .expect("write Unback error");
        let error = task
            .await
            .expect("Unback client task")
            .expect_err("device error should fail Unback");
        assert!(!error.to_string().contains("do-not-leak-this-password"));
        let _ = read_test_frame(&mut server_stream).await;
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_process_response_requires_error_code() {
        let root = test_backup_root("missing-error-code", "target-device");
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client
                .unback(&client_root, "target-device", None, None)
                .await
        });

        complete_device_link_handshake(&mut server_stream).await;
        let _ = read_test_frame(&mut server_stream).await;
        let response = plist::Value::Array(vec![
            plist::Value::String("DLMessageProcessMessage".into()),
            plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Content".to_string(),
                    plist::Value::String("unexpected-success".into()),
                ),
                (
                    "Password".to_string(),
                    plist::Value::String("secret-not-for-errors".into()),
                ),
            ])),
        ]);
        server_stream
            .write_all(&encode_test_frame(&response))
            .await
            .expect("write malformed process response");
        let error = task
            .await
            .expect("Unback client task")
            .expect_err("missing ErrorCode must fail the operation");
        assert!(error.to_string().contains("missing numeric ErrorCode"));
        assert!(!error.to_string().contains("secret-not-for-errors"));
        let _ = read_test_frame(&mut server_stream).await;
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn device_extract_rejects_path_escape_before_connecting() {
        let (client_stream, _server_stream) = duplex(1024);
        let mut client = Mobilebackup2Client::new(client_stream);
        let error = client
            .extract(
                Path::new("does-not-exist"),
                "target-device",
                "HomeDomain",
                "../outside",
                None,
                None,
            )
            .await
            .expect_err("path traversal must be rejected");
        assert!(error.to_string().contains("escapes"));
    }

    #[tokio::test]
    async fn device_link_upload_frame_is_written_with_bounded_payload_handling() {
        let root = test_backup_root("upload", "source-id");
        let layout = BackupDirectoryLayout {
            root: root.clone(),
            device_directory: root.join("source-id"),
            target_identifier: "source-id".into(),
        };
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let client_layout = layout.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(client_stream);
            client.run_loop(&client_layout).await
        });

        server_stream
            .write_all(&encode_test_frame(&plist::Value::Array(vec![
                plist::Value::String("DLMessageUploadFiles".into()),
            ])))
            .await
            .expect("write upload command");
        write_prefixed_string(&mut server_stream, "HomeDomain/数据.txt")
            .await
            .expect("write device path");
        write_prefixed_string(&mut server_stream, "HomeDomain/数据.txt")
            .await
            .expect("write local path");
        write_transfer_frame(
            &mut server_stream,
            FILE_TRANSFER_CODE_FILE_DATA,
            "内容".as_bytes(),
        )
        .await
        .expect("write upload data");
        write_transfer_frame(&mut server_stream, FILE_TRANSFER_CODE_SUCCESS, &[])
            .await
            .expect("write upload completion");
        write_prefixed_string(&mut server_stream, "")
            .await
            .expect("write upload terminator");

        assert_eq!(
            read_test_frame(&mut server_stream).await,
            plist::Value::Array(vec![
                plist::Value::String("DLMessageStatusResponse".into()),
                plist::Value::Integer(0i64.into()),
                plist::Value::String(EMPTY_PARAMETER_STRING.into()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ])
        );
        complete_process_message(&mut server_stream, 0, None).await;
        assert_eq!(task.await.expect("upload client task").unwrap(), None);
        assert_eq!(
            std::fs::read(root.join("source-id/HomeDomain/数据.txt")).unwrap(),
            "内容".as_bytes()
        );
        std::fs::remove_dir_all(root).expect("remove backup fixture");
    }

    #[tokio::test]
    async fn filtered_backup_moves_copies_lists_and_removes_then_cleans_only_placeholders() {
        let root = test_backup_root("filtered-host-work", "source-id");
        std::fs::write(root.join("source-id/user-data"), b"keep unrelated data").unwrap();
        let (stream, mut peer) = duplex(16 * 1024);
        let client_root = root.clone();
        let task = tokio::spawn(async move {
            let mut client = Mobilebackup2Client::new(stream);
            client
                .backup_with_options(
                    &client_root,
                    "source-id",
                    true,
                    &plist::Dictionary::new(),
                    BackupOptions {
                        filter: super::super::build_backup_filter(&["sms".into()], &[]).unwrap(),
                        ..BackupOptions::default()
                    },
                )
                .await
        });
        let exchange = async {
            complete_device_link_handshake(&mut peer).await;
            assert_eq!(
                read_test_frame(&mut peer).await,
                plist::Value::Array(vec![
                    "DLMessageProcessMessage".into(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        ("MessageName", plist::Value::String("Backup".into())),
                        ("TargetIdentifier", plist::Value::String("source-id".into())),
                    ])),
                ])
            );
            let empty_status = plist::Value::Array(vec![
                "DLMessageStatusResponse".into(),
                plist::Value::Integer(0i64.into()),
                EMPTY_PARAMETER_STRING.into(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ]);
            peer.write_all(&encode_test_frame(&plist::Value::Array(vec![
                "DLMessageUploadFiles".into(),
            ])))
            .await
            .unwrap();
            for name in ["HomeDomain/not-selected.txt", "HomeDomain/not-selected.txt"] {
                write_prefixed_string(&mut peer, name).await.unwrap();
            }
            write_transfer_frame(
                &mut peer,
                FILE_TRANSFER_CODE_FILE_DATA,
                b"discarded payload",
            )
            .await
            .unwrap();
            write_transfer_frame(&mut peer, FILE_TRANSFER_CODE_SUCCESS, &[])
                .await
                .unwrap();
            write_prefixed_string(&mut peer, "").await.unwrap();
            assert_eq!(read_test_frame(&mut peer).await, empty_status);

            for command in [
                vec![
                    "DLMessageMoveItems".into(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "HomeDomain/not-selected.txt",
                        plist::Value::String("HomeDomain/moved.txt".into()),
                    )])),
                ],
                vec![
                    "DLMessageCopyItem".into(),
                    "HomeDomain/moved.txt".into(),
                    "HomeDomain/copied.txt".into(),
                ],
                vec![
                    "DLMessageMoveFiles".into(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "HomeDomain/missing.txt",
                        plist::Value::String("HomeDomain/missing-moved.txt".into()),
                    )])),
                ],
                vec![
                    "DLMessageCopyItem".into(),
                    "HomeDomain/missing.txt".into(),
                    "HomeDomain/missing-copy.txt".into(),
                ],
            ] {
                peer.write_all(&encode_test_frame(&plist::Value::Array(command)))
                    .await
                    .unwrap();
                assert_eq!(read_test_frame(&mut peer).await, empty_status);
            }
            peer.write_all(&encode_test_frame(&plist::Value::Array(vec![
                "DLContentsOfDirectory".into(),
                "HomeDomain".into(),
            ])))
            .await
            .unwrap();
            let listing = read_test_frame(&mut peer).await;
            let listing = listing.as_array().unwrap();
            assert_eq!(&listing[..3], &empty_status.as_array().unwrap()[..3]);
            let entries = listing[3].as_dictionary().unwrap();
            assert_eq!(entries.len(), 2);
            for name in ["moved.txt", "copied.txt"] {
                assert_eq!(
                    entries[name].as_dictionary().unwrap()["DLFileSize"].as_unsigned_integer(),
                    Some(0)
                );
            }
            peer.write_all(&encode_test_frame(&plist::Value::Array(vec![
                "DLMessageRemoveFiles".into(),
                plist::Value::Array(vec!["HomeDomain/copied.txt".into()]),
            ])))
            .await
            .unwrap();
            assert_eq!(read_test_frame(&mut peer).await, empty_status);
            complete_process_message(&mut peer, 0, None).await;
            assert_eq!(
                read_test_frame(&mut peer).await,
                plist::Value::Array(vec![
                    "DLMessageDisconnect".into(),
                    EMPTY_PARAMETER_STRING.into(),
                ])
            );
            task.await.unwrap().unwrap()
        };
        let result = tokio::time::timeout(Duration::from_secs(5), exchange)
            .await
            .unwrap();
        assert_eq!(result.layout.target_identifier, "source-id");
        assert!(!root.join("source-id/HomeDomain").exists());
        assert_eq!(
            std::fs::read(root.join("source-id/user-data")).unwrap(),
            b"keep unrelated data"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn upload_failures_and_cancellation_preserve_old_file_and_clean_staging() {
        for failure in ["eof", "cancel", "remote-error"] {
            let root = test_backup_root(failure, "source-id");
            let parent = root.join("source-id/HomeDomain");
            std::fs::create_dir(&parent).unwrap();
            let output = parent.join("数据.txt");
            std::fs::write(&output, b"previous complete backup").unwrap();
            let layout = BackupDirectoryLayout {
                root: root.clone(),
                device_directory: root.join("source-id"),
                target_identifier: "source-id".into(),
            };
            let (client_stream, mut server_stream) = duplex(16 * 1024);
            let task = tokio::spawn(async move {
                Mobilebackup2Client::new(client_stream)
                    .run_loop(&layout)
                    .await
            });
            server_stream
                .write_all(&encode_test_frame(&plist::Value::Array(vec![
                    plist::Value::String("DLMessageUploadFiles".into()),
                ])))
                .await
                .unwrap();
            for _ in 0..2 {
                write_prefixed_string(&mut server_stream, "HomeDomain/数据.txt")
                    .await
                    .unwrap();
            }
            write_transfer_frame(&mut server_stream, FILE_TRANSFER_CODE_FILE_DATA, b"partial")
                .await
                .unwrap();
            // Observe an actual staged write before injecting the failure.
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let written = std::fs::read_dir(&parent).unwrap().any(|entry| {
                        let entry = entry.unwrap();
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".ios-output-")
                            && entry.metadata().unwrap().len() == 7
                    });
                    if written {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("staged partial payload");
            assert_eq!(std::fs::read(&output).unwrap(), b"previous complete backup");
            match failure {
                "cancel" => {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                }
                "eof" => {
                    drop(server_stream);
                    assert!(task.await.unwrap().is_err());
                }
                _ => {
                    write_transfer_frame(
                        &mut server_stream,
                        FILE_TRANSFER_CODE_REMOTE_ERROR,
                        b"synthetic device error",
                    )
                    .await
                    .unwrap();
                    write_prefixed_string(&mut server_stream, "").await.unwrap();
                    let _ = read_test_frame(&mut server_stream).await;
                    complete_process_message(&mut server_stream, 0, None).await;
                    // Per-file remote errors keep the established DeviceLink
                    // flow; a successful loop is not proof of every artifact.
                    assert_eq!(task.await.unwrap().unwrap(), None);
                }
            }
            assert_eq!(std::fs::read(&output).unwrap(), b"previous complete backup");
            assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 1);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn transfer_budgets_reject_excess_files_and_bytes() {
        let (stream, _) = duplex(16);
        let mut client = Mobilebackup2Client::new(stream);
        client.transfer_files = MAX_DEVICE_TRANSFER_FILES;
        assert!(client.account_transfer_file().is_err());
        client.transfer_bytes = MAX_DEVICE_TRANSFER_BYTES;
        assert!(client.account_transfer_bytes(1).is_err());
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

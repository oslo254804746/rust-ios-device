//! InstallationProxy – app listing, install, uninstall.
//!
//! Service: `com.apple.mobile.installation_proxy`
//! Protocol: plist-framed (same 4-byte BE length prefix as lockdown).
//!
//! Reference: go-ios/ios/installationproxy/installationproxy.go

crate::redaction::error_debug!(IpError);
crate::redaction::error_debug!(BrowseLimit);

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};

pub const SERVICE_NAME: &str = "com.apple.mobile.installation_proxy";

/// Maximum number of Browse response chunks accepted without a Complete status.
///
/// A healthy device normally returns only a handful of chunks. Keeping a hard
/// upper bound prevents a device or a broken connection from making the client
/// retain responses forever.
const MAX_BROWSE_CHUNKS: usize = 1_000;

/// Maximum number of app entries retained from one Browse operation.
const MAX_BROWSE_ENTRIES: usize = 1_000_000;
const MAX_PLIST_SIZE: usize = 4 * 1024 * 1024;

/// Caller ceilings for one bounded user-application Browse.
///
/// Reply wire bytes include each four-byte length prefix and declared plist
/// body. They exclude the fixed Browse request and are not a decoded-model,
/// serialized-output, allocator, or RSS measurement. Values above the existing
/// 1,000-frame and 1,000,000-entry limits cannot raise those hard limits. The
/// existing four-MiB limit still applies to each reply body.
#[derive(Debug, Clone, Copy)]
pub struct BrowseLimits {
    total_timeout: Duration,
    max_entries: usize,
    max_wire_bytes: usize,
    max_frames: usize,
}

impl BrowseLimits {
    /// Start with 10,000 entries, 16 MiB of replies, and 1,000 reply frames.
    /// The total starts when the bounded operation is first polled and includes
    /// sending/flushing the request and every reply, without restarting per frame.
    pub fn new(total_timeout: Duration) -> Self {
        Self {
            total_timeout,
            max_entries: 10_000,
            max_wire_bytes: 16 * 1024 * 1024,
            max_frames: MAX_BROWSE_CHUNKS,
        }
    }

    /// Zero allows a completed empty list. An oversized list is never truncated.
    pub fn with_max_entries(mut self, max_entries: usize) -> Self {
        self.max_entries = max_entries;
        self
    }

    /// Fewer than four bytes rejects before request I/O.
    pub fn with_max_wire_bytes(mut self, max_wire_bytes: usize) -> Self {
        self.max_wire_bytes = max_wire_bytes;
        self
    }

    /// Zero rejects before request I/O, because completion requires a reply.
    pub fn with_max_frames(mut self, max_frames: usize) -> Self {
        self.max_frames = max_frames;
        self
    }
}

#[derive(thiserror::Error)]
#[error("InstallationProxy Browse {kind} exceeds limit {limit} (received {received})")]
pub(crate) struct BrowseLimit {
    kind: &'static str,
    limit: u64,
    received: u64,
}

fn browse_limit(kind: &'static str, limit: usize, received: u64) -> IpError {
    IpError::Io(std::io::Error::other(BrowseLimit {
        kind,
        limit: limit as u64,
        received,
    }))
}

/// The complete device error observation, retained without echoing its text.
#[derive(thiserror::Error)]
#[error("device rejected InstallationProxy Browse")]
pub(crate) struct BrowseResponseError {
    code: String,
    description: Option<String>,
}

impl std::fmt::Debug for BrowseResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowseResponseError")
            .field("code", &crate::redaction::diagnostic_id(&self.code))
            .field("has_description", &self.description.is_some())
            .finish()
    }
}

struct BrowseGuard<'a, S> {
    stream: &'a mut S,
    usable: &'a mut bool,
    progressed: bool,
    failed: bool,
    completed: bool,
}

impl<S> Drop for BrowseGuard<'_, S> {
    fn drop(&mut self) {
        if !self.completed && (self.progressed || self.failed) {
            *self.usable = false;
        }
    }
}

struct BrowseIo<'a, S> {
    inner: &'a mut S,
    progressed: &'a mut bool,
    failed: &'a mut bool,
    deadline: tokio::time::Instant,
}

impl<S: AsyncRead + Unpin> AsyncRead for BrowseIo<'_, S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if tokio::time::Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let before = buf.filled().len();
        let result = Pin::new(&mut *self.inner).poll_read(cx, buf);
        *self.progressed |= buf.filled().len() > before;
        if matches!(result, Poll::Ready(Err(_))) {
            *self.failed = true;
        }
        if matches!(result, Poll::Ready(Ok(()))) && tokio::time::Instant::now() >= self.deadline {
            // The outer timer supplies the original Elapsed. Suppressing this
            // late success also prevents any subsequent read in this poll.
            Poll::Pending
        } else {
            result
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for BrowseIo<'_, S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if tokio::time::Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_write(cx, bytes);
        if matches!(result, Poll::Ready(Err(_))) {
            *self.failed = true;
        }
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            *self.progressed = true;
        }
        if matches!(result, Poll::Ready(Ok(_))) && tokio::time::Instant::now() >= self.deadline {
            Poll::Pending
        } else {
            result
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if tokio::time::Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_flush(cx);
        if matches!(result, Poll::Ready(Err(_))) {
            *self.failed = true;
        }
        if matches!(result, Poll::Ready(Ok(()))) && tokio::time::Instant::now() >= self.deadline {
            Poll::Pending
        } else {
            result
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if tokio::time::Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        let result = Pin::new(&mut *self.inner).poll_shutdown(cx);
        if matches!(result, Poll::Ready(Err(_))) {
            *self.failed = true;
        }
        if matches!(result, Poll::Ready(Ok(()))) && tokio::time::Instant::now() >= self.deadline {
            Poll::Pending
        } else {
            result
        }
    }
}

async fn browse_checkpoint(deadline: tokio::time::Instant) -> Result<(), IpError> {
    if tokio::time::Instant::now() < deadline {
        return Ok(());
    }
    match tokio::time::timeout_at(deadline, std::future::pending::<()>()).await {
        Err(elapsed) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into()),
        Ok(()) => unreachable!("pending future completed"),
    }
}

#[derive(thiserror::Error)]
pub enum IpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("install error: {0}")]
    Install(String),
}

/// Summary of an installed app.
#[derive(Debug, Clone)]
pub struct AppInfo {
    pub bundle_id: String,
    pub display_name: String,
    pub version: String,
    pub app_type: String,
    pub path: String,
    pub extra: HashMap<String, plist::Value>,
}

/// InstallationProxy client.
pub struct InstallationProxy<S> {
    stream: S,
    usable: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin> InstallationProxy<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            usable: true,
        }
    }

    /// Local admission after a bounded Browse, not peer liveness or retry safety.
    /// Legacy requests retain their original lifecycle behavior until a bounded
    /// operation invalidates this connection.
    pub fn is_usable(&self) -> bool {
        self.usable
    }

    fn ensure_usable(&self) -> Result<(), IpError> {
        if self.usable {
            Ok(())
        } else {
            Err(IpError::Protocol(
                "InstallationProxy connection is unusable; reconnect before another request".into(),
            ))
        }
    }

    /// Install an app from a staged IPA path on the device.
    pub async fn install(&mut self, package_path: &str) -> Result<(), IpError> {
        self.ensure_usable()?;
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command": "Install",
                "PackagePath": package_path,
                "ClientOptions": {},
            }),
        )
        .await?;

        self.wait_for_completion().await
    }

    /// Upgrade an app from a staged IPA path on the device.
    pub async fn upgrade(&mut self, package_path: &str) -> Result<(), IpError> {
        self.ensure_usable()?;
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command": "Upgrade",
                "PackagePath": package_path,
                "ClientOptions": {},
            }),
        )
        .await?;

        self.wait_for_completion().await
    }

    /// Install with one total budget for the request and all progress replies.
    /// Dropping after wire progress invalidates this connection. Expiry is not
    /// an installation rollback: the remote effect remains unknown.
    pub async fn install_with_timeout(
        &mut self,
        package_path: &str,
        budget: std::time::Duration,
    ) -> Result<(), IpError> {
        self.mutate_with_timeout("Install", package_path, budget)
            .await
    }

    /// Upgrade under the same ownership contract as `install_with_timeout`.
    pub async fn upgrade_with_timeout(
        &mut self,
        package_path: &str,
        budget: std::time::Duration,
    ) -> Result<(), IpError> {
        self.mutate_with_timeout("Upgrade", package_path, budget)
            .await
    }

    async fn mutate_with_timeout(
        &mut self,
        command: &str,
        package_path: &str,
        budget: std::time::Duration,
    ) -> Result<(), IpError> {
        self.ensure_usable()?;
        if budget.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "installation budget is zero",
            )
            .into());
        }
        let deadline = tokio::time::Instant::now()
            .checked_add(budget)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "installation deadline overflow",
                )
            })?;
        let mut guard = BrowseGuard {
            stream: &mut self.stream,
            usable: &mut self.usable,
            progressed: false,
            failed: false,
            completed: false,
        };
        let mut terminal = false;
        let result = {
            let mut io = BrowseIo {
                inner: &mut *guard.stream,
                progressed: &mut guard.progressed,
                failed: &mut guard.failed,
                deadline,
            };
            tokio::time::timeout_at(deadline, async {
                send_plist(
                    &mut io,
                    &serde_json::json!({
                        "Command": command, "PackagePath": package_path, "ClientOptions": {}
                    }),
                )
                .await?;
                loop {
                    let data = recv_plist_raw(&mut io).await?;
                    let reply: plist::Dictionary = plist::from_bytes(&data)?;
                    terminal =
                        reply.get("Status").and_then(plist::Value::as_string) == Some("Complete");
                    if let Some(error) = reply.get("Error") {
                        return Err(IpError::Install(format!(
                            "{error:?}: {:?}",
                            reply.get("ErrorDescription")
                        )));
                    }
                    if terminal {
                        return Ok(());
                    }
                    // Progress does not reset the total clock. Yield even for
                    // an always-ready peer so cancellation/timers can run.
                    tokio::task::yield_now().await;
                }
            })
            .await
            .map_err(|elapsed| {
                IpError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed))
            })?
        };
        guard.completed = terminal && !guard.failed;
        result
    }

    /// List all user-installed apps.
    pub async fn list_user_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("User", true, &[]).await
    }

    /// List user apps under one total deadline and caller reply/entry ceilings.
    ///
    /// Zero/overflowing deadlines, zero frame limits and fewer than four wire
    /// bytes fail before transport work. A zero entry limit allows an empty
    /// completed list. No partial result is returned on any failure.
    ///
    /// Cancellation/expiry after wire progress, native I/O failures, malformed
    /// replies, and local budget refusals after progress invalidate this client.
    /// Subsequent legacy and bounded methods reject before I/O; construct a
    /// fresh client. Complete
    /// success permits reuse. A complete legal Error response permits reuse
    /// only with an explicit Status Complete; missing/unknown status does not.
    /// The original device error is retained with safe Display/Debug. No retry,
    /// mutation replay, reconnect, or asynchronous Drop is introduced.
    pub async fn list_user_apps_with_limits(
        &mut self,
        limits: BrowseLimits,
    ) -> Result<Vec<AppInfo>, IpError> {
        if limits.total_timeout.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "InstallationProxy Browse total budget is zero",
            )
            .into());
        }
        let deadline = tokio::time::Instant::now()
            .checked_add(limits.total_timeout)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "InstallationProxy Browse deadline is too large",
                )
            })?;
        let max_frames = limits.max_frames.min(MAX_BROWSE_CHUNKS);
        let max_entries = limits.max_entries.min(MAX_BROWSE_ENTRIES);
        if max_frames == 0 {
            return Err(browse_limit("frames", max_frames, 1));
        }
        if limits.max_wire_bytes < 4 {
            return Err(browse_limit("reply wire bytes", limits.max_wire_bytes, 4));
        }
        self.ensure_usable()?;
        let mut guard = BrowseGuard {
            stream: &mut self.stream,
            usable: &mut self.usable,
            progressed: false,
            failed: false,
            completed: false,
        };
        let mut terminal_business_error = false;
        let result = {
            let mut io = BrowseIo {
                inner: &mut *guard.stream,
                progressed: &mut guard.progressed,
                failed: &mut guard.failed,
                deadline,
            };
            let operation = async {
                send_plist(
                    &mut io,
                    &serde_json::json!({
                        "Command": "Browse",
                        "ClientOptions": {
                            "ApplicationType": "User",
                            "ShowLaunchProhibitedApps": true,
                        },
                    }),
                )
                .await?;
                let mut apps = Vec::new();
                let mut frames = 0usize;
                let mut wire_bytes = 0u64;
                loop {
                    browse_checkpoint(deadline).await?;
                    if frames >= max_frames {
                        return Err(browse_limit("frames", max_frames, frames as u64 + 1));
                    }
                    if wire_bytes + 4 > limits.max_wire_bytes as u64 {
                        return Err(browse_limit(
                            "reply wire bytes",
                            limits.max_wire_bytes,
                            wire_bytes + 4,
                        ));
                    }
                    let mut prefix = [0; 4];
                    io.read_exact(&mut prefix).await?;
                    let length = u32::from_be_bytes(prefix) as u64;
                    // With at most 1,000 prior replies, each limited to four MiB,
                    // this u64 addition cannot overflow on 32- or 64-bit hosts.
                    let declared_wire_bytes = wire_bytes + 4 + length;
                    if declared_wire_bytes > limits.max_wire_bytes as u64 {
                        return Err(browse_limit(
                            "reply wire bytes",
                            limits.max_wire_bytes,
                            declared_wire_bytes,
                        ));
                    }
                    if length > MAX_PLIST_SIZE as u64 {
                        return Err(IpError::Protocol(format!(
                            "plist length {length} exceeds maximum of {MAX_PLIST_SIZE}"
                        )));
                    }
                    wire_bytes = declared_wire_bytes;
                    frames += 1;
                    browse_checkpoint(deadline).await?;
                    let mut data = vec![0; length as usize];
                    io.read_exact(&mut data).await?;
                    let mut response: plist::Dictionary = plist::from_bytes(&data)?;
                    if let Some(error) = response.remove("Error") {
                        let code = error.into_string().ok_or_else(|| {
                            IpError::Protocol("Browse Error field is not a string".into())
                        })?;
                        let description = response
                            .remove("ErrorDescription")
                            .map(|value| {
                                value.into_string().ok_or_else(|| {
                                    IpError::Protocol(
                                        "Browse ErrorDescription field is not a string".into(),
                                    )
                                })
                            })
                            .transpose()?;
                        terminal_business_error =
                            response.get("Status").and_then(plist::Value::as_string)
                                == Some("Complete");
                        return Err(IpError::Io(std::io::Error::other(BrowseResponseError {
                            code,
                            description,
                        })));
                    }
                    let status = response
                        .get("Status")
                        .and_then(plist::Value::as_string)
                        .ok_or_else(|| {
                            IpError::Protocol("Browse reply has no string Status".into())
                        })?;
                    let complete = status == "Complete";
                    let current_list = match response.remove("CurrentList") {
                        None => Vec::new(),
                        Some(value) => value.into_array().ok_or_else(|| {
                            IpError::Protocol("Browse CurrentList field is not an array".into())
                        })?,
                    };
                    if current_list
                        .iter()
                        .any(|value| value.as_dictionary().is_none())
                    {
                        return Err(IpError::Protocol(
                            "Browse app entry is not a dictionary".into(),
                        ));
                    }
                    let total_entries = apps
                        .len()
                        .checked_add(current_list.len())
                        .ok_or_else(|| browse_limit("entries", max_entries, u64::MAX))?;
                    if total_entries > max_entries {
                        return Err(browse_limit("entries", max_entries, total_entries as u64));
                    }
                    browse_checkpoint(deadline).await?;
                    for value in current_list {
                        apps.push(parse_app_info(value));
                    }
                    browse_checkpoint(deadline).await?;
                    if complete {
                        return Ok(apps);
                    }
                }
            };
            let result = match tokio::time::timeout_at(deadline, operation).await {
                Ok(result) => result,
                Err(elapsed) => {
                    Err(std::io::Error::new(std::io::ErrorKind::TimedOut, elapsed).into())
                }
            };
            if result.is_ok() {
                browse_checkpoint(deadline).await?;
            }
            result
        };
        guard.completed = result.is_ok() || terminal_business_error;
        result
    }

    /// List user-installed apps with a narrowed set of returned attributes.
    pub async fn list_user_apps_with_attributes(
        &mut self,
        return_attributes: &[&str],
    ) -> Result<Vec<AppInfo>, IpError> {
        self.browse("User", true, return_attributes).await
    }

    /// List all apps (user + system).
    pub async fn list_all_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("", true, &[]).await
    }

    /// List only system apps.
    pub async fn list_system_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("System", false, &[]).await
    }

    /// List only hidden apps.
    pub async fn list_hidden_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        self.browse("Hidden", true, &[]).await
    }

    /// List apps that expose iTunes/File Sharing.
    pub async fn list_file_sharing_apps(&mut self) -> Result<Vec<AppInfo>, IpError> {
        let apps = self.list_all_apps().await?;
        Ok(apps
            .into_iter()
            .filter(|app| {
                app.extra
                    .get("UIFileSharingEnabled")
                    .and_then(plist::Value::as_boolean)
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Uninstall an app by bundle ID.
    pub async fn uninstall(&mut self, bundle_id: &str) -> Result<(), IpError> {
        self.send_bundle_identifier_command("Uninstall", bundle_id)
            .await
    }

    /// Archive an app by bundle ID.
    pub async fn archive(&mut self, bundle_id: &str) -> Result<(), IpError> {
        self.send_bundle_identifier_command("Archive", bundle_id)
            .await
    }

    /// Restore an archived app by bundle ID.
    pub async fn restore(&mut self, bundle_id: &str) -> Result<(), IpError> {
        self.send_bundle_identifier_command("Restore", bundle_id)
            .await
    }

    async fn send_bundle_identifier_command(
        &mut self,
        command: &str,
        bundle_id: &str,
    ) -> Result<(), IpError> {
        self.ensure_usable()?;
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command":               command,
                "ApplicationIdentifier": bundle_id,
                "ClientOptions":         {},
            }),
        )
        .await?;

        self.wait_for_completion().await
    }

    /// Look up a single app by bundle ID.
    pub async fn lookup_app(&mut self, bundle_id: &str) -> Result<Option<AppInfo>, IpError> {
        self.lookup_app_with_attributes(bundle_id, &[]).await
    }

    /// Look up a single app by bundle ID with optional return-attribute filtering.
    pub async fn lookup_app_with_attributes(
        &mut self,
        bundle_id: &str,
        return_attributes: &[&str],
    ) -> Result<Option<AppInfo>, IpError> {
        let mut options = serde_json::json!({
            "BundleIDs": [bundle_id],
        });
        if !return_attributes.is_empty() {
            options["ReturnAttributes"] = serde_json::Value::Array(
                return_attributes
                    .iter()
                    .map(|attr| serde_json::Value::String((*attr).to_string()))
                    .collect(),
            );
        }

        let response = self.lookup(options).await?;

        Ok(response
            .into_iter()
            .next()
            .map(|(lookup_bundle_id, value)| {
                parse_app_info_with_bundle_id(&lookup_bundle_id, value)
            }))
    }

    async fn browse(
        &mut self,
        app_type: &str,
        show_prohibited: bool,
        return_attributes: &[&str],
    ) -> Result<Vec<AppInfo>, IpError> {
        self.ensure_usable()?;
        let mut client_opts = serde_json::json!({});
        if !app_type.is_empty() {
            client_opts["ApplicationType"] = serde_json::Value::String(app_type.to_string());
        }
        if show_prohibited {
            client_opts["ShowLaunchProhibitedApps"] = serde_json::Value::Bool(true);
        }
        if !return_attributes.is_empty() {
            client_opts["ReturnAttributes"] = serde_json::Value::Array(
                return_attributes
                    .iter()
                    .map(|attr| serde_json::Value::String((*attr).to_string()))
                    .collect(),
            );
        }

        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command":       "Browse",
                "ClientOptions": client_opts,
            }),
        )
        .await?;

        let mut apps = Vec::new();
        let mut chunks = 0usize;
        loop {
            let data = recv_plist_raw(&mut self.stream).await?;
            let resp: plist::Dictionary = plist::from_bytes(&data)?;
            chunks += 1;

            let current_list = resp
                .get("CurrentList")
                .and_then(plist::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let chunk_entries = current_list.len();
            let total_entries = apps
                .len()
                .checked_add(chunk_entries)
                .ok_or_else(|| IpError::Protocol("Browse entry count overflow".to_string()))?;
            if total_entries > MAX_BROWSE_ENTRIES {
                return Err(IpError::Protocol(format!(
                    "Browse returned {total_entries} app entries, exceeding maximum of {MAX_BROWSE_ENTRIES}"
                )));
            }

            for item in current_list {
                apps.push(parse_app_info(item));
            }

            let complete = resp.get("Status").and_then(plist::Value::as_string) == Some("Complete");
            if complete {
                break;
            }
            if chunks >= MAX_BROWSE_CHUNKS {
                return Err(IpError::Protocol(format!(
                    "Browse exceeded {MAX_BROWSE_CHUNKS} response chunks without Status Complete"
                )));
            }
        }
        Ok(apps)
    }

    async fn lookup(
        &mut self,
        client_options: serde_json::Value,
    ) -> Result<HashMap<String, plist::Value>, IpError> {
        self.ensure_usable()?;
        send_plist(
            &mut self.stream,
            &serde_json::json!({
                "Command": "Lookup",
                "ClientOptions": client_options,
            }),
        )
        .await?;

        let data = recv_plist_raw(&mut self.stream).await?;
        let mut dict: HashMap<String, plist::Value> = plist::from_bytes(&data)?;
        if let Some(e) = dict.get("Error") {
            return Err(IpError::Install(format!("{e:?}")));
        }

        let result = dict
            .remove("LookupResult")
            .and_then(|value| value.into_dictionary())
            .map(|items| items.into_iter().collect())
            .unwrap_or_default();
        Ok(result)
    }

    async fn wait_for_completion(&mut self) -> Result<(), IpError> {
        loop {
            let data = recv_plist_raw(&mut self.stream).await?;
            let dict: HashMap<String, plist::Value> = plist::from_bytes(&data)?;
            if let Some(error) = dict.get("Error") {
                let message = match dict.get("ErrorDescription").and_then(|v| v.as_string()) {
                    Some(description) => format!("{error:?}: {description}"),
                    None => format!("{error:?}"),
                };
                return Err(IpError::Install(message));
            }
            if dict.get("Status").and_then(|s| s.as_string()) == Some("Complete") {
                return Ok(());
            }
        }
    }
}

fn parse_app_info(val: plist::Value) -> AppInfo {
    parse_app_info_with_bundle_id("", val)
}

fn parse_app_info_with_bundle_id(lookup_bundle_id: &str, val: plist::Value) -> AppInfo {
    let dict = val.into_dictionary().unwrap_or_default();
    let get_str = |k: &str| {
        dict.get(k)
            .and_then(|v| v.as_string())
            .unwrap_or("")
            .to_string()
    };

    let bundle_id = if lookup_bundle_id.is_empty() {
        get_str("CFBundleIdentifier")
    } else {
        dict.get("CFBundleIdentifier")
            .and_then(|v| v.as_string())
            .unwrap_or(lookup_bundle_id)
            .to_string()
    };
    let display_name = get_str("CFBundleDisplayName");
    let version = get_str("CFBundleShortVersionString");
    let app_type = get_str("ApplicationType");
    let path = get_str("Path");

    let extra = dict.into_iter().collect();

    AppInfo {
        bundle_id,
        display_name,
        version,
        app_type,
        path,
        extra,
    }
}

// ── plist framing ──────────────────────────────────────────────────────────────

async fn send_plist<S>(stream: &mut S, value: &serde_json::Value) -> Result<(), IpError>
where
    S: AsyncWrite + Unpin,
{
    // Convert JSON value to plist
    let plist_val = json_to_plist(value);
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist_val)?;
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist_raw<S>(stream: &mut S) -> Result<Vec<u8>, IpError>
where
    S: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PLIST_SIZE {
        return Err(IpError::Protocol(format!(
            "plist length {len} exceeds maximum of {MAX_PLIST_SIZE}"
        )));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(buf)
}

fn json_to_plist(val: &serde_json::Value) -> plist::Value {
    match val {
        serde_json::Value::Null => plist::Value::String(String::new()),
        serde_json::Value::Bool(b) => plist::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                plist::Value::Integer(plist::Integer::from(i))
            } else {
                plist::Value::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => plist::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            plist::Value::Array(arr.iter().map(json_to_plist).collect())
        }
        serde_json::Value::Object(map) => {
            let dict: plist::Dictionary = map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_plist(v)))
                .collect();
            plist::Value::Dictionary(dict)
        }
    }
}

#[cfg(test)]
mod bounded_browse_tests;

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn actual_install_progress_does_not_reset_total_and_late_reply_cannot_be_reused() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (stream, mut peer) = tokio::io::duplex(8192);
        let service = tokio::spawn(async move {
            let size = peer.read_u32().await.unwrap() as usize;
            let mut bytes = vec![0; size];
            peer.read_exact(&mut bytes).await.unwrap();
            let request: plist::Dictionary = plist::from_bytes(&bytes).unwrap();
            assert_eq!(request["Command"].as_string(), Some("Install"));
            tokio::time::sleep(Duration::from_millis(60)).await;
            peer.write_all(&plist_frame(plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    ("Status", plist::Value::String("Installing".into())),
                    ("PercentComplete", plist::Value::Integer(40u64.into())),
                ]),
            )))
            .await
            .unwrap();
            peer
        });
        let mut proxy = InstallationProxy::new(stream);
        let started = tokio::time::Instant::now();
        let error = proxy
            .install_with_timeout("/PublicStaging/a.ipa", Duration::from_millis(100))
            .await
            .unwrap_err();
        assert_eq!(started.elapsed(), Duration::from_millis(100));
        assert!(
            matches!(&error, IpError::Io(e) if e.kind() == std::io::ErrorKind::TimedOut && e.get_ref().unwrap().is::<tokio::time::error::Elapsed>())
        );
        assert!(!proxy.is_usable());
        let mut peer = service.await.unwrap();
        peer.write_all(&plist_frame(plist::Value::Dictionary(
            plist::Dictionary::from_iter([("Status", plist::Value::String("Complete".into()))]),
        )))
        .await
        .unwrap();
        assert!(proxy.install("/must-not-repeat").await.is_err());
        drop(proxy);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn actual_install_partial_send_drop_invalidates_and_zero_budget_does_no_io() {
        use tokio::io::AsyncReadExt;
        let (stream, mut peer) = tokio::io::duplex(1);
        let mut proxy = InstallationProxy::new(stream);
        assert!(proxy
            .install_with_timeout("/a", Duration::ZERO)
            .await
            .is_err());
        assert!(proxy.is_usable());
        {
            let future = proxy.install_with_timeout("/a", Duration::from_secs(1));
            tokio::pin!(future);
            tokio::select! {
                result = &mut future => panic!("unexpected completion: {result:?}"),
                read = peer.read_u8() => { read.unwrap(); }
            }
        }
        assert!(!proxy.is_usable());
        assert!(proxy.upgrade("/a").await.is_err());
        drop(proxy);
        let mut remaining = Vec::new();
        peer.read_to_end(&mut remaining).await.unwrap();
        assert!(remaining.len() <= 1);
    }

    #[tokio::test]
    async fn bounded_install_and_upgrade_consume_all_progress_and_preserve_completed_connection() {
        let complete = || {
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status",
                plist::Value::String("Complete".into()),
            )])))
        };
        let stream = ResponseStream::with_frames(vec![complete(), complete()]);
        let mut proxy = InstallationProxy::new(stream);
        proxy
            .install_with_timeout("/a", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(proxy.is_usable());
        proxy
            .upgrade_with_timeout("/a", Duration::from_secs(1))
            .await
            .unwrap();
        assert!(proxy.is_usable());
    }

    #[derive(Default)]
    struct RecordingStream {
        written: Vec<u8>,
    }

    impl AsyncRead for RecordingStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "test stream has no responses",
            )))
        }
    }

    impl AsyncWrite for RecordingStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn list_system_apps_sends_system_filter() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.list_system_apps().await.unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        assert_eq!(
            client_options["ApplicationType"].as_string(),
            Some("System")
        );
        assert!(!client_options.contains_key("ShowLaunchProhibitedApps"));
    }

    #[tokio::test]
    async fn list_hidden_apps_sends_hidden_filter() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.list_hidden_apps().await.unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        assert_eq!(
            client_options["ApplicationType"].as_string(),
            Some("Hidden")
        );
        assert_eq!(
            client_options["ShowLaunchProhibitedApps"].as_boolean(),
            Some(true)
        );
    }

    #[tokio::test]
    async fn list_file_sharing_apps_filters_on_ui_file_sharing_enabled() {
        let responses = vec![
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "Status".to_string(),
                    plist::Value::String("BrowsingApplications".into()),
                ),
                (
                    "CurrentList".to_string(),
                    plist::Value::Array(vec![
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "CFBundleIdentifier".to_string(),
                                plist::Value::String("com.example.Files".into()),
                            ),
                            (
                                "CFBundleDisplayName".to_string(),
                                plist::Value::String("Files".into()),
                            ),
                            (
                                "UIFileSharingEnabled".to_string(),
                                plist::Value::Boolean(true),
                            ),
                        ])),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "CFBundleIdentifier".to_string(),
                                plist::Value::String("com.example.Hidden".into()),
                            ),
                            (
                                "CFBundleDisplayName".to_string(),
                                plist::Value::String("Hidden".into()),
                            ),
                            (
                                "UIFileSharingEnabled".to_string(),
                                plist::Value::Boolean(false),
                            ),
                        ])),
                    ]),
                ),
            ]))),
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]))),
        ];
        let mut stream = ResponseStream::with_frames(responses);
        let mut proxy = InstallationProxy::new(&mut stream);

        let apps = proxy.list_file_sharing_apps().await.unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].bundle_id, "com.example.Files");
    }

    #[tokio::test]
    async fn browse_rejects_endless_non_complete_responses() {
        let response = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("BrowsingApplications".into()),
        )])));
        let complete = plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Complete".into()),
        )])));
        let mut responses = vec![response; MAX_BROWSE_CHUNKS];
        responses.push(complete);
        let mut stream = ResponseStream::with_frames(responses);
        let mut proxy = InstallationProxy::new(&mut stream);

        let err = proxy
            .list_all_apps()
            .await
            .expect_err("browse must stop before retaining unbounded responses");
        assert!(matches!(err, IpError::Protocol(message) if message.contains("response chunks")));
    }

    #[tokio::test]
    async fn lookup_app_sends_lookup_command_with_bundle_ids() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.lookup_app("com.example.test").await.unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Lookup"));
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        let bundle_ids = client_options["BundleIDs"].as_array().unwrap();
        assert_eq!(bundle_ids.len(), 1);
        assert_eq!(bundle_ids[0].as_string(), Some("com.example.test"));
    }

    #[tokio::test]
    async fn lookup_app_with_attributes_sends_return_attributes() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy
                .lookup_app_with_attributes("com.example.test", &["CFBundleVersion", "Path"])
                .await
                .unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        let attrs = client_options["ReturnAttributes"].as_array().unwrap();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].as_string(), Some("CFBundleVersion"));
        assert_eq!(attrs[1].as_string(), Some("Path"));
    }

    #[tokio::test]
    async fn list_user_apps_with_attributes_sends_return_attributes() {
        let mut stream = RecordingStream::default();
        let err = {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy
                .list_user_apps_with_attributes(&[
                    "CFBundleIdentifier",
                    "ApplicationSINF",
                    "iTunesMetadata",
                ])
                .await
                .unwrap_err()
        };

        assert!(matches!(err, IpError::Io(_)));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Browse"));

        let client_options = dict["ClientOptions"].as_dictionary().unwrap();
        assert_eq!(client_options["ApplicationType"].as_string(), Some("User"));
        assert_eq!(
            client_options["ShowLaunchProhibitedApps"].as_boolean(),
            Some(true)
        );
        let attrs = client_options["ReturnAttributes"].as_array().unwrap();
        assert_eq!(attrs.len(), 3);
        assert_eq!(attrs[0].as_string(), Some("CFBundleIdentifier"));
        assert_eq!(attrs[1].as_string(), Some("ApplicationSINF"));
        assert_eq!(attrs[2].as_string(), Some("iTunesMetadata"));
    }

    #[tokio::test]
    async fn install_sends_package_path_and_waits_for_completion() {
        let responses = vec![
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Installing".into()),
            )]))),
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]))),
        ];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.install("/PublicStaging/Example.ipa").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Install"));
        assert_eq!(
            dict["PackagePath"].as_string(),
            Some("/PublicStaging/Example.ipa")
        );
        assert_eq!(
            dict["ClientOptions"].as_dictionary(),
            Some(&plist::Dictionary::new())
        );
    }

    #[tokio::test]
    async fn upgrade_sends_upgrade_command() {
        let responses = vec![
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Upgrading".into()),
            )]))),
            plist_frame(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]))),
        ];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.upgrade("/PublicStaging/Example.ipa").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Upgrade"));
    }

    #[tokio::test]
    async fn archive_sends_application_identifier() {
        let responses = vec![plist_frame(plist::Value::Dictionary(
            plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]),
        ))];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.archive("com.example.test").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Archive"));
        assert_eq!(
            dict["ApplicationIdentifier"].as_string(),
            Some("com.example.test")
        );
    }

    #[tokio::test]
    async fn restore_sends_application_identifier() {
        let responses = vec![plist_frame(plist::Value::Dictionary(
            plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Complete".into()),
            )]),
        ))];
        let mut stream = ResponseStream::with_frames(responses);
        {
            let mut proxy = InstallationProxy::new(&mut stream);
            proxy.restore("com.example.test").await.unwrap();
        }

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(dict["Command"].as_string(), Some("Restore"));
        assert_eq!(
            dict["ApplicationIdentifier"].as_string(),
            Some("com.example.test")
        );
    }

    fn plist_frame(value: plist::Value) -> Vec<u8> {
        let mut buf = Vec::new();
        plist::to_writer_xml(&mut buf, &value).unwrap();
        let mut framed = Vec::with_capacity(buf.len() + 4);
        framed.extend_from_slice(&(buf.len() as u32).to_be_bytes());
        framed.extend_from_slice(&buf);
        framed
    }

    struct ResponseStream {
        written: Vec<u8>,
        read_buf: Vec<u8>,
        read_pos: usize,
    }

    impl ResponseStream {
        fn with_frames(frames: Vec<Vec<u8>>) -> Self {
            let read_buf = frames.into_iter().flatten().collect();
            Self {
                written: Vec::new(),
                read_buf,
                read_pos: 0,
            }
        }
    }

    impl AsyncRead for ResponseStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let remaining = self.read_buf.len().saturating_sub(self.read_pos);
            if remaining == 0 {
                return Poll::Ready(Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "no more test data",
                )));
            }

            let to_copy = remaining.min(buf.remaining());
            let start = self.read_pos;
            let end = start + to_copy;
            buf.put_slice(&self.read_buf[start..end]);
            self.read_pos = end;
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for ResponseStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.written.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}

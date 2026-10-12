//! Bounded, optional crash artifacts for one selected XCTest invocation.
//! Reuses the real crash mover handshake, AFC codec and report parser. No
//! device report is deleted, and filenames never establish bundle ownership.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::{Context as _, Result};
use ios_core::{
    afc::AfcClient,
    crashreport::{parse_report_bytes, prepare_reports, prepare_reports_rsd},
    ServiceStream,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::Instant,
};

const MAX_ENTRIES: usize = 4096;
const MAX_DIRECTORIES: usize = 256;
const MAX_DEPTH: usize = 16;
const MAX_METADATA: usize = 1024 * 1024;
const MAX_CANDIDATES: usize = 64;
const MAX_FILE: usize = 4 * 1024 * 1024;
const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_WIRE: usize = 32 * 1024 * 1024;
const FINAL_BUDGET: Duration = Duration::from_secs(15);

pub(crate) fn validate_destination(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).context("reading workflow crash destination")?;
    let real_directory = metadata.is_dir() && !metadata.file_type().is_symlink();
    #[cfg(windows)]
    let real_directory = {
        use std::os::windows::fs::MetadataExt;
        real_directory && metadata.file_attributes() & 0x400 == 0
    };
    anyhow::ensure!(
        real_directory,
        "--crash-output must name an existing directory without a link or reparse point"
    );
    Ok(())
}

pub(crate) struct CrashCapture {
    directory: PathBuf,
    selected_bundles: BTreeSet<String>,
    before: BTreeSet<String>,
    baseline_complete: bool,
    summary: CrashSummary,
}

#[derive(Serialize)]
struct CrashSummary {
    schema_version: u8,
    status: &'static str,
    baseline_entries: usize,
    new_candidates: usize,
    completed_read_bytes: usize,
    unmatched: usize,
    unmatched_reports: Vec<UnmatchedReport>,
    artifacts: Vec<CrashArtifact>,
    error_class: Option<ios_core::ErrorClass>,
}

#[derive(Serialize)]
struct UnmatchedReport {
    remote_id: String,
    reason: &'static str,
    completed_bytes: usize,
    sha256: String,
}

#[derive(Serialize)]
struct CrashArtifact {
    remote_id: String,
    file: String,
    bundle_id: String,
    bytes: usize,
    sha256: String,
}

impl CrashCapture {
    pub(crate) fn new(root: &Path, bundles: impl IntoIterator<Item = String>) -> Self {
        Self {
            directory: root.join(format!("workflow-crash-{}", uuid::Uuid::new_v4())),
            selected_bundles: bundles.into_iter().collect(),
            before: BTreeSet::new(),
            baseline_complete: false,
            summary: CrashSummary {
                schema_version: 1,
                status: "baseline_pending",
                baseline_entries: 0,
                new_candidates: 0,
                completed_read_bytes: 0,
                unmatched: 0,
                unmatched_reports: vec![],
                artifacts: vec![],
                error_class: None,
            },
        }
    }

    pub(crate) async fn baseline(
        &mut self,
        udid: &str,
        deadline: Instant,
        native: &crate::cmd::tunnel::NativeScope,
    ) -> Result<()> {
        validate_destination(self.directory.parent().context("crash output parent")?)?;
        tokio::fs::create_dir(&self.directory)
            .await
            .context("creating this invocation's crash directory")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&self.directory, std::fs::Permissions::from_mode(0o700))
                .await?;
        }
        let result: Result<()> = async {
            let (_device, mut afc) = connect(udid, deadline, native).await?;
            let entries = snapshot(&mut afc, deadline).await?;
            self.before = entries.into_keys().collect();
            self.summary.baseline_entries = self.before.len();
            self.baseline_complete = true;
            self.summary.status = "baseline_complete";
            Ok(())
        }
        .await;
        if let Err(error) = &result {
            self.summary.status = "baseline_failed";
            self.summary.error_class = Some(crate::output::operation_error_class(error.as_ref()));
        }
        result
    }

    /// This owner remains outside cancellable work. A partial baseline cannot
    /// authorize copying every preexisting device report after cancellation.
    pub(crate) async fn finish(
        &mut self,
        udid: &str,
        native: &crate::cmd::tunnel::NativeScope,
    ) -> Result<()> {
        let deadline = Instant::now() + FINAL_BUDGET;
        let result = if self.baseline_complete {
            self.collect(udid, deadline, native).await
        } else {
            self.summary.status = "baseline_incomplete";
            Ok(())
        };
        if let Err(error) = &result {
            self.summary.status = "incomplete";
            self.summary.error_class = Some(crate::output::operation_error_class(error.as_ref()));
        }
        let written = self.write_summary().await;
        super::combine_results(result, written, "workflow crash summary failed")
    }

    async fn collect(
        &mut self,
        udid: &str,
        deadline: Instant,
        native: &crate::cmd::tunnel::NativeScope,
    ) -> Result<()> {
        let (_device, mut afc) = connect(udid, deadline, native).await?;
        let entries = snapshot(&mut afc, deadline).await?;
        let candidates: Vec<_> = entries
            .into_iter()
            .filter(|(path, _)| !self.before.contains(path))
            .collect();
        self.summary.new_candidates = candidates.len();
        if candidates.len() > MAX_CANDIDATES {
            return Err(crate::output::resource_decision(
                "workflow crash candidate limit exceeded",
            ));
        }
        for (path, size) in candidates {
            let remaining = MAX_INPUT.saturating_sub(self.summary.completed_read_bytes);
            if size > MAX_FILE as u64 || size > remaining as u64 {
                return Err(crate::output::resource_decision(
                    "workflow crash input byte limit exceeded",
                ));
            }
            let data = until(
                deadline,
                afc.read_file_with_limit(&path, MAX_FILE.min(remaining)),
            )
            .await?;
            self.summary.completed_read_bytes += data.len();
            let report = parse_report_bytes(&path, &data)?;
            let nested = report
                .raw
                .pointer("/body/bundleInfo/CFBundleIdentifier")
                .map(|value| {
                    value
                        .as_str()
                        .context("crash bundleInfo identifier was not a string")
                })
                .transpose()?;
            if let (Some(parsed), Some(nested)) = (report.bundle_id.as_deref(), nested) {
                anyhow::ensure!(
                    parsed == nested,
                    "crash report contains conflicting bundle identifiers"
                );
            }
            let Some(bundle) = report
                .bundle_id
                .as_deref()
                .or(nested)
                .filter(|bundle| self.selected_bundles.contains(*bundle))
                .map(str::to_owned)
            else {
                self.summary.unmatched += 1;
                self.summary.unmatched_reports.push(UnmatchedReport {
                    remote_id: ios_core::redaction::diagnostic_id(&path).to_string(),
                    reason: if report.bundle_id.as_deref().or(nested).is_some() {
                        "bundle_mismatch"
                    } else {
                        "missing_bundle_identifier"
                    },
                    completed_bytes: data.len(),
                    sha256: hex::encode(Sha256::digest(&data)),
                });
                continue;
            };
            let suffix = if path.ends_with(".ips") {
                "ips"
            } else {
                "crash"
            };
            let file = format!(
                "{}.{}",
                hex::encode(Sha256::digest(path.as_bytes())),
                suffix
            );
            let artifact = CrashArtifact {
                remote_id: ios_core::redaction::diagnostic_id(&path).to_string(),
                file: file.clone(),
                bundle_id: bundle,
                bytes: data.len(),
                sha256: hex::encode(Sha256::digest(&data)),
            };
            clock(deadline).await?;
            validate_destination(&self.directory)?;
            // Await the original atomic writer. A started filesystem commit is
            // not physically cancellable; do not detach it behind timeout.
            crate::cmd::file::write_local_bytes_atomic(&self.directory.join(file), &data, false)
                .await?;
            self.summary.artifacts.push(artifact);
            clock(deadline).await?;
        }
        self.summary.status = "complete";
        Ok(())
    }

    async fn write_summary(&self) -> Result<()> {
        if !self.directory.is_dir() {
            return Ok(());
        }
        validate_destination(&self.directory)?;
        let bytes = serde_json::to_vec(&self.summary)?;
        if bytes.len() > MAX_METADATA {
            return Err(crate::output::resource_decision(
                "workflow crash summary metadata limit exceeded",
            ));
        }
        crate::cmd::file::write_local_bytes_atomic(
            &self.directory.join("summary.json"),
            &bytes,
            true,
        )
        .await
    }
}

async fn connect(
    udid: &str,
    deadline: Instant,
    native: &crate::cmd::tunnel::NativeScope,
) -> Result<(
    crate::cmd::runtest::OwnedTestDevice,
    AfcClient<GuardedStream>,
)> {
    let device = until(
        deadline,
        Box::pin(crate::cmd::runtest::connect_testmanager_device_scoped(
            udid, native,
        )),
    )
    .await?;
    let remote = device.rsd().is_some();
    let mover_name = if remote {
        ios_core::crashreport::RSD_CRASHREPORT_MOVER_SERVICE
    } else {
        ios_core::crashreport::CRASHREPORT_MOVER_SERVICE
    };
    let copy_name = if remote {
        ios_core::crashreport::RSD_CRASHREPORT_COPY_MOBILE_SERVICE
    } else {
        ios_core::crashreport::CRASHREPORT_COPY_MOBILE_SERVICE
    };
    let mover = if remote {
        until(deadline, Box::pin(device.connect_rsd_service(mover_name))).await?
    } else {
        until(deadline, Box::pin(device.connect_service(mover_name))).await?
    };
    let mut mover = GuardedStream::new(mover, deadline);
    if remote {
        until(deadline, prepare_reports_rsd(&mut mover)).await?;
    } else {
        until(deadline, prepare_reports(&mut mover)).await?;
    }
    drop(mover);
    let copy = if remote {
        until(deadline, Box::pin(device.connect_rsd_service(copy_name))).await?
    } else {
        until(deadline, Box::pin(device.connect_service(copy_name))).await?
    };
    Ok((
        device,
        AfcClient::new(GuardedStream::new(copy, deadline)).with_max_frame_body(MAX_METADATA),
    ))
}

async fn snapshot<S: AsyncRead + AsyncWrite + Unpin>(
    afc: &mut AfcClient<S>,
    deadline: Instant,
) -> Result<BTreeMap<String, u64>> {
    let mut pending = VecDeque::from([(".".to_string(), 0usize)]);
    let mut directories = 1usize;
    let mut visited = 0usize;
    let mut metadata_bytes = 0usize;
    let mut reports = BTreeMap::new();
    while let Some((directory, depth)) = pending.pop_front() {
        let names = until(deadline, afc.list_dir(&directory)).await?;
        let mut unique = BTreeSet::new();
        for name in names {
            anyhow::ensure!(
                !name.is_empty()
                    && name != "."
                    && name != ".."
                    && !name
                        .bytes()
                        .any(|byte| byte < 0x20 || byte == 0x7f || byte == b'/' || byte == b'\\'),
                "workflow crash directory contains an unsafe component"
            );
            anyhow::ensure!(
                unique.insert(name.clone()),
                "workflow crash directory contains duplicate entries"
            );
            visited += 1;
            if visited > MAX_ENTRIES {
                return Err(crate::output::resource_decision(
                    "workflow crash snapshot entry limit exceeded",
                ));
            }
            let path = if directory == "." {
                name
            } else {
                format!("{directory}/{name}")
            };
            if path.len() > 4096 {
                return Err(crate::output::resource_decision(
                    "workflow crash path length limit exceeded",
                ));
            }
            let info = until(deadline, afc.stat_info(&path)).await?;
            metadata_bytes = metadata_bytes
                .checked_add(
                    path.len()
                        + info
                            .raw
                            .iter()
                            .map(|(key, value)| key.len() + value.len())
                            .sum::<usize>(),
                )
                .context("crash snapshot metadata overflow")?;
            if metadata_bytes > MAX_METADATA {
                return Err(crate::output::resource_decision(
                    "workflow crash snapshot metadata limit exceeded",
                ));
            }
            match info.file_type.as_deref() {
                Some("S_IFDIR") => {
                    directories += 1;
                    if directories > MAX_DIRECTORIES || depth >= MAX_DEPTH {
                        return Err(crate::output::resource_decision(
                            "workflow crash directory/depth limit exceeded",
                        ));
                    }
                    pending.push_back((path, depth + 1));
                }
                Some("S_IFREG") => {
                    if path.ends_with(".ips") || path.ends_with(".crash") {
                        reports.insert(
                            path,
                            info.size
                                .context("workflow crash report omitted size metadata")?,
                        );
                    }
                }
                _ => {
                    return Err(ios_core::CoreError::Protocol(
                        "workflow crash snapshot refuses links or missing/unknown file type".into(),
                    )
                    .into())
                }
            }
        }
    }
    Ok(reports)
}

async fn clock(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        tokio::time::timeout_at(deadline, std::future::pending::<()>()).await?;
    }
    Ok(())
}

fn until<'a, T: 'a, E: Into<anyhow::Error> + 'a, F>(
    deadline: Instant,
    future: F,
) -> impl Future<Output = Result<T>> + 'a
where
    F: Future<Output = std::result::Result<T, E>> + 'a,
{
    let mut future = Box::pin(future);
    async move {
        let guarded = std::future::poll_fn(|cx| {
            if Instant::now() >= deadline {
                return Poll::Pending;
            }
            match future.as_mut().poll(cx) {
                Poll::Ready(Ok(_)) if Instant::now() >= deadline => Poll::Pending,
                other => other,
            }
        });
        tokio::time::timeout_at(deadline, guarded)
            .await
            .context("workflow crash absolute deadline expired")?
            .map_err(Into::into)
    }
}

struct GuardedStream {
    inner: ServiceStream,
    deadline: Instant,
    wire_read: usize,
}
impl GuardedStream {
    fn new(inner: ServiceStream, deadline: Instant) -> Self {
        Self {
            inner,
            deadline,
            wire_read: 0,
        }
    }
}
impl AsyncRead for GuardedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        if self.wire_read >= MAX_WIRE {
            return Poll::Ready(Err(crate::output::resource_limit_error(
                "workflow crash wire byte limit exceeded",
            )));
        }
        let limit = (MAX_WIRE - self.wire_read).min(buf.remaining());
        let mut admitted = ReadBuf::new(&mut buf.initialize_unfilled()[..limit]);
        let result = Pin::new(&mut self.inner).poll_read(cx, &mut admitted);
        let count = admitted.filled().len();
        self.wire_read += count;
        buf.advance(count);
        result
    }
}
impl AsyncWrite for GuardedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if Instant::now() >= self.deadline {
            Poll::Pending
        } else {
            Pin::new(&mut self.inner).poll_flush(cx)
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

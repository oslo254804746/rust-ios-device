//! Bounded screenshot viewer, RFB transport and text pasteboard adapters.
pub mod clipboard;
mod device;
mod media;
#[cfg(test)]
pub mod tests;
pub mod vnc;
pub mod web;

use crate::access::{
    loopback_address, AccessControl, HttpBoundaryArgs, Permission, Principal, ServerAccessArgs,
};
use anyhow::{anyhow, Context, Result};
use clap::{Args, Subcommand};
use device::{DeviceInput, CLEANUP_TIMEOUT, OPERATION_TIMEOUT};
use std::{path::PathBuf, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{mpsc, watch},
    task::{JoinHandle, JoinSet},
};
use vnc::{VncServerState, VncSession};
use web::{create_viewer_router, DeviceCommand, DeviceRequest, WebViewerState};

#[derive(Args)]
pub struct ViewerCmd {
    #[command(subcommand)]
    pub sub: Option<ViewerSub>,
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub host: String,
    #[arg(short, long)]
    pub port: Option<u16>,
    #[command(flatten)]
    pub access: ServerAccessArgs,
    #[command(flatten)]
    pub boundary: HttpBoundaryArgs,
    /// RFB's legacy eight-byte password; policy mode also requires a scoped owner token.
    #[arg(long)]
    pub password: Option<String>,
    /// Private policy token for the VNC/clipboard owner (Debug and Connect scopes).
    #[arg(long)]
    pub vnc_token_file: Option<PathBuf>,
    /// Enable bounded bidirectional text clipboard sharing.
    #[arg(long)]
    pub share_clipboard: bool,
    /// Experimental Display/RTP HEVC via FFmpeg; requires a kernel tunnel. Default: PNG screenshots.
    #[arg(long)]
    pub hevc: bool,
    /// Recover this many disconnected media sessions after successful cleanup.
    #[arg(long, default_value_t = 3)]
    pub reconnect_attempts: u32,
    /// Initial reconnect delay; exponential backoff is capped at sixteen times this value.
    #[arg(long, default_value_t = 500)]
    pub reconnect_delay_ms: u64,
}
#[derive(Clone, Copy, Subcommand)]
pub enum ViewerSub {
    /// Serve PNG screenshots, flushed HID reports and text clipboard requests.
    Web,
    /// Serve the same decoded screenshots through RFB 3.8.
    Vnc,
    /// Synchronize local and device text clipboards until interrupted.
    Clipboard,
}
impl ViewerCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow!("--udid required for viewer"))?;
        let (_, access) = self.access.load(&self.host).await?;
        match self.sub.unwrap_or(ViewerSub::Web) {
            ViewerSub::Web => {
                run_web_server(
                    udid,
                    &self.host,
                    self.port.unwrap_or(8080),
                    access,
                    self.boundary,
                    self.share_clipboard,
                    self.hevc,
                    self.reconnect_attempts,
                    Duration::from_millis(self.reconnect_delay_ms),
                )
                .await
            }
            ViewerSub::Vnc => {
                run_vnc_server(
                    udid,
                    &self.host,
                    self.port.unwrap_or(5900),
                    access,
                    self.password,
                    self.vnc_token_file,
                    self.share_clipboard,
                    self.hevc,
                    self.reconnect_attempts,
                    Duration::from_millis(self.reconnect_delay_ms),
                )
                .await
            }
            ViewerSub::Clipboard => {
                let principal = owner_principal(
                    &access,
                    self.vnc_token_file.as_deref(),
                    &udid,
                    self.port.unwrap_or(5900),
                )?;
                run_clipboard_sync(udid, principal).await
            }
        }
    }
}
fn owner_principal(
    access: &AccessControl,
    token: Option<&std::path::Path>,
    udid: &str,
    port: u16,
) -> Result<Principal> {
    let mut headers = axum::http::HeaderMap::new();
    if let Some(path) = token {
        let bytes = ios_core::secret_file::read_secret(path, 66)?;
        let token = std::str::from_utf8(&bytes)?.trim_end_matches(['\r', '\n']);
        headers.insert(
            axum::http::header::AUTHORIZATION,
            format!("Bearer {token}").parse()?,
        );
    }
    let principal = access
        .authenticate(&headers)
        .context("viewer credential rejected; policy-mode VNC requires --vnc-token-file")?;
    principal
        .require(Permission::Debug, Some(udid), None)
        .map_err(|_| anyhow!("viewer Debug device scope rejected"))?;
    principal
        .require(Permission::Connect, Some(udid), Some(port))
        .map_err(|_| anyhow!("viewer Connect device/port scope rejected"))?;
    Ok(principal)
}
async fn device_worker(
    udid: String,
    share_clipboard: bool,
    mut requests: mpsc::Receiver<DeviceRequest>,
    mut frames: watch::Receiver<Option<media::DecodedFrame>>,
    mut stop: watch::Receiver<bool>,
    native: crate::cmd::tunnel::NativeScope,
) -> Result<()> {
    let mut device = DeviceInput::new(udid.clone(), share_clipboard, native);
    let operation = async {
        loop {
            let request = tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                request = requests.recv() => match request { Some(request) => request, None => return Ok(()) },
            };
            if request.reply.is_closed() { continue; }
            device.dimensions = frames.borrow_and_update().as_ref().map(|frame|(frame.width,frame.height)).unwrap_or((0,0));
            if !request.principal.allows(Permission::Debug, Some(&udid), None) {
                let _ = request.reply.send(Err(anyhow!("viewer request authorization revoked")));
                continue;
            }
            let work = async {
                match &request.command {
                    DeviceCommand::Input(action) => { device.apply(action).await?; Ok(None) },
                    DeviceCommand::ReadClipboard => device.read_clipboard().await,
                }
            };
            let result = tokio::select! {
                biased;
                _ = stop.changed() => return Ok(()),
                result = request.principal.run(tokio::time::timeout(OPERATION_TIMEOUT, work)) => {
                    result.map_err(|_| anyhow!("viewer request authorization revoked"))
                        .and_then(|value| value.context("viewer device operation deadline"))
                        .and_then(|value| value)
                }
            };
            let failed = result.is_err();
            let _ = request.reply.send(result);
            // A failed transition may have reached the device. Release before admitting another request.
            if failed { tokio::time::timeout(CLEANUP_TIMEOUT, device.close()).await.context("viewer failed transition cleanup deadline")??; }
        }
    }.await;
    let cleanup = tokio::time::timeout(CLEANUP_TIMEOUT, device.close())
        .await
        .context("viewer device cleanup deadline")
        .and_then(|value| value);
    finish(operation, cleanup)
}
async fn join_owned(task: &mut JoinHandle<Result<()>>) -> Result<()> {
    match tokio::time::timeout(CLEANUP_TIMEOUT + OPERATION_TIMEOUT, &mut *task).await {
        Ok(joined) => joined.context("viewer child join failed")?,
        Err(error) => {
            task.abort();
            if let Err(join_error) = task.await {
                if !join_error.is_cancelled() {
                    return Err(join_error.into());
                }
            }
            Err(error.into())
        }
    }
}
#[allow(clippy::too_many_arguments)]
async fn run_web_server(
    udid: String,
    host: &str,
    port: u16,
    access: AccessControl,
    boundary: HttpBoundaryArgs,
    share_clipboard: bool,
    hevc: bool,
    reconnect_attempts: u32,
    reconnect_delay: Duration,
) -> Result<()> {
    let listener = TcpListener::bind((loopback_address(host)?, port)).await?;
    let bound = listener.local_addr()?;
    let state = WebViewerState::new(access, udid.clone(), bound.port(), share_clipboard);
    let router = boundary.validate()?.for_listener(bound).protect(
        create_viewer_router(state.clone()),
        crate::http_diagnostics::ErrorShape::Manager,
    );
    let (stop, stop_rx) = watch::channel(false);
    let native = crate::cmd::tunnel::NativeScope::new()?;
    let (frames, mut frame_rx) = watch::channel(None);
    let (requests, receiver) = mpsc::channel(8);
    *state.backend.write().await = Some(requests);
    let mut device = tokio::spawn(device_worker(
        udid.clone(),
        share_clipboard,
        receiver,
        frame_rx.clone(),
        stop_rx.clone(),
        native.clone(),
    ));
    let mut producer = tokio::spawn(media::produce(
        udid,
        frames,
        stop_rx.clone(),
        native.clone(),
        hevc,
        reconnect_attempts,
        reconnect_delay,
    ));
    let state_frames = state.clone();
    let mut forward_stop = stop_rx.clone();
    let mut forward = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = forward_stop.changed() => return Ok(()),
                changed = frame_rx.changed() => {
                    changed.context("viewer frame producer ended")?;
                    let frame = frame_rx.borrow_and_update().clone();
                    if let Some(frame) = frame { state_frames.push_timed_frame("image/png", frame.png,
                        frame.width as u32, frame.height as u32, frame.timestamp_ms,frame.timestamp_clock).await; }
                }
            }
        }
    });
    crate::output::write_error_line(format_args!("Web viewer listening on http://{bound}/"))?;
    let mut shutdown = stop_rx;
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                while !*shutdown.borrow() {
                    if shutdown.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await
            .map_err(anyhow::Error::from)
    });
    let mut state_stop = state.stop_signal.subscribe();
    let mut server_finished = false;
    let mut producer_finished = false;
    let mut device_finished = false;
    let result = tokio::select! {
        signal = tokio::signal::ctrl_c() => signal.map_err(anyhow::Error::from),
        _ = async { if !*state_stop.borrow() { let _ = state_stop.changed().await; } } => Ok(()),
        result = &mut server => { server_finished = true; result.context("viewer HTTP task join failed").and_then(|value| value) },
        result = &mut producer => { producer_finished = true; result.context("viewer capture task join failed").and_then(|value| value) },
        result = &mut device => { device_finished = true; result.context("viewer device task join failed").and_then(|value| value) },
    };
    stop.send_replace(true);
    state.trigger_stop();
    state.backend.write().await.take();
    let mut cleanup = join_owned(&mut forward).await;
    if !producer_finished {
        cleanup = finish(cleanup, join_owned(&mut producer).await);
    }
    if !device_finished {
        cleanup = finish(cleanup, join_owned(&mut device).await);
    }
    if !server_finished {
        cleanup = finish(cleanup, join_owned(&mut server).await);
    }
    cleanup = finish(
        cleanup,
        native.close(OPERATION_TIMEOUT + CLEANUP_TIMEOUT).await,
    );
    finish(result, cleanup)
}
pub(super) fn finish<T>(primary: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (primary, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(primary), Err(cleanup)) => Err(anyhow::Error::new(ViewerCleanupFailure {
            primary,
            cleanup,
        })),
    }
}
struct ViewerCleanupFailure {
    primary: anyhow::Error,
    cleanup: anyhow::Error,
}
impl std::fmt::Display for ViewerCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("viewer operation and cleanup failed")
    }
}
impl std::error::Error for ViewerCleanupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.primary.as_ref())
    }
}
impl std::fmt::Debug for ViewerCleanupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ViewerCleanupFailure")
            .field(
                "primary_class",
                &ios_core::classify_error(self.primary.as_ref()),
            )
            .field(
                "cleanup_class",
                &ios_core::classify_error(self.cleanup.as_ref()),
            )
            .finish()
    }
}
#[allow(clippy::too_many_arguments)]
async fn run_vnc_server(
    udid: String,
    host: &str,
    port: u16,
    access: AccessControl,
    password: Option<String>,
    token: Option<PathBuf>,
    share_clipboard: bool,
    hevc: bool,
    reconnect_attempts: u32,
    reconnect_delay: Duration,
) -> Result<()> {
    let policy_mode = access.authenticate(&axum::http::HeaderMap::new()).is_none();
    if policy_mode && password.is_none() {
        anyhow::bail!("policy-mode VNC requires --password and --vnc-token-file");
    }
    if let Some(password) = &password {
        anyhow::ensure!(
            !password.is_empty() && password.len() <= 8,
            "RFB password must contain 1..8 bytes; the protocol truncates longer passwords"
        );
    }
    let listener = TcpListener::bind((loopback_address(host)?, port)).await?;
    let bound = listener.local_addr()?;
    let principal = owner_principal(&access, token.as_deref(), &udid, bound.port())?;
    let (stop, stop_rx) = watch::channel(false);
    let native = crate::cmd::tunnel::NativeScope::new()?;
    let (frames, mut frame_rx) = watch::channel(None);
    let (requests, receiver) = mpsc::channel(8);
    let state = VncServerState::new(0, 0, password, principal.clone(), share_clipboard, requests);
    let mut device = tokio::spawn(device_worker(
        udid.clone(),
        share_clipboard,
        receiver,
        frame_rx.clone(),
        stop_rx.clone(),
        native.clone(),
    ));
    let mut producer = tokio::spawn(media::produce(
        udid,
        frames,
        stop_rx,
        native.clone(),
        hevc,
        reconnect_attempts,
        reconnect_delay,
    ));
    let mut sessions = JoinSet::new();
    crate::output::write_error_line(format_args!("VNC server listening on vnc://{bound}"))?;
    let mut producer_finished = false;
    let mut device_finished = false;
    let result = async {
        loop { tokio::select! {
            biased;
            _ = principal.clone().revoked() => return Err(anyhow!("viewer VNC authorization revoked")),
            signal = tokio::signal::ctrl_c() => return signal.map_err(anyhow::Error::from),
            result = &mut producer => { producer_finished = true; return result.context("viewer capture join failed").and_then(|value| value); },
            result = &mut device => { device_finished = true; return result.context("viewer device join failed").and_then(|value| value); },
            changed = frame_rx.changed() => {
                changed.context("viewer capture ended")?;
                let frame = frame_rx.borrow_and_update().clone();
                if let Some(frame) = frame { state.update_decoded_frame(frame).await; }
            }
            result = listener.accept() => {
                let (stream, peer) = result?;
                if !sessions.is_empty() { drop(stream); continue; }
                let state = state.clone();
                sessions.spawn(async move { VncSession::new(stream, peer, state).run().await });
            }
            joined = sessions.join_next(), if !sessions.is_empty() => {
                if let Some(result) = joined { result.context("viewer VNC client join failed")??; }
            }
        } }
    }.await;
    state.stop();
    // Let a disconnected client's release commands run before stopping the device owner.
    let mut cleanup = Ok(());
    while let Some(result) = sessions.join_next().await {
        cleanup = finish(
            cleanup,
            result
                .context("viewer VNC client join failed")
                .and_then(|value| value),
        );
    }
    stop.send_replace(true);
    if !producer_finished {
        cleanup = finish(cleanup, join_owned(&mut producer).await);
    }
    if !device_finished {
        cleanup = finish(cleanup, join_owned(&mut device).await);
    }
    cleanup = finish(
        cleanup,
        native.close(OPERATION_TIMEOUT + CLEANUP_TIMEOUT).await,
    );
    finish(result, cleanup)
}
async fn run_clipboard_sync(udid: String, principal: Principal) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("local clipboard unavailable")?;
    let native = crate::cmd::tunnel::NativeScope::new()?;
    let mut device = DeviceInput::new(udid, true, native.clone());
    let mut sync = clipboard::ClipboardSyncState::new();
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    let mut baseline = true;
    let operation = async {
        loop {
            tokio::select! { biased;
                _ = principal.clone().revoked() => anyhow::bail!("clipboard owner authorization revoked"),
                signal = tokio::signal::ctrl_c() => return signal.map_err(anyhow::Error::from),
                _ = interval.tick() => {},
            }
            let work = async {
                if !baseline {
                    if let Ok(text) = clipboard.get_text() {
                        let payload = clipboard::ClipboardPayload::Text(text);
                        payload.validate_bounds()?;
                        if sync.should_sync(&payload, clipboard::ClipboardSource::Host) {
                            device.apply(&vnc::VncInputAction::ClipboardOut(payload)).await?;
                        }
                    }
                }
                if let Some(payload) = device.read_clipboard().await? {
                    if sync.should_sync(&payload, clipboard::ClipboardSource::Device) {
                        if let clipboard::ClipboardPayload::Text(text) = payload { clipboard.set_text(text)?; }
                    }
                }
                baseline = false;
                Ok::<_, anyhow::Error>(())
            };
            principal.run(tokio::time::timeout(OPERATION_TIMEOUT, work)).await
                .map_err(|_| anyhow!("clipboard authorization revoked"))?.context("clipboard operation deadline")??;
        }
    }.await;
    let released = finish(
        operation,
        tokio::time::timeout(CLEANUP_TIMEOUT, device.close())
            .await
            .context("clipboard cleanup deadline")
            .and_then(|value| value),
    );
    finish(
        released,
        native.close(OPERATION_TIMEOUT + CLEANUP_TIMEOUT).await,
    )
}

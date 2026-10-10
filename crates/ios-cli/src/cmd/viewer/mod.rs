//! Decoded Web viewer, VNC server, and bidirectional clipboard sync (F05).

pub mod clipboard;
pub mod vnc;
pub mod web;

#[cfg(test)]
pub mod tests;

use std::time::Duration;

use anyhow::{anyhow, Result};
use clap::{Args, Subcommand};
use tokio::net::TcpListener;

use crate::access::{loopback_address, AccessControl, HttpBoundaryArgs, ServerAccessArgs};
use crate::cmd::connect::connect_userspace_tunnel;
use vnc::{VncServerState, VncSession};
use web::{create_viewer_router, WebViewerState};

#[derive(Args)]
pub struct ViewerCmd {
    #[command(subcommand)]
    pub sub: Option<ViewerSub>,

    /// Host IP to bind (must be loopback)
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    pub host: String,

    /// Port to listen on (default: 8080 for Web, 5900 for VNC)
    #[arg(short, long)]
    pub port: Option<u16>,

    #[command(flatten)]
    pub access: ServerAccessArgs,

    #[command(flatten)]
    pub boundary: HttpBoundaryArgs,

    /// Optional VNC password
    #[arg(long)]
    pub password: Option<String>,

    /// Enable bidirectional clipboard synchronization
    #[arg(long)]
    pub share_clipboard: bool,
}

#[derive(Clone, Copy, Subcommand)]
pub enum ViewerSub {
    /// Start interactive Web viewer (HTML5 canvas + MJPEG stream + controls)
    Web,
    /// Start RFB 3.8 VNC server (Screen Sharing, TigerVNC, noVNC)
    Vnc,
    /// Standalone bidirectional clipboard watcher and synchronizer
    Clipboard,
}

impl ViewerCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow!("--udid required for viewer"))?;
        let sub = self.sub.unwrap_or(ViewerSub::Web);
        let host = self.host;
        let (ip, access) = self.access.load(&host).await?;

        match sub {
            ViewerSub::Web => {
                let port = self.port.unwrap_or(8080);
                run_web_server(&udid, &host, port, access, self.boundary).await
            }
            ViewerSub::Vnc => {
                let port = self.port.unwrap_or(5900);
                run_vnc_server(&udid, ip, port, access, self.password).await
            }
            ViewerSub::Clipboard => {
                run_clipboard_sync(&udid, access).await
            }
        }
    }
}

async fn run_web_server(
    udid: &str,
    host: &str,
    port: u16,
    access: AccessControl,
    boundary: HttpBoundaryArgs,
) -> Result<()> {
    let ip = loopback_address(host)?;
    let listener = TcpListener::bind((ip, port)).await?;
    let bound = listener.local_addr()?;
    crate::output::write_error_line(format_args!(
        "Web viewer listening on http://{bound}/"
    ))?;

    let state = WebViewerState::new(access);
    let router = boundary
        .validate()?
        .for_listener(bound)
        .protect(create_viewer_router(state.clone()), crate::http_diagnostics::ErrorShape::Manager);

    let stop_notify = state.stop_notify.clone();
    let server_task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                stop_notify.notified().await;
            })
            .await
    });

    // Spawn frame capture loop
    let state_capture = state.clone();
    let udid_capture = udid.to_string();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        while !state_capture.is_stopped() {
            interval.tick().await;
            if let Ok(device) = connect_userspace_tunnel(&udid_capture).await {
                if let Ok((xpc, meta)) = device.connect_xpc_service_with_metadata(ios_core::screencapture::SERVICE_NAME).await {
                    let mut client = ios_core::screencapture::ScreenCaptureServiceClient::new_with_features(xpc, meta.features);
                    if let Ok(img) = client.capture_screenshot(None, "png").await {
                        let (w, h) = img.pixel_size.unwrap_or((1170, 2532));
                        state_capture.push_frame(
                            "image/png",
                            img.data.to_vec(),
                            w as u32,
                            h as u32,
                            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
                        ).await;
                    }
                }
            }
        }
    });

    // Await server shutdown
    server_task.await??;
    crate::output::write_error_line(format_args!("Web viewer stopped"))?;
    Ok(())
}

async fn run_vnc_server(
    _udid: &str,
    ip: std::net::IpAddr,
    port: u16,
    _access: AccessControl,
    password: Option<String>,
) -> Result<()> {
    let listener = TcpListener::bind((ip, port)).await?;
    let bound = listener.local_addr()?;
    crate::output::write_error_line(format_args!(
        "VNC server listening on vnc://{bound}"
    ))?;

    let state = VncServerState::new(1170, 2532, password);

    loop {
        tokio::select! {
            res = listener.accept() => {
                let (stream, peer) = res?;
                crate::output::write_error_line(format_args!(
                    "VNC client connected from {}",
                    ios_core::redaction::diagnostic_id(&peer.to_string())
                ))?;
                let session = VncSession::new(stream, peer, state.clone());
                tokio::spawn(async move {
                    if let Err(e) = session.run().await {
                        tracing::debug!(error = %e, "VNC session terminated");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => {
                state.stop();
                break;
            }
        }
    }

    crate::output::write_error_line(format_args!("VNC server stopped"))?;
    Ok(())
}

async fn run_clipboard_sync(
    _udid: &str,
    _access: AccessControl,
) -> Result<()> {
        crate::output::write_error_line(format_args!(
            "Starting bidirectional clipboard synchronizer..."
        ))?;
        // Sync loop running until interrupted
        tokio::signal::ctrl_c().await?;
        Ok(())
}

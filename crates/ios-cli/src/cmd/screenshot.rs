use std::path::Path;

use anyhow::Result;
use bytes::Bytes;
use ios_core::screenshot::{ScreenshotFormat, ScreenshotImage};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};

use crate::cmd::connect::{connect_userspace_tunnel, probe_product_version};

#[derive(clap::Args)]
pub struct ScreenshotCmd {
    #[arg(short, long, default_value = "screenshot.png")]
    pub output: String,
    #[arg(long, help = "Overwrite the output file if it already exists")]
    pub force: bool,
    #[arg(short = 'j', long, help = "Output JSON metadata")]
    pub json: bool,
    #[arg(
        long,
        help = "Serve a multipart screenshot stream instead of saving one file"
    )]
    pub stream: bool,
    #[arg(
        long,
        default_value_t = 3333,
        help = "Port for screenshot streaming mode"
    )]
    pub port: u16,
    #[arg(
        short = 'H',
        long,
        default_value = "127.0.0.1",
        help = "Address to bind the screenshot stream to; the stream is unauthenticated, \
                so only widen this on a network you trust"
    )]
    pub host: String,
}

impl ScreenshotCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for screenshot"))?;

        if self.stream {
            return self.run_stream_server(&udid).await;
        }

        // Checked before the capture so a doomed shot costs nothing; stream mode
        // writes no file and is deliberately not guarded.
        crate::cmd::file::ensure_local_overwrite_allowed(Path::new(&self.output), self.force)?;
        let (image, transport) = capture_screenshot(&udid).await?;
        crate::cmd::file::write_local_bytes_atomic(
            Path::new(&self.output),
            &image.data,
            self.force,
        )
        .await?;
        print_screenshot_result(&self.output, &image, transport, self.json)?;
        Ok(())
    }

    async fn run_stream_server(&self, udid: &str) -> Result<()> {
        // Loopback by default: the stream is a live, unauthenticated view of the
        // device screen, and `forward` / `tunnel serve` bind loopback too.
        let bind_addr = format!("{}:{}", self.host, self.port);
        let listener = TcpListener::bind(&bind_addr).await?;
        eprintln!("Serving screenshot stream on http://{bind_addr}/");

        loop {
            let (socket, peer) = listener.accept().await?;
            eprintln!("Screenshot stream client connected: {peer}");
            let udid = udid.to_string();
            tokio::spawn(async move {
                if let Err(err) = serve_stream_client(socket, &udid).await {
                    eprintln!("screenshot stream client error: {err}");
                }
            });
        }
    }
}

async fn capture_screenshot(udid: &str) -> Result<(ScreenshotImage, &'static str)> {
    let version = probe_product_version(udid).await?;
    if version.major >= 17 {
        match connect_userspace_tunnel(udid).await {
            Ok(device) => match try_coredevice_screenshot(&device).await {
                Ok(image) => return Ok((ScreenshotImage::from_bytes(image.data), "coredevice")),
                Err(error) if coredevice_fallback_allowed(&error) => {
                    // Screenshot is read-only, so falling back after a modern
                    // service error cannot repeat a mutating operation. Keep the
                    // failure as context in debug logs and try legacy DTX/lockdown.
                    tracing::debug!("CoreDevice screenshot failed, falling back: {error}");
                }
                Err(error) => return Err(error),
            },
            Err(error) if coredevice_fallback_allowed(&error) => {
                tracing::debug!("CoreDevice tunnel unavailable, falling back: {error}");
            }
            Err(error) => return Err(error),
        }
    }
    match try_dtx_screenshot(udid).await {
        Ok(data) => Ok((ScreenshotImage::from_bytes(data), "dtx")),
        Err(e) if dtx_fallback_allowed(&e) => {
            tracing::debug!("DTX screenshot failed, falling back to legacy: {e}");
            let data = take_legacy_screenshot(udid).await?;
            Ok((data, "legacy"))
        }
        Err(e) => Err(e),
    }
}

/// Fallback is only valid when the preferred service is unavailable. Protocol,
/// permission, and malformed-response errors must reach the caller rather
/// than being hidden by a second transport attempt.
fn coredevice_fallback_allowed(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<ios_core::error::CoreError>()
            .is_some_and(|error| {
                matches!(
                    error,
                    ios_core::error::CoreError::Unsupported(message)
                        if message == "RSD not available (no tunnel or iOS <17)"
                            || message == "CoreDevice tunnel support requires ios-core feature 'tunnel'"
                            || message.contains(
                                "service 'com.apple.coredevice.screencaptureservice' not found",
                            )
                )
            })
            || cause
                .downcast_ref::<ios_core::error::CoreError>()
                .is_some_and(|error| matches!(error, ios_core::error::CoreError::Io(_)))
            || cause
                .downcast_ref::<ios_core::screencapture::ScreenCaptureError>()
                .is_some_and(|error| error.is_service_unavailable())
    })
}

fn dtx_fallback_allowed(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<ios_core::error::CoreError>()
            .is_some_and(|error| {
                matches!(
                    error,
                    ios_core::error::CoreError::Unsupported(message)
                        if message == "RSD not available (no tunnel or iOS <17)"
                            || message == "CoreDevice tunnel support requires ios-core feature 'tunnel'"
                            || message.contains(
                                "service 'com.apple.instruments.dtservicehub' not found",
                            )
                            || message.contains(
                                "service 'com.apple.instruments.server.services.deviceinfo' not found",
                            )
                )
            })
            || cause
                .downcast_ref::<ios_core::dtx::DtxError>()
                .is_some_and(|error| matches!(error, ios_core::dtx::DtxError::Io(_)))
    })
}

async fn try_coredevice_screenshot(
    device: &ios_core::ConnectedDevice,
) -> Result<ios_core::screencapture::ScreenCaptureImage> {
    let (xpc, metadata) = device
        .connect_xpc_service_with_metadata(ios_core::screencapture::SERVICE_NAME)
        .await
        .map_err(|error| anyhow::Error::new(error).context("CoreDevice screen capture service"))?;
    let mut service = ios_core::screencapture::ScreenCaptureServiceClient::new_with_features(
        xpc,
        metadata.features,
    );
    service
        .capture_screenshot(None, "png")
        .await
        .map_err(|error| anyhow::Error::new(error).context("CoreDevice screenshot"))
}

async fn try_dtx_screenshot(udid: &str) -> Result<Bytes> {
    use crate::cmd::instruments::connect_instruments;

    let (_device, stream) = connect_instruments(udid).await?;
    let data = ios_core::instruments::screenshot::take_screenshot_dtx(stream)
        .await
        .map_err(|e| anyhow::Error::new(e).context("DTX screenshot"))?;
    Ok(data)
}

async fn take_legacy_screenshot(udid: &str) -> Result<ScreenshotImage> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_core::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(udid, opts).await?;
    let mut stream = device
        .connect_service(ios_core::screenshot::SERVICE_NAME)
        .await?;

    eprintln!("Capturing screenshot (legacy screenshotr)...");
    Ok(ios_core::screenshot::take_screenshot(&mut stream).await?)
}

async fn serve_stream_client(mut socket: TcpStream, udid: &str) -> Result<()> {
    socket
        .write_all(stream_response_header().as_bytes())
        .await?;

    loop {
        let (frame, _) = capture_screenshot(udid).await?;
        let multipart = encode_multipart_frame(&frame);
        socket.write_all(&multipart).await?;
        socket.flush().await?;
        sleep(Duration::from_millis(750)).await;
    }
}

fn stream_response_header() -> &'static str {
    "HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
}

fn encode_multipart_frame(frame: &ScreenshotImage) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(frame.byte_len() + 128);
    chunk.extend_from_slice(b"--frame\r\n");
    chunk.extend_from_slice(format!("Content-Type: {}\r\n", frame.mime_type()).as_bytes());
    chunk.extend_from_slice(format!("Content-Length: {}\r\n\r\n", frame.byte_len()).as_bytes());
    chunk.extend_from_slice(&frame.data);
    chunk.extend_from_slice(b"\r\n");
    chunk
}

fn print_screenshot_result(
    output: &str,
    image: &ScreenshotImage,
    transport: &str,
    json: bool,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "output": output,
                "bytes": image.byte_len(),
                "sha256": sha256_hex(&image.data),
                "transport": transport,
                "format": image.format,
                "mime": image.mime_type(),
            }))?
        );
    } else {
        eprintln!(
            "Saved {} bytes -> {output} (via {transport}, {}, {})",
            image.byte_len(),
            format_label(image.format),
            image.mime_type()
        );
    }
    Ok(())
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn format_label(format: ScreenshotFormat) -> &'static str {
    match format {
        ScreenshotFormat::Png => "png",
        ScreenshotFormat::Jpeg => "jpeg",
        ScreenshotFormat::Tiff => "tiff",
        ScreenshotFormat::Heif => "heif",
        ScreenshotFormat::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: ScreenshotCmd,
    }

    #[test]
    fn parses_screenshot_json_flag() {
        let cmd = TestCli::parse_from(["screenshot", "--output", "shot.png", "--json"]);
        assert_eq!(cmd.command.output, "shot.png");
        assert!(cmd.command.json);
        assert!(!cmd.command.stream);
        assert!(!cmd.command.force);
        assert_eq!(cmd.command.port, 3333);
    }

    #[test]
    fn parses_screenshot_force_flag() {
        let cmd = TestCli::parse_from(["screenshot", "--output", "shot.png", "--force"]);
        assert!(cmd.command.force);
    }

    #[test]
    fn parses_screenshot_stream_flags() {
        let cmd = TestCli::parse_from(["screenshot", "--stream", "--port", "4444"]);
        assert!(cmd.command.stream);
        assert_eq!(cmd.command.port, 4444);
    }

    #[test]
    fn screenshot_result_json_contains_transport_and_size() {
        let rendered = serde_json::to_string_pretty(&serde_json::json!({
            "output": "shot.png",
            "bytes": 1234,
            "transport": "dtx",
            "format": "png",
            "mime": "image/png",
        }))
        .unwrap();
        assert!(rendered.contains("\"output\": \"shot.png\""));
        assert!(rendered.contains("\"bytes\": 1234"));
        assert!(rendered.contains("\"transport\": \"dtx\""));
        assert!(rendered.contains("\"format\": \"png\""));
        assert!(rendered.contains("\"mime\": \"image/png\""));
    }

    #[test]
    fn multipart_frame_uses_detected_content_type() {
        let frame = ScreenshotImage::from_bytes(Bytes::from_static(&[0xFF, 0xD8, 0xFF, 0xE0]));
        let frame = encode_multipart_frame(&frame);
        let rendered = String::from_utf8_lossy(&frame);
        assert!(rendered.starts_with("--frame\r\n"));
        assert!(rendered.contains("Content-Type: image/jpeg\r\n"));
        assert!(rendered.contains("Content-Length: 4\r\n\r\n"));
        assert!(frame.ends_with(b"\r\n"));
    }

    #[test]
    fn fallback_only_accepts_known_unavailable_endpoints() {
        let missing_capture = anyhow::Error::new(ios_core::error::CoreError::Unsupported(
            "service 'com.apple.coredevice.screencaptureservice' not found in RSD directory".into(),
        ));
        assert!(coredevice_fallback_allowed(&missing_capture));

        let rsd_missing = anyhow::Error::new(ios_core::error::CoreError::Unsupported(
            "RSD not available (no tunnel or iOS <17)".into(),
        ));
        assert!(coredevice_fallback_allowed(&rsd_missing));
        assert!(dtx_fallback_allowed(&rsd_missing));

        let handshake_failed = anyhow::Error::new(ios_core::error::CoreError::Unsupported(
            "RSD handshake failed after retries".into(),
        ));
        assert!(!coredevice_fallback_allowed(&handshake_failed));
        assert!(!dtx_fallback_allowed(&handshake_failed));

        let protocol = anyhow::Error::new(ios_core::screencapture::ScreenCaptureError::Protocol(
            "permission denied".into(),
        ));
        assert!(!coredevice_fallback_allowed(&protocol));

        let missing_dtx = anyhow::Error::new(ios_core::error::CoreError::Unsupported(
            "service 'com.apple.instruments.dtservicehub' not found in RSD directory".into(),
        ));
        assert!(dtx_fallback_allowed(&missing_dtx));

        let dtx_protocol = anyhow::Error::new(ios_core::dtx::DtxError::Protocol(
            "malformed response".into(),
        ));
        assert!(!dtx_fallback_allowed(&dtx_protocol));

        let dtx_io = anyhow::Error::new(ios_core::dtx::DtxError::Io(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "service disconnected",
        )));
        assert!(dtx_fallback_allowed(&dtx_io));
    }
}

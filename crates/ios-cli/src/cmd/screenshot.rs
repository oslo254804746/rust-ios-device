use anyhow::Result;
use bytes::Bytes;
use ios_services::screenshot::{ScreenshotFormat, ScreenshotImage};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};

#[derive(clap::Args)]
pub struct ScreenshotCmd {
    #[arg(short, long, default_value = "screenshot.png")]
    pub output: String,
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
}

impl ScreenshotCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for screenshot"))?;

        if self.stream {
            return self.run_stream_server(&udid).await;
        }

        let (image, transport) = capture_screenshot(&udid).await?;
        tokio::fs::write(&self.output, &image.data).await?;
        print_screenshot_result(&self.output, &image, transport, self.json)?;
        Ok(())
    }

    async fn run_stream_server(&self, udid: &str) -> Result<()> {
        let bind_addr = format!("0.0.0.0:{}", self.port);
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
    match try_dtx_screenshot(udid).await {
        Ok(data) => Ok((ScreenshotImage::from_bytes(data), "dtx")),
        Err(e) => {
            tracing::debug!("DTX screenshot failed, falling back to legacy: {e}");
            let data = take_legacy_screenshot(udid).await?;
            Ok((data, "legacy"))
        }
    }
}

async fn try_dtx_screenshot(udid: &str) -> Result<Bytes> {
    use crate::cmd::instruments::connect_instruments;

    let (_device, stream) = connect_instruments(udid).await?;
    let data = ios_services::instruments::screenshot::take_screenshot_dtx(stream)
        .await
        .map_err(|e| anyhow::anyhow!("DTX screenshot: {e}"))?;
    Ok(data)
}

async fn take_legacy_screenshot(udid: &str) -> Result<ScreenshotImage> {
    let opts = ios_core::device::ConnectOptions {
        tun_mode: ios_tunnel::TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    let device = ios_core::connect(udid, opts).await?;
    let mut stream = device
        .connect_service(ios_services::screenshot::SERVICE_NAME)
        .await?;

    eprintln!("Capturing screenshot (legacy screenshotr)...");
    Ok(ios_services::screenshot::take_screenshot(&mut stream).await?)
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

fn format_label(format: ScreenshotFormat) -> &'static str {
    match format {
        ScreenshotFormat::Png => "png",
        ScreenshotFormat::Jpeg => "jpeg",
        ScreenshotFormat::Tiff => "tiff",
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
        assert_eq!(cmd.command.port, 3333);
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
}

//! Capture a screenshot from an iOS device and save it to a file.
//!
//! Usage: cargo run --example screenshot -- <UDID> [output.png]

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let udid = std::env::args()
        .nth(1)
        .expect("Usage: screenshot <UDID> [output.png]");
    let output = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "screenshot.png".to_string());

    let opts = ios_core::ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    };
    let device = ios_core::connect(&udid, opts).await?;

    let mut stream = device
        .connect_service(ios_core::services::screenshot::SERVICE_NAME)
        .await?;

    let image = ios_core::services::screenshot::take_screenshot(&mut stream).await?;
    println!(
        "Captured screenshot: {} bytes, format: {}",
        image.byte_len(),
        image.mime_type()
    );

    std::fs::write(&output, &image.data)?;
    println!("Saved to {}", output);

    Ok(())
}

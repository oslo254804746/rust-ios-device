//! Discover connected iOS devices and print basic device information.
//!
//! Usage: cargo run --example device_info
//! Or with a specific UDID: cargo run --example device_info -- <UDID>

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // List all connected devices via usbmuxd
    let devices = ios_core::list_devices().await?;
    if devices.is_empty() {
        println!("No iOS devices connected.");
        return Ok(());
    }

    for dev in &devices {
        println!("Found device: {} ({})", dev.udid, dev.connection_type);
    }

    // Connect to the first device (or a specific UDID from args)
    let udid = std::env::args()
        .nth(1)
        .unwrap_or_else(|| devices[0].udid.clone());

    let opts = ios_core::ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    };
    let device = ios_core::connect(&udid, opts).await?;

    // Read device properties via lockdown
    let version = device.product_version().await?;
    println!("\nDevice: {}", udid);
    println!("  iOS Version: {}", version);

    if let Ok(val) = device.lockdown_get_value(Some("DeviceName")).await {
        println!("  Device Name: {}", val.as_string().unwrap_or("?"));
    }
    if let Ok(val) = device.lockdown_get_value(Some("ProductType")).await {
        println!("  Product Type: {}", val.as_string().unwrap_or("?"));
    }
    if let Ok(val) = device.lockdown_get_value(Some("SerialNumber")).await {
        println!("  Serial Number: {}", val.as_string().unwrap_or("?"));
    }
    if let Ok(val) = device.lockdown_get_value(Some("WiFiAddress")).await {
        println!("  WiFi MAC: {}", val.as_string().unwrap_or("?"));
    }

    Ok(())
}

//! Transfer files to/from an iOS device using Apple File Conduit (AFC).
//!
//! Usage: cargo run --example file_transfer -- <UDID>

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let udid = std::env::args()
        .nth(1)
        .expect("Usage: file_transfer <UDID>");

    let opts = ios_core::ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    };
    let device = ios_core::connect(&udid, opts).await?;

    let stream = device.connect_service("com.apple.afc").await?;
    let mut afc = ios_services::afc::AfcClient::new(stream);

    // List root directory
    let entries = afc.list_dir("/").await?;
    println!("Root directory ({} entries):", entries.len());
    for entry in &entries {
        println!("  {}", entry);
    }

    // Get device filesystem info
    let info = afc.device_info().await?;
    if let (Some(total), Some(free)) = (info.get("FSTotalBytes"), info.get("FSFreeBytes")) {
        let total_gb: f64 = total.parse::<f64>().unwrap_or(0.0) / 1_073_741_824.0;
        let free_gb: f64 = free.parse::<f64>().unwrap_or(0.0) / 1_073_741_824.0;
        println!(
            "\nStorage: {:.1} GB free / {:.1} GB total",
            free_gb, total_gb
        );
    }

    // Upload a test file
    let test_data = b"Hello from rust-ios-device!";
    afc.write_file("/test_upload.txt", test_data).await?;
    println!("\nUploaded /test_upload.txt ({} bytes)", test_data.len());

    // Read it back
    let downloaded = afc.read_file("/test_upload.txt").await?;
    println!("Downloaded: {:?}", std::str::from_utf8(&downloaded)?);

    // Clean up
    afc.remove("/test_upload.txt").await?;
    println!("Cleaned up /test_upload.txt");

    Ok(())
}

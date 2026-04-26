//! Monitor CPU usage on an iOS device using Instruments sysmontap.
//!
//! This example requires iOS 17+ with a tunnel connection.
//! Usage: cargo run --example instruments_cpu -- <UDID>
//! Press Ctrl+C to stop.

use ios_core::services::instruments::{SysmontapConfig, SysmontapService};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let udid = std::env::args()
        .nth(1)
        .expect("Usage: instruments_cpu <UDID>");

    // Instruments requires a tunnel on iOS 17+
    let opts = ios_core::ConnectOptions::default();
    let device = ios_core::connect(&udid, opts).await?;

    // Connect to the instruments service hub via RSD
    let stream = device
        .connect_rsd_service(ios_core::services::instruments::SERVICE_IOS17)
        .await?;

    // Start sysmontap with default config
    let config = SysmontapConfig::default();
    let mut sysmon = SysmontapService::start(stream, &config, None, None).await?;

    println!("Monitoring CPU on {}... (Ctrl+C to stop)\n", udid);
    println!("{:<6} {:<12} {:<10}", "CPUs", "Enabled", "Total Load");
    println!("{}", "-".repeat(30));

    for _ in 0..20 {
        if let Some(sample) = sysmon.next_cpu_sample().await? {
            println!(
                "{:<6} {:<12} {:.1}%",
                sample.cpu_count,
                sample.enabled_cpus,
                sample.cpu_total_load * 100.0
            );
        }
    }

    sysmon.stop().await?;
    println!("\nMonitoring stopped.");

    Ok(())
}

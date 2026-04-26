//! Stream real-time system logs from an iOS device.
//!
//! Usage: cargo run --example syslog_stream -- <UDID>
//! Press Ctrl+C to stop.

use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let udid = std::env::args()
        .nth(1)
        .expect("Usage: syslog_stream <UDID>");

    let opts = ios_core::ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    };
    let device = ios_core::connect(&udid, opts).await?;

    let stream = device.connect_service("com.apple.syslog_relay").await?;

    println!("Streaming syslog from {}... (Ctrl+C to stop)", udid);

    let log_stream = ios_core::syslog::into_stream(stream);
    tokio::pin!(log_stream);
    let mut count = 0u64;
    while let Some(result) = log_stream.next().await {
        match result {
            Ok(line) => {
                let entry = ios_core::syslog::LogEntry::parse(line);
                if let Some(msg) = &entry.message {
                    println!(
                        "[{}] {}: {}",
                        entry.timestamp.as_deref().unwrap_or("?"),
                        entry.process.as_deref().unwrap_or("?"),
                        msg
                    );
                }
                count += 1;
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }

    println!("Received {} log entries", count);
    Ok(())
}

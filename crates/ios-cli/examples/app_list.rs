//! List installed applications on an iOS device.
//!
//! Usage: cargo run --example app_list -- <UDID>

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let udid = std::env::args().nth(1).expect("Usage: app_list <UDID>");

    let opts = ios_core::ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    };
    let device = ios_core::connect(&udid, opts).await?;

    // Connect to the installation proxy service
    let stream = device
        .connect_service("com.apple.mobile.installation_proxy")
        .await?;
    let mut proxy = ios_core::apps::InstallationProxy::new(stream);

    // List user-installed apps
    let apps = proxy.list_user_apps().await?;
    println!("Installed apps ({}):", apps.len());
    for app in &apps {
        println!(
            "  {} ({}) v{}",
            app.display_name, app.bundle_id, app.version
        );
    }

    Ok(())
}

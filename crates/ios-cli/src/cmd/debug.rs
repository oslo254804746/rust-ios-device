use std::path::PathBuf;

use anyhow::{Context, Result};
use ios_core::device::ConnectOptions;
use ios_core::tunnel::TunMode;
use semver::Version;

const DEBUGPROXY_SERVICE: &str = "com.apple.internal.dt.remote.debugproxy";

#[derive(clap::Args)]
pub struct DebugCmd {
    #[arg(help = "Installed bundle identifier to debug")]
    pub bundle_id: String,
    #[arg(long, help = "Local .app path for LLDB target create")]
    pub local_app: Option<PathBuf>,
}

impl DebugCmd {
    pub async fn run(self, udid: Option<String>) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for debug"))?;
        eprintln!("UNTESTED: LLDB workflow generation is implemented from go-ios/pymobiledevice3 references, but no debugserver-capable device is available in this workspace.");

        let probe = ios_core::connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await
        .context("failed to probe device version for debugserver")?;
        let product_version = probe.product_version().await?;
        ensure_rsd_debugproxy_supported(&product_version)?;
        drop(probe);

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        };
        let device = ios_core::connect(&udid, opts)
            .await
            .context("failed to establish device tunnel for debugserver")?;

        let app = crate::cmd::runtest::lookup_installed_app(&device, &self.bundle_id).await?;
        let host = device
            .server_address()
            .ok_or_else(|| anyhow::anyhow!("missing tunnel server address"))?;
        let port = device
            .rsd()
            .and_then(|rsd| rsd.get_port(DEBUGPROXY_SERVICE))
            .ok_or_else(|| {
                anyhow::anyhow!("service '{DEBUGPROXY_SERVICE}' not found in RSD directory")
            })?;
        let local_target = self
            .local_app
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "/path/to/local/application.app".to_string());

        println!("platform select remote-ios");
        println!("target create \"{local_target}\"");
        println!(
            "script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec(\"{}\"))",
            app.path
        );
        println!("process connect connect://[{host}]:{port}");
        println!("process launch");

        Ok(())
    }
}

fn ensure_rsd_debugproxy_supported(product_version: &Version) -> Result<()> {
    if product_version.major < 17 {
        anyhow::bail!("debug currently supports only the iOS 17+ RSD/debugproxy path");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use semver::Version;

    use super::{ensure_rsd_debugproxy_supported, DebugCmd};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        command: DebugCmd,
    }

    #[test]
    fn parses_debug_command() {
        let cmd = TestCli::parse_from([
            "debug",
            "com.example.App",
            "--local-app",
            "C:/build/Demo.app",
        ]);
        assert_eq!(cmd.command.bundle_id, "com.example.App");
        assert_eq!(
            cmd.command.local_app,
            Some(std::path::PathBuf::from("C:/build/Demo.app"))
        );
    }

    #[test]
    fn rejects_pre_ios_17_debugproxy_path() {
        let err = ensure_rsd_debugproxy_supported(&Version::new(15, 5, 0)).unwrap_err();
        assert_eq!(
            err.to_string(),
            "debug currently supports only the iOS 17+ RSD/debugproxy path"
        );
    }

    #[test]
    fn accepts_ios_17_debugproxy_path() {
        ensure_rsd_debugproxy_supported(&Version::new(17, 0, 0)).unwrap();
    }
}

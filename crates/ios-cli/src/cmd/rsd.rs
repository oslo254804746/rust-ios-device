use anyhow::Result;
use ios_core::tunnel::TunMode;
use ios_core::{connect, ConnectOptions};

const COREDEVICE_PREFIX: &str = "com.apple.coredevice.";

#[derive(clap::Args)]
pub struct RsdCmd {
    #[command(subcommand)]
    sub: RsdSub,
}

#[derive(clap::Subcommand)]
enum RsdSub {
    /// List services exposed by the current RSD directory
    Services {
        #[arg(
            long,
            help = "Include every RSD service, not just com.apple.coredevice.*"
        )]
        all: bool,
        #[arg(long, help = "Filter services by prefix")]
        prefix: Option<String>,
    },
    /// Check whether a specific RSD service is currently exposed
    Check {
        #[arg(help = "Service name (e.g. com.apple.coredevice.deviceinfo)")]
        service: String,
    },
}

impl RsdCmd {
    pub async fn run(self, udid: Option<String>, json: bool) -> Result<()> {
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for rsd"))?;

        let opts = ConnectOptions {
            tun_mode: TunMode::Userspace,
            pair_record_path: None,
            skip_tunnel: false,
        };
        let device = connect(&udid, opts).await?;
        let rsd = device
            .into_rsd()
            .ok_or_else(|| anyhow::anyhow!("RSD not available on this device/session"))?;

        match self.sub {
            RsdSub::Services { all, prefix } => {
                let services = filtered_services(&rsd, all, prefix.as_deref());
                if json {
                    let list: Vec<_> = services
                        .iter()
                        .map(|(name, port)| serde_json::json!({ "name": name, "port": port }))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else {
                    for (name, port) in services {
                        println!("{:<55} {}", name, port);
                    }
                }
            }
            RsdSub::Check { service } => {
                let result = service_check(&rsd, &service);
                if json {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else if result.available {
                    if let Some(resolved) = result.resolved_name {
                        println!("available: true");
                        println!("service: {resolved}");
                        println!("port: {}", result.port.unwrap_or_default());
                    } else {
                        println!("available: true");
                    }
                } else {
                    println!("available: false");
                    println!("service: {service}");
                }
            }
        }

        Ok(())
    }
}

#[derive(serde::Serialize)]
struct ServiceCheckResult {
    requested_name: String,
    available: bool,
    resolved_name: Option<String>,
    port: Option<u16>,
}

fn service_check(rsd: &ios_core::xpc::rsd::RsdHandshake, service: &str) -> ServiceCheckResult {
    if let Some(descriptor) = rsd.services.get(service) {
        return ServiceCheckResult {
            requested_name: service.to_string(),
            available: true,
            resolved_name: Some(service.to_string()),
            port: Some(descriptor.port),
        };
    }

    let shim = format!("{service}.shim.remote");
    let shim_match = rsd.services.get(&shim);
    ServiceCheckResult {
        requested_name: service.to_string(),
        available: shim_match.is_some(),
        resolved_name: shim_match.map(|_| shim),
        port: shim_match.map(|descriptor| descriptor.port),
    }
}

fn filtered_services(
    rsd: &ios_core::xpc::rsd::RsdHandshake,
    all: bool,
    prefix: Option<&str>,
) -> Vec<(String, u16)> {
    let effective_prefix = prefix.or_else(|| (!all).then_some(COREDEVICE_PREFIX));
    let mut services: Vec<_> = rsd
        .services
        .iter()
        .filter(|(name, _)| effective_prefix.map_or(true, |prefix| name.starts_with(prefix)))
        .map(|(name, svc)| (name.clone(), svc.port))
        .collect();
    services.sort_by(|a, b| a.0.cmp(&b.0));
    services
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use clap::Parser;
    use ios_core::xpc::rsd::{RsdHandshake, ServiceDescriptor};

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: RsdSub,
    }

    #[test]
    fn parses_rsd_services_subcommand() {
        let cmd = TestCli::parse_from(["rsd", "services", "--all"]);
        match cmd.command {
            RsdSub::Services { all, prefix } => {
                assert!(all);
                assert_eq!(prefix, None);
            }
            _ => panic!("expected services subcommand"),
        }
    }

    #[test]
    fn filtered_services_defaults_to_coredevice_prefix() {
        let rsd = RsdHandshake {
            udid: "test".into(),
            services: HashMap::from([
                (
                    "com.apple.coredevice.appservice".into(),
                    ServiceDescriptor { port: 1234 },
                ),
                (
                    "com.apple.instruments.dtservicehub".into(),
                    ServiceDescriptor { port: 5678 },
                ),
            ]),
        };

        let services = filtered_services(&rsd, false, None);
        assert_eq!(
            services,
            vec![("com.apple.coredevice.appservice".into(), 1234)]
        );
    }

    #[test]
    fn filtered_services_honors_all_and_custom_prefix() {
        let rsd = RsdHandshake {
            udid: "test".into(),
            services: HashMap::from([
                (
                    "com.apple.coredevice.appservice".into(),
                    ServiceDescriptor { port: 1234 },
                ),
                (
                    "com.apple.instruments.dtservicehub".into(),
                    ServiceDescriptor { port: 5678 },
                ),
            ]),
        };

        let all_services = filtered_services(&rsd, true, None);
        assert_eq!(all_services.len(), 2);

        let instruments = filtered_services(&rsd, true, Some("com.apple.instruments."));
        assert_eq!(
            instruments,
            vec![("com.apple.instruments.dtservicehub".into(), 5678)]
        );
    }

    #[test]
    fn service_check_resolves_exact_and_shim_services() {
        let rsd = RsdHandshake {
            udid: "test".into(),
            services: HashMap::from([
                (
                    "com.apple.afc.shim.remote".into(),
                    ServiceDescriptor { port: 1234 },
                ),
                (
                    "com.apple.instruments.dtservicehub".into(),
                    ServiceDescriptor { port: 5678 },
                ),
            ]),
        };

        let exact = service_check(&rsd, "com.apple.instruments.dtservicehub");
        assert!(exact.available);
        assert_eq!(
            exact.resolved_name.as_deref(),
            Some("com.apple.instruments.dtservicehub")
        );
        assert_eq!(exact.port, Some(5678));

        let shim = service_check(&rsd, "com.apple.afc");
        assert!(shim.available);
        assert_eq!(
            shim.resolved_name.as_deref(),
            Some("com.apple.afc.shim.remote")
        );
        assert_eq!(shim.port, Some(1234));

        let missing = service_check(&rsd, "com.apple.coredevice.deviceinfo");
        assert!(!missing.available);
        assert_eq!(missing.resolved_name, None);
        assert_eq!(missing.port, None);
    }
}

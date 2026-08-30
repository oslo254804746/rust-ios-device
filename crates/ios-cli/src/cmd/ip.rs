use std::time::Duration;

use anyhow::Result;
use ios_core::pcap::{FindIpLimits, NetworkInfo, PcapClient, SERVICE_NAME};
use ios_core::{connect, ConnectOptions, TunMode};

#[derive(clap::Args)]
pub struct IpCmd {
    #[arg(long, default_value_t = 10, help = "Maximum search time in seconds")]
    timeout: u64,
    #[arg(long, default_value_t = 512, help = "Maximum pcap packets to inspect")]
    max_packets: usize,
    #[arg(
        long,
        default_value_t = 16 * 1024 * 1024,
        help = "Maximum captured bytes to inspect"
    )]
    max_bytes: usize,
    #[arg(short = 'j', long, help = "Output JSON")]
    json: bool,
}

impl IpCmd {
    pub async fn run(self, udid: Option<String>, global_json: bool) -> Result<()> {
        let json = self.json || global_json;
        let udid = udid.ok_or_else(|| anyhow::anyhow!("--udid required for ip"))?;
        let device = connect(
            &udid,
            ConnectOptions {
                tun_mode: TunMode::Userspace,
                pair_record_path: None,
                skip_tunnel: true,
            },
        )
        .await?;
        let mac = device
            .lockdown_get_value(Some("WiFiAddress"))
            .await?
            .as_string()
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("lockdown WiFiAddress was not a string"))?;
        let stream = device.connect_service(SERVICE_NAME).await?;
        let mut client = PcapClient::new(stream);
        let info = client
            .find_ip(
                &mac,
                FindIpLimits {
                    timeout: Duration::from_secs(self.timeout),
                    max_packets: self.max_packets,
                    max_bytes: self.max_bytes,
                },
            )
            .await?;
        print_network_info(&info, json)?;
        Ok(())
    }
}

fn print_network_info(info: &NetworkInfo, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(info)?);
    } else {
        println!("MAC:   {}", info.mac);
        println!("IPv4:  {}", info.ipv4);
        println!("IPv6:  {}", info.ipv6);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_info_json_is_structured() {
        let info = NetworkInfo {
            mac: "aa:bb:cc:dd:ee:ff".into(),
            ipv4: "192.0.2.7".into(),
            ipv6: "2001:db8::7".into(),
        };
        let value: serde_json::Value = serde_json::to_value(info).unwrap();
        assert_eq!(value["ipv4"], "192.0.2.7");
        assert_eq!(value["ipv6"], "2001:db8::7");
    }
}

#[cfg(feature = "mdns")]
fn load_wifi_mac_pairings() -> Result<HashMap<String, String>, CoreError> {
    let mut wifi_mac_to_udid = HashMap::new();
    let pair_record_dir = default_pair_record_dir();

    for entry in std::fs::read_dir(pair_record_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("plist") {
            continue;
        }

        let Some(udid) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if udid.starts_with("remote_") {
            continue;
        }

        let record = PairRecord::load_from_path(&path, udid)?;
        let Some(mac) = record.wifi_mac_address else {
            continue;
        };
        wifi_mac_to_udid.insert(mac.to_ascii_lowercase(), udid.to_string());
    }

    Ok(wifi_mac_to_udid)
}

#[cfg(feature = "mdns")]
fn match_paired_mobdev2_targets(
    services: &[BonjourService],
    wifi_mac_to_udid: &HashMap<String, String>,
) -> Vec<PairedMobdev2Device> {
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::<(String, String)>::new();

    for service in services {
        let Some(mac) = mobdev2_wifi_mac(&service.instance) else {
            continue;
        };
        let Some(udid) = wifi_mac_to_udid.get(&mac.to_ascii_lowercase()) else {
            continue;
        };
        let Some(host) = preferred_lockdown_address(&service.addresses) else {
            continue;
        };

        let key = (udid.clone(), host.to_string());
        if seen.insert(key.clone()) {
            targets.push(PairedMobdev2Device {
                udid: key.0,
                host: key.1,
            });
        }
    }

    targets
}

#[cfg(feature = "mdns")]
fn preferred_lockdown_address(addresses: &[String]) -> Option<&str> {
    addresses
        .iter()
        .find(|address| address.parse::<std::net::Ipv4Addr>().is_ok())
        .map(String::as_str)
        .or_else(|| {
            addresses
                .iter()
                .find(|address| {
                    !address.contains('%') && !address.to_ascii_lowercase().starts_with("fe80:")
                })
                .map(String::as_str)
        })
        .or_else(|| addresses.first().map(String::as_str))
}

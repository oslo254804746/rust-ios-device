#[cfg(feature = "mdns")]
#[derive(Clone)]
struct LockdownDiscoveryRecord {
    udid: String,
    record: Arc<PairRecord>,
}

#[cfg(feature = "mdns")]
fn load_wifi_pairings() -> Vec<LockdownDiscoveryRecord> {
    load_wifi_pairings_from_dir(&default_pair_record_dir())
}

#[cfg(feature = "mdns")]
fn load_wifi_pairings_from_dir(dir: &std::path::Path) -> Vec<LockdownDiscoveryRecord> {
    crate::lockdown::pair_record::discovery_pair_record_paths(dir)
        .into_iter()
        .filter_map(|path| {
            let udid = path.file_stem()?.to_str()?;
            if udid.starts_with("remote_") || udid == "SystemConfiguration" {
                return None;
            }
            // One broken record must not hide every other paired device. Do not
            // log its filename or parser input (both may contain secrets).
            let record = PairRecord::load_from_path(&path, udid).ok()?;
            if record.host_id.is_empty() {
                return None;
            }
            Some(LockdownDiscoveryRecord {
                udid: udid.to_string(),
                record: Arc::new(record),
            })
        })
        .collect()
}

/// A MAC lookup is an optimization only. With private Wi-Fi addresses the
/// HostID tag selects possible host records, then lockdown identifies the
/// actual device. A shared HostID is never interpreted as a unique UDID.
#[cfg(feature = "mdns")]
fn mobdev2_pair_record_candidates<'a>(
    service: &BonjourService,
    records: &'a [LockdownDiscoveryRecord],
) -> Vec<&'a LockdownDiscoveryRecord> {
    let Some(mac) = mobdev2_wifi_mac(&service.instance) else {
        return Vec::new();
    };
    let named: Vec<_> = records
        .iter()
        .filter(|candidate| {
            candidate
                .record
                .wifi_mac_address
                .as_deref()
                .is_some_and(|address| address.eq_ignore_ascii_case(mac))
        })
        .collect();
    let candidates: Vec<_> = if named.is_empty() {
        records.iter().collect()
    } else {
        named
    };
    candidates
        .into_iter()
        .filter(|candidate| {
            crate::discovery::auth::mobdev2_host_matches(
                &service.properties,
                &candidate.record.host_id,
            )
        })
        .take(16)
        .collect()
}

#[cfg(feature = "mdns")]
fn match_paired_mobdev2_targets(
    services: &[BonjourService],
    records: &[LockdownDiscoveryRecord],
) -> Vec<PairedMobdev2Device> {
    let mut targets = Vec::new();
    let mut seen = std::collections::HashSet::<(String, String)>::new();
    for service in services {
        let candidates = mobdev2_pair_record_candidates(service, records);
        let [candidate] = candidates.as_slice() else {
            continue;
        };
        let Some(mac) = mobdev2_wifi_mac(&service.instance) else {
            continue;
        };
        // A host authTag alone needs the bounded session probe below.
        if !candidate
            .record
            .wifi_mac_address
            .as_deref()
            .is_some_and(|address| address.eq_ignore_ascii_case(mac))
        {
            continue;
        }
        let Some(host) = preferred_lockdown_address(&service.addresses) else {
            continue;
        };
        if seen.insert((candidate.udid.clone(), host.to_string())) {
            targets.push(PairedMobdev2Device {
                udid: candidate.udid.clone(),
                host: host.to_string(),
            });
        }
    }
    targets
}

#[cfg(feature = "mdns")]
async fn resolve_private_mobdev2_targets(
    services: &[BonjourService],
    records: &[LockdownDiscoveryRecord],
    targets: &mut Vec<PairedMobdev2Device>,
) {
    resolve_private_mobdev2_targets_using(
        services,
        records,
        targets,
        |host, candidate| async move { probe_lockdown_pair_record(&host, &candidate).await },
    )
    .await;
}

#[cfg(feature = "mdns")]
async fn resolve_private_mobdev2_targets_using<F, Fut>(
    services: &[BonjourService],
    records: &[LockdownDiscoveryRecord],
    targets: &mut Vec<PairedMobdev2Device>,
    mut probe: F,
) where
    F: FnMut(String, LockdownDiscoveryRecord) -> Fut,
    Fut: std::future::Future<Output = Result<bool, CoreError>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    let mut attempts = 0;
    let mut probed = std::collections::HashSet::new();
    for service in services.iter().take(64) {
        let candidates = mobdev2_pair_record_candidates(service, records);
        if candidates.is_empty() {
            continue;
        }
        for host in ordered_lockdown_addresses(&service.addresses)
            .into_iter()
            .take(4)
        {
            if targets.iter().any(|target| target.host == host) {
                continue;
            }
            let mut found = false;
            for candidate in &candidates {
                // Different ads can share an address while advertising different
                // host tags. Deduplicate an actual credential attempt rather
                // than suppressing every later candidate for that address.
                if !probed.insert((host, candidate.udid.as_str())) {
                    continue;
                }
                if attempts >= 32 || tokio::time::Instant::now() >= deadline {
                    return;
                }
                attempts += 1;
                let attempt_deadline =
                    (tokio::time::Instant::now() + Duration::from_secs(3)).min(deadline);
                if matches!(
                    tokio::time::timeout_at(
                        attempt_deadline,
                        probe(host.to_string(), (*candidate).clone())
                    )
                    .await,
                    Ok(Ok(true))
                ) {
                    if !targets.iter().any(|target| target.udid == candidate.udid) {
                        targets.push(PairedMobdev2Device {
                            udid: candidate.udid.clone(),
                            host: host.to_string(),
                        });
                    }
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
    }
}

#[cfg(feature = "mdns")]
async fn probe_lockdown_pair_record(
    host: &str,
    candidate: &LockdownDiscoveryRecord,
) -> Result<bool, CoreError> {
    use crate::lockdown::protocol::{GetValueRequest, GetValueResponse, StopSessionRequest};
    let stream = TcpStream::connect((host, LOCKDOWN_PORT)).await?;
    let (session_id, mut reader, mut writer) =
        start_lockdown_session(stream, &candidate.record).await?;
    send_lockdown(
        &mut writer,
        &GetValueRequest {
            label: "ios-rs",
            request: "GetValue",
            domain: None,
            key: Some("UniqueDeviceID"),
        },
    )
    .await?;
    let response: GetValueResponse = recv_lockdown(&mut reader).await?;
    let matches = response.value.as_string() == Some(candidate.udid.as_str());
    // Dropping both halves also closes the connection on rejection, malformed
    // responses, timeout or cancellation.
    let _ = send_lockdown(
        &mut writer,
        &StopSessionRequest {
            label: "ios-rs",
            request: "StopSession",
            session_id,
        },
    )
    .await;
    // A device may close the connection before replying to StopSession. The
    // authenticated GetValue response is already sufficient; dropping the
    // stream still guarantees cleanup even if the acknowledgement is missing.
    Ok(matches)
}

#[cfg(feature = "mdns")]
fn ordered_lockdown_addresses(addresses: &[String]) -> Vec<&str> {
    let mut addresses: Vec<_> = addresses.iter().map(String::as_str).collect();
    addresses.sort_by_key(|address| {
        if address.parse::<std::net::Ipv4Addr>().is_ok() {
            0
        } else if !address.contains('%') && !address.to_ascii_lowercase().starts_with("fe80:") {
            1
        } else {
            2
        }
    });
    let mut seen = std::collections::HashSet::new();
    addresses.retain(|address| seen.insert(*address));
    addresses
}

#[cfg(feature = "mdns")]
fn preferred_lockdown_address(addresses: &[String]) -> Option<&str> {
    ordered_lockdown_addresses(addresses).into_iter().next()
}

#[cfg(all(test, feature = "mdns"))]
mod wifi_matching_tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::collections::HashMap;

    pub(super) fn record(udid: &str, host: &str, mac: Option<&str>) -> LockdownDiscoveryRecord {
        LockdownDiscoveryRecord {
            udid: udid.into(),
            record: Arc::new(PairRecord {
                device_certificate: Vec::new(),
                host_certificate: Vec::new(),
                host_private_key: zeroize::Zeroizing::new(Vec::new()),
                root_certificate: Vec::new(),
                host_id: host.into(),
                system_buid: "buid".into(),
                wifi_mac_address: mac.map(str::to_string),
            }),
        }
    }

    fn advert(mac: &str, host: Option<&str>) -> BonjourService {
        let mut properties = HashMap::new();
        if let Some(host) = host {
            properties.insert("identifier".into(), "opaque".into());
            properties.insert(
                "authTag#2".into(),
                STANDARD.encode(crate::discovery::auth::mobdev2_auth_tag(host, "opaque")),
            );
        }
        BonjourService {
            instance: format!("{mac}@host._apple-mobdev2._tcp.local."),
            port: 62078,
            addresses: vec!["192.0.2.1".into()],
            properties,
        }
    }

    #[test]
    fn private_mac_uses_host_tag_candidates_without_assigning_a_udid() {
        let records = vec![
            record("one", "shared-host", Some("aa:bb:cc:dd:ee:ff")),
            record("two", "shared-host", None),
            record("stranger", "other-host", None),
        ];
        let service = advert("ca:00:00:00:00:01", Some("shared-host"));
        let candidates = mobdev2_pair_record_candidates(&service, &records);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.udid.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert!(match_paired_mobdev2_targets(&[service], &records).is_empty());
    }

    #[test]
    fn known_mac_is_narrowed_and_duplicate_mac_is_ambiguous() {
        let mut records = vec![
            record("one", "shared-host", Some("aa:bb:cc:dd:ee:ff")),
            record("two", "shared-host", None),
        ];
        let service = advert("AA:BB:CC:DD:EE:FF", Some("shared-host"));
        assert_eq!(mobdev2_pair_record_candidates(&service, &records).len(), 1);
        assert_eq!(
            match_paired_mobdev2_targets(std::slice::from_ref(&service), &records).len(),
            1
        );
        records.push(record(
            "duplicate",
            "shared-host",
            Some("aa:bb:cc:dd:ee:ff"),
        ));
        assert!(match_paired_mobdev2_targets(&[service], &records).is_empty());
    }

    #[test]
    fn unknown_host_is_filtered_and_legacy_candidates_are_bounded() {
        let records: Vec<_> = (0..40)
            .map(|index| record(&format!("device-{index}"), "ours", None))
            .collect();
        assert!(mobdev2_pair_record_candidates(
            &advert("ca:00:00:00:00:01", Some("theirs")),
            &records
        )
        .is_empty());
        assert_eq!(
            mobdev2_pair_record_candidates(&advert("ca:00:00:00:00:01", None), &records).len(),
            16
        );
    }

    #[tokio::test]
    async fn private_mac_resolution_requires_accepted_record_and_skips_unknown_hosts() {
        let records = vec![
            record("first", "ours", None),
            record("second", "ours", None),
        ];
        let mut services = vec![
            advert("ca:00:00:00:00:01", Some("theirs")),
            advert("ca:00:00:00:00:02", Some("ours")),
        ];
        services[0].addresses = vec!["192.0.2.9".into()];
        services[1].addresses.push("192.0.2.2".into());
        let attempted = std::sync::Mutex::new(Vec::new());
        let mut targets = Vec::new();
        resolve_private_mobdev2_targets_using(
            &services,
            &records,
            &mut targets,
            |host, candidate| {
                attempted
                    .lock()
                    .unwrap()
                    .push((host, candidate.udid.clone()));
                std::future::ready(Ok(candidate.udid == "second"))
            },
        )
        .await;
        assert_eq!(
            *attempted.lock().unwrap(),
            vec![
                ("192.0.2.1".to_string(), "first".to_string()),
                ("192.0.2.1".to_string(), "second".to_string())
            ]
        );
        assert_eq!(
            targets,
            vec![PairedMobdev2Device {
                udid: "second".into(),
                host: "192.0.2.1".into()
            }]
        );
    }

    #[tokio::test]
    async fn candidate_probe_errors_are_isolated_and_total_attempts_are_bounded() {
        let records: Vec<_> = (0..40)
            .map(|index| record(&format!("device-{index}"), "ours", None))
            .collect();
        let services: Vec<_> = (0..8)
            .map(|index| {
                let mut service = advert("ca:00:00:00:00:01", None);
                service.addresses = vec![format!("192.0.2.{}", index + 1)];
                service
            })
            .collect();
        let mut attempts = 0;
        let mut targets = Vec::new();
        resolve_private_mobdev2_targets_using(&services, &records, &mut targets, |_, _| {
            attempts += 1;
            std::future::ready(Err(CoreError::Protocol("rejected candidate".into())))
        })
        .await;
        assert_eq!(attempts, 32);
        assert!(targets.is_empty());
    }

    #[tokio::test]
    async fn shared_address_allows_a_new_candidate_but_never_retries_the_same_record() {
        let records = vec![
            record("first", "host-one", None),
            record("second", "host-two", None),
        ];
        let services = vec![
            advert("ca:00:00:00:00:01", Some("host-one")),
            advert("ca:00:00:00:00:02", Some("host-two")),
            advert("ca:00:00:00:00:03", Some("host-two")),
        ];
        let mut attempted = Vec::new();
        let mut targets = Vec::new();
        resolve_private_mobdev2_targets_using(
            &services,
            &records,
            &mut targets,
            |host, candidate| {
                attempted.push((host, candidate.udid.clone()));
                std::future::ready(Ok(candidate.udid == "second"))
            },
        )
        .await;
        assert_eq!(
            attempted,
            vec![
                ("192.0.2.1".into(), "first".into()),
                ("192.0.2.1".into(), "second".into())
            ]
        );
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].udid, "second");
    }

    #[test]
    fn broken_and_remote_records_do_not_poison_lockdown_enumeration() {
        let dir = std::env::temp_dir().join(format!(
            "ios-discovery-records-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.plist"), b"not plist").unwrap();
        let plist = br#"<?xml version="1.0"?><plist version="1.0"><dict>
            <key>HostID</key><string>host</string><key>SystemBUID</key><string>buid</string>
            <key>DeviceCertificate</key><data></data><key>HostCertificate</key><data></data>
            <key>HostPrivateKey</key><data></data><key>RootCertificate</key><data></data>
            </dict></plist>"#;
        std::fs::write(dir.join("valid.plist"), plist).unwrap();
        std::fs::write(dir.join("remote_ignore.plist"), plist).unwrap();
        let records = load_wifi_pairings_from_dir(&dir);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].udid, "valid");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn remote_pairing_offline_matching_rejects_unmatched_and_ambiguous_keys() {
        let mut key = [0; 16];
        STANDARD
            .decode_slice("Mgp6ZGPzXM2ku9br46vsiw==", &mut key)
            .unwrap();
        let keys = vec![("device".into(), zeroize::Zeroizing::new(key))];
        let service = BonjourService {
            instance: "opaque._remotepairing._tcp.local.".into(),
            port: 49152,
            addresses: vec!["192.0.2.1".into(), "192.0.2.2".into()],
            properties: HashMap::from([
                (
                    "identifier".into(),
                    "2BE6E510-0325-4365-923E-B14C6F57DB3A".into(),
                ),
                ("authTag".into(), "kXjlTr2l".into()),
            ]),
        };
        let targets =
            match_remote_pairing_targets(std::slice::from_ref(&service), &keys, "", None).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].0, "device");
        assert!(
            match_remote_pairing_targets(std::slice::from_ref(&service), &keys, "other", None)
                .unwrap()
                .is_empty()
        );
        assert!(match_remote_pairing_targets(
            std::slice::from_ref(&service),
            &keys,
            "device",
            Some("192.0.2.3")
        )
        .unwrap()
        .is_empty());
        let mut duplicate = keys;
        duplicate.push(("other".into(), zeroize::Zeroizing::new(key)));
        assert!(match_remote_pairing_targets(
            std::slice::from_ref(&service),
            &duplicate,
            "device",
            None
        )
        .is_err());
        let mut missing = service;
        missing.properties.clear();
        assert!(
            match_remote_pairing_targets(&[missing], &duplicate, "", None)
                .unwrap()
                .is_empty()
        );
    }
}

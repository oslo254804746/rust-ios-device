//! Receipts contain only mechanism and current-browse indices. No raw private
//! MAC, host tag, IRK, HostID, endpoint or credential path is exported.
use super::*;

#[derive(Clone, Debug, serde::Serialize)]
pub struct PairedIdentityReceipt {
    pub mechanism: &'static str,
    pub advertisement_type: &'static str,
    pub advertisement_index: usize,
    pub selected_udid_match: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_mac_matches: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unique_device_id_verified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_record_validated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advertisement_auth_matched: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_validation_scope: Option<&'static str>,
}

pub async fn observe_paired_identity(
    udid: &str,
    budget: Duration,
) -> Result<Vec<PairedIdentityReceipt>, CoreError> {
    if udid.is_empty() || udid.len() > 4096 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "identity observer requires exact selected target",
        )
        .into());
    }
    let deadline = tokio::time::Instant::now()
        .checked_add(budget)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "identity observation deadline overflow",
            )
        })?;
    tokio::time::timeout_at(deadline, async {
        due(deadline, budget).await?;
        // These existing-record reads create no detached worker. Their
        // physical filesystem cancellation remains a separate platform limit.
        let records = load_wifi_pairings().into_iter().filter(|record| record.udid == udid).collect::<Vec<_>>();
        due(deadline, budget).await?;
        let mut receipts = Vec::new();
        if !records.is_empty() {
            let services = browse_mobdev2(MOBDEV2_DISCOVERY_TIMEOUT.min(deadline.saturating_duration_since(tokio::time::Instant::now()))).await?;
            let mut targets = Vec::new();
            resolve_private_mobdev2_targets_observed(&services, &records, &mut targets,
                |host, candidate| async move { probe_lockdown_pair_record(&host, &candidate).await },
                |index, service, candidate, _| {
                    if let Some(receipt) = private_match_receipt(index, service, candidate) { receipts.push(receipt); }
                }).await;
        }
        due(deadline, budget).await?;
        let indices = super::super::tunnel_connect::observe_alt_irk_matches(udid, deadline.saturating_duration_since(tokio::time::Instant::now())).await?;
        due(deadline, budget).await?;
        receipts.extend(indices.into_iter().map(alt_irk_receipt));
        Ok(receipts)
    }).await.map_err(|source| crate::error::context::deadline_io("paired identity observation", budget, source))?
}

async fn due(deadline: tokio::time::Instant, budget: Duration) -> Result<(), CoreError> {
    if tokio::time::Instant::now() < deadline {
        return Ok(());
    }
    tokio::time::timeout_at(deadline, std::future::pending::<()>())
        .await
        .map_err(|source| {
            crate::error::context::deadline_io("paired identity observation", budget, source)
        })?;
    Ok(())
}
fn private_match_receipt(
    index: usize,
    service: &BonjourService,
    candidate: &LockdownDiscoveryRecord,
) -> Option<PairedIdentityReceipt> {
    let (Some(advertised), Some(saved)) = (
        mobdev2_wifi_mac(&service.instance),
        candidate.record.wifi_mac_address.as_deref(),
    ) else {
        return None;
    };
    if advertised.eq_ignore_ascii_case(saved) {
        return None;
    }
    Some(PairedIdentityReceipt {
        mechanism: "private-mac-lockdown",
        advertisement_type: "mobdev2",
        advertisement_index: index,
        selected_udid_match: true,
        saved_mac_matches: Some(false),
        record_accepted: Some(true),
        unique_device_id_verified: Some(true),
        credential_record_validated: None,
        advertisement_auth_matched: None,
        credential_validation_scope: None,
    })
}

fn alt_irk_receipt(index: usize) -> PairedIdentityReceipt {
    PairedIdentityReceipt {
        mechanism: "remote-pairing-altirk",
        advertisement_type: "remotepairing",
        advertisement_index: index,
        selected_udid_match: true,
        saved_mac_matches: None,
        record_accepted: None,
        unique_device_id_verified: None,
        credential_record_validated: Some(true),
        advertisement_auth_matched: Some(true),
        credential_validation_scope: Some("structure-and-key-material"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::wifi_matching_tests::record;
    use super::*;
    #[tokio::test]
    async fn private_identity_receipt_requires_accepted_query_and_changed_present_saved_mac() {
        let records = vec![record(
            "raw-private-target-marker",
            "host",
            Some("aa:bb:cc:dd:ee:ff"),
        )];
        let services = vec![BonjourService {
            instance: "ca:00:00:00:00:01@host._apple-mobdev2._tcp.local.".into(),
            port: 62078,
            addresses: vec!["192.0.2.77".into()],
            properties: std::collections::HashMap::new(),
        }];
        for accepted in [false, true] {
            let mut targets = Vec::new();
            let mut receipts = Vec::new();
            resolve_private_mobdev2_targets_observed(
                &services,
                &records,
                &mut targets,
                |_, _| std::future::ready(Ok(accepted)),
                |index, service, candidate, _| {
                    if let Some(receipt) = private_match_receipt(index, service, candidate) {
                        receipts.push(receipt);
                    }
                },
            )
            .await;
            assert_eq!(receipts.len(), usize::from(accepted));
            if accepted {
                let json = serde_json::to_string(&receipts).unwrap();
                assert!(json.contains("unique_device_id_verified"));
                for private in ["aa:bb", "ca:00", "192.0.2", "raw-private-target-marker"] {
                    assert!(!json.contains(private));
                }
            }
        }
        let known = record("selected", "host", Some("ca:00:00:00:00:01"));
        assert!(private_match_receipt(0, &services[0], &known).is_none());
        let missing = record("selected", "host", None);
        assert!(private_match_receipt(0, &services[0], &missing).is_none());
    }
    #[test]
    fn alternate_irk_receipt_never_claims_online_record_acceptance_or_exports_identity() {
        let json = serde_json::to_value(alt_irk_receipt(3)).unwrap();
        assert_eq!(json["advertisement_index"], 3);
        assert_eq!(json["advertisement_auth_matched"], true);
        assert!(json.get("record_accepted").is_none());
        assert!(json.get("unique_device_id_verified").is_none());
    }

    #[tokio::test]
    async fn private_identity_real_tcp_tls_query_generates_receipt_only_for_matching_unique_id() {
        use crate::lockdown::{recv_lockdown, send_lockdown, test_assets};
        use tokio::io::AsyncReadExt;
        for matching in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let _ = rustls::crypto::ring::default_provider().install_default();
            let chain =
                rustls_pemfile::certs(&mut std::io::Cursor::new(test_assets::TEST_CERT_PEM))
                    .collect::<std::io::Result<Vec<_>>>()
                    .unwrap();
            let key =
                rustls_pemfile::private_key(&mut std::io::Cursor::new(test_assets::TEST_KEY_PEM))
                    .unwrap()
                    .unwrap();
            let mut config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap();
            config.send_tls13_tickets = 0;
            let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
            let records = vec![LockdownDiscoveryRecord {
                udid: "private-current-target".into(),
                record: Arc::new(PairRecord {
                    device_certificate: Vec::new(),
                    host_certificate: test_assets::TEST_CERT_PEM.to_vec(),
                    host_private_key: test_assets::TEST_KEY_PEM.to_vec().into(),
                    root_certificate: Vec::new(),
                    host_id: "accepted-existing-host".into(),
                    system_buid: "existing-buid".into(),
                    wifi_mac_address: Some("aa:bb:cc:dd:ee:ff".into()),
                }),
            }];
            let services = vec![BonjourService {
                instance: "ca:00:00:00:00:01@opaque._apple-mobdev2._tcp.local.".into(),
                port,
                addresses: vec!["127.0.0.1".into()],
                properties: std::collections::HashMap::new(),
            }];
            let operation = async {
                let mut targets = Vec::new();
                let mut receipts = Vec::new();
                resolve_private_mobdev2_targets_observed(
                    &services,
                    &records,
                    &mut targets,
                    |host, candidate| async move {
                        probe_lockdown_pair_record_at(&host, port, &candidate).await
                    },
                    |index, service, candidate, _| {
                        if let Some(receipt) = private_match_receipt(index, service, candidate) {
                            receipts.push(receipt);
                        }
                    },
                )
                .await;
                (targets, receipts)
            };
            let peer = async {
                let (mut tcp, _) = listener.accept().await.unwrap();
                let query: plist::Dictionary = recv_lockdown(&mut tcp).await.unwrap();
                assert_eq!(query["Request"].as_string(), Some("QueryType"));
                send_lockdown(
                    &mut tcp,
                    &plist::Dictionary::from_iter([
                        (
                            "Request".to_string(),
                            plist::Value::String("QueryType".into()),
                        ),
                        (
                            "Type".to_string(),
                            plist::Value::String("com.apple.mobile.lockdown".into()),
                        ),
                    ]),
                )
                .await
                .unwrap();
                let session: plist::Dictionary = recv_lockdown(&mut tcp).await.unwrap();
                assert_eq!(session["Request"].as_string(), Some("StartSession"));
                assert_eq!(
                    session["HostID"].as_string(),
                    Some("accepted-existing-host")
                );
                send_lockdown(
                    &mut tcp,
                    &plist::Dictionary::from_iter([
                        (
                            "Request".to_string(),
                            plist::Value::String("StartSession".into()),
                        ),
                        (
                            "SessionID".to_string(),
                            plist::Value::String("existing-session".into()),
                        ),
                        ("EnableSessionSSL".to_string(), plist::Value::Boolean(true)),
                    ]),
                )
                .await
                .unwrap();
                let mut tls = acceptor.accept(tcp).await.unwrap();
                let value: plist::Dictionary = recv_lockdown(&mut tls).await.unwrap();
                assert_eq!(value["Request"].as_string(), Some("GetValue"));
                assert_eq!(value["Key"].as_string(), Some("UniqueDeviceID"));
                send_lockdown(
                    &mut tls,
                    &plist::Dictionary::from_iter([
                        (
                            "Request".to_string(),
                            plist::Value::String("GetValue".into()),
                        ),
                        (
                            "Value".to_string(),
                            plist::Value::String(
                                if matching {
                                    "private-current-target"
                                } else {
                                    "another-device"
                                }
                                .into(),
                            ),
                        ),
                    ]),
                )
                .await
                .unwrap();
                let stop: plist::Dictionary = recv_lockdown(&mut tls).await.unwrap();
                assert_eq!(stop["Request"].as_string(), Some("StopSession"));
                let (mut raw, _) = tls.into_inner();
                let mut remaining = Vec::new();
                raw.read_to_end(&mut remaining).await.unwrap();
                assert!(
                    remaining.is_empty(),
                    "no extra request or discarded protocol bytes allowed"
                );
            };
            let ((targets, receipts), ()) = tokio::time::timeout(Duration::from_secs(5), async {
                tokio::join!(operation, peer)
            })
            .await
            .unwrap();
            assert_eq!(targets.len(), usize::from(matching));
            assert_eq!(receipts.len(), usize::from(matching));
        }
    }
}

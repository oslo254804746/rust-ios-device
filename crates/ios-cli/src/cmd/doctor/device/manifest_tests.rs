use super::*;

#[tokio::test]
async fn review_regression_local_manifest_uses_selected_device_identifiers() {
    let root = std::env::temp_dir().join(format!("doctor-selected-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("image.dmg"), b"offline-component").unwrap();
    let digest = <sha2::Sha256 as sha2::Digest>::digest(b"offline-component").to_vec();
    let component = dictionary([
        (
            "Info",
            plist::Value::Dictionary(dictionary([("Path", "image.dmg".into())])),
        ),
        ("Digest", plist::Value::Data(digest)),
    ]);
    let identity = dictionary([
        ("ApBoardID", 16u64.into()),
        ("ApChipID", 0x8101u64.into()),
        ("ApSecurityDomain", 1u64.into()),
        (
            "Manifest",
            plist::Value::Dictionary(dictionary([(
                "PersonalizedDMG",
                plist::Value::Dictionary(component),
            )])),
        ),
    ]);
    plist::Value::Dictionary(dictionary([(
        "BuildIdentities",
        plist::Value::Array(vec![plist::Value::Dictionary(identity)]),
    )]))
    .to_file_xml(root.join("BuildManifest.plist"))
    .unwrap();
    for board in [16u64, 17] {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let (image, mut peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
        let (check, (), ()) = within(async {
            tokio::join!(
                super::super::manifest::inspect_with_paths(
                    &source,
                    UDID,
                    Duration::from_secs(2),
                    std::slice::from_ref(&root)
                ),
                serve_service(lockdown_peer, service_reply()),
                async {
                    let request: plist::Dictionary = recv_lockdown(&mut peer).await.unwrap();
                    assert_eq!(
                        request["Command"].as_string(),
                        Some("QueryPersonalizationIdentifiers")
                    );
                    assert_eq!(
                        request["PersonalizedImageType"].as_string(),
                        Some("DeveloperDiskImage")
                    );
                    send_lockdown(
                        &mut peer,
                        &dictionary([(
                            "PersonalizationIdentifiers",
                            plist::Value::Dictionary(dictionary([
                                ("BoardId", board.into()),
                                ("ChipID", 0x8101u64.into()),
                                ("SecurityDomain", 1u64.into()),
                            ])),
                        )]),
                    )
                    .await
                    .unwrap();
                    image_request(&mut peer, "CopyDevices", None).await;
                    send_lockdown(
                        &mut peer,
                        &dictionary([("EntryList", plist::Value::Array(vec![]))]),
                    )
                    .await
                    .unwrap();
                }
            )
        })
        .await;
        if board != 16 {
            assert_eq!(check.status, Status::Failed);
        }
        let output = serde_json::to_value(check).unwrap();
        let local = &output["ddi_manifests"]["target_manifest"];
        assert_eq!(
            local["outcome"],
            if board == 16 {
                "matched"
            } else {
                "no_matching_identity"
            }
        );
        if board == 16 {
            assert_eq!(local["components"][0]["state"], "digest_matches");
            assert_eq!(local["components"][0]["signature_verified"], false);
            assert_eq!(local["components"][0]["device_availability_probed"], false);
        }
    }
    for file in ["image.dmg", "BuildManifest.plist"] {
        std::fs::remove_file(root.join(file)).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

fn entry(signature: Vec<u8>) -> plist::Value {
    plist::Value::Dictionary(dictionary([
        ("DiskImageType", "Personalized".into()),
        ("PersonalizedImageType", "DeveloperDiskImage".into()),
        ("ImageSignature", plist::Value::Data(signature)),
    ]))
}

async fn manifest_request(peer: &mut DuplexStream, signature: &[u8]) {
    let request: plist::Dictionary = recv_lockdown(peer).await.unwrap();
    assert_eq!(request.len(), 4);
    assert_eq!(
        request["Command"].as_string(),
        Some("QueryPersonalizationManifest")
    );
    assert_eq!(
        request["PersonalizedImageType"].as_string(),
        Some("DeveloperDiskImage")
    );
    assert_eq!(request["ImageType"].as_string(), Some("DeveloperDiskImage"));
    assert_eq!(request["ImageSignature"].as_data(), Some(signature));
}

async fn observe(
    response: plist::Dictionary,
    manifests: Vec<(Vec<u8>, plist::Dictionary)>,
) -> Check {
    let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
    let (image, mut image_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
    let (check, (), ()) = within(async {
        tokio::join!(
            super::super::manifest::inspect(&source, UDID, Duration::from_secs(2)),
            serve_service(lockdown_peer, service_reply()),
            async {
                image_request(&mut image_peer, "CopyDevices", None).await;
                send_lockdown(&mut image_peer, &response).await.unwrap();
                for (signature, reply) in manifests {
                    manifest_request(&mut image_peer, &signature).await;
                    send_lockdown(&mut image_peer, &reply).await.unwrap();
                }
                eof(&mut image_peer).await;
            }
        )
    })
    .await;
    assert!(source.ports.lock().unwrap().is_empty());
    assert_eq!(
        source.calls(),
        vec![
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Port(7, IMAGE_PORT)
        ]
    );
    safe(&check);
    check
}

#[tokio::test]
async fn manifest_observation_uses_only_existing_supported_readonly_wire_and_opaque_counts() {
    let signatures = [
        b"owned-private-signature-a".to_vec(),
        b"owned-private-signature-b".to_vec(),
    ];
    let payloads = [
        b"owned-private-manifest-a".to_vec(),
        b"owned-private-manifest-b".to_vec(),
    ];
    let check = observe(
        dictionary([(
            "EntryList",
            plist::Value::Array(signatures.iter().cloned().map(entry).collect()),
        )]),
        signatures
            .iter()
            .cloned()
            .zip(
                payloads
                    .iter()
                    .cloned()
                    .map(|value| dictionary([("ImageSignature", plist::Value::Data(value))])),
            )
            .collect(),
    )
    .await;
    assert_eq!(check.id, "ddi_manifests");
    assert_eq!(check.status, Status::Ok);
    assert_eq!(check.state, State::Available);
    assert_eq!(check.value, Some(true));
    let output = serde_json::to_value(&check).unwrap();
    assert_eq!(output["ddi_manifests"]["developer_entries"], 2);
    assert_eq!(output["ddi_manifests"]["manifests_returned"], 2);
    assert_eq!(
        output["ddi_manifests"]["manifest_bytes"],
        payloads.iter().map(Vec::len).sum::<usize>()
    );
    assert_eq!(output["ddi_manifests"]["complete"], true);
    assert!(check.detail.contains("cryptographic validity"));
}

#[tokio::test]
async fn incomplete_manifest_selection_sends_no_manifest_queries_or_inferred_false_values() {
    for response in [
        plist::Dictionary::new(),
        dictionary([(
            "EntryList",
            plist::Value::Array(vec![plist::Value::Dictionary(dictionary([(
                "DiskImageType",
                "Personalized".into(),
            )]))]),
        )]),
        dictionary([("EntryList", plist::Value::Array(vec![entry(Vec::new())]))]),
        dictionary([(
            "EntryList",
            plist::Value::Array(vec![
                entry(b"owned-private-valid".to_vec()),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ]),
        )]),
    ] {
        let check = observe(response, vec![]).await;
        assert_eq!(check.status, Status::Unknown);
        assert_eq!(check.state, State::MetadataMissing);
        assert_eq!(check.value, None);
        assert_eq!(
            serde_json::to_value(check).unwrap()["ddi_manifests"]["complete"],
            false
        );
    }
    let check = observe(
        dictionary([("EntryList", plist::Value::Array(vec![]))]),
        vec![],
    )
    .await;
    assert_eq!(check.value, Some(false));
    assert_eq!(
        serde_json::to_value(check).unwrap()["ddi_manifests"]["complete"],
        true
    );
}

#[tokio::test]
async fn malformed_or_oversized_manifest_selection_is_rejected_before_any_query() {
    for entries in [
        plist::Value::String("owned-private-malformed".into()),
        plist::Value::Array(vec![plist::Value::Boolean(true)]),
        plist::Value::Array(vec![plist::Value::Dictionary(dictionary([(
            "DiskImageType",
            plist::Value::Boolean(true),
        )]))]),
        plist::Value::Array((0..33).map(|_| entry(vec![1])).collect()),
        plist::Value::Array(vec![entry(vec![1; 1024 * 1024 + 1])]),
        plist::Value::Array(
            (0..129)
                .map(|_| {
                    plist::Value::Dictionary(dictionary([("DiskImageType", "Developer".into())]))
                })
                .collect(),
        ),
    ] {
        let check = observe(dictionary([("EntryList", entries)]), vec![]).await;
        assert_eq!(check.status, Status::Failed);
        assert_eq!(check.error_class, Some(ErrorClass::Protocol));
        assert!(check.error.as_ref().unwrap().is::<LockdownError>());
        machine_failure_matches_report(check);
    }
}

#[tokio::test]
async fn partial_manifest_observation_retains_counts_and_original_remote_error() {
    let signatures = [vec![1], vec![2]];
    let check = observe(
        dictionary([(
            "EntryList",
            plist::Value::Array(signatures.iter().cloned().map(entry).collect()),
        )]),
        vec![
            (
                signatures[0].clone(),
                dictionary([("ImageSignature", plist::Value::Data(vec![3, 4]))]),
            ),
            (
                signatures[1].clone(),
                dictionary([("Error", "owned-private-remote-error".into())]),
            ),
        ],
    )
    .await;
    assert_eq!(check.status, Status::Failed);
    assert_eq!(check.error_class, Some(ErrorClass::ServiceFailure));
    assert!(check.error.as_ref().unwrap().is::<RemoteResponseError>());
    let output = serde_json::to_value(&check).unwrap();
    assert_eq!(output["ddi_manifests"]["manifests_returned"], 1);
    assert_eq!(output["ddi_manifests"]["manifest_bytes"], 2);
    assert_eq!(output["ddi_manifests"]["complete"], false);
    machine_failure_matches_report(check);
}

#[tokio::test]
async fn omitted_empty_malformed_and_oversized_manifest_payloads_are_distinct() {
    for (payload, expected_state, expected_class) in [
        (None, State::MetadataMissing, None),
        (
            Some(plist::Value::Data(vec![])),
            State::MetadataMissing,
            None,
        ),
        (
            Some(plist::Value::String("owned-private-payload".into())),
            State::Failed,
            Some(ErrorClass::Protocol),
        ),
        (
            Some(plist::Value::Data(vec![0; 1024 * 1024 + 1])),
            State::Failed,
            Some(ErrorClass::Protocol),
        ),
    ] {
        let check = observe(
            dictionary([("EntryList", plist::Value::Array(vec![entry(vec![1])]))]),
            vec![(
                vec![1],
                payload
                    .map(|value| dictionary([("ImageSignature", value)]))
                    .unwrap_or_default(),
            )],
        )
        .await;
        assert_eq!(check.state, expected_state);
        assert_eq!(check.error_class, expected_class);
        assert_eq!(check.value, None);
        assert_eq!(
            serde_json::to_value(&check).unwrap()["ddi_manifests"]["complete"],
            false
        );
    }
}

#[tokio::test]
async fn manifest_probe_uses_one_total_deadline_and_closes_the_original_stalled_peer() {
    let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
    let (image, mut image_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
    let (check, (), ()) = within(async {
        tokio::join!(
            super::super::manifest::inspect(&source, UDID, Duration::from_millis(500)),
            serve_service(lockdown_peer, service_reply()),
            async {
                image_request(&mut image_peer, "CopyDevices", None).await;
                send_lockdown(
                    &mut image_peer,
                    &dictionary([("EntryList", plist::Value::Array(vec![entry(vec![1])]))]),
                )
                .await
                .unwrap();
                manifest_request(&mut image_peer, &[1]).await;
                eof(&mut image_peer).await;
            }
        )
    })
    .await;
    assert_eq!(check.state, State::Unreachable);
    assert_eq!(check.error_class, Some(ErrorClass::Timeout));
    safe(&check);
    machine_failure_matches_report(check);
}

#[tokio::test]
async fn manifest_startservice_permission_and_missing_service_preserve_native_categories() {
    for (code, state, class) in [
        (
            "PasswordProtected",
            State::PermissionDenied,
            ErrorClass::PermissionDenied,
        ),
        (
            "InvalidService",
            State::ServiceUnavailable,
            ErrorClass::ServiceMissing,
        ),
    ] {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
        let (check, ()) = within(async {
            tokio::join!(
                super::super::manifest::inspect(&source, UDID, Duration::from_secs(2)),
                serve_service(lockdown_peer, dictionary([("Error", code.into())])),
            )
        })
        .await;
        assert_eq!(check.state, state);
        assert_eq!(check.error_class, Some(class));
        assert!(
            LockdownResponseError::in_error_chain(check.error.as_ref().unwrap().as_ref()).is_some()
        );
        assert_eq!(
            source.calls(),
            vec![
                Call::Devices,
                Call::PairRecord,
                Call::Port(7, LOCKDOWN_PORT)
            ]
        );
        safe(&check);
        machine_failure_matches_report(check);
    }
}

#[tokio::test]
async fn manifest_probe_cancellation_discards_the_owned_session_and_stops_readonly_requests() {
    let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
    let (image, mut image_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (result, (), ()) = within(async {
        tokio::join!(
            cancel_probe(
                async {
                    Ok(
                        super::super::manifest::inspect(&source, UDID, Duration::from_secs(30))
                            .await,
                    )
                },
                async {
                    ready_rx.await.unwrap();
                    Ok(())
                },
            ),
            serve_service(lockdown_peer, service_reply()),
            async {
                image_request(&mut image_peer, "CopyDevices", None).await;
                send_lockdown(
                    &mut image_peer,
                    &dictionary([("EntryList", plist::Value::Array(vec![entry(vec![1])]))]),
                )
                .await
                .unwrap();
                manifest_request(&mut image_peer, &[1]).await;
                ready_tx.send(()).unwrap();
                eof(&mut image_peer).await;
            },
        )
    })
    .await;
    assert!(result.unwrap_err().is::<ios_core::OperationCancelled>());
    assert!(source.ports.lock().unwrap().is_empty());
}

#[tokio::test]
async fn review_regression_ddi_manifest_version_mismatch_fails_check() {
    let root = std::env::temp_dir().join(format!("doctor-ver-mismatch-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("image.dmg"), b"offline-component").unwrap();
    let digest = <sha2::Sha256 as sha2::Digest>::digest(b"offline-component").to_vec();
    let component = dictionary([
        (
            "Info",
            plist::Value::Dictionary(dictionary([("Path", "image.dmg".into())])),
        ),
        ("Digest", plist::Value::Data(digest)),
    ]);
    let identity = dictionary([
        ("ApBoardID", 16u64.into()),
        ("ApChipID", 0x8101u64.into()),
        ("ApSecurityDomain", 1u64.into()),
        (
            "Manifest",
            plist::Value::Dictionary(dictionary([(
                "PersonalizedDMG",
                plist::Value::Dictionary(component),
            )])),
        ),
    ]);
    plist::Value::Dictionary(dictionary([
        (
            "BuildIdentities",
            plist::Value::Array(vec![plist::Value::Dictionary(identity)]),
        ),
        ("ProductVersion", plist::Value::String("17.0".into())),
    ]))
    .to_file_xml(root.join("BuildManifest.plist"))
    .unwrap();

    let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
    let (image, mut peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
    let (check, (), ()) = within(async {
        tokio::join!(
            super::super::manifest::inspect_with_paths(
                &source,
                UDID,
                Duration::from_secs(2),
                std::slice::from_ref(&root)
            ),
            serve_service(lockdown_peer, service_reply()),
            async {
                let request: plist::Dictionary = recv_lockdown(&mut peer).await.unwrap();
                assert_eq!(
                    request["Command"].as_string(),
                    Some("QueryPersonalizationIdentifiers")
                );
                send_lockdown(
                    &mut peer,
                    &dictionary([(
                        "PersonalizationIdentifiers",
                        plist::Value::Dictionary(dictionary([
                            ("BoardId", 16u64.into()),
                            ("ChipID", 0x8101u64.into()),
                            ("SecurityDomain", 1u64.into()),
                            ("ProductVersion", plist::Value::String("16.5".into())),
                        ])),
                    )]),
                )
                .await
                .unwrap();
                image_request(&mut peer, "CopyDevices", None).await;
                send_lockdown(
                    &mut peer,
                    &dictionary([("EntryList", plist::Value::Array(vec![]))]),
                )
                .await
                .unwrap();
            }
        )
    })
    .await;

    assert_eq!(check.status, Status::Failed);
    assert_eq!(check.state, State::Failed);
    let output = serde_json::to_value(check).unwrap();
    let local = &output["ddi_manifests"]["target_manifest"];
    assert_eq!(local["outcome"], "version_mismatch");

    let _ = std::fs::remove_dir_all(root);
}

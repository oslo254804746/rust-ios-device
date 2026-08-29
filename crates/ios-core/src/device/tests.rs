mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[cfg(feature = "tunnel")]
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use tokio::io::duplex;

    #[cfg(feature = "tunnel")]
    use super::credentials::*;
    #[cfg(feature = "tunnel")]
    use super::pairing::{GuardedTunnelStream, RemotePairingControlChannel};
    #[cfg(feature = "tunnel")]
    use super::protocol::*;
    #[cfg(feature = "tunnel")]
    use super::tunnel_activation::TunnelTransport;
    use super::*;
    #[cfg(feature = "tunnel")]
    use crate::credentials::{PersistedCredentials, RemotePairingRecord};
    #[cfg(feature = "tunnel")]
    use crate::lockdown::pairing::HostIdentity;
    #[cfg(feature = "tunnel")]
    use crate::xpc::message::XpcValue;

    fn temp_test_dir(label: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ios_core_device_{label}_{unique}"))
    }

    #[cfg(feature = "tunnel")]
    fn make_remote_pair_record(identity: &HostIdentity) -> RemotePairingRecord {
        RemotePairingRecord {
            public_key: identity.public_key_bytes(),
            private_key: identity.private_key_bytes(),
            remote_unlock_host_key: None,
        }
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_transport_accepts_each_connection_strategy() {
        fn assert_transport<T: TunnelTransport>() {}

        // Lockdown's CoreDeviceProxy wrapper, direct RSD's XPC control-channel
        // guard, and remote pairing's framed control-channel guard all enter
        // the same activation path. A future protocol can exercise it with an
        // in-memory duplex stream before hardware integration exists.
        assert_transport::<ProxyStream>();
        assert_transport::<GuardedTunnelStream<XpcClient>>();
        assert_transport::<GuardedTunnelStream<RemotePairingControlChannel>>();
        assert_transport::<tokio::io::DuplexStream>();
    }

    #[test]
    fn try_load_pair_record_returns_none_for_missing_pair_record() {
        let missing_dir = temp_test_dir("missing_pair_record");

        let loaded = try_load_pair_record("missing-udid", Some(&missing_dir));

        assert!(loaded.is_none());

        let _ = std::fs::remove_dir_all(missing_dir);
    }

    #[test]
    fn require_pair_record_rejects_missing_lockdown_pair_record() {
        let err = require_pair_record(None, "test-udid", "remote pairing lockdown access requires")
            .expect_err("missing pair record should fail");

        assert!(err
            .to_string()
            .contains("remote pairing lockdown access requires"));
        assert!(err.to_string().contains("test-udid"));
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn load_remote_pairing_credentials_accepts_legacy_ios_rs_without_private_key_hex() {
        let base_dir = temp_test_dir("legacy_ios_rs");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let identity = HostIdentity::generate();

        make_remote_pair_record(&identity)
            .save_for_identifier(&ios_rs_dir, remote_identifier)
            .unwrap();
        PersistedCredentials {
            remote_identifier: Some(remote_identifier.into()),
            host_identifier: identity.identifier.clone(),
            host_public_key_hex: hex::encode(identity.public_key_bytes()),
            host_private_key_hex: None,
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&ios_rs_dir)
        .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            "unused-hostname",
        )
        .expect("legacy ios-rs credentials should load from remote pair record");

        assert_eq!(loaded.host_identity.identifier, identity.identifier);
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn load_remote_pairing_credentials_prefers_ios_rs_over_pymobiledevice3() {
        let base_dir = temp_test_dir("prefers_ios_rs");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let ios_rs_identity = HostIdentity::generate();
        let fallback_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier("example-host"),
            &[0x44; 32],
        )
        .unwrap();

        make_remote_pair_record(&ios_rs_identity)
            .save_for_identifier(&ios_rs_dir, remote_identifier)
            .unwrap();
        PersistedCredentials {
            remote_identifier: Some(remote_identifier.into()),
            host_identifier: ios_rs_identity.identifier.clone(),
            host_public_key_hex: hex::encode(ios_rs_identity.public_key_bytes()),
            host_private_key_hex: Some(hex::encode(ios_rs_identity.private_key_bytes())),
            remote_unlock_host_key: None,
            device_address: "fd00::1".into(),
            rsd_port: 58783,
        }
        .save(&ios_rs_dir)
        .unwrap();
        make_remote_pair_record(&fallback_identity)
            .save_for_identifier(&pymobiledevice3_dir, remote_identifier)
            .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            "example-host",
        )
        .expect("ios-rs credentials should take precedence");

        assert_eq!(loaded.host_identity.identifier, ios_rs_identity.identifier);
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            ios_rs_identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn load_remote_pairing_credentials_falls_back_to_pymobiledevice3_remote_record() {
        let base_dir = temp_test_dir("pymobiledevice3_fallback");
        let ios_rs_dir = base_dir.join("ios-rs");
        let pymobiledevice3_dir = base_dir.join(".pymobiledevice3");
        let remote_identifier = "test-remote";
        let hostname = "example-host";
        let expected_identity = HostIdentity::from_private_key_bytes(
            pymobiledevice3_host_identifier(hostname),
            &[0x22; 32],
        )
        .unwrap();

        make_remote_pair_record(&expected_identity)
            .save_for_identifier(&pymobiledevice3_dir, remote_identifier)
            .unwrap();

        let loaded = load_remote_pairing_credentials_from_dirs(
            remote_identifier,
            &ios_rs_dir,
            &pymobiledevice3_dir,
            hostname,
        )
        .expect("pymobiledevice3 remote record should be usable as fallback");

        assert_eq!(
            loaded.host_identity.identifier,
            pymobiledevice3_host_identifier(hostname)
        );
        assert_eq!(
            loaded.host_identity.public_key_bytes(),
            expected_identity.public_key_bytes()
        );

        let _ = std::fs::remove_dir_all(base_dir);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn direct_handshake_request_carries_attempt_pair_verify() {
        let request = build_direct_handshake_request(7);
        let envelope = request.as_dict().expect("envelope dict");
        assert_eq!(
            envelope.get("mangledTypeName").and_then(XpcValue::as_str),
            Some(DIRECT_CONTROL_CHANNEL_ENVELOPE_TYPE)
        );

        let handshake = envelope
            .get("value")
            .and_then(XpcValue::as_dict)
            .and_then(|value| value.get("message"))
            .and_then(XpcValue::as_dict)
            .and_then(|message| message.get("plain"))
            .and_then(XpcValue::as_dict)
            .and_then(|plain| plain.get("_0"))
            .and_then(XpcValue::as_dict)
            .and_then(|plain| plain.get("request"))
            .and_then(XpcValue::as_dict)
            .and_then(|request| request.get("_0"))
            .and_then(XpcValue::as_dict)
            .and_then(|request| request.get("handshake"))
            .and_then(XpcValue::as_dict)
            .and_then(|handshake| handshake.get("_0"))
            .and_then(XpcValue::as_dict)
            .expect("handshake dict");

        assert_eq!(
            handshake
                .get("hostOptions")
                .and_then(XpcValue::as_dict)
                .and_then(|options| options.get("attemptPairVerify")),
            Some(&XpcValue::Bool(true))
        );
        assert_eq!(
            handshake.get("wireProtocolVersion"),
            Some(&XpcValue::Int64(19))
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn remote_pairing_handshake_request_starts_at_plain_message_root() {
        let request = build_remote_pairing_handshake_request(0);
        assert_eq!(request["originatedBy"], "host");
        assert_eq!(request["sequenceNumber"], 0);
        assert_eq!(
            request["message"]["plain"]["_0"]["request"]["_0"]["handshake"]["_0"]["hostOptions"]
                ["attemptPairVerify"],
            true
        );
        assert_eq!(
            request["message"]["plain"]["_0"]["request"]["_0"]["handshake"]["_0"]
                ["wireProtocolVersion"],
            19
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn extract_direct_remote_identifier_reads_peer_device_info() {
        let body = build_direct_control_envelope(
            xpc_dict(&[(
                "plain",
                xpc_dict(&[(
                    "_0",
                    xpc_dict(&[(
                        "response",
                        xpc_dict(&[(
                            "_1",
                            xpc_dict(&[(
                                "handshake",
                                xpc_dict(&[(
                                    "_0",
                                    xpc_dict(&[(
                                        "peerDeviceInfo",
                                        xpc_dict(&[(
                                            "identifier",
                                            XpcValue::String("test-remote".into()),
                                        )]),
                                    )]),
                                )]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
            1,
        );

        let identifier = extract_direct_remote_identifier(&body).expect("identifier should parse");
        assert_eq!(identifier, "test-remote");
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn extract_direct_pairing_tlv_surfaces_rejection_message() {
        let body = build_direct_control_envelope(
            xpc_dict(&[(
                "plain",
                xpc_dict(&[(
                    "_0",
                    xpc_dict(&[(
                        "event",
                        xpc_dict(&[(
                            "_0",
                            xpc_dict(&[(
                                "pairingRejectedWithError",
                                xpc_dict(&[(
                                    "wrappedError",
                                    xpc_dict(&[(
                                        "userInfo",
                                        xpc_dict(&[(
                                            "NSLocalizedDescription",
                                            XpcValue::String("Trust denied".into()),
                                        )]),
                                    )]),
                                )]),
                            )]),
                        )]),
                    )]),
                )]),
            )]),
            2,
        );

        let err = extract_direct_pairing_tlv(&body).expect_err("rejection should error");
        assert!(err.to_string().contains("Trust denied"));
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn extract_remote_pairing_tlv_decodes_base64_payload() {
        let body = serde_json::json!({
            "message": {
                "plain": {
                    "_0": {
                        "event": {
                            "_0": {
                                "pairingData": {
                                    "_0": {
                                        "data": BASE64_STANDARD.encode([0x01, 0x02, 0x03]),
                                        "kind": "verifyManualPairing",
                                        "startNewSession": true
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let tlv = extract_remote_pairing_tlv(&body).expect("payload should decode");
        assert_eq!(tlv, vec![0x01, 0x02, 0x03]);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn extract_remote_pairing_tlv_surfaces_rejection_message() {
        let body = serde_json::json!({
            "message": {
                "plain": {
                    "_0": {
                        "event": {
                            "_0": {
                                "pairingRejectedWithError": {
                                    "wrappedError": {
                                        "userInfo": {
                                            "NSLocalizedDescription": "Pair denied"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let err = extract_remote_pairing_tlv(&body).expect_err("rejection should error");
        assert!(err.to_string().contains("Pair denied"));
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn make_direct_encrypted_nonce_uses_little_endian_sequence() {
        let nonce = make_direct_encrypted_nonce(0x0102_0304_0506_0708);
        assert_eq!(
            nonce,
            [0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0, 0, 0, 0]
        );
    }

    #[test]
    fn select_mux_device_prefers_usb_when_multiple_transports_match() {
        let selected = select_mux_device(
            vec![
                crate::mux::MuxDevice {
                    device_id: 7,
                    serial_number: "test-udid".into(),
                    connection_type: "Network".into(),
                    product_id: 0,
                },
                crate::mux::MuxDevice {
                    device_id: 8,
                    serial_number: "test-udid".into(),
                    connection_type: "USB".into(),
                    product_id: 0,
                },
            ],
            "test-udid",
        )
        .expect("matching device should be selected");

        assert_eq!(selected.device_id, 8);
        assert_eq!(selected.connection_type, "USB");
    }

    #[test]
    fn select_mux_device_falls_back_to_non_usb_match() {
        let selected = select_mux_device(
            vec![crate::mux::MuxDevice {
                device_id: 9,
                serial_number: "test-udid".into(),
                connection_type: "Network".into(),
                product_id: 0,
            }],
            "test-udid",
        )
        .expect("network-only match should still be selected");

        assert_eq!(selected.device_id, 9);
        assert_eq!(selected.connection_type, "Network");
    }

    #[test]
    fn strip_ssl_selection_matches_legacy_dtx_services() {
        assert!(should_strip_service_ssl(
            "com.apple.accessibility.axAuditDaemon.remoteserver"
        ));
        assert!(should_strip_service_ssl(
            "com.apple.instruments.remoteserver"
        ));
        assert!(!should_strip_service_ssl(
            "com.apple.instruments.remoteserver.DVTSecureSocketProxy"
        ));
        assert!(!should_strip_service_ssl("com.apple.mobile.screenshotr"));
        assert!(!should_strip_service_ssl("com.apple.webinspector"));
    }

    #[test]
    fn parses_string_array_values_for_international_configuration() {
        let value = plist::Value::Array(vec![
            plist::Value::String("en-US".into()),
            plist::Value::String("zh-Hans".into()),
        ]);

        let parsed = plist_value_to_string_vec(&value, "SupportedLanguages")
            .expect("string array should parse");

        assert_eq!(parsed, vec!["en-US".to_string(), "zh-Hans".to_string()]);
    }

    #[test]
    fn rejects_non_string_entries_in_international_configuration_arrays() {
        let value = plist::Value::Array(vec![plist::Value::Integer(1i64.into())]);

        let err = plist_value_to_string_vec(&value, "SupportedLocales")
            .expect_err("non-string entry should fail");

        let rendered = err.to_string();
        assert!(rendered.contains("SupportedLocales"));
        assert!(rendered.contains("string"));
    }

    #[test]
    fn resolve_rsd_service_reports_actual_shim_match() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.mobile.notification_proxy.shim.remote".into(),
                ServiceDescriptor::new(1234),
            )]),
        };

        let resolved = resolve_rsd_service(&rsd, "com.apple.mobile.notification_proxy")
            .expect("shim fallback should resolve");

        assert_eq!(
            resolved,
            (
                "com.apple.mobile.notification_proxy.shim.remote".into(),
                1234
            )
        );
    }

    #[test]
    fn resolve_rsd_service_prefers_canonical_entry_over_shim() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([
                (
                    "com.apple.mobile.notification_proxy".into(),
                    ServiceDescriptor::new(2345),
                ),
                (
                    "com.apple.mobile.notification_proxy.shim.remote".into(),
                    ServiceDescriptor::new(6789),
                ),
            ]),
        };

        assert_eq!(
            resolve_rsd_service(&rsd, "com.apple.mobile.notification_proxy"),
            Some(("com.apple.mobile.notification_proxy".to_string(), 2345,))
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn resolved_rsd_service_metadata_preserves_canonical_features() {
        let mut canonical = ServiceDescriptor::new(2345);
        canonical.features = vec!["com.apple.coredevice.feature.streamapplist".into()];
        let mut shim = ServiceDescriptor::new(6789);
        shim.features = vec!["shim-only-feature".into()];
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([
                ("com.apple.coredevice.appservice".into(), canonical),
                ("com.apple.coredevice.appservice.shim.remote".into(), shim),
            ]),
        };

        let resolved = resolve_rsd_service_details(&rsd, "com.apple.coredevice.appservice")
            .expect("canonical appservice should resolve");

        assert_eq!(
            resolved.resolved_service_name,
            "com.apple.coredevice.appservice"
        );
        assert_eq!(resolved.port, 2345);
        assert_eq!(
            resolved.features,
            ["com.apple.coredevice.feature.streamapplist"]
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn resolved_rsd_service_metadata_falls_back_to_shim_features() {
        let mut shim = ServiceDescriptor::new(6789);
        shim.features = vec!["shim-feature".into()];
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([("com.apple.coredevice.appservice.shim.remote".into(), shim)]),
        };

        let resolved = resolve_rsd_service_details(&rsd, "com.apple.coredevice.appservice")
            .expect("shim appservice should resolve");

        assert_eq!(
            resolved.resolved_service_name,
            "com.apple.coredevice.appservice.shim.remote"
        );
        assert_eq!(resolved.port, 6789);
        assert_eq!(resolved.features, ["shim-feature"]);
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn resolved_rsd_service_metadata_keeps_empty_features_for_fallback() {
        let rsd = RsdHandshake {
            udid: "test-udid".into(),
            services: HashMap::from([(
                "com.apple.coredevice.appservice".into(),
                ServiceDescriptor::new(2345),
            )]),
        };

        let resolved = resolve_rsd_service_details(&rsd, "com.apple.coredevice.appservice")
            .expect("canonical appservice should resolve without feature metadata");

        assert_eq!(
            resolved.resolved_service_name,
            "com.apple.coredevice.appservice"
        );
        assert!(resolved.features.is_empty());
    }

    #[test]
    fn remote_xpc_route_rejects_shim_before_opening_socket() {
        let error = ensure_remote_xpc_service(
            "com.apple.mobile.notification_proxy",
            "com.apple.mobile.notification_proxy.shim.remote",
        )
        .expect_err("lockdown shims must not enter XPC/H2 bootstrap");
        let message = error.to_string();
        assert!(message.contains("lockdown shim"));
        assert!(message.contains("connect_rsd_service"));
        assert!(message.contains("RSDCheckin"));
    }

    #[test]
    fn remote_xpc_route_accepts_canonical_service() {
        ensure_remote_xpc_service(
            "com.apple.coredevice.appservice",
            "com.apple.coredevice.appservice",
        )
        .expect("canonical RemoteXPC services should be accepted");
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint_uses_userspace_proxy_when_available() {
        let endpoint = TunnelEndpoint::resolve("fd00::1", Some(60105)).expect("valid endpoint");

        assert_eq!(
            endpoint,
            TunnelEndpoint::UserspaceProxy {
                proxy_port: 60105,
                remote_addr: Ipv6Addr::from_str("fd00::1").expect("valid IPv6"),
            }
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint_falls_back_to_direct_ipv6() {
        let endpoint = TunnelEndpoint::resolve("fd00::2", None).expect("valid endpoint");

        assert_eq!(
            endpoint,
            TunnelEndpoint::DirectIpv6 {
                remote_addr: Ipv6Addr::from_str("fd00::2").expect("valid IPv6"),
            }
        );
    }

    #[test]
    #[cfg(feature = "tunnel")]
    fn tunnel_endpoint_rejects_invalid_ipv6() {
        let err = TunnelEndpoint::resolve("not-an-ipv6", Some(60105))
            .expect_err("invalid IPv6 should fail");

        assert!(err.to_string().contains("invalid IPv6 addr"));
    }

    #[tokio::test]
    #[cfg(feature = "tunnel")]
    async fn userspace_tunnel_endpoint_writes_go_ios_routing_header() {
        use tokio::io::AsyncReadExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy listener");
        let proxy_port = listener.local_addr().expect("listener address").port();
        let remote_addr = Ipv6Addr::from_str("fd00::42").expect("valid IPv6");
        let endpoint = TunnelEndpoint::UserspaceProxy {
            proxy_port,
            remote_addr,
        };

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept proxy connection");
            let mut header = [0u8; 20];
            stream
                .read_exact(&mut header)
                .await
                .expect("read routing header");
            header
        });

        let _stream = endpoint
            .connect(62000)
            .await
            .expect("connect through proxy");
        let header = server.await.expect("join proxy task");

        assert_eq!(&header[..16], &remote_addr.octets());
        assert_eq!(u32::from_le_bytes(header[16..].try_into().unwrap()), 62000);
    }

    #[test]
    #[cfg(feature = "mdns")]
    fn preferred_lockdown_address_prefers_ipv4() {
        let addresses = vec![
            "fe80::1%Ethernet".to_string(),
            "192.168.31.247".to_string(),
            "fd00::1".to_string(),
        ];

        assert_eq!(
            preferred_lockdown_address(&addresses),
            Some("192.168.31.247")
        );
    }

    #[test]
    #[cfg(feature = "mdns")]
    fn match_paired_mobdev2_targets_uses_wifi_mac_and_dedupes() {
        let services = vec![
            BonjourService {
                instance: "34:10:be:1b:a6:4c@fe80::1._apple-mobdev2._tcp.local.".into(),
                port: 32498,
                addresses: vec!["192.168.31.247".into()],
                properties: HashMap::new(),
            },
            BonjourService {
                instance: "34:10:be:1b:a6:4c@fe80::1._apple-mobdev2._tcp.local.".into(),
                port: 32498,
                addresses: vec!["192.168.31.247".into()],
                properties: HashMap::new(),
            },
        ];
        let wifi_mac_to_udid =
            HashMap::from([("34:10:be:1b:a6:4c".to_string(), "test-udid".to_string())]);

        let targets = match_paired_mobdev2_targets(&services, &wifi_mac_to_udid);

        assert_eq!(
            targets,
            vec![PairedMobdev2Device {
                udid: "test-udid".into(),
                host: "192.168.31.247".into(),
            }]
        );
    }

    #[tokio::test]
    async fn rsd_checkin_sends_request_and_consumes_two_responses() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let request: plist::Value = recv_lockdown(&mut server).await.expect("request frame");
        let dict = request
            .into_dictionary()
            .expect("RSDCheckin request should be a plist dictionary");
        assert_eq!(
            dict.get("Request").and_then(plist::Value::as_string),
            Some("RSDCheckin")
        );
        assert_eq!(
            dict.get("ProtocolVersion")
                .and_then(plist::Value::as_string),
            Some("2")
        );

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("RSDCheckin".into()),
                ),
                (
                    String::from("Status"),
                    plist::Value::String("Acknowledged".into()),
                ),
            ])),
        )
        .await
        .expect("checkin response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("StartService".into()),
                ),
                (String::from("Service"), plist::Value::String("shim".into())),
            ])),
        )
        .await
        .expect("start service response");

        task.await
            .expect("join")
            .expect("rsd checkin should succeed");
    }

    #[tokio::test]
    async fn rsd_checkin_rejects_unexpected_first_response() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let _: plist::Value = recv_lockdown(&mut server).await.expect("request frame");

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("StartService".into()),
            )])),
        )
        .await
        .expect("unexpected first response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("StartService".into()),
            )])),
        )
        .await
        .expect("second response");

        let err = task
            .await
            .expect("join")
            .expect_err("rsd checkin should reject mismatched first response");
        let rendered = err.to_string();
        assert!(rendered.contains("RSD check-in response"));
        assert!(rendered.contains("Request=RSDCheckin"));
    }

    #[tokio::test]
    async fn rsd_checkin_rejects_start_service_error() {
        let (mut client, mut server) = duplex(4096);
        let task = tokio::spawn(async move { rsd_checkin(&mut client).await });

        let _: plist::Value = recv_lockdown(&mut server).await.expect("request frame");

        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([(
                String::from("Request"),
                plist::Value::String("RSDCheckin".into()),
            )])),
        )
        .await
        .expect("checkin response");
        send_lockdown(
            &mut server,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    String::from("Request"),
                    plist::Value::String("StartService".into()),
                ),
                (
                    String::from("Error"),
                    plist::Value::String("ServiceProhibited".into()),
                ),
            ])),
        )
        .await
        .expect("start service error response");

        let err = task
            .await
            .expect("join")
            .expect_err("rsd checkin should surface start service errors");
        let rendered = err.to_string();
        assert!(rendered.contains("RSD start-service response"));
        assert!(rendered.contains("ServiceProhibited"));
    }

    #[tokio::test]
    async fn rsd_checkin_times_out_when_peer_stalls() {
        let (mut client, _server) = duplex(4096);

        let err = rsd_checkin_with_timeout(&mut client, std::time::Duration::from_millis(20))
            .await
            .expect_err("a peer that never answers must not block forever");

        assert!(matches!(
            err,
            CoreError::Io(ref io_error) if io_error.kind() == std::io::ErrorKind::TimedOut
        ));
        assert!(err.to_string().contains("RSD check-in timed out"));
    }
}

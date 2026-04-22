#[cfg(feature = "testmanager")]
mod tests {
    use bytes::Bytes;
    use ios_proto::nskeyedarchiver_encode::{NsUrl, XcTestConfiguration, XctCapabilities};
    use ios_services::dtx::primitive_enc::{archived_object, encode_primitive_dict};
    use ios_services::dtx::{read_dtx_frame, DtxError, DtxPayload, NSObject};
    use ios_services::testmanager::TestmanagerClient;
    use tokio::io::{duplex, AsyncWriteExt};
    use uuid::Uuid;

    fn expect_dict<'a>(
        value: &'a NSObject,
        context: &str,
    ) -> &'a indexmap::IndexMap<String, NSObject> {
        match value {
            NSObject::Dict(dict) => dict,
            other => panic!("{context}: expected dict, got {other:?}"),
        }
    }

    fn assert_archived_uuid(value: &NSObject, expected: Uuid) {
        let dict = expect_dict(value, "archived uuid");
        assert_eq!(
            dict.get("ObjectType").and_then(NSObject::as_str),
            Some("NSUUID")
        );
        match dict.get("NS.uuidbytes") {
            Some(NSObject::Data(bytes)) => assert_eq!(bytes.as_ref(), expected.as_bytes()),
            other => panic!("archived uuid: expected NS.uuidbytes data, got {other:?}"),
        }
    }

    fn assert_archived_capabilities(value: &NSObject) {
        let dict = expect_dict(value, "archived capabilities");
        assert_eq!(
            dict.get("ObjectType").and_then(NSObject::as_str),
            Some("XCTCapabilities")
        );

        let capabilities = expect_dict(
            dict.get("capabilities-dictionary").unwrap_or_else(|| {
                panic!("archived capabilities: missing capabilities-dictionary")
            }),
            "archived capabilities dictionary",
        );
        assert_eq!(
            capabilities
                .get("XCTIssue capability")
                .and_then(NSObject::as_bool),
            Some(true)
        );
    }

    fn assert_xctest_configuration(value: &NSObject) {
        let dict = expect_dict(value, "xctest configuration");
        assert_eq!(
            dict.get("ObjectType").and_then(NSObject::as_str),
            Some("XCTestConfiguration")
        );
        assert_eq!(
            dict.get("reportResultsToIDE").and_then(NSObject::as_bool),
            Some(true)
        );
        assert_eq!(
            dict.get("testsMustRunOnMainThread")
                .and_then(NSObject::as_bool),
            Some(true)
        );
        assert_eq!(
            dict.get("initializeForUITesting")
                .and_then(NSObject::as_bool),
            Some(true)
        );
        assert_eq!(
            dict.get("testTimeoutsEnabled").and_then(NSObject::as_bool),
            Some(false)
        );
        assert_eq!(
            dict.get("automationFrameworkPath")
                .and_then(NSObject::as_str),
            Some("/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework")
        );

        let bundle_url = expect_dict(
            dict.get("testBundleURL")
                .unwrap_or_else(|| panic!("xctest configuration: missing testBundleURL")),
            "xctest configuration testBundleURL",
        );
        assert_eq!(
            bundle_url.get("ObjectType").and_then(NSObject::as_str),
            Some("NSURL")
        );
        assert_eq!(
            bundle_url.get("NS.relative").and_then(NSObject::as_str),
            Some("file:///private/tmp/WebDriverAgentRunner.xctest")
        );

        assert_archived_uuid(
            dict.get("sessionIdentifier")
                .unwrap_or_else(|| panic!("xctest configuration: missing sessionIdentifier")),
            Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
        );
        assert_archived_capabilities(
            dict.get("IDECapabilities")
                .unwrap_or_else(|| panic!("xctest configuration: missing IDECapabilities")),
        );
    }

    #[tokio::test]
    async fn connect_requests_xctestmanager_channels_on_both_connections() {
        let (client_a, mut server_a) = duplex(4096);
        let (client_b, mut server_b) = duplex(4096);

        let connect =
            tokio::spawn(async move { TestmanagerClient::connect(client_a, client_b).await });

        let req_a = read_dtx_frame(&mut server_a).await.unwrap();
        match &req_a.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].as_int(), Some(1));
                assert_eq!(
                    args[1].as_str(),
                    Some("dtxproxy:XCTestManager_IDEInterface:XCTestManager_DaemonConnectionInterface")
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        server_a
            .write_all(&ios_services::dtx::encode_dtx(
                req_a.identifier,
                1,
                0,
                false,
                3,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let req_b = read_dtx_frame(&mut server_b).await.unwrap();
        match &req_b.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
                assert_eq!(args.len(), 2);
                assert_eq!(args[0].as_int(), Some(1));
                assert_eq!(
                    args[1].as_str(),
                    Some("dtxproxy:XCTestManager_IDEInterface:XCTestManager_DaemonConnectionInterface")
                );
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        server_b
            .write_all(&ios_services::dtx::encode_dtx(
                req_b.identifier,
                1,
                0,
                false,
                3,
                &[],
                &[],
            ))
            .await
            .unwrap();

        connect.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn start_executing_uses_protocol_36_on_channel_minus_one() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 7);

        let send = tokio::spawn(async move {
            testmanager.start_executing_test_plan().await.unwrap();
        });

        let msg = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(msg.channel_code, -1);
        match msg.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_startExecutingTestPlanWithProtocolVersion:");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].as_int(), Some(36));
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        send.await.unwrap();
    }

    #[tokio::test]
    async fn initiate_session_sends_identifier_and_capabilities_blob() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);
        let session_identifier = Bytes::from_static(b"session");
        let capabilities = Bytes::from_static(b"caps");

        let send = tokio::spawn(async move {
            testmanager
                .initiate_session(session_identifier, capabilities)
                .await
                .unwrap();
        });

        let msg = read_dtx_frame(&mut server).await.unwrap();
        match msg.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_initiateSessionWithIdentifier:capabilities:");
                assert_eq!(args.len(), 2);
                match &args[0] {
                    NSObject::Data(data) => assert_eq!(data.as_ref(), b"session"),
                    other => panic!("unexpected first arg: {other:?}"),
                }
                match &args[1] {
                    NSObject::Data(data) => assert_eq!(data.as_ref(), b"caps"),
                    other => panic!("unexpected second arg: {other:?}"),
                }
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        server
            .write_all(&ios_services::dtx::encode_dtx(
                msg.identifier,
                1,
                3,
                false,
                3,
                &[],
                &[],
            ))
            .await
            .unwrap();

        send.await.unwrap();
    }

    #[tokio::test]
    async fn request_driver_channel_is_used_for_start_executing() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);

        let task = tokio::spawn(async move {
            testmanager.request_driver_channel().await.unwrap();
            testmanager.start_executing_test_plan().await.unwrap();
        });

        let request = read_dtx_frame(&mut server).await.unwrap();
        match request.payload {
            DtxPayload::MethodInvocation { selector, .. } => {
                assert_eq!(selector, "_requestChannelWithCode:identifier:");
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        server
            .write_all(&ios_services::dtx::encode_dtx(
                request.identifier,
                1,
                0,
                false,
                3,
                &[],
                &[],
            ))
            .await
            .unwrap();

        let start = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(start.channel_code, 1);
        match start.payload {
            DtxPayload::MethodInvocation { selector, .. } => {
                assert_eq!(selector, "_IDE_startExecutingTestPlanWithProtocolVersion:");
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        task.await.unwrap();
    }

    #[tokio::test]
    async fn test_runner_ready_event_can_be_answered_with_configuration_payload() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);

        let incoming = ios_services::dtx::encode_dtx(
            42,
            0,
            3,
            true,
            2,
            &ios_proto::nskeyedarchiver_encode::archive_string(
                "_XCT_testRunnerReadyWithCapabilities:",
            ),
            &[],
        );
        server.write_all(&incoming).await.unwrap();

        let event = testmanager.recv_startup_event().await.unwrap();
        let msg = match event {
            ios_services::testmanager::StartupEvent::TestRunnerReady { message } => message,
            other => panic!("unexpected event: {other:?}"),
        };

        testmanager
            .respond_test_runner_ready_with_configuration(
                &msg,
                XcTestConfiguration {
                    session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff")
                        .unwrap(),
                    test_bundle_url: NsUrl {
                        path: "/private/tmp/WebDriverAgentRunner.xctest".to_string(),
                    },
                    ide_capabilities: XctCapabilities {
                        capabilities: vec![(
                            "XCTIssue capability".to_string(),
                            plist::Value::Boolean(true),
                        )],
                    },
                    automation_framework_path:
                        "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                            .to_string(),
                    initialize_for_ui_testing: true,
                    report_results_to_ide: true,
                    tests_must_run_on_main_thread: true,
                    test_timeouts_enabled: false,
                    additional_fields: Vec::new(),
                },
            )
            .await
            .unwrap();

        let response = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(response.identifier, 42);
        assert_eq!(response.conversation_idx, 1);
        match response.payload {
            DtxPayload::Response(value) => {
                assert_xctest_configuration(&value);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bundle_ready_event_exposes_protocol_versions() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);

        let selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_XCT_testBundleReadyWithProtocolVersion:minimumVersion:",
        );
        let aux = encode_primitive_dict(&[
            archived_object(ios_proto::nskeyedarchiver_encode::archive_int(36)),
            archived_object(ios_proto::nskeyedarchiver_encode::archive_int(25)),
        ]);
        let incoming = ios_services::dtx::encode_dtx(7, 0, 3, false, 2, &selector, &aux);
        server.write_all(&incoming).await.unwrap();

        let event = testmanager.recv_startup_event().await.unwrap();
        match event {
            ios_services::testmanager::StartupEvent::TestBundleReady {
                protocol_version,
                minimum_version,
                ..
            } => {
                assert_eq!(protocol_version, 36);
                assert_eq!(minimum_version, 25);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_startup_answers_runner_ready_and_collects_bundle_versions() {
        let (client, mut server) = duplex(8192);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);

        let configuration = XcTestConfiguration {
            session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            test_bundle_url: NsUrl {
                path: "/private/tmp/WebDriverAgentRunner.xctest".to_string(),
            },
            ide_capabilities: XctCapabilities {
                capabilities: vec![(
                    "XCTIssue capability".to_string(),
                    plist::Value::Boolean(true),
                )],
            },
            automation_framework_path:
                "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                    .to_string(),
            initialize_for_ui_testing: true,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields: Vec::new(),
        };

        let task = tokio::spawn(async move {
            testmanager
                .complete_startup_with_configuration(configuration)
                .await
                .unwrap()
        });

        let bundle_selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_XCT_testBundleReadyWithProtocolVersion:minimumVersion:",
        );
        let bundle_aux = encode_primitive_dict(&[
            archived_object(ios_proto::nskeyedarchiver_encode::archive_int(36)),
            archived_object(ios_proto::nskeyedarchiver_encode::archive_int(25)),
        ]);
        let bundle_ready =
            ios_services::dtx::encode_dtx(8, 0, 3, false, 2, &bundle_selector, &bundle_aux);
        server.write_all(&bundle_ready).await.unwrap();

        let runner_selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_XCT_testRunnerReadyWithCapabilities:",
        );
        let runner_aux = encode_primitive_dict(&[archived_object(
            ios_proto::nskeyedarchiver_encode::archive_xct_capabilities(XctCapabilities {
                capabilities: vec![(
                    "XCTIssue capability".to_string(),
                    plist::Value::Boolean(true),
                )],
            }),
        )]);
        let runner_ready =
            ios_services::dtx::encode_dtx(9, 0, 3, true, 2, &runner_selector, &runner_aux);
        server.write_all(&runner_ready).await.unwrap();

        let response = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(response.identifier, 9);
        assert_eq!(response.conversation_idx, 1);
        match response.payload {
            DtxPayload::Response(value) => {
                assert_xctest_configuration(&value);
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let summary = task.await.unwrap();
        assert_eq!(summary.protocol_version, 36);
        assert_eq!(summary.minimum_version, 25);
    }

    #[tokio::test]
    async fn typed_initiate_session_archives_uuid_and_capabilities() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);
        let session_identifier = Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();

        let send = tokio::spawn(async move {
            testmanager
                .initiate_session_with_capabilities(
                    session_identifier,
                    XctCapabilities {
                        capabilities: vec![(
                            "XCTIssue capability".to_string(),
                            plist::Value::Boolean(true),
                        )],
                    },
                )
                .await
                .unwrap();
        });

        let msg = read_dtx_frame(&mut server).await.unwrap();
        match msg.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_initiateSessionWithIdentifier:capabilities:");
                assert_eq!(args.len(), 2);
                assert_archived_uuid(&args[0], session_identifier);
                assert_archived_capabilities(&args[1]);
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        server
            .write_all(&ios_services::dtx::encode_dtx(
                msg.identifier,
                1,
                3,
                false,
                3,
                &[],
                &[],
            ))
            .await
            .unwrap();

        send.await.unwrap();
    }

    #[tokio::test]
    async fn await_driver_channel_request_binds_default_driver_channel() {
        let (client, mut server) = duplex(4096);
        let mut testmanager = TestmanagerClient::from_session_connection_for_test(client, 3);

        let task = tokio::spawn(async move {
            let channel = testmanager.await_driver_channel_request().await.unwrap();
            assert_eq!(channel, -1);
            testmanager.start_executing_test_plan().await.unwrap();
        });

        let selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_requestChannelWithCode:identifier:",
        );
        let aux = encode_primitive_dict(&[
            ios_services::dtx::PrimArg::Int32(1),
            archived_object(ios_proto::nskeyedarchiver_encode::archive_string(
                "dtxproxy:XCTestDriverInterface:XCTestManager_IDEInterface",
            )),
        ]);
        let inbound_request = ios_services::dtx::encode_dtx(11, 0, 0, true, 2, &selector, &aux);
        server.write_all(&inbound_request).await.unwrap();

        let ack = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(ack.identifier, 11);
        assert_eq!(ack.conversation_idx, 1);

        let start = read_dtx_frame(&mut server).await.unwrap();
        assert_eq!(start.channel_code, -1);
        match start.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_startExecutingTestPlanWithProtocolVersion:");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].as_int(), Some(36));
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        task.await.unwrap();
    }

    #[tokio::test]
    async fn authorize_and_start_test_plan_drives_post_launch_startup() {
        let (session_client, mut session_server) = duplex(8192);
        let (control_client, mut control_server) = duplex(4096);
        let mut testmanager =
            TestmanagerClient::from_connections_for_test(session_client, 3, control_client, 4);

        let configuration = XcTestConfiguration {
            session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            test_bundle_url: NsUrl {
                path: "/private/tmp/WebDriverAgentRunner.xctest".to_string(),
            },
            ide_capabilities: XctCapabilities {
                capabilities: vec![(
                    "XCTIssue capability".to_string(),
                    plist::Value::Boolean(true),
                )],
            },
            automation_framework_path:
                "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                    .to_string(),
            initialize_for_ui_testing: true,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields: Vec::new(),
        };

        let task = tokio::spawn(async move {
            testmanager
                .authorize_and_start_test_plan_with_configuration(4242, configuration)
                .await
                .unwrap()
        });

        let authorize = read_dtx_frame(&mut control_server).await.unwrap();
        assert_eq!(authorize.channel_code, 4);
        match authorize.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_authorizeTestSessionWithProcessID:");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].as_int(), Some(4242));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        control_server
            .write_all(&ios_services::dtx::encode_dtx(
                authorize.identifier,
                1,
                4,
                false,
                3,
                &ios_proto::nskeyedarchiver_encode::archive_bool(true),
                &[],
            ))
            .await
            .unwrap();

        let bundle_selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_XCT_testBundleReadyWithProtocolVersion:minimumVersion:",
        );
        let bundle_aux = encode_primitive_dict(&[
            archived_object(ios_proto::nskeyedarchiver_encode::archive_int(36)),
            archived_object(ios_proto::nskeyedarchiver_encode::archive_int(25)),
        ]);
        let bundle_ready =
            ios_services::dtx::encode_dtx(12, 0, 3, false, 2, &bundle_selector, &bundle_aux);
        session_server.write_all(&bundle_ready).await.unwrap();

        let runner_selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_XCT_testRunnerReadyWithCapabilities:",
        );
        let runner_aux = encode_primitive_dict(&[archived_object(
            ios_proto::nskeyedarchiver_encode::archive_xct_capabilities(XctCapabilities {
                capabilities: vec![(
                    "XCTIssue capability".to_string(),
                    plist::Value::Boolean(true),
                )],
            }),
        )]);
        let runner_ready =
            ios_services::dtx::encode_dtx(13, 0, 3, true, 2, &runner_selector, &runner_aux);
        session_server.write_all(&runner_ready).await.unwrap();

        let runner_response = read_dtx_frame(&mut session_server).await.unwrap();
        assert_eq!(runner_response.identifier, 13);
        assert_eq!(runner_response.conversation_idx, 1);

        let request_selector = ios_proto::nskeyedarchiver_encode::archive_string(
            "_requestChannelWithCode:identifier:",
        );
        let request_aux = encode_primitive_dict(&[
            ios_services::dtx::PrimArg::Int32(1),
            archived_object(ios_proto::nskeyedarchiver_encode::archive_string(
                "dtxproxy:XCTestDriverInterface:XCTestManager_IDEInterface",
            )),
        ]);
        let inbound_request =
            ios_services::dtx::encode_dtx(14, 0, 0, true, 2, &request_selector, &request_aux);
        session_server.write_all(&inbound_request).await.unwrap();

        let ack = read_dtx_frame(&mut session_server).await.unwrap();
        assert_eq!(ack.identifier, 14);
        assert_eq!(ack.conversation_idx, 1);

        let start = read_dtx_frame(&mut session_server).await.unwrap();
        assert_eq!(start.channel_code, -1);
        match start.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_startExecutingTestPlanWithProtocolVersion:");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].as_int(), Some(36));
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let summary = task.await.unwrap();
        assert_eq!(summary.protocol_version, 36);
        assert_eq!(summary.minimum_version, 25);
    }

    #[tokio::test]
    async fn authorize_and_start_test_plan_fails_when_authorization_is_rejected() {
        let (session_client, _session_server) = duplex(8192);
        let (control_client, mut control_server) = duplex(4096);
        let mut testmanager =
            TestmanagerClient::from_connections_for_test(session_client, 3, control_client, 4);

        let configuration = XcTestConfiguration {
            session_identifier: Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap(),
            test_bundle_url: NsUrl {
                path: "/private/tmp/WebDriverAgentRunner.xctest".to_string(),
            },
            ide_capabilities: XctCapabilities {
                capabilities: vec![(
                    "XCTIssue capability".to_string(),
                    plist::Value::Boolean(true),
                )],
            },
            automation_framework_path:
                "/System/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework"
                    .to_string(),
            initialize_for_ui_testing: true,
            report_results_to_ide: true,
            tests_must_run_on_main_thread: true,
            test_timeouts_enabled: false,
            additional_fields: Vec::new(),
        };

        let task = tokio::spawn(async move {
            testmanager
                .authorize_and_start_test_plan_with_configuration(4242, configuration)
                .await
        });

        let authorize = read_dtx_frame(&mut control_server).await.unwrap();
        assert_eq!(authorize.channel_code, 4);
        match authorize.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_authorizeTestSessionWithProcessID:");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].as_int(), Some(4242));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        control_server
            .write_all(&ios_services::dtx::encode_dtx(
                authorize.identifier,
                1,
                4,
                false,
                3,
                &ios_proto::nskeyedarchiver_encode::archive_bool(false),
                &[],
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        match err {
            DtxError::Protocol(message) => {
                assert!(message.contains("rejected test session authorization"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn authorize_test_session_fails_on_non_boolean_reply() {
        let (session_client, _session_server) = duplex(4096);
        let (control_client, mut control_server) = duplex(4096);
        let mut testmanager =
            TestmanagerClient::from_connections_for_test(session_client, 3, control_client, 4);

        let task = tokio::spawn(async move {
            testmanager
                .authorize_test_session_with_process_id(4242)
                .await
        });

        let authorize = read_dtx_frame(&mut control_server).await.unwrap();
        assert_eq!(authorize.channel_code, 4);
        match authorize.payload {
            DtxPayload::MethodInvocation { selector, args } => {
                assert_eq!(selector, "_IDE_authorizeTestSessionWithProcessID:");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0].as_int(), Some(4242));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
        control_server
            .write_all(&ios_services::dtx::encode_dtx(
                authorize.identifier,
                1,
                4,
                false,
                3,
                &ios_proto::nskeyedarchiver_encode::archive_string("selector failed"),
                &[],
            ))
            .await
            .unwrap();

        let err = task.await.unwrap().unwrap_err();
        match err {
            DtxError::Protocol(message) => {
                assert!(message.contains("unexpected authorize test session response"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}

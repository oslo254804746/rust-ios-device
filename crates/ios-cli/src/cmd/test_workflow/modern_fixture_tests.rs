//! Loopback protocol fixture for the same engine used by both XCTest CLIs.
//! It does not create a native tunnel or exercise physical device discovery.
use super::*;
use crate::cmd::runtest::{WorkflowLoopbackFixture, WORKFLOW_LOOPBACK};
use futures_util::FutureExt;
use indexmap::IndexMap;
use ios_core::dtx::{
    archived_object, encode_dtx, encode_primitive_dict, read_dtx_frame, DtxPayload, PrimArg,
};
use ios_core::test_support::{decode_message, encode_message, flags, XpcMessage};
use ios_core::testmanager::workflow::{InstalledAppInfo, TestLaunchPlan};
use ios_core::XpcValue;
use std::panic::AssertUnwindSafe;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const UDID: &str = "loopback-workflow-fixture";
const LIST: &str = "com.apple.coredevice.feature.listprocesses";
const LAUNCH: &str = "com.apple.coredevice.feature.launchapplication";
const KILL: &str = "com.apple.coredevice.feature.sendsignaltoprocess";

fn dict(items: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn frame(kind: u8, flag: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    output.extend_from_slice(&[kind, flag]);
    output.extend_from_slice(&stream.to_be_bytes());
    output.extend_from_slice(body);
    output
}
async fn read_frame(peer: &mut TcpStream) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let n = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
    assert!(n <= 1024 * 1024);
    let mut body = vec![0; n];
    peer.read_exact(&mut body).await.unwrap();
    (
        header[3],
        header[4],
        u32::from_be_bytes(header[5..9].try_into().unwrap()),
        body,
    )
}
async fn xpc_reply(peer: &mut TcpStream, stream: u32, id: u64, output: Option<XpcValue>) {
    let message = XpcMessage {
        flags: flags::ALWAYS_SET
            | if output.is_some() {
                flags::DATA | flags::REPLY
            } else {
                0
            },
        msg_id: id,
        body: output.map(|value| dict([("CoreDevice.output", value)])),
    };
    peer.write_all(&frame(0, 0, stream, &encode_message(&message).unwrap()))
        .await
        .unwrap();
}
async fn xpc_request(peer: &mut TcpStream) -> XpcMessage {
    loop {
        let (kind, _, _, body) = read_frame(peer).await;
        if kind == 0 {
            return decode_message(body.into()).unwrap();
        }
    }
}
async fn rsd_peer(listener: TcpListener, dtx_port: u16, xpc_port: u16, shim: bool) {
    for _ in 0..2 {
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        assert_eq!(read_frame(&mut peer).await.0, 4);
        assert_eq!(read_frame(&mut peer).await.0, 8);
        peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
        for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
            let got = read_frame(&mut peer).await;
            assert_eq!((got.0, got.2), (kind, stream));
        }
        let ack = read_frame(&mut peer).await;
        assert_eq!((ack.0, ack.1, ack.2), (4, 1, 0));
        let host = read_frame(&mut peer).await;
        assert_eq!((host.0, host.2), (0, 1));
        let host = decode_message(host.3.into()).unwrap();
        assert_eq!(
            host.body.unwrap().as_dict().unwrap()["MessageType"].as_str(),
            Some("Handshake")
        );
        let dtx_name = if shim {
            format!("{}.shim.remote", ios_core::testmanager::SERVICE_IOS17)
        } else {
            ios_core::testmanager::SERVICE_IOS17.into()
        };
        let mut services = IndexMap::new();
        services.insert(
            dtx_name,
            dict([("Port", XpcValue::Uint64(dtx_port.into()))]),
        );
        services.insert(
            ios_core::apps::APPSERVICE_SERVICE.into(),
            dict([
                ("Port", XpcValue::Uint64(xpc_port.into())),
                (
                    "Properties",
                    dict([(
                        "Features",
                        XpcValue::Array(
                            [LIST, LAUNCH, KILL]
                                .into_iter()
                                .map(|s| XpcValue::String(s.into()))
                                .collect(),
                        ),
                    )]),
                ),
            ]),
        );
        let message = XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA,
            msg_id: 0,
            body: Some(dict([
                ("MessageType", XpcValue::String("Handshake".into())),
                (
                    "Properties",
                    dict([("UniqueDeviceID", XpcValue::String(UDID.into()))]),
                ),
                ("Services", XpcValue::Dictionary(services)),
            ])),
        };
        peer.write_all(&frame(0, 0, 1, &encode_message(&message).unwrap()))
            .await
            .unwrap();
    }
}
async fn readiness_peer(listener: TcpListener) {
    let (mut peer, _) = listener.accept().await.unwrap();
    let request: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
    assert_eq!(
        request["Command"].as_string(),
        Some("QueryDeveloperModeStatus")
    );
    ios_core::send_lockdown(
        &mut peer,
        &plist::Dictionary::from_iter([("DeveloperModeStatus", plist::Value::Boolean(true))]),
    )
    .await
    .unwrap();
    let request: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
    assert_eq!(request["Command"].as_string(), Some("LookupImage"));
    assert_eq!(request["ImageType"].as_string(), Some("Personalized"));
    ios_core::send_lockdown(
        &mut peer,
        &plist::Dictionary::from_iter([(
            "ImageSignature",
            plist::Value::Array(vec![plist::Value::Data(vec![1])]),
        )]),
    )
    .await
    .unwrap();
}
async fn app_peer(listener: TcpListener, killed: Arc<AtomicBool>) {
    let (mut peer, _) = listener.accept().await.unwrap();
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
    peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    let mut data = 0;
    while data < 3 {
        let (kind, _, stream, bytes) = read_frame(&mut peer).await;
        if kind == 0 {
            decode_message(bytes.into()).unwrap();
            xpc_reply(&mut peer, stream, 0, None).await;
            data += 1;
        }
    }
    let list = xpc_request(&mut peer).await;
    assert_eq!(
        list.body.as_ref().unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(),
        Some(LIST)
    );
    xpc_reply(
        &mut peer,
        3,
        list.msg_id,
        Some(dict([("processTokens", XpcValue::Array(Vec::new()))])),
    )
    .await;
    let launch = xpc_request(&mut peer).await;
    let body = launch.body.as_ref().unwrap().as_dict().unwrap();
    assert_eq!(body["CoreDevice.featureIdentifier"].as_str(), Some(LAUNCH));
    assert_eq!(
        body["CoreDevice.input"].as_dict().unwrap()["applicationSpecifier"]
            .as_dict()
            .unwrap()["bundleIdentifier"]
            .as_dict()
            .unwrap()["_0"]
            .as_str(),
        Some("example.runner")
    );
    xpc_reply(
        &mut peer,
        3,
        launch.msg_id,
        Some(dict([("processIdentifier", XpcValue::Uint64(42))])),
    )
    .await;
    let kill = xpc_request(&mut peer).await;
    let body = kill.body.as_ref().unwrap().as_dict().unwrap();
    assert_eq!(body["CoreDevice.featureIdentifier"].as_str(), Some(KILL));
    assert_eq!(
        body["CoreDevice.input"].as_dict().unwrap()["process"]
            .as_dict()
            .unwrap()["processIdentifier"],
        XpcValue::Int64(42)
    );
    killed.store(true, Ordering::SeqCst);
    xpc_reply(
        &mut peer,
        3,
        kill.msg_id,
        Some(XpcValue::Dictionary(IndexMap::new())),
    )
    .await;
    let mut tail = Vec::new();
    peer.read_to_end(&mut tail).await.unwrap();
    while !tail.is_empty() {
        assert!(tail.len() >= 9);
        assert_ne!(tail[3], 0, "no additional AppService request after kill");
        let size = u32::from_be_bytes([0, tail[0], tail[1], tail[2]]) as usize;
        tail.drain(..9 + size);
    }
}
async fn checkin(peer: &mut TcpStream, shim: bool) {
    if shim {
        let request: plist::Dictionary = ios_core::recv_lockdown(peer).await.unwrap();
        assert_eq!(request["Request"].as_string(), Some("RSDCheckin"));
        for value in ["RSDCheckin", "StartService"] {
            ios_core::send_lockdown(
                peer,
                &plist::Dictionary::from_iter([("Request", plist::Value::String(value.into()))]),
            )
            .await
            .unwrap();
        }
    }
}
async fn dtx_call(peer: &mut TcpStream, expected: &str, response: &[u8]) {
    let request = read_dtx_frame(peer).await.unwrap();
    let DtxPayload::MethodInvocation { selector, args } = &request.payload else {
        panic!("expected DTX invocation")
    };
    assert_eq!(selector, expected);
    if expected == ios_core::testmanager::REQUEST_CHANNEL_SELECTOR {
        assert_eq!(args[0].as_int(), Some(1));
        assert_eq!(
            args[1].as_str(),
            Some(ios_core::testmanager::DAEMON_CONNECTION_INTERFACE)
        );
    }
    if expected == ios_core::testmanager::AUTHORIZE_TEST_SESSION_SELECTOR {
        assert_eq!(args[0].as_int(), Some(42));
    }
    peer.write_all(&encode_dtx(
        request.identifier,
        1,
        request.channel_code,
        false,
        3,
        response,
        &[],
    ))
    .await
    .unwrap();
}
async fn result_event(peer: &mut TcpStream, id: u32, selector: &str, args: &[PrimArg]) {
    let selector = ios_core::archive_string(selector);
    let args = encode_primitive_dict(args);
    peer.write_all(&encode_dtx(id, 0, -1, false, 2, &selector, &args))
        .await
        .unwrap();
}
async fn dtx_peer(listener: TcpListener, shim: bool, authorize: bool, killed: Arc<AtomicBool>) {
    let (mut session, _) = listener.accept().await.unwrap();
    checkin(&mut session, shim).await;
    let (mut control, _) = listener.accept().await.unwrap();
    checkin(&mut control, shim).await;
    dtx_call(&mut session, "_requestChannelWithCode:identifier:", &[]).await;
    dtx_call(&mut control, "_requestChannelWithCode:identifier:", &[]).await;
    dtx_call(
        &mut session,
        ios_core::testmanager::INITIATE_SESSION_SELECTOR,
        &[],
    )
    .await;
    dtx_call(
        &mut control,
        ios_core::testmanager::INITIATE_CONTROL_SESSION_SELECTOR,
        &[],
    )
    .await;
    dtx_call(
        &mut control,
        ios_core::testmanager::AUTHORIZE_TEST_SESSION_SELECTOR,
        &ios_core::archive_bool(authorize),
    )
    .await;
    if authorize {
        let bundle = ios_core::archive_string(ios_core::testmanager::TEST_BUNDLE_READY_SELECTOR);
        let args = encode_primitive_dict(&[
            archived_object(ios_core::archive_int(36)),
            archived_object(ios_core::archive_int(25)),
        ]);
        session
            .write_all(&encode_dtx(12, 0, 1, false, 2, &bundle, &args))
            .await
            .unwrap();
        let ready = ios_core::archive_string(ios_core::testmanager::TEST_RUNNER_READY_SELECTOR);
        session
            .write_all(&encode_dtx(13, 0, 1, true, 2, &ready, &[]))
            .await
            .unwrap();
        assert_eq!(read_dtx_frame(&mut session).await.unwrap().identifier, 13);
        let selector = ios_core::archive_string(ios_core::testmanager::REQUEST_CHANNEL_SELECTOR);
        let args = encode_primitive_dict(&[
            PrimArg::Int32(1),
            archived_object(ios_core::archive_string(
                ios_core::testmanager::DRIVER_INTERFACE,
            )),
        ]);
        session
            .write_all(&encode_dtx(14, 0, 0, true, 2, &selector, &args))
            .await
            .unwrap();
        assert_eq!(read_dtx_frame(&mut session).await.unwrap().identifier, 14);
        let start = read_dtx_frame(&mut session).await.unwrap();
        assert!(
            matches!(start.payload, DtxPayload::MethodInvocation { ref selector, .. } if selector == ios_core::testmanager::START_EXECUTING_SELECTOR)
        );
        use ios_core::testmanager::results::*;
        result_event(
            &mut session,
            16,
            DID_BEGIN_EXECUTING_TEST_PLAN_SELECTOR,
            &[],
        )
        .await;
        result_event(
            &mut session,
            17,
            TEST_SUITE_STARTED_SELECTOR,
            &[
                archived_object(ios_core::archive_string("LoginTests")),
                archived_object(ios_core::archive_string("2026-10-27 10:00:00 +0000")),
            ],
        )
        .await;
        result_event(
            &mut session,
            18,
            TEST_CASE_STARTED_SELECTOR,
            &[
                archived_object(ios_core::archive_string("LoginTests")),
                archived_object(ios_core::archive_string("testHappyPath")),
            ],
        )
        .await;
        result_event(
            &mut session,
            19,
            TEST_CASE_FINISHED_SELECTOR,
            &[
                archived_object(ios_core::archive_string("LoginTests")),
                archived_object(ios_core::archive_string("testHappyPath")),
                archived_object(ios_core::archive_string("passed")),
                PrimArg::Double(1.25),
            ],
        )
        .await;
        result_event(
            &mut session,
            20,
            TEST_SUITE_FINISHED_SELECTOR,
            &[
                archived_object(ios_core::archive_string("LoginTests")),
                archived_object(ios_core::archive_string("2026-10-27 10:00:01 +0000")),
                PrimArg::Int64(1),
                PrimArg::Int64(0),
                PrimArg::Int64(0),
                PrimArg::Double(1.25),
                PrimArg::Double(1.25),
            ],
        )
        .await;
        result_event(
            &mut session,
            21,
            DID_FINISH_EXECUTING_TEST_PLAN_SELECTOR,
            &[],
        )
        .await;
    }
    let (mut session_read, mut session_write) = session.into_split();
    let (mut control_read, mut control_write) = control.into_split();
    let finish_session = async {
        let mut tail = Vec::new();
        session_read.read_to_end(&mut tail).await.unwrap();
        assert!(
            killed.load(Ordering::SeqCst),
            "PID kill must precede DTX shutdown"
        );
        session_write.shutdown().await.unwrap();
    };
    let finish_control = async {
        let mut tail = Vec::new();
        control_read.read_to_end(&mut tail).await.unwrap();
        control_write.shutdown().await.unwrap();
    };
    tokio::join!(finish_session, finish_control);
}

async fn listener() -> TcpListener {
    TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, 0))
        .await
        .unwrap()
}

#[tokio::test]
async fn active_fixture_rejects_other_targets_before_any_io() {
    let rsd = listener().await;
    let readiness = listener().await;
    let fixture = WorkflowLoopbackFixture {
        udid: UDID.into(),
        rsd_port: rsd.local_addr().unwrap().port(),
        readiness_port: readiness.local_addr().unwrap().port(),
    };
    WORKFLOW_LOOPBACK
        .scope(fixture, async {
            let error = crate::cmd::runtest::workflow_fixture_for("other-target").unwrap_err();
            assert!(error.to_string().contains("fixture target mismatch"));
            let error =
                preflight_device_readiness("other-target", Instant::now() + Duration::from_secs(1))
                    .await
                    .unwrap_err();
            assert!(error.to_string().contains("fixture target mismatch"));
            let native = crate::cmd::tunnel::NativeScope::new().unwrap();
            let error = match crate::cmd::runtest::connect_testmanager_device_scoped(
                "other-target",
                &native,
            )
            .await
            {
                Ok(_) => panic!("mismatched target was connected"),
                Err(error) => error,
            };
            assert!(error.to_string().contains("fixture target mismatch"));
            native.close(Duration::from_secs(1)).await.unwrap();
        })
        .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), rsd.accept())
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), readiness.accept())
            .await
            .is_err()
    );
}

async fn run_fixture(shim: bool, authorize: bool) {
    let rsd = listener().await;
    let dtx = listener().await;
    let app = listener().await;
    let readiness = listener().await;
    let rsd_port = rsd.local_addr().unwrap().port();
    let readiness_port = readiness.local_addr().unwrap().port();
    let killed = Arc::new(AtomicBool::new(false));
    let mut peers = tokio::task::JoinSet::new();
    peers.spawn(rsd_peer(
        rsd,
        dtx.local_addr().unwrap().port(),
        app.local_addr().unwrap().port(),
        shim,
    ));
    peers.spawn(readiness_peer(readiness));
    peers.spawn(app_peer(app, killed.clone()));
    peers.spawn(dtx_peer(dtx, shim, authorize, killed));
    let directory = std::env::temp_dir().join(format!("ios-workflow-{}", uuid::Uuid::new_v4()));
    let scenario = AssertUnwindSafe(async {
        std::fs::create_dir(&directory).unwrap();
        let junit = directory.join("results.xml");
        let workflow = TestWorkflowArgs::default();
        let engine = TestWorkflowEngine::new(
            UDID,
            &workflow,
            Instant::now() + Duration::from_secs(20),
            true,
            Some(&junit),
        );
        let options = TestWorkflowRunOptions {
            startup_timeout: Duration::from_secs(10),
            result_timeout: Duration::from_secs(10),
            json_output: false,
            untested_banner: false,
            untested_receipt: false,
            monitored_bundle_ids: Vec::new(),
        };
        let result = WORKFLOW_LOOPBACK
            .scope(
                WorkflowLoopbackFixture {
                    udid: UDID.into(),
                    rsd_port,
                    readiness_port,
                },
                engine.run(options, |_| {
                    Box::pin(async {
                        Ok((
                            "fixture".into(),
                            TestLaunchPlan {
                                runner: InstalledAppInfo {
                                    bundle_id: "example.runner".into(),
                                    path: "/Applications/Runner.app".into(),
                                    executable: "Runner".into(),
                                    container: None,
                                },
                                target: None,
                                xctest_bundle_name: "RunnerTests.xctest".into(),
                                is_xctest: true,
                                args: Vec::new(),
                                env: Default::default(),
                                tests_to_run: Vec::new(),
                                tests_to_skip: Vec::new(),
                            },
                        ))
                    })
                }),
            )
            .await;
        if authorize {
            result.unwrap();
        } else {
            let error = result.unwrap_err();
            assert!(error.chain().any(|cause| cause
                .to_string()
                .contains("rejected test session authorization")));
        }
        let report = std::fs::read_to_string(&junit).unwrap();
        assert!(report.contains("testsuite"));
        let attempts: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.join("results.xml.attempts.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(attempts["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(
            attempts["attempts"][0]["state"],
            if authorize {
                "completed"
            } else {
                "interrupted"
            }
        );
        if authorize {
            assert!(report.contains("<testsuites name=\"XCTest\" tests=\"1\" failures=\"0\""));
            assert!(report.contains("<testcase classname=\"LoginTests\" name=\"testHappyPath\""));
            let summary = &attempts["attempts"][0]["summary"];
            assert_eq!(summary["began"], true);
            assert_eq!(summary["finished"], true);
            assert_eq!(summary["total_tests"], 1);
            assert_eq!(summary["failed_tests"], 0);
            assert_eq!(summary["suites"][0]["cases"][0]["class_name"], "LoginTests");
            assert_eq!(
                summary["suites"][0]["cases"][0]["method_name"],
                "testHappyPath"
            );
            assert_eq!(summary["suites"][0]["cases"][0]["status"], "passed");
        }
        let peer_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(peer_deadline, peers.join_next()).await {
                Ok(Some(result)) => result.unwrap(),
                Ok(None) => break,
                Err(error) => panic!(
                    "loopback peers failed to close within their independent deadline: {error}"
                ),
            }
        }
    });
    // This is a test safety bound, not a replacement for production cleanup.
    let outcome = tokio::time::timeout(Duration::from_secs(35), scenario.catch_unwind()).await;
    peers.abort_all();
    while peers.join_next().await.is_some() {}
    let remove = if directory.exists() {
        std::fs::remove_dir_all(&directory)
    } else {
        Ok(())
    };
    match outcome {
        Ok(Ok(())) => remove.unwrap(),
        Ok(Err(panic)) => std::panic::resume_unwind(panic),
        Err(error) => panic!("loopback workflow fixture exceeded its safety deadline: {error}"),
    }
}

#[tokio::test]
async fn modern_workflow_canonical_and_shim_success_and_authorization_failure() {
    run_fixture(false, true).await;
    run_fixture(true, true).await;
    run_fixture(true, false).await;
}

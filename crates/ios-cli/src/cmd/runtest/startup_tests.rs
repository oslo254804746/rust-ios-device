//! Actual AppService XPC requests through the production startup cleanup owner.
use super::*;
use indexmap::IndexMap;
use ios_core::test_support::{decode_message, encode_message, flags, XpcMessage};
use ios_core::{XpcClient, XpcValue};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

fn dict(values: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(values.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
async fn frame(peer: &mut DuplexStream) -> (u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let size = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
    assert!(size <= 64 * 1024);
    let mut data = vec![0; size];
    peer.read_exact(&mut data).await.unwrap();
    (
        header[3],
        u32::from_be_bytes(header[5..9].try_into().unwrap()),
        data,
    )
}
async fn send(peer: &mut DuplexStream, stream: u32, message: XpcMessage) {
    let bytes = encode_message(&message).unwrap();
    let mut header = [0u8; 9];
    header[..3].copy_from_slice(&(bytes.len() as u32).to_be_bytes()[1..]);
    header[5..].copy_from_slice(&stream.to_be_bytes());
    peer.write_all(&header).await.unwrap();
    peer.write_all(&bytes).await.unwrap();
}
async fn pair() -> (XpcClient, DuplexStream) {
    let (stream, mut peer) = tokio::io::duplex(64 * 1024);
    let (client, ()) = tokio::join!(XpcClient::connect_stream(stream), async {
        let mut preface = [0; 24];
        peer.read_exact(&mut preface).await.unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        peer.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0]).await.unwrap();
        let mut data_frames = 0;
        while data_frames < 3 {
            let (kind, stream, bytes) = frame(&mut peer).await;
            if kind == 0 {
                decode_message(bytes.into()).unwrap();
                send(
                    &mut peer,
                    stream,
                    XpcMessage {
                        flags: flags::ALWAYS_SET,
                        msg_id: 0,
                        body: None,
                    },
                )
                .await;
                data_frames += 1;
            }
        }
    });
    (client.unwrap(), peer)
}
async fn request(peer: &mut DuplexStream) -> XpcMessage {
    loop {
        let (kind, _, data) = frame(peer).await;
        if kind == 0 {
            return decode_message(data.into()).unwrap();
        }
    }
}
async fn reply(peer: &mut DuplexStream, id: u64, output: XpcValue) {
    send(
        peer,
        3,
        XpcMessage {
            flags: flags::ALWAYS_SET | flags::DATA | flags::REPLY,
            msg_id: id,
            body: Some(dict([("CoreDevice.output", output)])),
        },
    )
    .await;
}
fn processes(ids: &[u64]) -> XpcValue {
    dict([(
        "processTokens",
        XpcValue::Array(
            ids.iter()
                .map(|id| {
                    dict([
                        ("processIdentifier", XpcValue::Uint64(*id)),
                        (
                            "bundleIdentifier",
                            XpcValue::String("example.runner".into()),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}
async fn peer_eof(peer: &mut DuplexStream) {
    let mut tail = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    // Window updates may precede EOF; a subsequent application DATA request may not.
    while !tail.is_empty() {
        assert!(tail.len() >= 9);
        assert_ne!(tail[3], 0);
        let n = u32::from_be_bytes([0, tail[0], tail[1], tail[2]]) as usize;
        tail.drain(..9 + n);
    }
}

fn plan() -> TestLaunchPlan {
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
        args: vec!["fixture-argument".into()],
        env: HashMap::new(),
        tests_to_run: Vec::new(),
        tests_to_skip: Vec::new(),
    }
}

#[tokio::test]
async fn submitted_launch_cancel_reconnects_original_factory_and_kills_only_unique_new_pid() {
    let (original, mut peer) = pair().await;
    let (fresh, mut cleanup_peer) = pair().await;
    let fresh = Arc::new(Mutex::new(Some(fresh)));
    let original = original.with_reconnector(move || {
        let fresh = fresh.clone();
        async move { Ok(fresh.lock().unwrap().take().unwrap()) }
    });
    let mut cleanup = TestStartupCleanup {
        controller: Some(RunnerController::CoreDevice(Box::new(
            AppServiceClient::new_with_features(
                original,
                "selected-device",
                [
                    "com.apple.coredevice.feature.listprocesses",
                    "com.apple.coredevice.feature.launchapplication",
                    "com.apple.coredevice.feature.sendsignaltoprocess",
                ],
            ),
        ))),
        ..Default::default()
    };
    {
        let plan = plan();
        let env = HashMap::new();
        let launched = launch_coredevice_runner(
            &mut cleanup.controller,
            &mut cleanup.pending_runner,
            &mut cleanup.launch_submitted,
            &mut cleanup.pid,
            &plan,
            &env,
        );
        tokio::pin!(launched);
        tokio::select! {
            result = &mut launched => panic!("baseline should be pending: {result:?}"),
            message = request(&mut peer) => {
                assert_eq!(message.body.as_ref().unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(), Some("com.apple.coredevice.feature.listprocesses"));
                reply(&mut peer, message.msg_id, processes(&[7])).await;
            }
        }
        tokio::select! {
            result = &mut launched => panic!("PID should be unconfirmed: {result:?}"),
            message = request(&mut peer) => {
                assert_eq!(message.body.as_ref().unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(),Some("com.apple.coredevice.feature.launchapplication"));
            }
        }
    }
    assert!(cleanup.launch_submitted);
    assert!(cleanup.pid.is_none());
    let remote = tokio::spawn(async move {
        let list = request(&mut cleanup_peer).await;
        assert_eq!(
            list.body.as_ref().unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(),
            Some("com.apple.coredevice.feature.listprocesses")
        );
        reply(&mut cleanup_peer, list.msg_id, processes(&[7, 8])).await;
        let kill = request(&mut cleanup_peer).await;
        let body = kill.body.as_ref().unwrap().as_dict().unwrap();
        assert_eq!(
            body["CoreDevice.featureIdentifier"].as_str(),
            Some("com.apple.coredevice.feature.sendsignaltoprocess")
        );
        let input = body["CoreDevice.input"].as_dict().unwrap();
        assert_eq!(
            input["process"].as_dict().unwrap()["processIdentifier"],
            XpcValue::Int64(8)
        );
        reply(
            &mut cleanup_peer,
            kill.msg_id,
            XpcValue::Dictionary(IndexMap::new()),
        )
        .await;
        peer_eof(&mut cleanup_peer).await;
    });
    cleanup.close().await.unwrap();
    peer_eof(&mut peer).await;
    remote.await.unwrap();
}

#[tokio::test]
async fn ambiguous_reconciliation_keeps_unknown_effect_and_never_kills_a_guessed_pid() {
    let (original, mut peer) = pair().await;
    let (fresh, mut cleanup_peer) = pair().await;
    let fresh = Arc::new(Mutex::new(Some(fresh)));
    let original = original.with_reconnector(move || {
        let fresh = fresh.clone();
        async move { Ok(fresh.lock().unwrap().take().unwrap()) }
    });
    let mut cleanup = TestStartupCleanup {
        controller: Some(RunnerController::CoreDevice(Box::new(
            AppServiceClient::new(original, "selected-device"),
        ))),
        pending_runner: Some(("example.runner".into(), Some(vec![7]))),
        launch_submitted: true,
        ..Default::default()
    };
    let remote = tokio::spawn(async move {
        let list = request(&mut cleanup_peer).await;
        reply(&mut cleanup_peer, list.msg_id, processes(&[7, 8, 9])).await;
        peer_eof(&mut cleanup_peer).await;
    });
    let error = cleanup.close().await.unwrap_err();
    assert!(error.downcast_ref::<RunnerLaunchUnconfirmed>().is_some());
    peer_eof(&mut peer).await;
    remote.await.unwrap();
}

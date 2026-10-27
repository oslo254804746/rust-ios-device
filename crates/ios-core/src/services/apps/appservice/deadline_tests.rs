use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, DuplexStream};

async fn request(peer: &mut DuplexStream) -> XpcMessage {
    loop {
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let size = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
        assert!(size <= 64 * 1024);
        let mut body = vec![0; size];
        peer.read_exact(&mut body).await.unwrap();
        if header[3] == 0 {
            return crate::decode_xpc_message(body.into()).unwrap();
        }
    }
}

fn original_elapsed(error: &AppServiceError) {
    let AppServiceError::Xpc(XpcError::Io(io)) = error else {
        panic!("native timeout must retain its XPC IO boundary")
    };
    assert_eq!(io.kind(), std::io::ErrorKind::TimedOut);
    assert!(io.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
}

#[tokio::test(start_paused = true)]
async fn finite_call_zero_budget_does_no_io_and_submitted_launch_timeout_invalidates() {
    let (xpc, mut peer, _open) = crate::test_native::connected_xpc_pair().await;
    let mut client = AppServiceClient::new(xpc, "selected-device");
    client.set_request_timeout(Some(Duration::ZERO));
    let error = client.kill_process(42).await.unwrap_err();
    assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
    assert!(client.is_usable());
    client.set_request_timeout(Some(Duration::from_millis(100)));
    let remote = tokio::spawn(async move {
        let launch = request(&mut peer).await;
        assert_eq!(
            launch.body.unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(),
            Some(FEATURE_LAUNCH_APPLICATION)
        );
        peer
    });
    let started = tokio::time::Instant::now();
    let mut submitted = false;
    let error = client
        .launch_application_observed(
            "example.runner",
            &LaunchApplicationOptions::default(),
            &mut submitted,
        )
        .await
        .unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_millis(100));
    assert!(submitted);
    assert!(!client.is_usable());
    original_elapsed(&error);
    let mut peer = remote.await.unwrap();
    drop(client);
    let mut tail = Vec::new();
    peer.read_to_end(&mut tail).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn streaming_app_collection_uses_the_total_but_monitoring_keeps_its_idle_policy() {
    let (xpc, mut peer, _) = crate::test_native::connected_xpc_pair().await;
    let mut client =
        AppServiceClient::new_with_features(xpc, "selected-device", [FEATURE_STREAM_APPS]);
    client.set_request_timeout(Some(Duration::from_millis(100)));
    let remote = tokio::spawn(async move {
        let message = request(&mut peer).await;
        assert_eq!(
            message.body.unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(),
            Some(FEATURE_STREAM_APPS)
        );
        peer
    });
    let error = client
        .list_apps(ListAppsOptions::default())
        .await
        .unwrap_err();
    original_elapsed(&error);
    assert!(!client.is_usable());
    let mut peer = remote.await.unwrap();
    drop(client);
    let mut tail = Vec::new();
    peer.read_to_end(&mut tail).await.unwrap();

    let (xpc, mut peer, _) = crate::test_native::connected_xpc_pair().await;
    let mut client = AppServiceClient::new(xpc, "selected-device");
    client.set_request_timeout(Some(Duration::ZERO));
    {
        let monitor = client.monitor_process_termination(42);
        tokio::pin!(monitor);
        tokio::select! {
            result=&mut monitor=>panic!("finite-call zero budget must not govern monitoring: {result:?}"),
            message=request(&mut peer)=>assert_eq!(message.body.unwrap().as_dict().unwrap()["CoreDevice.featureIdentifier"].as_str(),Some(FEATURE_MONITOR_PROCESS_TERMINATION)),
        }
    }
    assert!(!client.is_usable());
    drop(client);
    let mut tail = Vec::new();
    peer.read_to_end(&mut tail).await.unwrap();
}

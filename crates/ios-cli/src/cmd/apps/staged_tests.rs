use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

async fn request(peer: &mut DuplexStream) -> (u64, u64, Vec<u8>) {
    let mut header = [0; 40];
    peer.read_exact(&mut header).await.unwrap();
    assert_eq!(&header[..8], b"CFA6LPAA");
    let length = u64::from_le_bytes(header[8..16].try_into().unwrap()) as usize;
    assert!((40..=128 * 1024).contains(&length));
    let mut body = vec![0; length - 40];
    peer.read_exact(&mut body).await.unwrap();
    (
        u64::from_le_bytes(header[24..32].try_into().unwrap()),
        u64::from_le_bytes(header[32..40].try_into().unwrap()),
        body,
    )
}
async fn response(peer: &mut DuplexStream, id: u64, opcode: u64, value: u64) {
    let mut header = Vec::from(*b"CFA6LPAA");
    for value in [48, 48, id, opcode, value] {
        header.extend_from_slice(&value.to_le_bytes());
    }
    peer.write_all(&header).await.unwrap();
}

async fn exercise_staged_cleanup(cancel_upload: bool, cleanup_fails: bool) {
    let path = std::env::temp_dir().join(format!("staged-{}.ipa", uuid::Uuid::new_v4()));
    std::fs::write(&path, b"owned IPA payload").unwrap();
    let paths = build_install_paths(&path).unwrap();
    let (stage, mut stage_peer) = tokio::io::duplex(4096);
    let (install, mut install_peer) = tokio::io::duplex(4096);
    let (cleanup, mut cleanup_peer) = tokio::io::duplex(4096);
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    let stage_task = tokio::spawn(async move {
        let mut cancel = Some(cancel);
        for opcode in [9, 8, 8, 13, 16, 20, 24] {
            let (id, actual, _) = request(&mut stage_peer).await;
            assert_eq!(actual, opcode);
            if cancel_upload && opcode == 16 {
                cancel.take().unwrap().send(()).unwrap();
                break;
            }
            response(
                &mut stage_peer,
                id,
                if opcode == 13 { 14 } else { 1 },
                if opcode == 13 { 77 } else { 0 },
            )
            .await;
        }
        // Keep the cancellation sender alive in the deadline scenario.
        let mut remaining = Vec::new();
        stage_peer.read_to_end(&mut remaining).await.unwrap();
        drop(cancel);
    });
    let install_task = tokio::spawn(async move {
        if cancel_upload {
            return;
        }
        let length = install_peer.read_u32().await.unwrap();
        let mut bytes = vec![0; length as usize];
        install_peer.read_exact(&mut bytes).await.unwrap();
        let value = plist::Value::from_reader(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(
            value.as_dictionary().unwrap()["Command"].as_string(),
            Some("Upgrade")
        );
        let mut remaining = Vec::new();
        install_peer.read_to_end(&mut remaining).await.unwrap();
        assert!(remaining.is_empty(), "submitted install is not replayed");
    });
    let expected = [
        paths.temp_remote_path.clone(),
        paths.final_remote_path.clone(),
    ];
    let cleanup_task = tokio::spawn(async move {
        for path in expected {
            let (id, opcode, body) = request(&mut cleanup_peer).await;
            assert_eq!(opcode, 8);
            assert_eq!(body, format!("{path}\0").as_bytes());
            response(&mut cleanup_peer, id, 1, if cleanup_fails { 10 } else { 0 }).await;
        }
        let mut remaining = Vec::new();
        cleanup_peer.read_to_end(&mut remaining).await.unwrap();
        assert!(remaining.is_empty());
    });
    let mut peers = std::collections::VecDeque::from([("com.apple.afc", stage)]);
    if !cancel_upload {
        peers.push_back((ios_core::apps::INSTALLATION_PROXY_SERVICE, install));
    }
    peers.push_back(("com.apple.afc", cleanup));
    let error = install_staged_package(
        |service| {
            let (expected, stream) = peers.pop_front().unwrap();
            assert_eq!(service, expected);
            std::future::ready(Ok(stream))
        },
        &path,
        &paths,
        !cancel_upload,
        Duration::from_millis(150),
        async {
            let _ = cancelled.await;
            if cancel_upload {
                Ok(())
            } else {
                std::future::pending().await
            }
        },
    )
    .await
    .unwrap_err();
    if cancel_upload {
        assert!(error.is::<ios_core::OperationCancelled>());
    } else {
        assert_eq!(
            ios_core::classify_error(error.as_ref()),
            ios_core::ErrorClass::Timeout
        );
    }
    if cleanup_fails {
        let secondary = error
            .downcast_ref::<crate::cmd::test_workflow::WorkflowSecondaryFailure>()
            .unwrap();
        assert!(secondary.error.is::<ios_core::afc::AfcError>());
    }
    assert!(peers.is_empty());
    for task in [stage_task, install_task, cleanup_task] {
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
    }
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn upload_cancellation_reconnects_and_removes_both_staging_paths() {
    exercise_staged_cleanup(true, false).await;
}
#[tokio::test]
async fn submitted_upgrade_deadline_still_cleans_staging_without_replay() {
    exercise_staged_cleanup(false, false).await;
}
#[tokio::test]
async fn upload_cancellation_preserves_independent_cleanup_failure() {
    exercise_staged_cleanup(true, true).await;
}

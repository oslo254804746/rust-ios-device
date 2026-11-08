//! Kernel's private IP composition does not enable selected userspace mode.
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn kernel_private_ip_keeps_public_userspace_selection_unsupported_after_original_handshake() {
    let baseline = tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks();
    let (stream, mut peer) = tokio::io::duplex(4096);
    let peer_task = tokio::spawn(async move {
        let mut header = [0; 10];
        peer.read_exact(&mut header).await.unwrap();
        assert_eq!(&header[..8], b"CDTunnel");
        let length = u16::from_be_bytes([header[8], header[9]]) as usize;
        let mut body = vec![0; length];
        peer.read_exact(&mut body).await.unwrap();
        let response = serde_json::to_vec(&serde_json::json!({
            "serverAddress": "fd00::1", "serverRSDPort": 58783,
            "clientParameters": { "address": "fd00::2", "mtu": 1280 },
        }))
        .unwrap();
        peer.write_all(b"CDTunnel").await.unwrap();
        peer.write_all(&(response.len() as u16).to_be_bytes())
            .await
            .unwrap();
        peer.write_all(&response).await.unwrap();
        peer
    });
    let connection = TunnelConnection::new(
        DeviceInfo {
            udid: "private-engine-feature-fixture".into(),
            device_id: 0,
            connection_type: "Network".into(),
            product_id: 0,
        },
        None,
        LockdownTransport::Tcp {
            host: String::new(),
        },
        "scripted private feature",
    );
    let result = activate_tunnel(connection, stream, TunMode::Userspace, None).await;
    let mut peer = peer_task.await.unwrap();
    let eof = tokio::time::timeout(Duration::from_secs(2), peer.read(&mut [0; 1])).await;
    assert!(matches!(eof, Ok(Ok(0))));
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        baseline
    );
    let Err(error @ CoreError::Unsupported(_)) = result else {
        panic!("private composition enabled a public userspace fallback")
    };
    assert_eq!(error.class(), crate::ErrorClass::Unsupported);
}

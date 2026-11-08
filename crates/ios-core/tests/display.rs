#![cfg(feature = "display")]

use std::net::Ipv6Addr;
use std::time::Duration;

use bytes::Bytes;
use indexmap::IndexMap;
use ios_core::display::{
    DisplayError, DisplayServiceClient, MediaStreamOptions, MEDIA_SUPPORT_FEATURE,
    START_MEDIA_FEATURE, STOP_MEDIA_FEATURE,
};
use ios_core::{
    decode_xpc_message as decode_message, encode_xpc_message as encode_message,
    xpc_message_flags as flags, XpcClient,
};
use ios_core::{XpcMessage, XpcValue};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

fn dict<const N: usize>(entries: [(&str, XpcValue); N]) -> XpcValue {
    XpcValue::Dictionary(
        entries
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

async fn read_frame<S: AsyncRead + Unpin>(stream: &mut S) -> (u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    stream.read_exact(&mut header).await.unwrap();
    let size = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
    assert!(size <= 65536);
    let mut data = vec![0; size];
    stream.read_exact(&mut data).await.unwrap();
    (
        header[3],
        u32::from_be_bytes(header[5..9].try_into().unwrap()),
        data,
    )
}

async fn frame<S: AsyncWrite + Unpin>(stream: &mut S, kind: u8, channel: u32, bytes: &[u8]) {
    let mut header = [0; 9];
    header[..3].copy_from_slice(&(bytes.len() as u32).to_be_bytes()[1..]);
    header[3] = kind;
    header[5..].copy_from_slice(&channel.to_be_bytes());
    stream.write_all(&header).await.unwrap();
    stream.write_all(bytes).await.unwrap();
}

async fn request<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> XpcMessage {
    let mut preface = [0; 24];
    stream.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
    assert_eq!(read_frame(stream).await.0, 4); // settings
    assert_eq!(read_frame(stream).await.0, 8); // window update
    frame(stream, 4, 0, &[]).await;
    let empty = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: 0,
        body: None,
    })
    .unwrap();
    // Acknowledge each of the three RemoteXPC initialization DATA messages.
    let mut initialized = 0;
    loop {
        let (kind, channel, data) = read_frame(stream).await;
        if kind != 0 {
            continue;
        }
        if initialized < 3 {
            frame(stream, 0, channel, &empty).await;
            initialized += 1;
        } else {
            return decode_message(Bytes::from(data)).unwrap();
        }
    }
}

async fn reply<S: AsyncWrite + Unpin>(stream: &mut S, id: u64, body: XpcValue) {
    let bytes = encode_message(&XpcMessage {
        flags: flags::ALWAYS_SET,
        msg_id: id,
        body: Some(body),
    })
    .unwrap();
    frame(stream, 0, 3, &bytes).await;
}

#[tokio::test]
async fn display_start_and_stop_use_distinct_connections_and_stop_request_shape() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let mut connections = Vec::new();
            for feature in [
                MEDIA_SUPPORT_FEATURE,
                START_MEDIA_FEATURE,
                STOP_MEDIA_FEATURE,
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = request(&mut stream).await;
                let body = request.body.unwrap();
                let body = body.as_dict().unwrap();
                assert_eq!(body["CoreDevice.featureIdentifier"].as_str(), Some(feature));
                if feature == STOP_MEDIA_FEATURE {
                    assert_eq!(
                        body["CoreDevice.input"],
                        dict([("stopAll", XpcValue::Bool(true))])
                    );
                }
                reply(
                    &mut stream,
                    request.msg_id,
                    dict([("CoreDevice.output", dict([]))]),
                )
                .await;
                connections.push(stream);
            }
            // No second request may have reached any of the used channels.
            for mut stream in connections {
                let mut remaining = Vec::new();
                stream.read_to_end(&mut remaining).await.unwrap();
                // Flow-control acknowledgements are allowed after a reply.
                let mut cursor = remaining.as_slice();
                while !cursor.is_empty() {
                    assert_ne!(read_frame(&mut cursor).await.0, 0, "reused display channel");
                }
            }
        });
        let xpc = XpcClient::connect(Ipv6Addr::LOCALHOST, port).await.unwrap();
        let mut client = DisplayServiceClient::new(xpc);
        client.get_media_support_info().await.unwrap();
        let start = client
            .start_video_stream(&MediaStreamOptions {
                sender_ip: "fd00::2".into(),
                receiver_ip: "fd00::1".into(),
                receiver_port: 4000,
                ..MediaStreamOptions::default()
            })
            .await
            .unwrap();
        client
            .stop_media_stream(start.client_session_id)
            .await
            .unwrap();
        drop(client);
        server.await.unwrap();
    })
    .await
    .expect("display exchange must finish");
}

#[tokio::test]
async fn display_reports_media_in_use_without_exposing_device_payload() {
    for code in [XpcValue::Int64(9022), XpcValue::Uint64(9022)] {
        let (stream, mut peer) = tokio::io::duplex(32768);
        let server = tokio::spawn(async move {
            let request = request(&mut peer).await;
            reply(
                &mut peer,
                request.msg_id,
                dict([(
                    "CoreDevice.error",
                    dict([
                        ("code", code),
                        (
                            "userInfo",
                            dict([(
                                "NSLocalizedDescription",
                                XpcValue::String("private fixture".into()),
                            )]),
                        ),
                    ]),
                )]),
            )
            .await;
        });
        let xpc = XpcClient::connect_stream(stream).await.unwrap();
        let mut client = DisplayServiceClient::new(xpc);
        let error = client
            .start_video_stream(&MediaStreamOptions {
                sender_ip: "fd00::2".into(),
                receiver_ip: "fd00::1".into(),
                receiver_port: 4000,
                ..MediaStreamOptions::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(error, DisplayError::MediaInUse));
        assert!(!error.to_string().contains("private fixture"));
        // A custom stream without a factory must fail instead of reusing the channel.
        assert!(matches!(
            client.stop_media_streams(true, &[]).await,
            Err(DisplayError::Reconnect(_))
        ));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn targeted_stop_encodes_unsigned_stream_tokens() {
    let (stream, mut peer) = tokio::io::duplex(32768);
    let server = tokio::spawn(async move {
        let request = request(&mut peer).await;
        assert_eq!(
            request.body.unwrap().as_dict().unwrap()["CoreDevice.input"],
            dict([
                ("stopAll", XpcValue::Bool(false)),
                (
                    "identifiers",
                    XpcValue::Array(vec![XpcValue::Uint64(u64::from(u32::MAX))])
                ),
            ])
        );
        reply(
            &mut peer,
            request.msg_id,
            dict([("CoreDevice.output", XpcValue::Dictionary(IndexMap::new()))]),
        )
        .await;
    });
    let xpc = XpcClient::connect_stream(stream).await.unwrap();
    let mut client = DisplayServiceClient::new(xpc);
    assert!(client.stop_media_streams(false, &[]).await.is_err());
    client.stop_media_streams(false, &[u32::MAX]).await.unwrap();
    server.await.unwrap();
}

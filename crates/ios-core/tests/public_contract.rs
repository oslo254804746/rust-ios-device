//! Representative external consumer: no private modules or device I/O.

use std::collections::HashMap;
use std::future::Future;

#[test]
fn lifecycle_backup2_public_budget_state_cleanup_and_send_futures_keep_exhaustive_errors() {
    use ios_core::backup2::{
        BackupOptions, Mobilebackup2Client, Mobilebackup2Error, RestoreOptions,
        SessionCleanupFailure,
    };
    use std::{path::Path, time::Duration};
    use tokio::io::DuplexStream;
    let _: fn(&Mobilebackup2Client<DuplexStream>) -> bool = Mobilebackup2Client::is_usable;
    let _: fn(&mut Mobilebackup2Client<DuplexStream>, Option<Duration>) =
        Mobilebackup2Client::set_operation_timeout;
    let _: fn(&Mobilebackup2Error) -> Option<&SessionCleanupFailure> =
        Mobilebackup2Error::cleanup_failure;
    let _: fn(&SessionCleanupFailure) -> &Mobilebackup2Error =
        SessionCleanupFailure::operation_error;
    let _: fn(&SessionCleanupFailure) -> &Mobilebackup2Error = SessionCleanupFailure::cleanup_error;
    fn send<F: Future + Send>(future: F) {
        drop(future);
    }
    fn operations(client: &mut Mobilebackup2Client<DuplexStream>) {
        let root = Path::new("owned-backup");
        let info = plist::Dictionary::new();
        send(client.version_exchange());
        send(client.backup(root, "owned-device", false, &info));
        send(client.backup_with_options(
            root,
            "owned-device",
            false,
            &info,
            BackupOptions::default(),
        ));
        send(client.change_password(root, "owned-device", None, None));
        send(client.restore(
            root,
            "owned-device",
            RestoreOptions {
                system: false,
                reboot: false,
                copy: false,
                settings: false,
                remove: false,
                password: None,
                source_identifier: None,
            },
        ));
        send(client.info(root, "owned-device", None));
        send(client.list(root, "owned-device", None));
        send(client.unback(root, "owned-device", None, None));
        send(client.extract(root, "owned-device", "HomeDomain", "file", None, None));
    }
    let _: fn(&mut Mobilebackup2Client<DuplexStream>) = operations;
    fn family(error: Mobilebackup2Error) -> &'static str {
        match error {
            Mobilebackup2Error::DeviceLink(_) => "device-link",
            Mobilebackup2Error::Io(_) => "io",
            Mobilebackup2Error::Plist(_) => "plist",
            Mobilebackup2Error::Protocol(_) => "protocol",
        }
    }
    assert_eq!(
        family(Mobilebackup2Error::Protocol("owned".into())),
        "protocol"
    );
}

#[test]
fn lifecycle_device_link_public_state_budget_and_send_futures_keep_exhaustive_errors() {
    use ios_core::device_link::{DeviceLinkClient, DeviceLinkError};
    use std::time::Duration;
    use tokio::io::DuplexStream;
    let _: fn(&DeviceLinkClient<DuplexStream>) -> bool = DeviceLinkClient::is_usable;
    let _: fn(&mut DeviceLinkClient<DuplexStream>, Option<Duration>) =
        DeviceLinkClient::set_operation_timeout;
    let _: fn(DeviceLinkClient<DuplexStream>) -> DuplexStream = DeviceLinkClient::into_inner;
    let _: fn(&mut DeviceLinkClient<DuplexStream>) -> &mut DuplexStream =
        DeviceLinkClient::stream_mut;
    fn send<F: Future + Send>(future: F) {
        drop(future);
    }
    fn operations(client: &mut DeviceLinkClient<DuplexStream>) {
        let message = plist::Dictionary::new();
        send(client.version_exchange());
        send(client.send_message(&message));
        send(client.recv_message());
        send(client.send_process_message(&message));
        send(client.recv_process_message());
        send(client.call_process_message(&message));
        send(client.disconnect());
    }
    let _: fn(&mut DeviceLinkClient<DuplexStream>) = operations;
    fn family(error: DeviceLinkError) -> &'static str {
        match error {
            DeviceLinkError::Io(_) => "io",
            DeviceLinkError::Plist(_) => "plist",
            DeviceLinkError::Protocol(_) => "protocol",
        }
    }
    assert_eq!(
        family(DeviceLinkError::Protocol("owned".into())),
        "protocol"
    );
}

#[test]
fn lifecycle_lockdown_public_state_budget_and_send_future_signatures_keep_exhaustive_errors() {
    use ios_core::{LockdownClient, LockdownError};
    use std::time::Duration;
    let _: fn(&LockdownClient) -> bool = LockdownClient::is_usable;
    let _: fn(&mut LockdownClient, Option<Duration>) = LockdownClient::set_request_timeout;
    fn send<F: Future + Send>(future: F) {
        drop(future);
    }
    fn requests(client: &mut LockdownClient) {
        send(client.get_value(None, None));
        send(client.get_value_preserving_errors(None, None));
        send(client.set_value(None, None, false));
        send(client.remove_value(None, None));
        send(client.start_service("owned-service"));
        send(client.deactivate());
        send(client.stop_session());
        send(client.product_version());
    }
    let _: fn(&mut LockdownClient) = requests;
    fn family(error: LockdownError) -> &'static str {
        match error {
            LockdownError::Io(_) => "io",
            LockdownError::Protocol(_) => "protocol",
            LockdownError::PairRecord(_) => "pair-record",
        }
    }
    assert_eq!(family(LockdownError::Protocol("owned".into())), "protocol");
}

use ios_core::{
    ConnectOptions, CoreError, DeviceEvent, DeviceInfo, RsdHandshake, ServiceDescriptor, TunMode,
    TunnelInfo, XpcMessage, XpcValue,
};

#[cfg(feature = "tunnel")]
#[tokio::test]
async fn lifecycle_xpc_public_deadlines_state_and_exhaustive_error_contract() {
    use ios_core::{encode_xpc_message, xpc_message_flags, XpcClient, XpcError};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn family(error: &XpcError) -> &str {
        match error {
            XpcError::Io(_) => "io",
            XpcError::NotConnected => "not_connected",
            XpcError::ServiceNotFound(_) => "missing",
            XpcError::Tls(_) => "tls",
        }
    }
    async fn frame(peer: &mut tokio::io::DuplexStream) -> (u8, u32, Vec<u8>) {
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let size = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        assert!(size <= 16_384);
        let mut body = vec![0; size];
        peer.read_exact(&mut body).await.unwrap();
        (
            header[3],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            body,
        )
    }
    fn response(id: u64, stream: u32) -> Vec<u8> {
        let message = encode_xpc_message(&XpcMessage {
            flags: xpc_message_flags::ALWAYS_SET,
            msg_id: id,
            body: None,
        })
        .unwrap();
        let mut bytes = vec![0, 0, message.len() as u8, 0, 0];
        bytes.extend_from_slice(&stream.to_be_bytes());
        bytes.extend_from_slice(&message);
        bytes
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (client, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(XpcClient::connect_stream(stream), async {
            let mut preface = [0; 58];
            peer.read_exact(&mut preface).await.unwrap();
            peer.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0]).await.unwrap();
            assert_eq!(frame(&mut peer).await.0, 4);
            for (index, stream) in [1u32, 1, 3].into_iter().enumerate() {
                if index < 2 {
                    assert_eq!(frame(&mut peer).await.0, 1);
                }
                assert_eq!(frame(&mut peer).await.0, 0);
                peer.write_all(&response(0, stream)).await.unwrap();
            }
        })
    })
    .await
    .unwrap();
    let mut client = client.unwrap();
    let error = client
        .call_with_timeout(XpcValue::Bool(true), Duration::ZERO)
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Timeout
    );
    assert!(client.is_usable());
    let error = client
        .recv_with_timeout(Duration::from_millis(5))
        .await
        .unwrap_err();
    let XpcError::Io(io) = &error else {
        panic!("native timer")
    };
    assert!(io.get_ref().unwrap().is::<tokio::time::error::Elapsed>());
    assert!(client.is_usable());
    let (reply, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(
            client.call_with_timeout(XpcValue::Bool(true), Duration::from_secs(1)),
            async {
                let (_, _, bytes) = frame(&mut peer).await;
                let request = ios_core::decode_xpc_message(bytes.into()).unwrap();
                assert_eq!(request.msg_id, 1);
                peer.write_all(&response(1, 3)).await.unwrap();
            }
        )
    })
    .await
    .unwrap();
    assert_eq!(reply.unwrap().msg_id, 1);
    assert!(client.is_usable());
    assert_eq!(
        family(
            &client
                .call_recv_client_server_with_timeout(XpcValue::Bool(true), Duration::ZERO)
                .await
                .unwrap_err()
        ),
        "io"
    );
    assert_eq!(
        family(
            &client
                .recv_client_server_with_timeout(Duration::ZERO)
                .await
                .unwrap_err()
        ),
        "io"
    );
    assert_eq!(
        family(
            &client
                .recv_any_with_timeout(Duration::ZERO)
                .await
                .unwrap_err()
        ),
        "io"
    );
    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(
            client.call_with_timeout(XpcValue::Bool(true), Duration::from_millis(5)),
            async {
                let (_, _, bytes) = frame(&mut peer).await;
                assert_eq!(
                    ios_core::decode_xpc_message(bytes.into()).unwrap().msg_id,
                    2
                );
            }
        )
    })
    .await
    .unwrap();
    assert_eq!(family(&result.unwrap_err()), "io");
    assert!(!client.is_usable());
    peer.write_all(&response(2, 3)).await.unwrap();
    let error = client.recv().await.unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Protocol
    );
}

#[cfg(feature = "tunnel")]
#[tokio::test]
async fn lifecycle_h2_public_xpc_partial_frame_cancellation_rejects_reuse() {
    use ios_core::{encode_xpc_message, xpc_message_flags, XpcClient, XpcError};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn family(error: &XpcError) -> &str {
        match error {
            XpcError::Io(_) => "io",
            XpcError::NotConnected => "not_connected",
            XpcError::ServiceNotFound(_) => "missing",
            XpcError::Tls(_) => "tls",
        }
    }
    async fn read_frame(peer: &mut tokio::io::DuplexStream) -> (u8, u32, Vec<u8>) {
        let mut header = [0; 9];
        peer.read_exact(&mut header).await.unwrap();
        let size = ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
        let mut body = vec![0; size];
        peer.read_exact(&mut body).await.unwrap();
        (
            header[3],
            u32::from_be_bytes(header[5..9].try_into().unwrap()),
            body,
        )
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let (client, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(XpcClient::connect_stream(stream), async {
            let mut bootstrap = [0; 58];
            peer.read_exact(&mut bootstrap).await.unwrap();
            assert_eq!(&bootstrap[..24], b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
            peer.write_all(&[0, 0, 0, 4, 0, 0, 0, 0, 0]).await.unwrap();
            assert_eq!(read_frame(&mut peer).await, (4, 0, vec![]));
            for (index, stream_id) in [1u32, 1, 3].into_iter().enumerate() {
                if index < 2 {
                    let opened_stream = if index == 0 { 1 } else { 3 };
                    assert_eq!(read_frame(&mut peer).await, (1, opened_stream, vec![]));
                }
                let (kind, sent_stream, _) = read_frame(&mut peer).await;
                assert_eq!((kind, sent_stream), (0, stream_id));
                let reply = encode_xpc_message(&XpcMessage {
                    flags: xpc_message_flags::ALWAYS_SET,
                    msg_id: 0,
                    body: None,
                })
                .unwrap();
                let mut frame = vec![0, 0, reply.len() as u8, 0, 0];
                frame.extend_from_slice(&stream_id.to_be_bytes());
                frame.extend_from_slice(&reply);
                peer.write_all(&frame).await.unwrap();
            }
        })
    })
    .await
    .unwrap();
    let mut client = client.unwrap();
    peer.write_all(&[0, 0, 1, 0]).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), client.recv())
            .await
            .is_err()
    );
    let error = client.recv().await.unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Protocol
    );
    assert!(client
        .send(XpcValue::Dictionary(Default::default()))
        .await
        .is_err());
    drop(client);
    let mut byte = [0];
    assert_eq!(peer.read(&mut byte).await.unwrap(), 0);
}

#[cfg(feature = "instruments")]
#[test]
fn lifecycle_fps_public_iterator_borrows_chunk_and_copies_timebase() {
    use ios_core::instruments::{
        iter_frame_commit_timestamps, parse_frame_commit_timestamps, MachTimeInfo,
    };
    let mut chunk = [0u8; 64];
    chunk[..8].copy_from_slice(&12u64.to_le_bytes());
    chunk[48..52].copy_from_slice(&0x3180_0318u32.to_le_bytes());
    let iterator = {
        let info = MachTimeInfo { numer: 5, denom: 2 };
        iter_frame_commit_timestamps(&chunk, &info)
    };
    assert_eq!(iterator.collect::<Vec<_>>(), vec![30]);
    let legacy: Vec<i64> =
        parse_frame_commit_timestamps(&chunk, &MachTimeInfo { numer: 5, denom: 2 });
    assert_eq!(legacy, vec![30]);
}

#[cfg(feature = "afc")]
#[tokio::test]
async fn lifecycle_afc_public_budget_state_and_exhaustive_errors() {
    use ios_core::afc::{AfcClient, AfcError};
    use std::error::Error;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn family(error: &AfcError) -> &str {
        match error {
            AfcError::Io(_) => "io",
            AfcError::Status(_) => "status",
            AfcError::Protocol(_) => "protocol",
        }
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = AfcClient::new(stream);
    client.set_request_timeout(Some(Duration::ZERO));
    let error = client.list_dir("/").await.unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Timeout
    );
    let AfcError::Io(io) = &error else {
        unreachable!()
    };
    assert!(io.source().is_none());
    assert!(client.is_usable());
    client.set_request_timeout(Some(Duration::from_secs(1)));
    let caller = client.list_dir("/");
    let server = async {
        let mut request = [0; 42];
        peer.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[24..32], &1u64.to_le_bytes());
        assert_eq!(&request[40..], b"/\0");
        let mut reply = request[..40].to_vec();
        reply[8..16].copy_from_slice(&46u64.to_le_bytes());
        reply[16..24].copy_from_slice(&40u64.to_le_bytes());
        reply[32..40].copy_from_slice(&2u64.to_le_bytes()); // Data
        reply.extend_from_slice(b"entry\0");
        peer.write_all(&reply).await.unwrap();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(caller, server)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap(), vec!["entry"]);
    assert!(client.is_usable());
    client.set_request_timeout(None);
}

#[cfg(feature = "dtx")]
#[tokio::test]
async fn lifecycle_dtx_public_deadlines_keep_exhaustive_errors_and_success_reuse() {
    use ios_core::dtx::{encode_dtx, read_dtx_frame, DtxConnection, DtxError};
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    fn family(error: &DtxError) -> &str {
        match error {
            DtxError::Io(_) => "io",
            DtxError::BadMagic(_) => "magic",
            DtxError::Protocol(_) => "protocol",
        }
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut connection = DtxConnection::new(stream);
    let zero = connection
        .request_channel_with_timeout("owned service", Duration::ZERO)
        .await
        .unwrap_err();
    assert_eq!(family(&zero), "io");
    assert_eq!(
        ios_core::classify_error(&zero),
        ios_core::ErrorClass::Timeout
    );
    assert!(connection.is_usable());
    let caller =
        connection.method_call_with_timeout(1, "owned operation", &[], Duration::from_secs(1));
    let server = async {
        let request = read_dtx_frame(&mut peer).await.unwrap();
        assert_eq!(request.identifier, 5);
        peer.write_all(&encode_dtx(request.identifier, 1, 1, false, 0, &[], &[]))
            .await
            .unwrap();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(caller, server)
    })
    .await
    .unwrap();
    assert_eq!(result.unwrap().identifier, 5);
    assert!(connection.is_usable());
}

#[tokio::test]
async fn response_codes_public_start_service_keeps_legacy_api_and_owned_observation() {
    use ios_core::{
        classify_error, recv_lockdown, send_lockdown, start_service,
        start_service_preserving_errors, CoreError, ErrorClass, LockdownError,
        LockdownResponseError,
    };
    fn family(error: &LockdownError) -> &str {
        match error {
            LockdownError::Io(_) => "io",
            LockdownError::Protocol(_) => "protocol",
            LockdownError::PairRecord(_) => "pair-record",
        }
    }
    for code in [
        Some("InvalidService"),
        Some("MissingValue"),
        Some("owned-private-InvalidService"),
        None,
    ] {
        let mut requests = Vec::new();
        for preserve in [false, true] {
            let (host, mut peer) = tokio::io::duplex(4096);
            let (mut reader, mut writer) = tokio::io::split(host);
            let caller = async {
                if preserve {
                    start_service_preserving_errors(
                        &mut reader,
                        &mut writer,
                        "owned-private-service",
                    )
                    .await
                } else {
                    start_service(&mut reader, &mut writer, "owned-private-service").await
                }
            };
            let server = async {
                let request: plist::Value = recv_lockdown(&mut peer).await.unwrap();
                let mut reply = plist::Dictionary::from_iter([
                    ("Port".to_owned(), plist::Value::Integer(1234.into())),
                    ("EnableServiceSSL".to_owned(), plist::Value::Boolean(true)),
                ]);
                if let Some(code) = code {
                    reply.insert("Error".into(), plist::Value::String(code.into()));
                }
                send_lockdown(&mut peer, &reply).await.unwrap();
                request
            };
            let (result, request) =
                tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    tokio::join!(caller, server)
                })
                .await
                .unwrap();
            requests.push(request);
            if let Some(code) = code {
                let error = result.unwrap_err();
                assert_eq!(family(&error), if preserve { "io" } else { "protocol" });
                if preserve {
                    let core = CoreError::from(error);
                    let observed = LockdownResponseError::in_error_chain(&core).unwrap();
                    assert_eq!(observed.operation(), "StartService");
                    assert_eq!(observed.code(), code);
                    assert_eq!(
                        classify_error(&core),
                        if code == "InvalidService" {
                            ErrorClass::ServiceMissing
                        } else {
                            ErrorClass::ServiceFailure
                        }
                    );
                    assert!(!format!("{observed:?} {observed} {core:?} {core}")
                        .contains("owned-private"));
                } else {
                    assert_eq!(classify_error(&error), ErrorClass::Protocol);
                    assert!(LockdownResponseError::in_error_chain(&error).is_none());
                    assert_eq!(
                        error.to_string(),
                        format!(
                            "protocol error: StartService 'owned-private-service' failed: {code}"
                        )
                    );
                }
            } else {
                assert_eq!(result.unwrap(), (1234, true));
            }
        }
        assert_eq!(requests[0], requests[1]);
        let request = requests[0].as_dictionary().unwrap();
        assert_eq!(request["Request"].as_string(), Some("StartService"));
        assert_eq!(
            request["Service"].as_string(),
            Some("owned-private-service")
        );
        assert_eq!(request["Label"].as_string(), Some("ios-rs"));
    }
}

#[cfg(feature = "afc")]
#[tokio::test]
async fn public_guarded_sources_afc_rejects_received_length_with_owned_stream() {
    use ios_core::afc::{AfcClient, AfcError};
    use std::error::Error;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn family(error: &AfcError) -> &str {
        match error {
            AfcError::Io(_) => "io",
            AfcError::Status(_) => "status",
            AfcError::Protocol(_) => "protocol",
        }
    }
    let (stream, mut peer) = tokio::io::duplex(256);
    let caller = async move {
        let mut client = AfcClient::new(stream);
        let error = client.list_dir("/").await.unwrap_err();
        assert_eq!(
            ios_core::classify_error(&error),
            ios_core::ErrorClass::Protocol
        );
        if usize::BITS < 64 {
            assert_eq!(family(&error), "io");
            let AfcError::Io(io) = &error else {
                unreachable!()
            };
            assert!(io.source().unwrap().is::<std::num::TryFromIntError>());
        } else {
            // This host reaches the existing frame ceiling, not the native
            // conversion failure that requires a narrower usize.
            assert_eq!(family(&error), "protocol");
            assert!(error.to_string().contains("frame too large"));
        }
        drop(client);
    };
    let server = async move {
        for field in [0x4141504c36414643_u64, u64::MAX, 40, 1, 2] {
            peer.write_all(&field.to_le_bytes()).await.unwrap();
        }
        peer.shutdown().await.unwrap();
        let mut request = Vec::new();
        peer.read_to_end(&mut request).await.unwrap();
        assert_eq!(request.len(), 42);
        assert_eq!(&request[..8], b"CFA6LPAA");
        assert_eq!(&request[32..40], &3_u64.to_le_bytes());
        assert_eq!(&request[40..], b"/\0");
    };
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        tokio::join!(caller, server);
    })
    .await
    .unwrap();
}

#[cfg(feature = "dproxy")]
#[tokio::test]
async fn public_dproxy_recording_observer_owns_native_error_without_device() {
    use ios_core::dproxy::{
        proxy_bidirectional_with_observer, DecodeFailure, Direction, DproxyError, ProxyProtocol,
        ProxyRecorder,
    };
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory =
        Directory(std::env::temp_dir().join(format!("ios-public-dproxy-{}", uuid::Uuid::new_v4())));
    let mut recorder = ProxyRecorder::new(&directory.0, ProxyProtocol::Lockdown).unwrap();
    let payload = b"bplist00owned-private-invalid-plist";
    assert!(plist::from_bytes::<plist::Value>(payload).is_err());
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    let mut failures: Vec<DecodeFailure> = Vec::new();
    recorder
        .record_chunk_with_observer(Direction::HostToDevice, &bytes, &mut |failure| {
            failures.push(failure)
        })
        .unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0].error_class(),
        Some(ios_core::ErrorClass::Protocol)
    );
    assert!(!format!("{:?}", failures[0]).contains("owned-private"));
    assert!(failures
        .remove(0)
        .into_error()
        .unwrap()
        .is::<plist::Error>());
    fn signature(_: impl Future<Output = Result<(), DproxyError>>) {}
    // The proxy signature is compiled; this future is dropped before polling.
    // Core tests separately run both owned duplex directions and fatal EOF.
    let (local, _) = tokio::io::duplex(64);
    let (remote, _) = tokio::io::duplex(64);
    signature(proxy_bidirectional_with_observer(
        local,
        remote,
        recorder,
        |failure| failures.push(failure),
    ));
}

#[cfg(feature = "dproxy")]
#[test]
fn public_dproxy_observations_preserve_sources_and_legacy_shapes() {
    use ios_core::dproxy::{
        DecodeFailureKind, DecodePhase, Direction, DproxyError, ProxyEvent, ProxyProtocol,
        StreamDecoder,
    };
    fn family(error: &DproxyError) -> &str {
        match error {
            DproxyError::Io(_) => "io",
            DproxyError::Serde(_) => "serde",
            DproxyError::Dtx(_) => "dtx",
            DproxyError::Protocol(_) => "protocol",
        }
    }
    let ProxyEvent {
        timestamp_ms,
        direction,
        protocol,
        summary,
        decoded,
    } = ProxyEvent {
        timestamp_ms: 0,
        direction: "host->device".into(),
        protocol: "lockdown".into(),
        summary: "synthetic".into(),
        decoded: serde_json::Value::Null,
    };
    assert_eq!(
        (
            timestamp_ms,
            direction.as_str(),
            protocol.as_str(),
            summary.as_str(),
            decoded
        ),
        (
            0,
            "host->device",
            "lockdown",
            "synthetic",
            serde_json::Value::Null
        )
    );
    let payload = b"bplist00owned-private-invalid-plist";
    assert!(plist::from_bytes::<plist::Value>(payload).is_err());
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    let result = StreamDecoder::new(ProxyProtocol::Lockdown)
        .push_preserving_errors(Direction::HostToDevice, &bytes);
    assert_eq!(result.failures().len(), 1);
    let failure = &result.failures()[0];
    assert_eq!(failure.phase(), DecodePhase::Lockdown);
    assert_eq!(failure.kind(), DecodeFailureKind::Error);
    assert_eq!(failure.event_index(), Some(0));
    assert_eq!(failure.error_class(), Some(ios_core::ErrorClass::Protocol));
    assert!(failure.error().unwrap().is::<plist::Error>());
    assert!(!format!("{result:?}").contains("owned-private"));
    let (events, mut failures, fatal) = result.into_parts();
    assert_eq!(events.len(), 1);
    assert!(fatal.is_none());
    assert!(failures
        .remove(0)
        .into_error()
        .unwrap()
        .is::<plist::Error>());
    let fatal = StreamDecoder::new(ProxyProtocol::Xpc)
        .push_preserving_errors(Direction::HostToDevice, &[0, 0x40, 1, 0, 0, 0, 0, 0, 3]);
    assert_eq!(family(fatal.fatal_error().unwrap()), "protocol");
    assert_eq!(fatal.failures()[0].kind(), DecodeFailureKind::ResourceLimit);
    assert_eq!(fatal.failures()[0].error_class(), None);
    assert!(fatal.into_result().is_err());
}

#[tokio::test]
async fn root_input_native_gpx_keeps_exhaustive_family_and_preflight_source() {
    use ios_core::{
        simlocation::{parse_gpx_route, replay_gpx_route, GpxRoutePoint, SimLocationError},
        ErrorClass,
    };
    fn family(error: &SimLocationError) -> &str {
        match error {
            SimLocationError::Io(_) => "io",
            SimLocationError::GpxParse(_) => "gpx",
        }
    }
    let GpxRoutePoint {
        latitude,
        longitude,
        delay_from_previous,
    } = GpxRoutePoint {
        latitude: "1".into(),
        longitude: "2".into(),
        delay_from_previous: std::time::Duration::ZERO,
    };
    assert_eq!((latitude.as_str(), longitude.as_str()), ("1", "2"));
    assert!(delay_from_previous.is_zero());
    let gpx = "<gpx><trk><trkseg><trkpt lat=\"owned-private\"/></trkseg></trk></gpx>";
    let parsed = parse_gpx_route(gpx).unwrap_err();
    let (mut stream, mut peer) = tokio::io::duplex(64);
    let replayed = replay_gpx_route(&mut stream, gpx).await.unwrap_err();
    for error in [parsed, replayed] {
        assert_eq!(family(&error), "io");
        assert_eq!(ios_core::classify_error(&error), ErrorClass::InvalidInput);
        let SimLocationError::Io(io) = &error else {
            unreachable!()
        };
        assert!(std::error::Error::source(io)
            .unwrap()
            .is::<quick_xml::DeError>());
        assert!(!format!("{error:?}").contains("owned-private"));
    }
    drop(stream);
    let mut wire = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio::io::AsyncReadExt::read_to_end(&mut peer, &mut wire),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(wire.is_empty());
}

#[cfg(feature = "testmanager")]
#[tokio::test]
async fn root_input_native_test_pid_keeps_exhaustive_dtx_family() {
    use ios_core::{dtx::DtxError, testmanager::TestmanagerClient, ErrorClass};
    fn family(error: &DtxError) -> &str {
        match error {
            DtxError::Io(_) => "io",
            DtxError::BadMagic(_) => "magic",
            DtxError::Protocol(_) => "protocol",
        }
    }
    let (stream, mut peer) = tokio::io::duplex(64);
    let mut client = TestmanagerClient::from_session_connection_for_test(stream, 1);
    let error = client
        .authorize_test_session_with_process_id(u64::MAX)
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(ios_core::classify_error(&error), ErrorClass::InvalidInput);
    let DtxError::Io(io) = &error else {
        unreachable!()
    };
    assert!(std::error::Error::source(io)
        .unwrap()
        .is::<std::num::TryFromIntError>());
    drop(client);
    let mut wire = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        tokio::io::AsyncReadExt::read_to_end(&mut peer, &mut wire),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(wire.is_empty());
}

#[cfg(feature = "restore")]
#[tokio::test]
async fn root_tss_batch_observations_keep_legacy_literals_and_owned_native_errors() {
    use ios_core::{
        restore::{
            tss::{
                prefetch_tss_batch_preserving_errors, sign_tss_request_with_client,
                TssBatchFailureKind, TssBatchPhase, TssBatchResult, TssRider,
            },
            RestoreError,
        },
        ErrorClass,
    };
    use std::{error::Error, future::ready};
    let legacy = TssBatchResult {
        tickets: HashMap::new(),
        failed_updaters: vec![],
    };
    let TssBatchResult {
        tickets,
        failed_updaters,
    } = legacy;
    assert!(tickets.is_empty() && failed_updaters.is_empty());
    let rider = TssRider {
        updater: "owned-private".into(),
        ticket_key: "A,Ticket".into(),
        nonce: vec![1],
        request: plist::Dictionary::from_iter([("@A,Ticket", plist::Value::Boolean(true))]),
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let request =
        plist::Dictionary::from_iter([("owned-private", plist::Value::Uid(plist::Uid::new(1)))]);
    let original = sign_tss_request_with_client(&client, "http://127.0.0.1:1", &request)
        .await
        .unwrap_err();
    let mut original = Some(original);
    let outcome = prefetch_tss_batch_preserving_errors(&[rider], |_| {
        ready(
            original
                .take()
                .map_or_else(|| Ok(plist::Dictionary::new()), Err),
        )
    })
    .await
    .unwrap();
    assert_eq!(outcome.result().failed_updaters, ["owned-private"]);
    assert_eq!(outcome.failures().len(), 2);
    assert_eq!(
        outcome.failures()[0].phase(),
        TssBatchPhase::SignCombinedRequest
    );
    assert_eq!(outcome.failures()[0].kind(), TssBatchFailureKind::Error);
    assert_eq!(outcome.failures()[0].rider_index(), None);
    assert_eq!(
        outcome.failures()[0].error_class(),
        Some(ErrorClass::InvalidInput)
    );
    assert_eq!(
        outcome.failures()[1].kind(),
        TssBatchFailureKind::MissingTicket
    );
    assert_eq!(outcome.failures()[1].rider_index(), Some(0));
    assert!(outcome.failures()[1].error_class().is_none());
    assert!(!format!("{outcome:?} {:?}", outcome.failures()).contains("owned-private"));
    let (_, mut failures) = outcome.into_parts();
    let original = failures.remove(0).into_error().unwrap();
    let family = match &original {
        RestoreError::Io(_) => "io",
        RestoreError::Plist(_) => "plist",
        RestoreError::Protocol(_) => "protocol",
    };
    assert_eq!(family, "io");
    let RestoreError::Io(io) = original else {
        unreachable!()
    };
    assert!(io.source().unwrap().is::<plist::Error>());
}

#[cfg(feature = "restore")]
#[tokio::test]
async fn root_restore_local_package_and_tss_keep_exhaustive_native_sources() {
    use ios_core::{
        restore::{
            preflight::{IpswIdentitySelector, IpswPreflight},
            tss::sign_tss_request_with_client,
            RestoreError,
        },
        ErrorClass,
    };
    use std::{error::Error, io::Cursor};
    fn family(error: &RestoreError) -> &str {
        match error {
            RestoreError::Io(_) => "io",
            RestoreError::Plist(_) => "plist",
            RestoreError::Protocol(_) => "protocol",
        }
    }
    let error = IpswPreflight::open(
        Cursor::new(b"owned-private malformed"),
        IpswIdentitySelector {
            board_id: 1,
            chip_id: 2,
            device_class: "owned",
            restore_behavior: "Update",
        },
    )
    .err()
    .unwrap();
    assert_eq!(family(&error), "io");
    assert_eq!(ios_core::classify_error(&error), ErrorClass::InvalidInput);
    let RestoreError::Io(outer) = &error else {
        unreachable!()
    };
    assert!(outer.source().unwrap().is::<zip::result::ZipError>());
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let request =
        plist::Dictionary::from_iter([("owned-private", plist::Value::Uid(plist::Uid::new(1)))]);
    let error = sign_tss_request_with_client(&client, "http://127.0.0.1:1", &request)
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(ios_core::classify_error(&error), ErrorClass::InvalidInput);
    let RestoreError::Io(outer) = &error else {
        unreachable!()
    };
    assert!(outer.source().unwrap().is::<plist::Error>());
    assert!(!format!("{error:?}").contains("owned-private"));
}

#[tokio::test]
async fn root_mobilebackup2_timeout_evidence_keeps_family_and_issuer_scope() {
    use ios_core::backup2::{
        DeviceOperation, DeviceOperationTimeout, DeviceOperationTimeoutPhase,
        Mobilebackup2Error as E,
    };
    use std::{
        error::Error,
        fmt, io,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };
    fn family(error: &E) -> &str {
        match error {
            E::DeviceLink(_) => "device_link",
            E::Io(_) => "io",
            E::Plist(_) => "plist",
            E::Protocol(_) => "protocol",
        }
    }
    // Verify the downstream observation API without constructing an issued timer.
    fn observation(value: &DeviceOperationTimeout) {
        let _: DeviceOperation = value.operation();
        let _: DeviceOperationTimeoutPhase = value.phase();
        let _: Duration = value.duration();
        let _: Option<&E> = value.operation_error();
        let _: &(dyn Error + Send + Sync) = value;
    }
    let _: fn(&DeviceOperationTimeout) = observation;
    assert_ne!(DeviceOperation::Unback, DeviceOperation::Extract);
    assert_ne!(
        DeviceOperationTimeoutPhase::Operation,
        DeviceOperationTimeoutPhase::Disconnect
    );
    #[derive(Debug)]
    struct Cycle(Arc<AtomicUsize>);
    impl fmt::Display for Cycle {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("issuer lookup must not format source")
        }
    }
    impl Error for Cycle {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Some(self)
        }
    }
    #[derive(Debug, thiserror::Error)]
    #[error("foreign timer")]
    struct Foreign(#[source] tokio::time::error::Elapsed);
    let reads = Arc::new(AtomicUsize::new(0));
    let errors = [
        E::Io(io::Error::new(io::ErrorKind::TimedOut, "foreign IO")),
        E::Io(io::Error::other(Foreign(
            tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
                .await
                .unwrap_err(),
        ))),
        E::Io(io::Error::other(Cycle(Arc::clone(&reads)))),
        E::Protocol("timeout Disconnect Unback".into()),
    ];
    for error in errors {
        let _ = family(&error);
        assert!(error.device_operation_timeout().is_none());
    }
    assert!(reads.load(Ordering::SeqCst) > 0);
    assert!(reads.load(Ordering::SeqCst) <= 32);
}

#[cfg(feature = "imagemounter")]
#[tokio::test]
async fn root_ddi_cache_native_io_retains_exhaustive_family() {
    use ios_core::{
        imagemounter::{DdiDownloader, ImageMounterError},
        ErrorClass,
    };
    use std::{error::Error, path::PathBuf};
    fn family(error: &ImageMounterError) -> &str {
        match error {
            ImageMounterError::Io(_) => "io",
            ImageMounterError::Plist(_) => "plist",
            ImageMounterError::Protocol(_) => "protocol",
            ImageMounterError::DeviceError(_) => "device",
            ImageMounterError::Tss(_) => "tss",
            ImageMounterError::Download(_) => "download",
        }
    }
    struct OwnedDirectory(PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-image-{}", uuid::Uuid::new_v4())),
    );
    let cached = root.0.join("standard/16.4");
    std::fs::create_dir_all(cached.join("DeveloperDiskImage.dmg")).unwrap();
    std::fs::write(
        cached.join("DeveloperDiskImage.dmg.signature"),
        b"owned-private",
    )
    .unwrap();
    let error = DdiDownloader::new(Some(root.0.clone()))
        .download_standard(&semver::Version::new(16, 4, 0))
        .await
        .err()
        .unwrap();
    assert_eq!(family(&error), "io");
    let ImageMounterError::Io(outer) = &error else {
        unreachable!()
    };
    let original = outer
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert!(original.raw_os_error().is_some());
    assert_eq!(outer.kind(), original.kind());
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::classify_error(original)
    );
    assert_eq!(
        ios_core::classify_error(&ImageMounterError::Tss("owned status".into())),
        ErrorClass::ServiceFailure
    );
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[cfg(any(
    feature = "cryptex",
    feature = "mobileactivation",
    feature = "installcoordination",
    feature = "pasteboard"
))]
#[tokio::test]
async fn root_service_deadline_getters_keep_exhaustive_families_and_issuer_scope() {
    use std::{error::Error, fmt, io, time::Duration};
    #[derive(Debug)]
    struct Cycle;
    impl fmt::Display for Cycle {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("getter must not format errors")
        }
    }
    impl Error for Cycle {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self)
        }
    }
    #[derive(Debug)]
    struct Wrap(Cycle);
    impl fmt::Display for Wrap {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("getter must not format wrappers")
        }
    }
    impl Error for Wrap {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }
    #[derive(Debug, thiserror::Error)]
    #[error("unissued deadline")]
    struct Unissued(#[source] tokio::time::error::Elapsed);
    async fn unissued() -> io::Error {
        io::Error::other(Unissued(
            tokio::time::timeout(Duration::ZERO, std::future::pending::<()>())
                .await
                .unwrap_err(),
        ))
    }
    #[cfg(any(
        feature = "mobileactivation",
        feature = "installcoordination",
        feature = "pasteboard"
    ))]
    let duration = Duration::from_millis(1250);
    #[cfg(feature = "cryptex")]
    {
        use ios_core::cryptex::CryptexError as E;
        fn family(error: &E) -> &str {
            match error {
                E::Xpc(_) => "xpc",
                E::Io(_) => "io",
                E::Plist(_) => "plist",
                E::Protocol(_) => "protocol",
                E::Device(_) => "device",
                E::Download(_) => "download",
                E::Timeout(_) => "timeout",
                E::Unsupported(_) => "unsupported",
                E::Conflict(_) => "conflict",
                E::Connection(_) => "connection",
                E::ImageMounter(_) => "image-mounter",
            }
        }
        assert_eq!(E::Timeout("install").timeout_phase(), Some("install"));
        for error in [
            E::Io(io::Error::from(io::ErrorKind::TimedOut)),
            E::Io(io::Error::other(Wrap(Cycle))),
            E::Io(unissued().await),
        ] {
            assert_eq!(family(&error), "io");
            assert_eq!(error.timeout_phase(), None);
        }
    }
    #[cfg(feature = "mobileactivation")]
    {
        use ios_core::mobileactivation::MobileActivationError as E;
        fn family(error: &E) -> &str {
            match error {
                E::Io(_) => "io",
                E::Plist(_) => "plist",
                E::Protocol(_) => "protocol",
                E::Timeout(_) => "timeout",
                E::Http(_) => "http",
            }
        }
        assert_eq!(E::Timeout(duration).timeout_duration(), Some(duration));
        for error in [
            E::Io(io::Error::from(io::ErrorKind::TimedOut)),
            E::Io(io::Error::other(Wrap(Cycle))),
            E::Io(unissued().await),
        ] {
            assert_eq!(family(&error), "io");
            assert_eq!(error.timeout_duration(), None);
        }
        let (stream, _peer) = tokio::io::duplex(4096);
        let mut client = ios_core::mobileactivation::MobileActivationClient::with_timeout(
            stream,
            Duration::from_millis(10),
        );
        let error = client.request_session_info().await.unwrap_err();
        assert_eq!(family(&error), "io");
        assert_eq!(error.timeout_duration(), Some(Duration::from_millis(10)));
        assert_eq!(
            ios_core::classify_error(&error),
            ios_core::ErrorClass::Timeout
        );
        let mut next: Option<&(dyn Error + 'static)> = Some(&error);
        let mut retained = false;
        for _ in 0..32 {
            let Some(source) = next else { break };
            retained |= source.is::<tokio::time::error::Elapsed>();
            next = source.source();
        }
        assert!(retained);
    }
    #[cfg(feature = "installcoordination")]
    {
        use ios_core::installcoordination::InstallCoordinationError as E;
        fn family(error: &E) -> &str {
            match error {
                E::Xpc(_) => "xpc",
                E::Protocol(_) => "protocol",
                E::Timeout(_) => "timeout",
            }
        }
        assert_eq!(E::Timeout(duration).timeout_duration(), Some(duration));
        for io in [
            io::Error::from(io::ErrorKind::TimedOut),
            io::Error::other(Wrap(Cycle)),
            unissued().await,
        ] {
            let error = E::Xpc(ios_core::XpcError::Io(io));
            assert_eq!(family(&error), "xpc");
            assert_eq!(error.timeout_duration(), None);
        }
    }
    #[cfg(feature = "pasteboard")]
    {
        use ios_core::pasteboard::PasteboardError as E;
        fn family(error: &E) -> &str {
            match error {
                E::Notification(_) => "notification",
                E::Xpc(_) => "xpc",
                E::Protocol(_) => "protocol",
                E::Service { .. } => "service",
                E::Timeout { .. } => "timeout",
                E::Closed => "closed",
                E::Limit(_) => "limit",
            }
        }
        assert_eq!(
            E::Timeout {
                seconds: duration.as_secs()
            }
            .timeout_seconds(),
            Some(1)
        );
        for io in [
            io::Error::from(io::ErrorKind::TimedOut),
            io::Error::other(Wrap(Cycle)),
            unissued().await,
        ] {
            let error = E::Xpc(ios_core::XpcError::Io(io));
            assert_eq!(family(&error), "xpc");
            assert_eq!(error.timeout_seconds(), None);
        }
    }
}

#[test]
fn root_xpc_codec_error_retains_native_source_and_exhaustive_family() {
    use ios_core::{
        decode_xpc_message as decode_message, encode_xpc_message as encode_message, XpcError,
    };
    use std::error::Error;
    fn family(error: &XpcError) -> &'static str {
        match error {
            XpcError::Io(_) => "io",
            XpcError::NotConnected => "not-connected",
            XpcError::ServiceNotFound(_) => "service-not-found",
            XpcError::Tls(_) => "protocol",
        }
    }
    let message = XpcMessage {
        flags: 1,
        msg_id: 42,
        body: Some(XpcValue::String("owned-private".into())),
    };
    let valid = encode_message(&message).unwrap();
    assert_eq!(decode_message(valid.clone()).unwrap().body, message.body);
    let mut malformed = valid.to_vec();
    let position = malformed
        .windows(b"owned-private".len())
        .position(|s| s == b"owned-private")
        .unwrap();
    malformed[position] = 0xff;
    let error = decode_message(bytes::Bytes::from(malformed)).unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Protocol
    );
    let XpcError::Io(outer) = &error else {
        unreachable!()
    };
    assert!(outer.source().unwrap().is::<std::str::Utf8Error>());
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[cfg(feature = "pcap")]
#[tokio::test]
async fn root_pcap_reader_retains_native_metadata_source_and_exhaustive_family() {
    use ios_core::pcap::{PcapClient, PcapError};
    use std::error::Error;
    fn family(error: &PcapError) -> &'static str {
        match error {
            PcapError::Io(_) => "io",
            PcapError::Plist(_) => "plist",
            PcapError::Protocol(_) => "protocol",
        }
    }
    let mut raw = vec![0; 95];
    raw[..4].copy_from_slice(&95u32.to_be_bytes());
    raw[25..39].copy_from_slice(b"owned-private\xff");
    let mut payload = Vec::new();
    plist::to_writer_binary(&mut payload, &plist::Value::Data(raw)).unwrap();
    let mut wire = (payload.len() as u32).to_be_bytes().to_vec();
    wire.extend_from_slice(&payload);
    let mut reader = PcapClient::new(std::io::Cursor::new(wire));
    let error = reader.next_packet().await.unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::Protocol
    );
    let PcapError::Io(outer) = &error else {
        unreachable!()
    };
    let native = outer
        .source()
        .unwrap()
        .downcast_ref::<std::string::FromUtf8Error>()
        .unwrap();
    assert_eq!(native.as_bytes(), b"owned-private\xff");
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
}

#[test]
fn root_backup_required_file_error_keeps_native_io_and_public_family() {
    use ios_core::{
        backup2::{initialize_backup_directory, load_backup_applications, Mobilebackup2Error},
        classify_error, ErrorClass,
    };
    use std::error::Error;
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-backup-host-{}", uuid::Uuid::new_v4())),
    );
    let info = plist::Dictionary::from_iter([(
        "Applications",
        plist::Value::Dictionary(plist::Dictionary::new()),
    )]);
    let layout = initialize_backup_directory(&root.0, "owned-private-device", &info, true).unwrap();
    assert_eq!(
        load_backup_applications(&root.0, "owned-private-device").unwrap(),
        info.get("Applications").cloned()
    );
    let path = layout.device_directory.join("Status.plist");
    std::fs::remove_file(&path).unwrap();
    let expected = std::fs::symlink_metadata(&path).unwrap_err();
    let error = load_backup_applications(&root.0, "owned-private-device").unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::NotFound);
    let Mobilebackup2Error::Io(outer) = &error else {
        panic!("expected native IO family");
    };
    assert_eq!(outer.kind(), std::io::ErrorKind::NotFound);
    let original = outer
        .source()
        .unwrap()
        .downcast_ref::<std::io::Error>()
        .unwrap();
    assert_eq!(original.raw_os_error(), expected.raw_os_error());
    assert!(!format!("{error:?} {:?}", outer.get_ref()).contains("owned-private"));
    assert!(layout.device_directory.join("Info.plist").is_file());
    assert!(!path.exists());
}

#[test]
fn root_mux_constructor_keeps_its_typed_error_future() {
    fn signature<E, F>(future: F)
    where
        E: std::error::Error + Send + Sync + 'static,
        F: Future<Output = Result<ios_core::MuxClient, E>>,
    {
        drop(future);
    }
    // External type check only; no daemon endpoint or device is contacted.
    signature(ios_core::MuxClient::connect());
}

#[cfg(feature = "backup2-manifest")]
#[test]
fn root_backup_metadata_error_keeps_exhaustive_family_and_native_sqlite_source() {
    use ios_core::{
        backup2::{list_backup_entries, Mobilebackup2Error},
        classify_error, ErrorClass,
    };
    use std::error::Error;
    fn family(error: &Mobilebackup2Error) -> &'static str {
        match error {
            Mobilebackup2Error::DeviceLink(_) => "device-link",
            Mobilebackup2Error::Io(_) => "io",
            Mobilebackup2Error::Plist(_) => "plist",
            Mobilebackup2Error::Protocol(_) => "protocol",
        }
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<Mobilebackup2Error>();
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-backup-{}", uuid::Uuid::new_v4())),
    );
    let device = root.0.join("owned-device");
    std::fs::create_dir_all(&device).unwrap();
    std::fs::write(device.join("Info.plist"), b"owned-info").unwrap();
    std::fs::write(device.join("Status.plist"), b"owned-status").unwrap();
    plist::to_file_xml(
        device.join("Manifest.plist"),
        &plist::Dictionary::from_iter([
            ("IsEncrypted", plist::Value::Boolean(false)),
            (
                "Lockdown",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ProductVersion",
                    "17.0",
                )])),
            ),
        ]),
    )
    .unwrap();
    std::fs::write(device.join("Manifest.db"), b"owned-private not a database").unwrap();
    let error = list_backup_entries(&root.0, "owned-device", None).unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    let Mobilebackup2Error::Io(source) = &error else {
        unreachable!()
    };
    let original = source
        .source()
        .unwrap()
        .downcast_ref::<rusqlite::Error>()
        .unwrap();
    assert_eq!(
        original.sqlite_error_code(),
        Some(rusqlite::ErrorCode::NotADatabase)
    );
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
}

#[cfg(feature = "backup2-manifest")]
#[test]
fn root_legacy_backup_rejection_keeps_native_utf8_bytes_without_public_variant_changes() {
    use ios_core::{
        backup2::{list_backup_entries, Mobilebackup2Error},
        classify_error, ErrorClass,
    };
    use std::error::Error;
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-public-mbdb-{}", uuid::Uuid::new_v4())),
    );
    let device = root.0.join("owned-device");
    std::fs::create_dir_all(&device).unwrap();
    std::fs::write(device.join("Info.plist"), b"owned-info").unwrap();
    std::fs::write(device.join("Status.plist"), b"owned-status").unwrap();
    plist::to_file_xml(
        device.join("Manifest.plist"),
        &plist::Dictionary::from_iter([
            ("IsEncrypted", plist::Value::Boolean(false)),
            (
                "Lockdown",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ProductVersion",
                    "10.2",
                )])),
            ),
        ]),
    )
    .unwrap();
    let bytes = [
        b"mbdb\x05\x00".as_slice(),
        b"\x00\x0eowned-private\xff".as_slice(),
    ]
    .concat();
    std::fs::write(device.join("Manifest.mbdb"), bytes).unwrap();
    let error = list_backup_entries(&root.0, "owned-device", None).unwrap_err();
    assert!(matches!(&error, Mobilebackup2Error::Io(_)));
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    assert!(!format!("{error:?}").contains("owned-private"));
    let mut current: Option<&(dyn Error + 'static)> = Some(&error);
    let mut found = false;
    for _ in 0..32 {
        let Some(source) = current else { break };
        if let Some(native) = source.downcast_ref::<std::string::FromUtf8Error>() {
            assert_eq!(native.as_bytes(), b"owned-private\xff");
            found = true;
            break;
        }
        current = source.source();
    }
    assert!(
        found,
        "native UTF8 source must remain reachable from root API"
    );
}

#[tokio::test]
async fn default_lockdown_root_api_retains_sources_and_exhaustive_errors() {
    use ios_core::{
        classify_error, ErrorClass, LockdownClient, LockdownError, PairRecord, PairRecordError,
    };
    use std::{error::Error, io};
    use tokio::io::AsyncWriteExt;
    fn family(error: &LockdownError) -> &'static str {
        match error {
            LockdownError::Io(_) => "io",
            LockdownError::Protocol(_) => "protocol",
            LockdownError::PairRecord(_) => "pair-record",
        }
    }
    fn record_family(error: &PairRecordError) -> &'static str {
        match error {
            PairRecordError::NotFound(_) => "missing",
            PairRecordError::Read { .. } => "read",
            PairRecordError::Parse(_) => "parse",
        }
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    fn signature<F: Future<Output = Result<LockdownClient, LockdownError>>>(future: F) {
        drop(future);
    }
    assert_error::<LockdownError>();
    assert_error::<PairRecordError>();
    let record = PairRecord {
        device_certificate: Vec::new(),
        host_certificate: b"-----BEGIN CERTIFICATE-----\nowned-private!\n-----END CERTIFICATE-----"
            .to_vec(),
        host_private_key: Vec::new().into(),
        root_certificate: Vec::new(),
        host_id: "owned-host".into(),
        system_buid: "owned-buid".into(),
        wifi_mac_address: None,
    };
    let (stream, peer) = tokio::io::duplex(64);
    // Root constructor is type-checked but not polled/session-established.
    signature(LockdownClient::connect_with_stream(stream, &record));
    drop(peer);
    let (stream, peer) = tokio::io::duplex(64);
    let error = ios_core::wrap_service_tls(stream, &record)
        .await
        .unwrap_err();
    drop(peer);
    assert_eq!(family(&error), "io");
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(source) = &error else {
        unreachable!()
    };
    assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
    assert!(source.source().unwrap().is::<io::Error>());
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));

    let (mut stream, mut peer) = tokio::io::duplex(64);
    let payload = b"<plist><dict>";
    peer.write_all(&(payload.len() as u32).to_be_bytes())
        .await
        .unwrap();
    peer.write_all(payload).await.unwrap();
    let error = ios_core::recv_lockdown::<_, plist::Value>(&mut stream)
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    assert_eq!(classify_error(&error), ErrorClass::Protocol);
    let LockdownError::Io(source) = &error else {
        unreachable!()
    };
    assert!(source.source().unwrap().is::<plist::Error>());
    assert_eq!(
        record_family(&PairRecordError::Parse("owned-private".into())),
        "parse"
    );
}

#[cfg(feature = "supervised-pair")]
#[tokio::test]
async fn supervised_pairing_root_api_keeps_native_sources_and_record_fields() {
    use ios_core::{
        classify_error, pair_supervised, save_pair_record, ErrorClass, FullPairRecord,
        LockdownError,
    };
    use std::{error::Error, io};
    let (mut stream, peer) = tokio::io::duplex(64);
    let error = pair_supervised(
        &mut stream,
        b"owned-private invalid DER",
        "owned-password",
        "owned-buid",
    )
    .await
    .unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(source) = &error else {
        panic!("root native cause lost")
    };
    assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
    assert!(source.source().unwrap().is::<openssl::error::ErrorStack>());
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    let record = FullPairRecord {
        device_certificate: vec![1],
        host_certificate: vec![2],
        host_private_key: vec![3],
        root_certificate: vec![4],
        root_private_key: vec![5],
        host_id: "owned-host".into(),
        system_buid: "owned-buid".into(),
    };
    struct OwnedDirectory(std::path::PathBuf);
    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let owned = OwnedDirectory(
        std::env::temp_dir().join(format!("ios-supervised-root-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir(&owned.0).unwrap();
    let path = owned.0.join("owned-private\0.plist");
    let error = save_pair_record(&record, b"escrow", None, &path).unwrap_err();
    assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
    let LockdownError::Io(source) = &error else {
        panic!("root native IO lost")
    };
    assert!(source.source().unwrap().is::<io::Error>());
    drop(peer);
}

#[cfg(feature = "mcinstall")]
#[tokio::test]
async fn mcinstall_root_error_family_and_feature_decisions_remain_observable() {
    use ios_core::{
        classify_error,
        mcinstall::{McInstallClient, McInstallError},
        ErrorClass,
    };
    use std::{error::Error, io};
    fn family(error: &McInstallError) -> &'static str {
        match error {
            McInstallError::Io(_) => "io",
            McInstallError::Plist(_) => "plist",
            McInstallError::Protocol(_) => "protocol",
            McInstallError::Crypto(_) => "crypto",
            McInstallError::PasscodeSet => "passcode",
        }
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<McInstallError>();
    let (stream, peer) = tokio::io::duplex(64);
    let mut client = McInstallClient::new(stream);
    let error = client
        .escalate_with_p12(b"owned-private invalid DER", "owned-password")
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    let McInstallError::Io(source) = &error else {
        unreachable!()
    };
    #[cfg(feature = "supervised-pair")]
    {
        assert_eq!(source.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(classify_error(&error), ErrorClass::InvalidInput);
        assert!(source.source().unwrap().is::<openssl::error::ErrorStack>());
    }
    #[cfg(not(feature = "supervised-pair"))]
    {
        assert_eq!(source.kind(), io::ErrorKind::Other);
        assert_eq!(classify_error(&error), ErrorClass::Unsupported);
        assert!(source.source().unwrap().is::<CoreError>());
    }
    assert!(!format!("{error:?} {:?}", source.get_ref()).contains("owned-private"));
    assert_eq!(
        family(&McInstallError::Crypto("owned-private".into())),
        "crypto"
    );
    assert_eq!(
        classify_error(&McInstallError::PasscodeSet),
        ErrorClass::Precondition
    );
    drop(peer);
}

#[cfg(all(feature = "tunnel", feature = "mdns"))]
#[test]
fn manual_pairing_root_api_preserves_its_exhaustive_error_and_future() {
    use ios_core::{pair_new_device, PairedCredentials, PairingTransportError};
    use std::{error::Error, net::Ipv6Addr};
    fn family(error: &PairingTransportError) -> &'static str {
        match error {
            PairingTransportError::Io(_) => "io",
            PairingTransportError::Xpc(_) => "xpc",
            PairingTransportError::Rsd(_) => "rsd",
            PairingTransportError::Crypto(_) => "crypto",
            PairingTransportError::Failed(_) => "failed",
            PairingTransportError::Rejected(_) => "rejected",
            PairingTransportError::MissingField(_) => "missing",
            PairingTransportError::UnexpectedType(_) => "type",
            PairingTransportError::ServiceNotFound => "service-missing",
        }
    }
    fn signature<F: Future<Output = Result<PairedCredentials, PairingTransportError>>>(future: F) {
        drop(future);
    }
    fn assert_error<E: Error + Send + Sync + 'static>() {}
    assert_error::<PairingTransportError>();
    // The real root API future is type-checked but never polled/device-connected.
    signature(pair_new_device(Ipv6Addr::LOCALHOST));
    assert_eq!(
        family(&PairingTransportError::ServiceNotFound),
        "service-missing"
    );
}

#[cfg(feature = "mobileactivation")]
#[test]
fn deactivation_fallback_observation_is_public_borrowed_and_thread_safe() {
    use ios_core::{DeactivationFallbackError, ErrorClass};
    fn assert_error<T: std::error::Error + Send + Sync + 'static>() {}
    assert_error::<DeactivationFallbackError>();
    let _: fn(&DeactivationFallbackError) -> &CoreError = DeactivationFallbackError::service_error;
    let _: fn(&DeactivationFallbackError) -> &CoreError = DeactivationFallbackError::fallback_error;
    let _: fn(&DeactivationFallbackError) -> ErrorClass = DeactivationFallbackError::service_class;
    let _: fn(&DeactivationFallbackError) -> ErrorClass = DeactivationFallbackError::fallback_class;
}

// These enums are currently exhaustive. Adding a variant/non_exhaustive is a
// downstream source change, even if every in-workspace caller still compiles.
fn error_family(error: &CoreError) -> &'static str {
    match error {
        CoreError::Io(_) => "io",
        CoreError::Mux(_) => "mux",
        CoreError::Lockdown(_) => "lockdown",
        CoreError::PairRecord(_) => "pair-record",
        CoreError::Tunnel(_) => "tunnel",
        CoreError::Xpc(_) => "xpc",
        CoreError::Json(_) => "json",
        CoreError::Protocol(_) => "protocol",
        CoreError::DeviceNotFound(_) => "device-not-found",
        CoreError::Unsupported(_) => "unsupported",
        CoreError::Other(_) => "other",
    }
}

#[cfg(feature = "apps")]
#[tokio::test]
async fn checked_install_observer_is_additive_and_keeps_error_enum_exhaustive() {
    use ios_core::apps::{
        install_ipa, install_ipa_with_checked_progress, CheckedProgressCallback, ZipConduitError,
    };
    fn family(error: &ZipConduitError) -> &'static str {
        match error {
            ZipConduitError::Io(_) => "io",
            ZipConduitError::Plist(_) => "plist",
            ZipConduitError::Protocol(_) => "protocol",
            ZipConduitError::Zip(_) => "zip",
            ZipConduitError::Install(_) => "install",
        }
    }
    let path = std::env::temp_dir().join(format!("ios-uncreated-{}.ipa", uuid::Uuid::new_v4()));
    let (mut stream, _peer) = tokio::io::duplex(64);
    let callback: ios_core::apps::zipconduit::ProgressCallback = Box::new(|_, _| {});
    let old = install_ipa(&mut stream, &path, Some(callback));
    drop(old);
    let callback: CheckedProgressCallback =
        Box::new(|_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)));
    let error = install_ipa_with_checked_progress(&mut stream, &path, Some(callback))
        .await
        .unwrap_err();
    assert_eq!(family(&error), "io");
    let ZipConduitError::Io(native) = error else {
        panic!("local source lost");
    };
    assert_eq!(native.kind(), std::io::ErrorKind::NotFound);
}

#[cfg(feature = "webinspector")]
#[test]
fn webinspector_facade_models_clients_and_sessions_remain_public() {
    use ios_core::webinspector::{
        Application, ApplicationPage, AutomationAvailability, AutomationSession, By,
        InspectorSession, Page, WebInspectorClient, WebInspectorEvent, WirType,
    };
    let application = Application {
        id: "PID:42".into(),
        bundle_identifier: "com.example.synthetic".into(),
        pid: 42,
        name: "Synthetic".into(),
        availability: AutomationAvailability::Available,
        is_active: true,
        is_proxy: false,
        is_ready: true,
        host_application_identifier: None,
    };
    let page = Page {
        id: 1,
        listing_key: "page-1".into(),
        page_type: WirType::WebPage,
        title: Some("Synthetic page".into()),
        url: None,
        automation_is_paired: None,
        automation_name: None,
        automation_version: None,
        automation_session_id: None,
        automation_connection_id: None,
    };
    let row = ApplicationPage { application, page };
    assert_eq!(
        serde_json::to_value(&row).unwrap()["page"]["page_type"],
        "web_page"
    );
    let _: ios_core::services::webinspector::WebInspectorEvent =
        WebInspectorEvent::ConnectedDrivers;
    let _: ios_core::services::webinspector::By = By::CssSelector;
    let (stream, _peer) = tokio::io::duplex(64);
    let mut client = WebInspectorClient::with_connection_id(stream, "synthetic-connection");
    let mut inspector = InspectorSession::with_session_id("PID:42", 1, "synthetic-session");
    let mut automation = AutomationSession::with_session_id(
        "PID:42",
        "com.example.synthetic",
        "synthetic-automation",
    );
    automation.set_implicit_wait_timeout(std::time::Duration::ZERO);
    assert_eq!(client.connection_id(), "synthetic-connection");
    assert_eq!(inspector.session_id(), "synthetic-session");
    assert_eq!(automation.session_id(), "synthetic-automation");
    // Check old method paths without polling any service I/O.
    drop(client.start(std::time::Duration::ZERO));
    drop(client.start_preserving_errors(std::time::Duration::ZERO));
    drop(inspector.attach_unpaused(&mut client, true, std::time::Duration::ZERO));
    drop(automation.attach(&mut client, std::time::Duration::ZERO));
    drop(automation.find_element(&mut client, By::CssSelector, ".synthetic"));
    let _browser = ios_core::webinspector::cdp_browser::BrowserCdpState::new("synthetic-browser");
}

#[cfg(feature = "webinspector")]
#[tokio::test]
async fn public_webinspector_start_preserves_typed_io_without_changing_legacy_api() {
    use ios_core::webinspector::{WebInspectorClient, WebInspectorError};
    // Existing callers may exhaustively match this enum; the additive API
    // does not require a new variant or a fallback in their old match.
    fn family(error: &WebInspectorError) -> &'static str {
        match error {
            WebInspectorError::Io(_) => "io",
            WebInspectorError::Plist(_) => "plist",
            WebInspectorError::Json(_) => "json",
            WebInspectorError::Disabled => "disabled",
            WebInspectorError::Protocol(_) => "protocol",
            WebInspectorError::Timeout(_) => "timeout",
        }
    }
    for preserve in [false, true] {
        let (stream, peer) = tokio::io::duplex(64);
        drop(peer);
        let mut client = WebInspectorClient::with_connection_id(stream, "synthetic");
        let duration = std::time::Duration::from_millis(20);
        let error = if preserve {
            client.start_preserving_errors(duration).await
        } else {
            client.start(duration).await
        }
        .unwrap_err();
        assert_eq!(family(&error), if preserve { "io" } else { "disabled" });
        assert_eq!(
            ios_core::classify_error(&error),
            if preserve {
                ios_core::ErrorClass::Disconnected
            } else {
                ios_core::ErrorClass::Precondition
            }
        );
    }
}

#[cfg(all(
    feature = "webinspector",
    feature = "companion",
    feature = "notificationproxy"
))]
#[tokio::test]
async fn public_discovery_deadlines_keep_native_sources_and_exhaustive_families() {
    use ios_core::{
        companion::{CompanionError, CompanionProxyClient},
        notificationproxy::{NotificationProxyClient, NotificationProxyError},
        webinspector::{WebInspectorClient, WebInspectorError},
    };
    use std::error::Error;
    fn has_elapsed(error: &(dyn Error + 'static)) -> bool {
        let mut next = Some(error);
        for _ in 0..32 {
            let Some(error) = next else { break };
            if error.is::<tokio::time::error::Elapsed>() {
                return true;
            }
            next = error.source();
        }
        false
    }
    fn web_family(error: &WebInspectorError) -> &'static str {
        match error {
            WebInspectorError::Io(_) => "io",
            WebInspectorError::Plist(_) => "plist",
            WebInspectorError::Json(_) => "json",
            WebInspectorError::Disabled => "disabled",
            WebInspectorError::Protocol(_) => "protocol",
            WebInspectorError::Timeout(_) => "timeout",
        }
    }
    fn companion_family(error: &CompanionError) -> &'static str {
        match error {
            CompanionError::Io(_) => "io",
            CompanionError::Plist(_) => "plist",
            CompanionError::Timeout(_) => "timeout",
            CompanionError::Protocol(_) => "protocol",
        }
    }
    fn notification_family(error: &NotificationProxyError) -> &'static str {
        match error {
            NotificationProxyError::Io(_) => "io",
            NotificationProxyError::Plist(_) => "plist",
            NotificationProxyError::Timeout => "timeout",
            NotificationProxyError::ProxyDeath => "death",
            NotificationProxyError::Protocol(_) => "protocol",
        }
    }
    let duration = std::time::Duration::from_millis(5);
    let (stream, _peer) = tokio::io::duplex(16 * 1024);
    let error = WebInspectorClient::new(stream)
        .next_event_with_timeout(duration)
        .await
        .unwrap_err();
    assert_eq!(web_family(&error), "io");
    assert!(has_elapsed(&error));
    assert_eq!(error.timeout_duration(), Some(duration));
    let (stream, _peer) = tokio::io::duplex(16 * 1024);
    let error = CompanionProxyClient::with_timeout(stream, duration)
        .list()
        .await
        .unwrap_err();
    assert_eq!(companion_family(&error), "io");
    assert!(has_elapsed(&error));
    assert_eq!(error.timeout_duration(), Some(duration));
    let (stream, _peer) = tokio::io::duplex(16 * 1024);
    let error = NotificationProxyClient::new(stream)
        .next_event(duration)
        .await
        .unwrap_err();
    assert_eq!(notification_family(&error), "io");
    assert!(has_elapsed(&error));
}

#[cfg(feature = "companion")]
#[tokio::test]
async fn lifecycle_companion_public_shared_state_budget_send_and_exhaustive_families() {
    use ios_core::companion::{CompanionError, CompanionProxyClient};
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    fn family(error: &CompanionError) -> &'static str {
        match error {
            CompanionError::Io(_) => "io",
            CompanionError::Plist(_) => "plist",
            CompanionError::Protocol(_) => "protocol",
            CompanionError::Timeout(_) => "timeout",
        }
    }
    fn send<T: Send>(_: &T) {}
    for budget in [Duration::ZERO, Duration::MAX] {
        let (stream, mut peer) = tokio::io::duplex(4096);
        let mut client = CompanionProxyClient::with_timeout(stream, budget);
        let query: fn(&CompanionProxyClient<tokio::io::DuplexStream>) -> bool =
            CompanionProxyClient::is_usable;
        assert!(query(&client));
        assert_eq!(client.timeout(), budget);
        let request = client.list();
        send(&request);
        let error = request.await.unwrap_err();
        assert_eq!(
            family(&error),
            if budget.is_zero() { "timeout" } else { "io" }
        );
        assert!(client.is_usable());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), peer.read_u8())
                .await
                .is_err()
        );
    }
    let (stream, _peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::with_timeout(stream, Duration::from_millis(5));
    let error = client.list().await.unwrap_err();
    assert_eq!(family(&error), "io");
    assert!(!client.is_usable());
    assert_eq!(family(&client.list().await.unwrap_err()), "protocol");
}

#[cfg(feature = "companion")]
#[tokio::test]
async fn lifecycle_companion_public_consumed_close_is_owned_send_and_has_no_drop_fallback() {
    use ios_core::companion::CompanionProxyClient;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn send<T: Send>(_: &T) {}
    async fn request(peer: &mut tokio::io::DuplexStream) -> plist::Dictionary {
        let len = peer.read_u32().await.unwrap() as usize;
        assert!(len < 4096);
        let mut body = vec![0; len];
        peer.read_exact(&mut body).await.unwrap();
        plist::from_bytes(&body).unwrap()
    }
    let (stream, mut peer) = tokio::io::duplex(4096);
    let mut client = CompanionProxyClient::new(stream);
    let operation = async {
        let (handle, ()) = tokio::join!(
            client.start_forwarding_service_port_handle(42, None, None),
            async {
                assert_eq!(
                    request(&mut peer).await["Command"].as_string(),
                    Some("StartForwardingServicePort")
                );
                let mut value = plist::Dictionary::new();
                value.insert("CompanionProxyServicePort".into(), 43u64.into());
                let mut body = Vec::new();
                plist::to_writer_xml(&mut body, &value).unwrap();
                peer.write_u32(body.len() as u32).await.unwrap();
                peer.write_all(&body).await.unwrap();
            }
        );
        let closing = handle.unwrap().close_with_timeout(Duration::MAX);
        send(&closing);
        drop(closing);
        assert!(client.is_usable());
        assert!(
            tokio::time::timeout(Duration::from_millis(20), peer.read_u8())
                .await
                .is_err()
        );
        drop(client);
        assert_eq!(peer.read(&mut [0; 1]).await.unwrap(), 0);
    };
    tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .unwrap();
}

#[cfg(feature = "notificationproxy")]
#[test]
fn lifecycle_notification_public_budget_state_send_and_exhaustive_families() {
    use ios_core::notificationproxy::{
        NotificationEvent, NotificationProxyClient, NotificationProxyError, NotificationProxyEvent,
    };
    use std::time::Duration;
    fn send<T: Send>(_: T) {}
    fn error_family(error: &NotificationProxyError) -> &'static str {
        match error {
            NotificationProxyError::Io(_) => "io",
            NotificationProxyError::Plist(_) => "plist",
            NotificationProxyError::Protocol(_) => "protocol",
            NotificationProxyError::ProxyDeath => "death",
            NotificationProxyError::Timeout => "timeout",
        }
    }
    fn event_family(event: NotificationEvent) -> &'static str {
        match event {
            NotificationEvent::Notification(_) => "notification",
            NotificationEvent::ProxyDeath => "death",
        }
    }
    fn next_family(event: NotificationProxyEvent) -> &'static str {
        match event {
            NotificationProxyEvent::Notification(_) => "notification",
            NotificationProxyEvent::ProxyDeath => "death",
        }
    }
    let (stream, _peer) = tokio::io::duplex(4096);
    let mut client = NotificationProxyClient::new(stream);
    let usable: fn(&NotificationProxyClient<tokio::io::DuplexStream>) -> bool =
        NotificationProxyClient::is_usable;
    let setter: fn(&mut NotificationProxyClient<tokio::io::DuplexStream>, Option<Duration>) =
        NotificationProxyClient::set_operation_timeout;
    setter(&mut client, Some(Duration::from_secs(1)));
    send(client.observe("owned"));
    send(client.post("owned"));
    send(client.wait_for("owned", Duration::from_secs(1)));
    send(client.wait_for_springboard(Duration::from_secs(1)));
    send(client.next_event(Duration::from_secs(1)));
    send(client.recv_event());
    send(client.shutdown());
    assert!(usable(&client));
    assert_eq!(error_family(&NotificationProxyError::Timeout), "timeout");
    assert_eq!(event_family(NotificationEvent::ProxyDeath), "death");
    assert_eq!(next_family(NotificationProxyEvent::ProxyDeath), "death");
}

#[cfg(all(feature = "crashreport", feature = "ostrace"))]
#[tokio::test]
async fn public_diagnostic_services_keep_native_sources_and_exhaustive_families() {
    use ios_core::{
        crashreport::{parse_report_bytes, CrashReportError},
        ostrace::{MessageFilterOptions, OsTraceClient, OsTraceError, TraceOptions},
    };
    use std::error::Error;
    fn has_source<T: Error + 'static>(error: &(dyn Error + 'static)) -> bool {
        let mut next = Some(error);
        for _ in 0..32 {
            let Some(error) = next else { break };
            if error.is::<T>() {
                return true;
            }
            next = error.source();
        }
        false
    }
    fn crash_family(error: &CrashReportError) -> &'static str {
        match error {
            CrashReportError::Io(_) => "io",
            CrashReportError::Afc(_) => "afc",
            CrashReportError::Protocol(_) => "protocol",
            CrashReportError::Timeout => "timeout",
            CrashReportError::InvalidPattern { .. } => "pattern",
        }
    }
    fn trace_family(error: &OsTraceError) -> &'static str {
        match error {
            OsTraceError::Io(_) => "io",
            OsTraceError::Plist(_) => "plist",
            OsTraceError::Protocol(_) => "protocol",
            OsTraceError::Timeout => "timeout",
        }
    }
    let error = parse_report_bytes("owned-private.ips", &[0xff]).unwrap_err();
    assert_eq!(crash_family(&error), "io");
    assert!(has_source::<std::str::Utf8Error>(&error));
    let (stream, _peer) = tokio::io::duplex(4096);
    let error = OsTraceClient::new(stream)
        .start_activity_with_timeout(TraceOptions::default(), std::time::Duration::from_millis(5))
        .await
        .err()
        .expect("owned idle startup");
    assert_eq!(trace_family(&error), "io");
    assert!(has_source::<tokio::time::error::Elapsed>(&error));
    let options = MessageFilterOptions {
        regex: vec!["[owned-private".into()],
        ..MessageFilterOptions::default()
    };
    assert!(options.compile().is_err());
    let error: OsTraceError = options.compile_preserving_errors().unwrap_err();
    assert_eq!(trace_family(&error), "io");
    assert_eq!(
        ios_core::classify_error(&error),
        ios_core::ErrorClass::InvalidInput
    );
    assert!(has_source::<regex::Error>(&error));
}

fn event_id(event: DeviceEvent) -> u32 {
    match event {
        DeviceEvent::Attached(DeviceInfo { device_id, .. })
        | DeviceEvent::Detached { device_id, .. } => device_id,
    }
}

#[cfg(feature = "tunnel")]
#[tokio::test]
async fn public_xpc_client_retains_native_h2_failure_in_existing_error_enum() {
    use ios_core::{classify_error, ErrorClass, XpcClient, XpcError};
    use std::error::Error;
    use std::io::ErrorKind;

    let (stream, peer) = tokio::io::duplex(64);
    drop(peer);
    let error = match XpcClient::connect_stream(stream).await {
        Err(error) => error,
        Ok(_) => panic!("closed transport connected"),
    };
    // The existing exhaustive error enum is unchanged. The native IO source
    // crosses the public root API instead of becoming a TLS/protocol string.
    match &error {
        XpcError::Io(outer) => assert_eq!(outer.kind(), ErrorKind::BrokenPipe),
        XpcError::NotConnected | XpcError::ServiceNotFound(_) | XpcError::Tls(_) => {
            panic!("native H2 failure flattened")
        }
    }
    assert_eq!(classify_error(&error), ErrorClass::Disconnected);
    let mut current: Option<&(dyn Error + 'static)> = error.source();
    let mut io_sources = 0;
    for _ in 0..32 {
        let Some(source) = current else { break };
        if let Some(io) = source.downcast_ref::<std::io::Error>() {
            assert_eq!(io.kind(), ErrorKind::BrokenPipe);
            io_sources += 1;
        }
        current = source.source();
    }
    assert_eq!(io_sources, 2, "outer adapter and original transport IO");
}

#[test]
fn public_error_classes_preserve_existing_exhaustive_core_matches() {
    use ios_core::{classify_error, ErrorClass};
    let error = CoreError::Io(std::io::Error::from(std::io::ErrorKind::TimedOut));
    assert_eq!(error_family(&error), "io");
    assert_eq!(error.class(), ErrorClass::Timeout);
    assert_eq!(classify_error(&error).as_str(), "timeout");
    let code = match error.class() {
        ErrorClass::Timeout => "wait budget",
        _ => "other or future category",
    };
    assert_eq!(code, "wait budget");
    let error = CoreError::Xpc(ios_core::XpcError::ServiceNotFound(
        "synthetic service".into(),
    ));
    assert_eq!(error_family(&error), "xpc");
    assert_eq!(classify_error(&error), ErrorClass::ServiceMissing);
    assert_eq!(
        serde_json::to_value(error.class()).unwrap(),
        "service_missing"
    );
}

#[test]
fn public_cancellation_marker_is_additive_and_distinct_from_plain_interrupted_io() {
    let marker: ios_core::error::OperationCancelled = ios_core::OperationCancelled::new();
    assert_eq!(ios_core::classify_error(&marker).as_str(), "cancelled");
    assert!(std::error::Error::source(&marker).is_none());
    assert_eq!(marker.to_string(), "operation cancelled");
    assert_eq!(format!("{marker:?}"), "OperationCancelled");
    let io = CoreError::Io(std::io::Error::from(std::io::ErrorKind::Interrupted));
    assert_eq!(error_family(&io), "io");
    assert_eq!(io.class().as_str(), "io");
}

#[test]
fn public_rsd_evidence_keeps_unknown_metadata_distinct_from_missing_service() {
    use ios_core::RsdFeatureEvidence as Evidence;
    // The new observation is borrowed and the legacy directory getter remains.
    let _read: fn(
        &ios_core::ConnectedDevice,
    ) -> (&ios_core::InitialRsdProbe, Option<&RsdHandshake>) =
        |device| (device.initial_rsd_probe(), device.rsd());
    let state: ios_core::device::InitialRsdProbeState =
        ios_core::InitialRsdProbeState::NotAttempted;
    assert_eq!(serde_json::to_value(state).unwrap(), "not_attempted");
    let _future_state = match state {
        ios_core::InitialRsdProbeState::NotAttempted => "no initial request",
        ios_core::InitialRsdProbeState::Available => "initial directory",
        ios_core::InitialRsdProbeState::Failed => "initial failure",
        _ => "future observation",
    };
    let legacy = ServiceDescriptor::new(1234);
    assert!(legacy.supports_feature("requested"));
    assert_eq!(
        legacy.feature_evidence("requested"),
        Evidence::MetadataUnavailable
    );
    let directory = RsdHandshake {
        udid: "synthetic-device".into(),
        services: HashMap::from([
            ("legacy".into(), legacy),
            (
                "modern".into(),
                ServiceDescriptor {
                    port: 2345,
                    features: vec!["requested".into()],
                },
            ),
            ("modern.shim.remote".into(), ServiceDescriptor::new(3456)),
            (
                "shim-only.shim.remote".into(),
                ServiceDescriptor {
                    port: 4567,
                    features: vec!["requested".into()],
                },
            ),
        ]),
    };
    assert_eq!(
        directory.feature_evidence("missing", "requested"),
        Evidence::ServiceMissing
    );
    assert_eq!(
        directory.feature_evidence("legacy", "requested"),
        Evidence::MetadataUnavailable
    );
    assert_eq!(
        directory.supports_feature("legacy", "requested"),
        Some(true)
    );
    assert_eq!(
        directory.feature_evidence("modern", "requested"),
        Evidence::Advertised
    );
    assert_eq!(
        directory.feature_evidence("modern", "other"),
        Evidence::NotAdvertised
    );
    assert_eq!(
        directory.resolved_feature_evidence("modern", "requested"),
        Evidence::Advertised
    );
    assert_eq!(
        directory.feature_evidence("shim-only", "requested"),
        Evidence::ServiceMissing
    );
    assert_eq!(
        directory.resolved_feature_evidence("shim-only", "requested"),
        Evidence::Advertised
    );
    assert_eq!(
        serde_json::to_value(Evidence::MetadataUnavailable).unwrap(),
        "metadata_unavailable"
    );
}

#[test]
fn public_plist_conversion_policies_select_historical_data_formats() {
    use ios_core::plist_json::{into_json, to_json, DataEncoding, PlistJsonPolicy};
    let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
        ("data".to_string(), plist::Value::Data(vec![0, 127, 255])),
        (
            "uid".to_string(),
            plist::Value::Uid(plist::Uid::new(u64::MAX)),
        ),
    ]));
    let hex = PlistJsonPolicy::new(DataEncoding::HexString);
    assert_eq!(
        to_json(&value, hex),
        serde_json::json!({"data":"007fff","uid":u64::MAX})
    );
    let numeric = PlistJsonPolicy::new(DataEncoding::NumericArray);
    assert_eq!(
        to_json(&value, numeric),
        serde_json::json!({"data":[0,127,255],"uid":u64::MAX})
    );
    let legacy = hex.without_date_and_uid().with_zero_integer_fallback();
    assert_eq!(
        into_json(value, legacy),
        serde_json::json!({"data":"007fff","uid":null})
    );
}

fn mode_name(mode: TunMode) -> &'static str {
    match mode {
        TunMode::Kernel => "kernel",
        TunMode::Userspace => "userspace",
    }
}

#[test]
fn backup_facade_types_functions_and_client_methods_remain_public() {
    use ios_core::backup2::{
        BackupDirectoryLayout, BackupOptions, BackupResult, Mobilebackup2Client, RestoreOptions,
        RestoreResult, VersionExchange,
    };
    let layout = BackupDirectoryLayout {
        root: "synthetic-backup".into(),
        device_directory: "synthetic-backup/source".into(),
        target_identifier: "source".into(),
    };
    let version = VersionExchange {
        device_link_version: 300,
        protocol_version: 2.1,
        local_versions: vec![2.0, 2.1],
    };
    let _: BackupResult = BackupResult {
        layout: layout.clone(),
        device_link_version: version.device_link_version,
        protocol_version: version.protocol_version,
    };
    let _: RestoreResult = RestoreResult {
        layout: layout.clone(),
        device_link_version: 300,
        protocol_version: 2.1,
    };
    let restore = RestoreOptions {
        system: false,
        reboot: true,
        copy: false,
        settings: true,
        remove: false,
        password: None,
        source_identifier: None,
    };
    let options = BackupOptions {
        filter: None,
        patch_manifest: false,
        password: None,
    };
    assert_eq!(ios_core::backup2::BackupSelection::NAMES.len(), 6);
    let _initialize = ios_core::backup2::initialize_backup_directory;
    let _encrypted = ios_core::services::backup2::backup_is_encrypted;
    let (stream, _peer) = tokio::io::duplex(64);
    let mut client = Mobilebackup2Client::new(stream);
    let info = plist::Dictionary::new();
    // Type-check method paths without polling or touching a device/filesystem.
    drop(client.version_exchange());
    drop(client.backup_with_options(&layout.root, "source", false, &info, options));
    drop(client.restore(&layout.root, "source", restore));
    drop(client.info(&layout.root, "source", None));
    drop(client.list(&layout.root, "source", None));
    #[cfg(feature = "backup2-manifest")]
    {
        let _list = ios_core::backup2::list_backup_entries;
        let _patch = ios_core::backup2::patch_backup_directory;
        let _unback = ios_core::backup2::unback_backup;
        let _extract = ios_core::backup2::extract_backup_file;
    }
}

#[test]
fn published_struct_literals_and_root_paths_remain_usable() {
    let options = ConnectOptions {
        tun_mode: TunMode::Userspace,
        pair_record_path: None,
        skip_tunnel: true,
    };
    assert_eq!(mode_name(options.tun_mode), "userspace");
    let device = DeviceInfo {
        udid: "synthetic-downstream-device".into(),
        device_id: 42,
        connection_type: "USB".into(),
        product_id: 1234,
    };
    assert_eq!(event_id(DeviceEvent::Attached(device.clone())), 42);
    assert_eq!(
        event_id(DeviceEvent::Detached {
            udid: device.udid,
            device_id: 42
        }),
        42
    );
    let info = TunnelInfo {
        server_address: "::1".into(),
        server_rsd_port: 58783,
        client_address: "::1".into(),
        client_mtu: 1280,
    };
    let (handle, receiver) = ios_core::TunnelHandle::new("synthetic".into(), info, None);
    assert!(handle.is_alive());
    drop(handle);
    assert!(receiver.has_changed().is_err());
    assert_eq!(
        error_family(&CoreError::Unsupported("synthetic".into())),
        "unsupported"
    );
}

#[test]
fn public_service_metadata_preserves_absent_and_unadvertised_distinction() {
    let metadata = ServiceDescriptor {
        port: 1234,
        features: Vec::new(),
    };
    assert_eq!(ServiceDescriptor::new(1234), metadata);
    let rsd = RsdHandshake {
        udid: "synthetic-rsd".into(),
        services: HashMap::from([("synthetic.service".into(), metadata)]),
    };
    assert_eq!(
        rsd.supports_feature("synthetic.service", "synthetic.feature"),
        Some(true)
    );
    assert_eq!(rsd.supports_feature("missing", "synthetic.feature"), None);
    assert_eq!(
        rsd.get_service_features("synthetic.service"),
        Some([].as_slice())
    );
}

#[test]
fn public_async_signatures_compile_without_polling_device_operations() {
    fn returns<T>(_: impl Future<Output = Result<T, CoreError>>) {}
    returns::<Vec<DeviceInfo>>(ios_core::list_devices());
    returns::<ios_core::ConnectedDevice>(ios_core::connect("synthetic", ConnectOptions::default()));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect(
        "synthetic",
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::connect_direct_usb_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect_direct_usb_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::connect_tcp_lockdown_tunnel(
        "synthetic",
        "127.0.0.1",
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect_tcp_lockdown_tunnel(
        "synthetic",
        "127.0.0.1",
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::connect_remote_pairing_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    returns::<ios_core::ConnectedDevice>(ios_core::device::connect_remote_pairing_tunnel(
        "synthetic",
        None,
        ConnectOptions::default(),
    ));
    #[cfg(feature = "mdns")]
    {
        returns::<Vec<ios_core::PairedMobdev2Device>>(ios_core::discover_paired_mobdev2_devices());
        returns::<Vec<ios_core::PairedMobdev2Device>>(
            ios_core::device::discover_paired_mobdev2_devices(),
        );
    }
    // Futures are dropped before their first poll; this does not discover a
    // device, establish a connection or certify cancellation after any I/O.
    drop(ios_core::watch_devices());
}

#[test]
fn public_xpc_message_shape_and_codec_roundtrip_remain_compatible() {
    let message = XpcMessage {
        flags: ios_core::xpc_message_flags::ALWAYS_SET,
        msg_id: 42,
        body: Some(XpcValue::String("synthetic".into())),
    };
    let encoded = ios_core::encode_xpc_message(&message).unwrap();
    let decoded = ios_core::decode_xpc_message(encoded).unwrap();
    assert_eq!(decoded.flags, message.flags);
    assert_eq!(decoded.msg_id, 42);
    assert_eq!(decoded.body.unwrap().as_str(), Some("synthetic"));
}

#[cfg(all(feature = "icon_service", feature = "screen_capture"))]
#[test]
fn service_feature_aliases_retain_identical_public_types() {
    fn icon_identity(
        value: ios_core::icon_service::IconServiceClient,
    ) -> ios_core::iconservice::IconServiceClient {
        value
    }
    fn screen_identity(
        value: ios_core::screen_capture::ScreenCaptureServiceClient,
    ) -> ios_core::screencapture::ScreenCaptureServiceClient {
        value
    }
    let _ = icon_identity;
    let _ = screen_identity;
}

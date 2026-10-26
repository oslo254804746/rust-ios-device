//! Real owned TCP proxy peers exercise the exported production observation.
//! They accept only the proxy prelude, H2 bootstrap and RSD handshake; an
//! additional service request or a reused stream fails the wire assertions.

use std::{io, sync::Arc};

use bytes::Bytes;
use clap::Parser;
use futures_util::{future::BoxFuture, FutureExt};
use indexmap::IndexMap;
use ios_core::{
    decode_xpc_message, encode_xpc_message, xpc_message_flags as flags, DeviceInfo, PairRecord,
    ServiceStream, XpcMessage, XpcValue,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use super::super::super::{cancellable, DoctorCmd, Report, TargetReport};
use super::super::{Assessment, Source};
use super::*;

const UDID: &str = "owned-private-rsd-cli-device";
const SERVICE: &str = "owned.private.rsd.cli.service";
const FEATURE: &str = "owned-private-rsd-cli-feature";
const ENDPOINT: &str = "[fd00::da7a]:58783";
const BUDGET: Duration = Duration::from_secs(3);
const FIXTURE_BOUND: Duration = Duration::from_secs(5);

#[tokio::test]
async fn selected_existing_route_developer_probe_uses_actual_canonical_or_shim_dtx_and_original_socket_eof(
) {
    use ios_core::dtx::{encode_dtx, read_dtx_frame, DtxPayload};
    for (shim, malformed) in [(false, false), (true, false), (false, true)] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let request = request(&listener, None);
        let (check, ()) = tokio::time::timeout(FIXTURE_BOUND, async { tokio::join!(
            developer_probe(&request, UDID, BUDGET),
            async {
                let mut discovery = bootstrap(&listener).await;
                let mut catalog = dictionary();
                services(&mut catalog).clear();
                services(&mut catalog).insert(format!("{}{}", ios_core::instruments::SERVICE_IOS17, if shim { ".shim.remote" } else { "" }), service(Some(23456), Some(vec![])));
                reply(&mut discovery, catalog).await;
                eof(&mut discovery).await;
                let (mut peer, _) = listener.accept().await.unwrap();
                let mut prelude = [0; 20];
                peer.read_exact(&mut prelude).await.unwrap();
                let endpoint: SocketAddrV6 = ENDPOINT.parse().unwrap();
                assert_eq!(&prelude[..16], &endpoint.ip().octets());
                assert_eq!(u32::from_le_bytes(prelude[16..].try_into().unwrap()), 23456);
                if shim {
                    let checkin: plist::Dictionary = ios_core::recv_lockdown(&mut peer).await.unwrap();
                    assert_eq!(checkin["Request"].as_string(), Some("RSDCheckin"));
                    for operation in ["RSDCheckin", "StartService"] {
                        ios_core::send_lockdown(&mut peer, &plist::Dictionary::from_iter([("Request".to_string(), plist::Value::String(operation.into()))])).await.unwrap();
                    }
                }
                let channel = read_dtx_frame(&mut peer).await.unwrap();
                assert!(matches!(channel.payload, DtxPayload::MethodInvocation { ref selector, .. } if selector == "_requestChannelWithCode:identifier:"));
                peer.write_all(&encode_dtx(channel.identifier, 1, channel.channel_code, false, 3, &ios_core::archive_null(), &[])).await.unwrap();
                let query = read_dtx_frame(&mut peer).await.unwrap();
                assert!(matches!(query.payload, DtxPayload::MethodInvocation { ref selector, ref args } if selector == "runningProcesses" && args.is_empty()));
                let payload = if malformed { ios_core::archive_string("owned-private-malformed-result") } else { ios_core::archive_array(vec![]) };
                peer.write_all(&encode_dtx(query.identifier, 1, query.channel_code, false, 3, &payload, &[])).await.unwrap();
                assert_eq!(peer.read(&mut [0]).await.unwrap(), 0, "the original service socket was not shut down");
            }
        ) }).await.unwrap();
        assert_eq!(
            check.status,
            if malformed {
                Status::Failed
            } else {
                Status::Ok
            }
        );
        assert_eq!(check.value, if malformed { None } else { Some(true) });
        assert!(!serde_json::to_string(&check)
            .unwrap()
            .contains("owned-private"));
    }
}

#[derive(Parser)]
struct Arguments {
    #[command(flatten)]
    doctor: DoctorCmd,
}

fn arguments(args: &[&str]) -> DoctorCmd {
    Arguments::try_parse_from(std::iter::once("fixture").chain(args.iter().copied()))
        .unwrap()
        .doctor
}

fn dictionary() -> IndexMap<String, XpcValue> {
    IndexMap::from([
        ("MessageType".into(), XpcValue::String("Handshake".into())),
        (
            "Properties".into(),
            XpcValue::Dictionary(IndexMap::from([(
                "UniqueDeviceID".into(),
                XpcValue::String(UDID.into()),
            )])),
        ),
        (
            "Services".into(),
            XpcValue::Dictionary(IndexMap::from([(
                SERVICE.into(),
                service(Some(23456), Some(vec![FEATURE.into()])),
            )])),
        ),
    ])
}

fn service(port: Option<u64>, features: Option<Vec<String>>) -> XpcValue {
    let mut entry = IndexMap::new();
    if let Some(port) = port {
        entry.insert("Port".into(), XpcValue::Uint64(port));
    }
    if let Some(features) = features {
        entry.insert(
            "Properties".into(),
            XpcValue::Dictionary(IndexMap::from([(
                "Features".into(),
                XpcValue::Array(features.into_iter().map(XpcValue::String).collect()),
            )])),
        );
    }
    XpcValue::Dictionary(entry)
}

fn services(dictionary: &mut IndexMap<String, XpcValue>) -> &mut IndexMap<String, XpcValue> {
    let XpcValue::Dictionary(services) = dictionary.get_mut("Services").unwrap() else {
        panic!("fixture service directory must be a dictionary");
    };
    services
}

fn wire(dictionary: IndexMap<String, XpcValue>) -> Bytes {
    encode_xpc_message(&XpcMessage {
        flags: flags::ALWAYS_SET | flags::DATA,
        msg_id: 0,
        body: Some(XpcValue::Dictionary(dictionary)),
    })
    .unwrap()
}

fn frame(kind: u8, flag: u8, stream: u32, body: &[u8]) -> Vec<u8> {
    assert!(body.len() <= 0xff_ffff);
    let size = body.len() as u32;
    let mut output = Vec::with_capacity(9 + body.len());
    output.extend_from_slice(&size.to_be_bytes()[1..]);
    output.extend_from_slice(&[kind, flag]);
    output.extend_from_slice(&stream.to_be_bytes());
    output.extend_from_slice(body);
    output
}

async fn read_frame(peer: &mut TcpStream) -> (u8, u8, u32, Vec<u8>) {
    let mut header = [0; 9];
    peer.read_exact(&mut header).await.unwrap();
    let size =
        (usize::from(header[0]) << 16) | (usize::from(header[1]) << 8) | usize::from(header[2]);
    assert!(size <= 4096, "unexpected outgoing request allocation");
    let mut body = vec![0; size];
    peer.read_exact(&mut body).await.unwrap();
    (
        header[3],
        header[4],
        u32::from_be_bytes(header[5..].try_into().unwrap()),
        body,
    )
}

async fn bootstrap_prefix(listener: &TcpListener) -> TcpStream {
    let (mut peer, _) = listener.accept().await.unwrap();
    peer.set_nodelay(true).unwrap();
    let mut prelude = [0; 20];
    peer.read_exact(&mut prelude).await.unwrap();
    let endpoint: SocketAddrV6 = ENDPOINT.parse().unwrap();
    assert_eq!(&prelude[..16], &endpoint.ip().octets());
    assert_eq!(
        u32::from_le_bytes(prelude[16..].try_into().unwrap()),
        u32::from(endpoint.port())
    );
    let mut preface = [0; 24];
    peer.read_exact(&mut preface).await.unwrap();
    assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
    assert_eq!(read_frame(&mut peer).await.0, 4);
    assert_eq!(read_frame(&mut peer).await.0, 8);
    peer
}

async fn bootstrap(listener: &TcpListener) -> TcpStream {
    let mut peer = bootstrap_prefix(listener).await;
    peer.write_all(&frame(4, 0, 0, &[])).await.unwrap();
    for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
        let received = read_frame(&mut peer).await;
        assert_eq!((received.0, received.2), (kind, stream));
    }
    let ack = read_frame(&mut peer).await;
    assert_eq!((ack.0, ack.1, ack.2), (4, 1, 0));
    let sent = read_frame(&mut peer).await;
    assert_eq!((sent.0, sent.2), (0, 1));
    let handshake = decode_xpc_message(Bytes::from(sent.3)).unwrap();
    let body = handshake.body.unwrap();
    let body = body.as_dict().unwrap();
    assert_eq!(
        body.get("MessageType").and_then(XpcValue::as_str),
        Some("Handshake")
    );
    assert_eq!(
        body.get("Services")
            .and_then(XpcValue::as_dict)
            .unwrap()
            .len(),
        0
    );
    peer
}

async fn reply(peer: &mut TcpStream, dictionary: IndexMap<String, XpcValue>) {
    for chunk in wire(dictionary).chunks(1024) {
        peer.write_all(&frame(0, 0, 1, chunk)).await.unwrap();
    }
}

async fn eof(peer: &mut TcpStream) {
    let mut tail = Vec::new();
    if let Err(err) = peer.read_to_end(&mut tail).await {
        if err.kind() != std::io::ErrorKind::ConnectionReset {
            panic!("unexpected io error reading eof: {err:?}");
        }
    }
    let mut remaining = tail.as_slice();
    while !remaining.is_empty() {
        assert!(remaining.len() >= 9);
        let size = (usize::from(remaining[0]) << 16)
            | (usize::from(remaining[1]) << 8)
            | usize::from(remaining[2]);
        assert_eq!(
            remaining[3], 8,
            "observation executed an additional XPC/service request"
        );
        assert_eq!(size, 4);
        assert!(remaining.len() >= 9 + size);
        remaining = &remaining[9 + size..];
    }
}

fn request(listener: &TcpListener, selected: Option<&str>) -> Request {
    Request::from_selectors(
        Some(UDID),
        Some(ENDPOINT.parse().unwrap()),
        Some(listener.local_addr().unwrap().port()),
        selected,
    )
    .unwrap()
    .unwrap()
}

async fn roundtrip(dictionary: IndexMap<String, XpcValue>, selected: Option<&str>) -> [Check; 2] {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, selected);
    let (checks, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(inspect(&request, UDID, BUDGET), async {
            let mut peer = bootstrap(&listener).await;
            reply(&mut peer, dictionary).await;
            eof(&mut peer).await;
        })
    })
    .await
    .expect("owned RSD fixture exceeded its bound");
    assert!(
        listener.accept().now_or_never().is_none(),
        "matching reply must not start a compatibility fallback"
    );
    checks
}

fn assessment(checks: [Check; 2]) -> Assessment {
    Assessment {
        target: ios_core::redaction::diagnostic_id(UDID).to_string(),
        transport: "usbmux_existing_trust",
        checks: checks.into_iter().collect(),
    }
}

fn machine_error(assessment: &mut Assessment) -> (anyhow::Error, serde_json::Value) {
    let error = assessment
        .take_first_failed_error()
        .unwrap()
        .context("doctor selected device assessment");
    let json = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "doctor",
        &error,
    )
    .unwrap();
    for secret in [UDID, SERVICE, FEATURE, "fd00::da7a", "23456", "58783"] {
        assert!(
            !json.contains(secret),
            "machine error leaked private fixture data"
        );
    }
    (error, serde_json::from_str(&json).unwrap())
}

fn unknown(checks: &[Check; 2]) {
    for check in checks {
        assert_eq!(check.status, Status::Unknown);
        assert_eq!(check.state, State::MetadataMissing);
        assert_eq!(check.value, None);
        assert!(check.error.is_none());
    }
}

#[tokio::test]
async fn fresh_proxy_handshake_matches_identity_and_redacts_report() {
    let checks = roundtrip(dictionary(), Some(SERVICE)).await;
    for check in &checks {
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.state, State::Available);
        assert_eq!(check.value, Some(true));
    }
    let host = Report {
        schema_version: 1,
        scope: "host",
        os: "test",
        arch: "test",
        version: "test",
        checks: vec![],
    };
    let report = TargetReport::new(host, assessment(checks));
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["scope"], "host_and_device");
    assert_eq!(
        json["device"]["checks"][0]["rsd"]["path"],
        "userspace_proxy"
    );
    assert_eq!(
        json["device"]["checks"][0]["rsd"]["selected_service"],
        "advertised"
    );
    assert_eq!(
        json["device"]["checks"][0]["rsd"]["features"],
        serde_json::json!({"state":"present","count":1})
    );
    let surfaces = format!("{} {report:?}", serde_json::to_string(&report).unwrap());
    for secret in [UDID, SERVICE, FEATURE, "fd00::da7a", "23456", "58783"] {
        assert!(
            !surfaces.contains(secret),
            "safe report leaked private fixture data"
        );
    }
    assert!(!report.failed());
}

#[tokio::test]
async fn missing_identity_directory_or_port_never_infers_a_negative_capability() {
    let mut no_identity = dictionary();
    no_identity.insert("Properties".into(), XpcValue::Dictionary(IndexMap::new()));
    let mut no_directory = dictionary();
    no_directory.shift_remove("Services");
    let mut no_port = dictionary();
    services(&mut no_port).insert(SERVICE.into(), service(None, None));
    let mut canonical_incomplete = dictionary();
    services(&mut canonical_incomplete).insert(SERVICE.into(), service(None, None));
    services(&mut canonical_incomplete).insert(
        format!("{SERVICE}.shim.remote"),
        service(Some(34567), Some(vec![])),
    );
    let mut unrelated_incomplete = dictionary();
    services(&mut unrelated_incomplete).clear();
    services(&mut unrelated_incomplete)
        .insert("owned.other.private.service".into(), service(None, None));
    for dictionary in [
        no_identity,
        no_directory,
        no_port,
        canonical_incomplete,
        unrelated_incomplete,
    ] {
        unknown(&roundtrip(dictionary, Some(SERVICE)).await);
    }
}

#[tokio::test]
async fn identity_mismatch_fails_without_attributing_directory_to_target() {
    let mut dictionary = dictionary();
    dictionary.insert(
        "Properties".into(),
        XpcValue::Dictionary(IndexMap::from([(
            "UniqueDeviceID".into(),
            XpcValue::String("owned-other-private-device".into()),
        )])),
    );
    let checks = roundtrip(dictionary, Some(SERVICE)).await;
    for check in &checks {
        assert_eq!(check.state, State::Failed);
        assert_eq!(check.status, Status::Failed);
        assert_eq!(check.value, None);
        assert_eq!(check.error_class, Some(ErrorClass::Precondition));
        assert_eq!(check.rsd.as_ref().unwrap().directory_complete, None);
    }
    let (_, output) = machine_error(&mut assessment(checks));
    assert_eq!(output["error"]["class"], "precondition");
    assert!(!output.to_string().contains("owned-other-private-device"));
}

#[tokio::test]
async fn malformed_matching_handshakes_preserve_protocol_failure_without_fallback() {
    let mut wrong_identity = dictionary();
    wrong_identity.insert(
        "Properties".into(),
        XpcValue::Dictionary(IndexMap::from([(
            "UniqueDeviceID".into(),
            XpcValue::Bool(true),
        )])),
    );
    let mut wrong_directory = dictionary();
    wrong_directory.insert("Services".into(), XpcValue::Array(vec![]));
    let mut wrong_entry = dictionary();
    services(&mut wrong_entry).insert(
        SERVICE.into(),
        XpcValue::String("owned-private-invalid-entry".into()),
    );
    let mut zero_port = dictionary();
    services(&mut zero_port).insert(SERVICE.into(), service(Some(0), None));
    let mut over_port = dictionary();
    services(&mut over_port).insert(SERVICE.into(), service(Some(65536), None));
    let mut wrong_features = dictionary();
    services(&mut wrong_features).insert(
        SERVICE.into(),
        XpcValue::Dictionary(IndexMap::from([
            ("Port".into(), XpcValue::Uint64(23456)),
            (
                "Properties".into(),
                XpcValue::Dictionary(IndexMap::from([(
                    "Features".into(),
                    XpcValue::Array(vec![XpcValue::Bool(true)]),
                )])),
            ),
        ])),
    );
    for dictionary in [
        wrong_identity,
        wrong_directory,
        wrong_entry,
        zero_port,
        over_port,
        wrong_features,
    ] {
        let checks = roundtrip(dictionary, Some(SERVICE)).await;
        assert_eq!(checks[0].error_class, Some(ErrorClass::Protocol));
        assert_eq!(checks[1].value, None);
        let (error, output) = machine_error(&mut assessment(checks));
        assert!(error.downcast_ref::<ios_core::CoreError>().is_some());
        assert_eq!(output["error"]["class"], "protocol");
        assert!(!error
            .chain()
            .any(|source| source.is::<tokio::time::error::Elapsed>()));
    }
}

#[tokio::test]
async fn complete_directory_absence_is_only_advertisement_evidence() {
    let mut empty = dictionary();
    services(&mut empty).clear();
    let checks = roundtrip(empty, Some(SERVICE)).await;
    assert_eq!(checks[0].state, State::ServiceUnavailable);
    assert_eq!(checks[0].value, Some(false));
    assert_eq!(checks[1].state, State::Available);
    assert_eq!(checks[1].value, Some(true));
    assert!(checks[0].detail.contains("did not advertise"));
    let (_, output) = machine_error(&mut assessment(checks));
    assert_eq!(output["error"]["class"], "service_missing");
    let checks = roundtrip(dictionary(), None).await;
    assert_eq!(checks[0].value, Some(true));
    assert!(checks[0].rsd.as_ref().unwrap().selected_service.is_none());
}

#[tokio::test]
async fn missing_and_empty_features_remain_distinct_without_rejecting_advertised_service() {
    for (features, expected) in [
        (None, serde_json::json!({"state":"metadata_missing"})),
        (Some(vec![]), serde_json::json!({"state":"empty","count":0})),
        (
            Some(vec![FEATURE.into()]),
            serde_json::json!({"state":"present","count":1}),
        ),
    ] {
        let mut dictionary = dictionary();
        services(&mut dictionary).insert(SERVICE.into(), service(Some(23456), features));
        let checks = roundtrip(dictionary, Some(SERVICE)).await;
        assert_eq!(checks[0].state, State::Available);
        assert_eq!(checks[0].value, Some(true));
        assert_eq!(
            serde_json::to_value(&checks[0]).unwrap()["rsd"]["features"],
            expected
        );
    }
    let mut shim = dictionary();
    services(&mut shim).shift_remove(SERVICE);
    services(&mut shim).insert(format!("{SERVICE}.shim.remote"), service(Some(34567), None));
    assert_eq!(roundtrip(shim, Some(SERVICE)).await[0].value, Some(true));
}

#[tokio::test]
async fn retained_metadata_limit_is_resource_limit_and_exact_limit_succeeds() {
    for (size, expected) in [(4096, None), (4097, Some(ErrorClass::ResourceLimit))] {
        let name = "n".repeat(size);
        let mut dictionary = dictionary();
        services(&mut dictionary).clear();
        services(&mut dictionary).insert(name, service(Some(23456), None));
        let checks = roundtrip(dictionary, None).await;
        assert_eq!(checks[0].error_class, expected);
        if expected.is_some() {
            let (_, output) = machine_error(&mut assessment(checks));
            assert_eq!(output["error"]["class"], "resource_limit");
        } else {
            assert_eq!(checks[0].state, State::Available);
        }
    }
}

#[tokio::test]
async fn deadline_closes_current_peer_preserves_elapsed_and_later_query_uses_fresh_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, Some(SERVICE));
    // Resolve the production host identity before the short stalled-wire
    // budget, including the separately bounded macOS identity subprocess.
    let initial = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(inspect(&request, UDID, BUDGET), async {
            let mut peer = bootstrap(&listener).await;
            reply(&mut peer, dictionary()).await;
            eof(&mut peer).await;
        })
    })
    .await
    .unwrap()
    .0;
    assert_eq!(initial[0].state, State::Available);
    let (checks, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(inspect(&request, UDID, Duration::from_millis(500)), async {
            let mut peer = bootstrap(&listener).await;
            eof(&mut peer).await;
        })
    })
    .await
    .unwrap();
    assert_eq!(checks[0].state, State::Unreachable);
    assert_eq!(checks[0].error_class, Some(ErrorClass::Timeout));
    let (error, output) = machine_error(&mut assessment(checks));
    assert_eq!(output["error"]["class"], "timeout");
    assert!(error.downcast_ref::<ios_core::CoreError>().is_some());
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    let (fresh, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(inspect(&request, UDID, BUDGET), async {
            let mut peer = bootstrap(&listener).await;
            let mut empty = dictionary();
            services(&mut empty).clear();
            reply(&mut peer, empty).await;
            eof(&mut peer).await;
        })
    })
    .await
    .unwrap();
    assert_eq!(
        fresh[0].state,
        State::ServiceUnavailable,
        "fresh metadata must replace the old directory"
    );
    assert!(listener.accept().now_or_never().is_none());
}

#[tokio::test]
async fn blocked_h2_bootstrap_respects_total_budget_and_does_not_dial_a_late_fallback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, None);
    let (checks, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(inspect(&request, UDID, Duration::from_millis(500)), async {
            let mut peer = bootstrap_prefix(&listener).await;
            // The active compatibility mode queues its five bootstrap frames
            // before waiting for SETTINGS. Withhold SETTINGS deliberately.
            for (kind, stream) in [(1, 1), (0, 1), (1, 3), (0, 1), (0, 3)] {
                let received = read_frame(&mut peer).await;
                assert_eq!((received.0, received.2), (kind, stream));
            }
            eof(&mut peer).await;
        })
    })
    .await
    .unwrap();
    assert_eq!(checks[0].error_class, Some(ErrorClass::Timeout));
    let (error, output) = machine_error(&mut assessment(checks));
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    assert_eq!(output["error"]["class"], "timeout");
    assert!(
        listener.accept().now_or_never().is_none(),
        "expired total budget must not poll the next connector"
    );
}

#[tokio::test]
async fn cancellation_drops_only_current_owned_socket_and_keeps_proxy_listener_usable() {
    let cancel = super::super::super::ProbeCancel::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, None);
    let (signal, wait) = tokio::sync::oneshot::channel();
    let (result, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(
            cancellable(
                super::super::super::until_cancelled(
                    async { Ok(inspect(&request, UDID, BUDGET).await) },
                    &cancel
                ),
                async {
                    wait.await
                        .map_err(|_| io::Error::other("owned cancellation sender dropped"))
                },
                &cancel
            ),
            async {
                let mut peer = bootstrap(&listener).await;
                signal.send(()).unwrap();
                eof(&mut peer).await;
            }
        )
    })
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert!(error.is::<ios_core::OperationCancelled>());
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ErrorClass::Cancelled
    );
    let (fresh, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(inspect(&request, UDID, BUDGET), async {
            let mut peer = bootstrap(&listener).await;
            reply(&mut peer, dictionary()).await;
            eof(&mut peer).await;
        })
    })
    .await
    .unwrap();
    assert_eq!(fresh[1].value, Some(true));
}

#[tokio::test]
async fn direct_future_drop_closes_current_socket_without_stopping_existing_proxy() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, None);
    let mut query = Box::pin(inspect(&request, UDID, BUDGET));
    let mut peer = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::select! {
            peer = bootstrap(&listener) => peer,
            _ = &mut query => panic!("stalled discovery unexpectedly completed"),
        }
    })
    .await
    .unwrap();
    drop(query);
    tokio::time::timeout(FIXTURE_BOUND, eof(&mut peer))
        .await
        .unwrap();
    assert!(listener.accept().now_or_never().is_none());
}

#[tokio::test]
async fn refused_owned_proxy_retains_native_io_and_safe_machine_category() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, None);
    drop(listener);
    let checks = tokio::time::timeout(FIXTURE_BOUND, inspect(&request, UDID, BUDGET))
        .await
        .unwrap();
    assert_eq!(checks[0].state, State::Unreachable);
    assert_eq!(checks[0].error_class, Some(ErrorClass::Io));
    let (error, output) = machine_error(&mut assessment(checks));
    assert!(error.downcast_ref::<ios_core::CoreError>().is_some());
    assert!(error.chain().any(|source| source
        .downcast_ref::<io::Error>()
        .is_some_and(|io| io.kind() == io::ErrorKind::ConnectionRefused)));
    assert_eq!(output["error"]["class"], "io");
}

struct NeverSource;

impl Source for NeverSource {
    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>>> {
        panic!("unexpected device enumeration")
    }
    fn pair_record<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Arc<PairRecord>>> {
        panic!("unexpected trust read")
    }
    fn port(&self, _: u32, _: u16) -> BoxFuture<'_, Result<ServiceStream>> {
        panic!("unexpected device connection")
    }
}

struct EmptySource;

impl Source for EmptySource {
    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>>> {
        Box::pin(async { Ok(vec![]) })
    }
    fn pair_record<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Arc<PairRecord>>> {
        panic!("unexpected trust read")
    }
    fn port(&self, _: u32, _: u16) -> BoxFuture<'_, Result<ServiceStream>> {
        panic!("unexpected device connection")
    }
}

fn assert_manifest_check_retains_separate_missing_target_failure(check: &super::super::Check) {
    assert_eq!(check.id, "ddi_manifests");
    assert_eq!(check.status, Status::Failed);
    assert_eq!(check.state, State::Unreachable);
    assert_eq!(check.value, None);
    assert_eq!(check.error_class, Some(ErrorClass::DeviceNotFound));
    assert!(matches!(
        check
            .error
            .as_ref()
            .unwrap()
            .downcast_ref::<ios_core::CoreError>(),
        Some(ios_core::CoreError::DeviceNotFound(_))
    ));
    assert!(check.rsd.is_none());
    let json = serde_json::to_value(check).unwrap();
    assert_eq!(json["ddi_manifests"]["mounted_entries"], 0);
    assert_eq!(json["ddi_manifests"]["developer_entries"], 0);
    assert_eq!(json["ddi_manifests"]["manifests_returned"], 0);
    assert_eq!(json["ddi_manifests"]["signature_bytes"], 0);
    assert_eq!(json["ddi_manifests"]["manifest_bytes"], 0);
    assert_eq!(json["ddi_manifests"]["complete"], false);
    assert!(!serde_json::to_string(check).unwrap().contains(UDID));
}

#[tokio::test]
async fn integrated_rsd_checks_run_after_existing_trust_failures_and_keep_original_order() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let request = request(&listener, Some(SERVICE));
    let (mut assessment, ()) = tokio::time::timeout(FIXTURE_BOUND, async {
        tokio::join!(
            super::super::inspect_with_rsd(&EmptySource, UDID, BUDGET, &request),
            async {
                let mut peer = bootstrap(&listener).await;
                reply(&mut peer, dictionary()).await;
                eof(&mut peer).await;
            }
        )
    })
    .await
    .unwrap();
    let ids: Vec<_> = assessment.checks.iter().map(|check| check.id).collect();
    assert_eq!(
        ids,
        [
            "trust_session",
            "developer_mode",
            "ddi_signatures",
            "rsd_services",
            "device_route",
            "ddi_mounted",
            "ddi_usable",
            "ddi_manifests"
        ]
    );
    assert!(assessment.checks[..3]
        .iter()
        .all(|check| check.error_class == Some(ErrorClass::DeviceNotFound)));
    assert!(assessment.checks[3..5]
        .iter()
        .all(|check| check.value == Some(true)));
    // Existing RSD success is independent of the appended existing-trust
    // manifest probe; it cannot supply missing mounted signatures for it.
    assert_manifest_check_retains_separate_missing_target_failure(&assessment.checks[7]);
    // Failure ownership/order is unchanged: successful RSD evidence does not
    // erase or replace an earlier acquired existing-trust native cause.
    let error = assessment.take_first_failed_error().unwrap();
    assert!(matches!(
        error.downcast_ref::<ios_core::CoreError>(),
        Some(ios_core::CoreError::DeviceNotFound(_))
    ));
    let json = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "doctor",
        &error,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["error"]["class"],
        "device_not_found"
    );
    assert!(!json.contains(UDID));
}

#[tokio::test]
async fn selectors_are_rejected_before_host_source_or_listener_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let endpoint = ENDPOINT.parse::<SocketAddrV6>().unwrap();
    for (udid, address, proxy, selected, class) in [
        (
            None,
            Some(endpoint),
            Some(port),
            None,
            ErrorClass::InvalidInput,
        ),
        (Some(UDID), None, Some(port), None, ErrorClass::InvalidInput),
        (
            Some(UDID),
            None,
            None,
            Some(SERVICE),
            ErrorClass::InvalidInput,
        ),
        (
            Some("../private"),
            Some(endpoint),
            Some(port),
            None,
            ErrorClass::InvalidInput,
        ),
        (
            Some(UDID),
            Some("[::1]:0".parse().unwrap()),
            None,
            None,
            ErrorClass::InvalidInput,
        ),
        (
            Some(UDID),
            Some(endpoint),
            Some(0),
            None,
            ErrorClass::InvalidInput,
        ),
        (
            Some(UDID),
            Some(SocketAddrV6::new(*endpoint.ip(), endpoint.port(), 0, 3)),
            Some(port),
            None,
            ErrorClass::InvalidInput,
        ),
    ] {
        let error = Request::from_selectors(udid, address, proxy, selected)
            .err()
            .unwrap();
        assert_eq!(ios_core::classify_error(error.as_ref()), class);
    }
    let command = arguments(&["--rsd-address", ENDPOINT]);
    let error = command.run(None, true).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ErrorClass::InvalidInput
    );
    let mut command = arguments(&["--rsd-address", ENDPOINT]);
    command.rsd_proxy_port = Some(port);
    command.rsd_address = None;
    let error = command.run(Some(UDID.into()), true).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ErrorClass::InvalidInput
    );
    let request = request(&listener, None);
    let assessment =
        super::super::inspect_with_rsd(&NeverSource, "../private", BUDGET, &request).await;
    assert!(assessment
        .checks
        .iter()
        .all(|check| check.error_class == Some(ErrorClass::InvalidInput)));
    assert!(listener.accept().now_or_never().is_none());
}

#[test]
fn selector_byte_limit_and_ipv6_numeric_scope_are_validated_without_io() {
    let endpoint: SocketAddrV6 = ENDPOINT.parse().unwrap();
    let exact = "é".repeat(2048);
    assert!(Request::from_selectors(Some(UDID), Some(endpoint), None, Some(&exact)).is_ok());
    let over = format!("{exact}x");
    let error = Request::from_selectors(Some(UDID), Some(endpoint), None, Some(&over))
        .err()
        .unwrap();
    let json = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "doctor",
        &error,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap()["error"]["class"],
        "resource_limit"
    );
    assert!(!json.contains(&over));
    let scoped = arguments(&["--rsd-address", "[fe80::1%3]:58783"]);
    let request = scoped.rsd_request(Some(UDID)).unwrap().unwrap();
    assert_eq!(request.route.endpoint().scope_id(), 3);
    assert!(request.route.proxy_port().is_none());
    for args in [
        vec!["fixture", "--rsd-proxy-port", "1234"],
        vec!["fixture", "--rsd-service", SERVICE],
        vec!["fixture", "--rsd-address", "127.0.0.1:1234"],
        vec![
            "fixture",
            "--rsd-address",
            ENDPOINT,
            "--rsd-proxy-port",
            "0",
        ],
    ] {
        assert!(Arguments::try_parse_from(args).is_err());
    }
}

#[tokio::test]
async fn no_selectors_preserve_schema_and_existing_checks_with_separate_ddi_probes() {
    let command = arguments(&[]);
    assert!(command.rsd_request(None).unwrap().is_none());
    assert!(command.rsd_request(Some(UDID)).unwrap().is_none());
    let device = super::super::inspect(&EmptySource, UDID, BUDGET).await;
    let ids: Vec<_> = device.checks.iter().map(|check| check.id).collect();
    assert_eq!(
        ids,
        [
            "trust_session",
            "developer_mode",
            "ddi_signatures",
            "rsd_services",
            "device_route",
            "ddi_mounted",
            "ddi_usable",
            "ddi_manifests"
        ]
    );
    assert_eq!(device.checks[3].state, State::NotProbed);
    assert_eq!(device.checks[4].state, State::NotProbed);
    // Omitting existing-route selectors leaves only the two route checks
    // unprobed; an explicit target still owns its independent manifest probe.
    assert_manifest_check_retains_separate_missing_target_failure(&device.checks[7]);
    let host = Report {
        schema_version: 1,
        scope: "host",
        os: "test",
        arch: "test",
        version: "test",
        checks: vec![],
    };
    let host_json = serde_json::to_value(&host).unwrap();
    assert_eq!(host_json["schema_version"], 1);
    assert_eq!(host_json["scope"], "host");
    assert!(host_json.get("device").is_none());
    let report = TargetReport::new(host, device);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["device"]["transport"], "usbmux_existing_trust");
    for check in json["device"]["checks"].as_array().unwrap() {
        assert!(check.get("rsd").is_none());
    }
}

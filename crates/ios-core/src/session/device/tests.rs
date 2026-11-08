//! Real owned peers record wire and drain EOF before behavioral assertions.
//! They emulate usbmux + lockdown only; no hardware functionality is certified.

use super::*;
use crate::session::SharedAdmission;
use std::{error::Error, io};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::lockdown::test_assets;

const UDID: &str = "owned-private-device-query";
const TOTAL: Duration = Duration::from_secs(3);
const FIXTURE: Duration = Duration::from_secs(5);

fn dictionary<const N: usize>(fields: [(&str, plist::Value); N]) -> plist::Dictionary {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

fn record() -> Arc<PairRecord> {
    Arc::new(PairRecord {
        device_certificate: vec![],
        host_certificate: test_assets::TEST_CERT_PEM.to_vec(),
        host_private_key: test_assets::TEST_KEY_PEM.to_vec().into(),
        root_certificate: vec![],
        host_id: "owned-private-query-host".into(),
        system_buid: "owned-private-query-buid".into(),
        wifi_mac_address: None,
    })
}

fn entry(udid: &str, id: u32, connection: &str, product: Option<u16>) -> plist::Value {
    let mut properties = dictionary([
        ("SerialNumber", udid.into()),
        ("ConnectionType", connection.into()),
    ]);
    if let Some(product) = product {
        properties.insert("ProductID".into(), u64::from(product).into());
    }
    plist::Value::Dictionary(dictionary([
        ("DeviceID", u64::from(id).into()),
        ("Properties", plist::Value::Dictionary(properties)),
    ]))
}

fn metadata(entries: Vec<plist::Value>) -> plist::Dictionary {
    dictionary([("DeviceList", plist::Value::Array(entries))])
}

fn frame(stage: usize, reply: &plist::Dictionary) -> Vec<u8> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, reply).unwrap();
    if stage < 2 {
        crate::mux::protocol::encode_message(&body, stage as u32 + 1).unwrap()
    } else {
        crate::lockdown::protocol::encode_frame(&body)
    }
}

#[derive(Clone)]
struct Script {
    frames: [Vec<u8>; 5],
    stop: usize,
    fault: Fault,
    tls: bool,
}

#[derive(Clone)]
enum Fault {
    None,
    Header(usize),
    Partial(usize),
    Wait,
    WaitTls,
    Raw(Vec<u8>),
}

impl Script {
    fn new(reply: plist::Dictionary) -> Self {
        let frames = [
            frame(0, &metadata(vec![entry(UDID, 7, "USB", None)])),
            frame(
                1,
                &dictionary([("MessageType", "Result".into()), ("Number", 0u64.into())]),
            ),
            frame(
                2,
                &dictionary([
                    ("Request", "QueryType".into()),
                    ("Type", "com.apple.mobile.lockdown".into()),
                ]),
            ),
            frame(
                3,
                &dictionary([
                    ("Request", "StartSession".into()),
                    ("SessionID", "owned-private-query-session".into()),
                    ("EnableSessionSSL", false.into()),
                ]),
            ),
            frame(4, &reply),
        ];
        Self {
            frames,
            stop: 4,
            fault: Fault::None,
            tls: false,
        }
    }
    fn value(value: &str) -> Self {
        Self::new(dictionary([
            ("Request", "GetValue".into()),
            ("Value", value.into()),
        ]))
    }
    fn tls(mut self) -> Self {
        self.tls = true;
        self.frames[3] = frame(
            3,
            &dictionary([
                ("Request", "StartSession".into()),
                ("SessionID", "owned-private-query-session".into()),
                ("EnableSessionSSL", true.into()),
            ]),
        );
        self
    }
    fn wire(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }
}

struct Receipt {
    requests: Vec<plist::Dictionary>,
    tags: Vec<u32>,
    tail: Vec<u8>,
    eof: bool,
}

async fn mux_request<S: AsyncRead + Unpin>(peer: &mut S) -> (plist::Dictionary, u32) {
    let mut header = [0; 16];
    peer.read_exact(&mut header).await.unwrap();
    let length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let mut body = vec![0; length - 16];
    peer.read_exact(&mut body).await.unwrap();
    (
        plist::from_bytes(&body).unwrap(),
        u32::from_le_bytes(header[12..].try_into().unwrap()),
    )
}

async fn lockdown_request<S: AsyncRead + Unpin>(peer: &mut S) -> plist::Dictionary {
    crate::lockdown::recv_lockdown(peer).await.unwrap()
}

fn acceptor() -> tokio_rustls::TlsAcceptor {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let chain = rustls_pemfile::certs(&mut io::Cursor::new(test_assets::TEST_CERT_PEM))
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    let key = rustls_pemfile::private_key(&mut io::Cursor::new(test_assets::TEST_KEY_PEM))
        .unwrap()
        .unwrap();
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    config.send_tls13_tickets = 0;
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

async fn write_fault<S: AsyncWrite + Unpin>(peer: &mut S, bytes: &[u8], fault: &Fault) {
    match fault {
        Fault::None => peer.write_all(bytes).await.unwrap(),
        Fault::Header(length) | Fault::Partial(length) => {
            peer.write_all(&bytes[..*length]).await.unwrap()
        }
        Fault::Raw(bytes) => peer.write_all(bytes).await.unwrap(),
        Fault::Wait | Fault::WaitTls => {}
    }
}

async fn serve(listener: TcpListener, script: Script, ready: oneshot::Sender<()>) -> Receipt {
    let (peer, _) = listener.accept().await.unwrap();
    peer.set_nodelay(true).unwrap();
    serve_peer(peer, script, ready).await
}

async fn serve_peer<S: AsyncRead + AsyncWrite + Unpin + Send>(
    mut peer: S,
    script: Script,
    ready: oneshot::Sender<()>,
) -> Receipt {
    let mut requests = Vec::new();
    let mut tags = Vec::new();
    let mut ready = Some(ready);
    for stage in 0..=script.stop.min(3) {
        if stage < 2 {
            let (request, tag) = mux_request(&mut peer).await;
            requests.push(request);
            tags.push(tag);
        } else {
            requests.push(lockdown_request(&mut peer).await);
        }
        let fault = if stage == script.stop {
            &script.fault
        } else {
            &Fault::None
        };
        if stage == script.stop && matches!(fault, Fault::WaitTls) {
            peer.write_all(&script.frames[stage]).await.unwrap();
            let mut hello = [0; 5];
            peer.read_exact(&mut hello).await.unwrap();
            let _ = ready.take().unwrap().send(());
            let mut tail = hello.to_vec();
            peer.read_to_end(&mut tail).await.unwrap();
            return Receipt {
                requests,
                tags,
                tail,
                eof: true,
            };
        }
        write_fault(&mut peer, &script.frames[stage], fault).await;
        if stage == script.stop {
            let _ = ready.take().unwrap().send(());
        }
    }
    let mut tail = Vec::new();
    if script.stop == 4 && script.tls {
        let mut tls = acceptor().accept(peer).await.unwrap();
        requests.push(lockdown_request(&mut tls).await);
        write_fault(&mut tls, &script.frames[4], &script.fault).await;
        let _ = ready.take().unwrap().send(());
        let (mut raw, _) = tls.into_inner();
        raw.read_to_end(&mut tail).await.unwrap();
    } else {
        if script.stop == 4 {
            requests.push(lockdown_request(&mut peer).await);
            write_fault(&mut peer, &script.frames[4], &script.fault).await;
            let _ = ready.take().unwrap().send(());
        }
        peer.read_to_end(&mut tail).await.unwrap();
    }
    Receipt {
        requests,
        tags,
        tail,
        eof: true,
    }
}

struct Case {
    device: ExistingTrustDevice,
    shared: SharedAdmission,
    cancel: OperationCancel,
    ready: oneshot::Receiver<()>,
    job: JoinHandle<Result<DeviceQueryObservation, CoreError>>,
    peer: JoinHandle<Receipt>,
}

fn limits(wire: usize, output: usize) -> DeviceQueryLimits {
    DeviceQueryLimits::new(wire, output).unwrap()
}

async fn start(
    script: Script,
    query: DeviceQuery,
    limits: DeviceQueryLimits,
    total: Duration,
) -> Case {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = UsbmuxEndpoint::tcp(listener.local_addr().unwrap());
    let shared = SharedAdmission::new(1).unwrap();
    let owner = SessionOwner::new(shared.clone(), 1).unwrap();
    let device = ExistingTrustDevice::new(UDID.into(), record(), owner, endpoint).unwrap();
    let cancel = OperationCancel::new();
    let future = device
        .prepare_query(query, limits, OperationOptions::new(total), cancel.clone())
        .unwrap();
    let (tx, ready) = oneshot::channel();
    let peer = tokio::spawn(serve(listener, script, tx));
    let job = tokio::spawn(future); // Meaningful Send + 'static public-future check.
    Case {
        device,
        shared,
        cancel,
        ready,
        job,
        peer,
    }
}

async fn finish(case: Case) -> (Result<DeviceQueryObservation, CoreError>, Receipt) {
    let Case {
        device,
        shared,
        job,
        peer,
        ..
    } = case;
    let result = tokio::time::timeout(FIXTURE, job).await.unwrap().unwrap();
    let receipt = tokio::time::timeout(FIXTURE, peer).await.unwrap().unwrap();
    device.close(TOTAL).await.unwrap();
    device.close(TOTAL).await.unwrap();
    // Cleanup above precedes these and every semantic assertion below.
    assert!(receipt.eof);
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    (result, receipt)
}

fn wire(receipt: &Receipt, key: Option<&str>, id: u32) {
    assert_eq!(receipt.tags, [1, 2]);
    assert_eq!(receipt.requests.len(), 5);
    assert!(receipt.tail.is_empty());
    let names = [
        "ListDevices",
        "Connect",
        "QueryType",
        "StartSession",
        "GetValue",
    ];
    for (index, request) in receipt.requests.iter().enumerate() {
        let name = if index < 2 {
            request["MessageType"].as_string()
        } else {
            request["Request"].as_string()
        };
        assert_eq!(name, Some(names[index]));
    }
    assert_eq!(
        receipt.requests[1]["DeviceID"].as_unsigned_integer(),
        Some(u64::from(id))
    );
    assert_eq!(
        receipt.requests[1]["PortNumber"].as_unsigned_integer(),
        Some(u64::from(62078u16.swap_bytes()))
    );
    assert_eq!(receipt.requests[2].len(), 2);
    assert_eq!(
        receipt.requests[3]["HostID"].as_string(),
        Some("owned-private-query-host")
    );
    assert_eq!(
        receipt.requests[3]["SystemBUID"].as_string(),
        Some("owned-private-query-buid")
    );
    assert_eq!(
        receipt.requests[3]["ProtocolVersion"].as_string(),
        Some("2")
    );
    assert_eq!(
        receipt.requests[4]
            .get("Key")
            .and_then(plist::Value::as_string),
        key
    );
    assert!(!receipt.requests[4].contains_key("Domain"));
    assert_eq!(receipt.requests[4].len(), if key.is_some() { 3 } else { 2 });
}

fn source<'a, T: Error + 'static>(mut error: &'a (dyn Error + 'static)) -> Option<&'a T> {
    for _ in 0..32 {
        if let Some(value) = error.downcast_ref::<T>() {
            return Some(value);
        }
        error = if let Some(io) = error.downcast_ref::<io::Error>() {
            io.get_ref()
                .map(|inner| inner as &(dyn Error + 'static))
                .or_else(|| io.source())?
        } else {
            error.source()?
        };
    }
    None
}

fn class(result: Result<DeviceQueryObservation, CoreError>, expected: ErrorClass) -> CoreError {
    let error = result.unwrap_err();
    assert_eq!(crate::classify_error(&error), expected);
    let text = format!("{error:?} {}", crate::redaction::diagnostic_error(&error));
    assert!(!text.contains("owned-private"));
    error
}

#[tokio::test]
async fn typed_fields_plaintext_and_tls_have_exact_wire_and_empty_string_presence() {
    for (query, value) in [
        (DeviceQuery::product_type(), "iPhone42,1"),
        (DeviceQuery::product_version(), "26.0"),
        (DeviceQuery::device_name(), ""),
    ] {
        for tls in [false, true] {
            let script = if tls {
                Script::value(value).tls()
            } else {
                Script::value(value)
            };
            let (result, receipt) =
                finish(start(script.clone(), query, limits(script.wire(), 256), TOTAL).await).await;
            let value_result = result.unwrap();
            wire(&receipt, query.key(), 7);
            assert!(value_result.value_present());
            assert_eq!(value_result.enable_session_ssl(), tls);
            assert_eq!(value_result.metadata().product_id(), None);
            let returned = match query.key().unwrap() {
                "ProductType" => value_result.product_type(),
                "ProductVersion" => value_result.product_version(),
                _ => value_result.device_name(),
            };
            assert_eq!(returned, Some(value));
            assert!(!format!("{value_result:?}").contains(value) || value.is_empty());
        }
    }
}

#[tokio::test]
async fn root_subset_and_absent_value_are_distinct_from_present_empty_dictionary() {
    let fields = DeviceInfoFields::new()
        .with_product_type()
        .with_device_name();
    let query = DeviceQuery::root(fields).unwrap();
    for (reply, present, product, name) in [
        (
            dictionary([("Request", "GetValue".into())]),
            false,
            None,
            None,
        ),
        (
            dictionary([("Value", plist::Value::Dictionary(plist::Dictionary::new()))]),
            true,
            None,
            None,
        ),
        (
            dictionary([(
                "Value",
                plist::Value::Dictionary(dictionary([
                    ("ProductType", "iPhone42,1".into()),
                    ("DeviceName", "".into()),
                    ("ProductVersion", false.into()),
                    ("PrivateIgnoredField", "owned-private-unselected".into()),
                ])),
            )]),
            true,
            Some("iPhone42,1"),
            Some(""),
        ),
    ] {
        let script = Script::new(reply);
        let (result, receipt) =
            finish(start(script.clone(), query, limits(script.wire(), 256), TOTAL).await).await;
        let result = result.unwrap();
        wire(&receipt, None, 7);
        assert_eq!(result.fields(), fields);
        assert_eq!(result.value_present(), present);
        assert_eq!(result.product_type(), product);
        assert_eq!(result.device_name(), name);
        assert_eq!(result.product_version(), None);
        assert!(!format!("{result:?}").contains("owned-private"));
    }
    let script = Script::new(plist::Dictionary::new());
    let (result, receipt) = finish(
        start(
            script.clone(),
            DeviceQuery::device_name(),
            limits(script.wire(), 3),
            TOTAL,
        )
        .await,
    )
    .await;
    assert!(!result.unwrap().value_present());
    wire(&receipt, Some("DeviceName"), 7);
}

#[tokio::test]
async fn explicit_remote_error_precedes_value_and_strict_selected_types() {
    for (reply, expected, code) in [
        (
            dictionary([("Error", "MissingValue".into()), ("Value", false.into())]),
            ErrorClass::NotFound,
            Some("MissingValue"),
        ),
        (
            dictionary([
                ("Error", "owned-private-unknown-code".into()),
                ("Value", "".into()),
            ]),
            ErrorClass::ServiceFailure,
            Some("owned-private-unknown-code"),
        ),
        (
            dictionary([("Error", false.into()), ("Value", "".into())]),
            ErrorClass::Protocol,
            None,
        ),
        (
            dictionary([("Value", false.into())]),
            ErrorClass::Protocol,
            None,
        ),
        (
            dictionary([("Value", 0u64.into())]),
            ErrorClass::Protocol,
            None,
        ),
        (
            dictionary([("Value", plist::Value::Dictionary(plist::Dictionary::new()))]),
            ErrorClass::Protocol,
            None,
        ),
        (
            dictionary([("Request", "StartService".into()), ("Value", "".into())]),
            ErrorClass::Protocol,
            None,
        ),
    ] {
        let script = Script::new(reply);
        let (result, _) = finish(
            start(
                script.clone(),
                DeviceQuery::device_name(),
                limits(script.wire(), 256),
                TOTAL,
            )
            .await,
        )
        .await;
        let error = class(result, expected);
        assert_eq!(
            source::<crate::LockdownResponseError>(&error).map(|error| error.code()),
            code
        );
    }
    let query = DeviceQuery::root(DeviceInfoFields::all()).unwrap();
    for value in [
        false.into(),
        plist::Value::Dictionary(dictionary([("ProductVersion", 0u64.into())])),
    ] {
        let script = Script::new(dictionary([("Value", value)]));
        let (result, _) =
            finish(start(script.clone(), query, limits(script.wire(), 256), TOTAL).await).await;
        class(result, ErrorClass::Protocol);
    }
}

#[tokio::test]
async fn metadata_is_fully_validated_and_usb_selection_retains_optional_product_id() {
    for product in [None, Some(0), Some(1234)] {
        let mut script = Script::value("26.0");
        script.frames[0] = frame(
            0,
            &metadata(vec![
                entry("other-device", 2, "USB", None),
                entry(UDID, 9, "Network", Some(55)),
                entry(UDID, 8, "uSb", product),
                entry(UDID, 10, "USB", Some(66)),
            ]),
        );
        let (result, receipt) = finish(
            start(
                script.clone(),
                DeviceQuery::product_version(),
                limits(script.wire(), 256),
                TOTAL,
            )
            .await,
        )
        .await;
        let result = result.unwrap();
        wire(&receipt, Some("ProductVersion"), 8);
        assert_eq!(result.metadata().product_id(), product);
        assert_eq!(result.metadata().connection_type(), "uSb");
    }
    for entries in [
        vec![],
        vec![entry(&format!("{UDID}-suffix"), 7, "USB", None)],
        vec![entry(UDID, 7, "USB", None), plist::Value::Boolean(false)],
    ] {
        let expected = if entries
            .last()
            .is_some_and(|value| value.as_boolean().is_some())
        {
            ErrorClass::Protocol
        } else {
            ErrorClass::DeviceNotFound
        };
        let mut script = Script::value("26.0");
        script.frames[0] = frame(0, &metadata(entries));
        script.stop = 0;
        let (result, receipt) = finish(
            start(
                script.clone(),
                DeviceQuery::product_version(),
                limits(script.wire(), 256),
                TOTAL,
            )
            .await,
        )
        .await;
        class(result, expected);
        assert_eq!(receipt.requests.len(), 1);
        assert!(receipt.tail.is_empty());
    }
}

#[tokio::test]
async fn exact_wire_metadata_and_unicode_output_limits_reject_plus_one_without_truncation() {
    let query = DeviceQuery::root(DeviceInfoFields::all()).unwrap();
    let script = Script::new(dictionary([(
        "Value",
        plist::Value::Dictionary(dictionary([
            ("ProductType", "é".into()),
            ("ProductVersion", "26".into()),
            ("DeviceName", "设备".into()),
        ])),
    )]));
    let output = 3 + "é".len() + "26".len() + "设备".len();
    let metadata_bytes = UDID.len() + 3;
    let exact = limits(script.wire(), output)
        .with_metadata_limits(1, metadata_bytes)
        .unwrap();
    let (result, receipt) = finish(start(script.clone(), query, exact, TOTAL).await).await;
    let result = result.unwrap();
    wire(&receipt, None, 7);
    assert_eq!(result.device_name(), Some("设备"));
    for (limits, stop, prefix_only) in [
        (limits(script.wire() - 1, output), 4, true),
        (limits(script.wire(), output - 1), 4, false),
        (
            limits(script.wire(), output)
                .with_metadata_limits(1, metadata_bytes - 1)
                .unwrap(),
            0,
            false,
        ),
        (
            limits(script.wire(), output)
                .with_mux_frame_bytes(script.frames[0].len() - 1)
                .unwrap(),
            0,
            true,
        ),
    ] {
        let mut rejected = script.clone();
        rejected.stop = stop;
        if prefix_only {
            rejected.fault = Fault::Header(if stop == 0 { 16 } else { 4 });
        }
        let (result, receipt) = finish(start(rejected, query, limits, TOTAL).await).await;
        class(result, ErrorClass::ResourceLimit);
        assert_eq!(receipt.requests.len(), stop + 1);
    }
    let mut two = script.clone();
    two.frames[0] = frame(
        0,
        &metadata(vec![
            entry(UDID, 7, "USB", None),
            entry("other", 8, "Network", None),
        ]),
    );
    two.stop = 0;
    let limits = limits(two.wire(), output)
        .with_metadata_limits(1, 4096)
        .unwrap();
    let (result, _) = finish(start(two, query, limits, TOTAL).await).await;
    class(result, ErrorClass::ResourceLimit);
}

#[tokio::test]
async fn declared_header_limits_and_bad_mux_headers_retire_socket_before_body() {
    for (stage, declared, expected) in [
        (0, MAX_MUX_FRAME + 1, ErrorClass::ResourceLimit),
        (0, 15, ErrorClass::Protocol),
        (4, MAX_LOCKDOWN_BODY + 1, ErrorClass::ResourceLimit),
        (4, u32::MAX as usize, ErrorClass::ResourceLimit),
    ] {
        let mut script = Script::value("ok");
        script.stop = stage;
        let raw = if stage == 0 {
            let mut header = crate::mux::protocol::encode_message(&[], 1).unwrap();
            header[..4].copy_from_slice(&(declared as u32).to_le_bytes());
            header
        } else {
            (declared as u32).to_be_bytes().to_vec()
        };
        script.fault = Fault::Raw(raw);
        let (result, receipt) =
            finish(start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await).await;
        class(result, expected);
        assert_eq!(receipt.requests.len(), stage + 1);
        assert!(receipt.tail.is_empty());
    }
    for offset in [4, 8, 12] {
        let mut script = Script::value("ok");
        script.stop = 0;
        let mut header = script.frames[0][..16].to_vec();
        header[offset..offset + 4].copy_from_slice(&77u32.to_le_bytes());
        script.fault = Fault::Raw(header);
        let (result, _) =
            finish(start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await).await;
        class(result, ErrorClass::Protocol);
    }
    let mut script = Script::value(&"x".repeat(1024));
    script.stop = 4;
    script.fault = Fault::Header(4);
    let limit = script.frames[4].len() - 5;
    // Earlier small replies fit; only the declared GetValue body exceeds cap.
    let configured = limits(8192, 4096).with_lockdown_body_bytes(limit).unwrap();
    let (result, _) =
        finish(start(script, DeviceQuery::device_name(), configured, TOTAL).await).await;
    class(result, ErrorClass::ResourceLimit);
}

#[tokio::test]
async fn partial_reply_eof_and_real_parser_failure_preserve_original_causes() {
    for stage in 0..=4 {
        let mut script = Script::value("ok");
        script.stop = stage;
        script.fault = Fault::Raw(vec![1, 2]);
        // Raw partial data followed by an actual FIN, not a synthetic read error.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = UsbmuxEndpoint::tcp(listener.local_addr().unwrap());
        let shared = SharedAdmission::new(1).unwrap();
        let device = ExistingTrustDevice::new(
            UDID.into(),
            record(),
            SessionOwner::new(shared.clone(), 1).unwrap(),
            endpoint,
        )
        .unwrap();
        let future = device
            .prepare_query(
                DeviceQuery::device_name(),
                limits(8192, 256),
                OperationOptions::new(TOTAL),
                OperationCancel::new(),
            )
            .unwrap();
        let peer = tokio::spawn(async move {
            let (mut peer, _) = listener.accept().await.unwrap();
            let mut requests = Vec::new();
            for index in 0..=stage {
                if index < 2 {
                    requests.push(mux_request(&mut peer).await.0);
                } else {
                    requests.push(lockdown_request(&mut peer).await);
                }
                if index == stage {
                    peer.write_all(&[1, 2]).await.unwrap();
                    peer.shutdown().await.unwrap();
                } else {
                    peer.write_all(&script.frames[index]).await.unwrap();
                }
            }
            let mut tail = Vec::new();
            peer.read_to_end(&mut tail).await.unwrap();
            (requests, tail)
        });
        let result = tokio::time::timeout(FIXTURE, future).await.unwrap();
        let (_, tail) = tokio::time::timeout(FIXTURE, peer).await.unwrap().unwrap();
        device.close(TOTAL).await.unwrap();
        assert_eq!(device.active(), 0);
        assert_eq!(shared.available(), 1);
        let error = result.unwrap_err();
        assert!(tail.is_empty());
        assert_eq!(
            source::<io::Error>(&error).unwrap().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    for stage in [0, 4] {
        let mut script = Script::value("ok");
        script.stop = stage;
        script.fault = Fault::Raw(if stage == 0 {
            crate::mux::protocol::encode_message(b"not a plist", 1).unwrap()
        } else {
            crate::lockdown::protocol::encode_frame(b"not a plist")
        });
        let (result, _) =
            finish(start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await).await;
        assert!(source::<plist::Error>(&result.unwrap_err()).is_some());
    }
}

#[tokio::test]
async fn cancel_and_total_during_each_reply_close_original_peer_and_release_admission() {
    for stage in 0..=4 {
        for by_cancel in [true, false] {
            let mut script = Script::value("ok");
            script.stop = stage;
            script.fault = Fault::Wait;
            let mut case = start(
                script,
                DeviceQuery::device_name(),
                limits(8192, 256),
                if by_cancel {
                    TOTAL
                } else {
                    Duration::from_millis(500)
                },
            )
            .await;
            tokio::time::timeout(FIXTURE, &mut case.ready)
                .await
                .unwrap()
                .unwrap();
            if by_cancel {
                case.cancel.cancel();
            }
            let (result, receipt) = finish(case).await;
            let error = class(
                result,
                if by_cancel {
                    ErrorClass::Cancelled
                } else {
                    ErrorClass::Timeout
                },
            );
            assert_eq!(receipt.requests.len(), stage + 1);
            assert!(receipt.tail.is_empty());
            if !by_cancel {
                assert!(source::<tokio::time::error::Elapsed>(&error).is_some());
            }
        }
    }
    for by_cancel in [true, false] {
        let mut script = Script::value("ok").tls();
        script.stop = 3;
        script.fault = Fault::WaitTls;
        let mut case = start(
            script,
            DeviceQuery::device_name(),
            limits(8192, 256),
            if by_cancel {
                TOTAL
            } else {
                Duration::from_millis(500)
            },
        )
        .await;
        tokio::time::timeout(FIXTURE, &mut case.ready)
            .await
            .unwrap()
            .unwrap();
        if by_cancel {
            case.cancel.cancel();
        }
        let (result, receipt) = finish(case).await;
        let error = class(
            result,
            if by_cancel {
                ErrorClass::Cancelled
            } else {
                ErrorClass::Timeout
            },
        );
        assert_eq!(receipt.tail.first(), Some(&22));
        if !by_cancel {
            assert!(source::<tokio::time::error::Elapsed>(&error).is_some());
        }
    }
}

#[tokio::test]
async fn last_client_drop_cancels_active_query_and_does_not_hide_an_owner_clone() {
    let mut script = Script::value("ok");
    script.fault = Fault::Wait;
    let mut case = start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await;
    tokio::time::timeout(FIXTURE, &mut case.ready)
        .await
        .unwrap()
        .unwrap();
    let Case {
        device,
        shared,
        job,
        peer,
        ..
    } = case;
    drop(device);
    let result = tokio::time::timeout(FIXTURE, job).await.unwrap().unwrap();
    let receipt = tokio::time::timeout(FIXTURE, peer).await.unwrap().unwrap();
    assert!(receipt.eof);
    assert!(receipt.tail.is_empty());
    assert_eq!(shared.available(), 1);
    class(result, ErrorClass::Cancelled);
}

#[tokio::test]
async fn unpolled_close_timeout_keeps_counts_and_factory_never_dials_after_fence() {
    for mode in 0..4 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let shared = SharedAdmission::new(1).unwrap();
        let device = ExistingTrustDevice::new(
            UDID.into(),
            record(),
            SessionOwner::new(shared.clone(), 1).unwrap(),
            UsbmuxEndpoint::tcp(listener.local_addr().unwrap()),
        )
        .unwrap();
        let cancel = OperationCancel::new();
        let future = device
            .prepare_query(
                DeviceQuery::device_name(),
                limits(8192, 256),
                OperationOptions::new(if mode == 3 {
                    Duration::from_millis(1)
                } else {
                    TOTAL
                }),
                cancel.clone(),
            )
            .unwrap();
        let close_error = if mode == 0 {
            Some(device.close(Duration::from_millis(1)).await.unwrap_err())
        } else {
            None
        };
        let retained = (device.active(), shared.available(), device.is_closed());
        let result = match mode {
            0 => {
                drop(future);
                None
            }
            1 => {
                cancel.cancel();
                Some(future.await)
            }
            2 => {
                device.request_close();
                Some(future.await)
            }
            _ => {
                tokio::time::sleep(Duration::from_millis(5)).await;
                Some(future.await)
            }
        };
        let no_dial = tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err();
        drop(listener);
        device.close(TOTAL).await.unwrap();
        device.close(TOTAL).await.unwrap();
        assert_eq!(device.active(), 0);
        assert_eq!(shared.available(), 1);
        assert!(no_dial);
        if let Some(error) = close_error {
            assert_eq!(retained, (1, 0, true));
            assert!(source::<tokio::time::error::Elapsed>(&error).is_some());
        }
        if let Some(result) = result {
            class(
                result,
                if mode == 3 {
                    ErrorClass::Timeout
                } else {
                    ErrorClass::Cancelled
                },
            );
        }
    }
}

#[tokio::test]
async fn zero_overflow_and_precancel_fail_before_native_factory() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        UsbmuxEndpoint::tcp(listener.local_addr().unwrap()),
    )
    .unwrap();
    let mut errors = Vec::new();
    for total in [Duration::ZERO, Duration::MAX] {
        errors.push(
            device
                .prepare_query(
                    DeviceQuery::device_name(),
                    limits(8192, 256),
                    OperationOptions::new(total),
                    OperationCancel::new(),
                )
                .unwrap_err(),
        );
    }
    let cancel = OperationCancel::new();
    cancel.cancel();
    errors.push(
        device
            .prepare_query(
                DeviceQuery::device_name(),
                limits(8192, 256),
                OperationOptions::new(TOTAL),
                cancel,
            )
            .unwrap_err(),
    );
    let no_dial = tokio::time::timeout(Duration::from_millis(10), listener.accept())
        .await
        .is_err();
    drop(listener);
    device.close(TOTAL).await.unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    assert!(no_dial);
    assert_eq!(
        errors
            .iter()
            .map(|error| crate::classify_error(error))
            .collect::<Vec<_>>(),
        [
            ErrorClass::Timeout,
            ErrorClass::InvalidInput,
            ErrorClass::Cancelled
        ]
    );
    for (wire, output) in [(0, 1), (1, 0), (usize::MAX, 1), (1, usize::MAX)] {
        assert_eq!(
            crate::classify_error(&DeviceQueryLimits::new(wire, output).unwrap_err()),
            ErrorClass::InvalidInput
        );
    }
    let defaults = limits(8192, 256);
    assert!(defaults.with_mux_frame_bytes(MAX_MUX_FRAME + 1).is_err());
    assert!(defaults
        .with_lockdown_body_bytes(MAX_LOCKDOWN_BODY + 1)
        .is_err());
    assert!(defaults.with_metadata_limits(0, 1).is_err());
    assert!(DeviceQuery::root(DeviceInfoFields::new()).is_err());
}

#[tokio::test]
async fn cancelled_partial_old_reply_is_retired_and_next_query_rediscovers_fresh_id() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        UsbmuxEndpoint::tcp(listener.local_addr().unwrap()),
    )
    .unwrap();
    let mut old = Script::value("owned-private-old-value");
    // Exactly the length prefix: charge is observable only after the actual
    // client consumes all four bytes. The declared body remains incomplete.
    old.fault = Fault::Partial(4);
    let charged_remaining = 8192 - old.wire();
    let mut fresh = Script::value("fresh-value");
    fresh.frames[0] = frame(0, &metadata(vec![entry(UDID, 93, "Network", Some(0))]));
    let (ready_tx, ready) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (peer, _) = listener.accept().await.unwrap();
        let old = serve_peer(peer, old, ready_tx).await;
        let (peer, _) = listener.accept().await.unwrap();
        let (tx, _) = oneshot::channel();
        let fresh = serve_peer(peer, fresh, tx).await;
        (old, fresh)
    });
    let cancel = OperationCancel::new();
    let old = device
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            cancel.clone(),
        )
        .unwrap();
    let observed = Arc::clone(&old.observed_budget);
    let old = tokio::spawn(old);
    tokio::time::timeout(FIXTURE, ready).await.unwrap().unwrap();
    tokio::time::timeout(FIXTURE, async {
        while observed.remaining() != charged_remaining {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    cancel.cancel();
    let old_result = tokio::time::timeout(FIXTURE, old).await.unwrap().unwrap();
    let fresh = tokio::spawn(
        device
            .prepare_query(
                DeviceQuery::device_name(),
                limits(8192, 256),
                OperationOptions::new(TOTAL),
                OperationCancel::new(),
            )
            .unwrap(),
    );
    let fresh_result = tokio::time::timeout(FIXTURE, fresh).await.unwrap().unwrap();
    let (old, fresh) = tokio::time::timeout(FIXTURE, server)
        .await
        .unwrap()
        .unwrap();
    device.close(TOTAL).await.unwrap();
    assert!(old.eof && fresh.eof);
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    class(old_result, ErrorClass::Cancelled);
    wire(&fresh, Some("DeviceName"), 93);
    let fresh_result = fresh_result.unwrap();
    assert_eq!(fresh_result.device_name(), Some("fresh-value"));
    assert_eq!(fresh_result.metadata().product_id(), Some(0));
}

#[tokio::test]
async fn no_queue_and_two_device_owners_close_independently_on_actual_sockets() {
    let shared = SharedAdmission::new(3).unwrap();
    let mut devices = Vec::new();
    let mut jobs = Vec::new();
    let mut peers = Vec::new();
    let mut readies = Vec::new();
    for _ in 0..2 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let device = ExistingTrustDevice::new(
            UDID.into(),
            record(),
            SessionOwner::new(shared.clone(), 1).unwrap(),
            UsbmuxEndpoint::tcp(listener.local_addr().unwrap()),
        )
        .unwrap();
        let mut script = Script::value("ok");
        script.fault = Fault::Wait;
        let (tx, ready) = oneshot::channel();
        peers.push(tokio::spawn(serve(listener, script, tx)));
        jobs.push(tokio::spawn(
            device
                .prepare_query(
                    DeviceQuery::device_name(),
                    limits(8192, 256),
                    OperationOptions::new(TOTAL),
                    OperationCancel::new(),
                )
                .unwrap(),
        ));
        devices.push(device);
        readies.push(ready);
    }
    for ready in readies {
        tokio::time::timeout(FIXTURE, ready).await.unwrap().unwrap();
    }
    let conflict = devices[0]
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap_err();
    devices[0].close(TOTAL).await.unwrap();
    let isolated = (
        devices[1].is_closed(),
        devices[1].active(),
        jobs[1].is_finished(),
    );
    devices[1].close(TOTAL).await.unwrap();
    let mut results = Vec::new();
    for job in jobs {
        results.push(tokio::time::timeout(FIXTURE, job).await.unwrap().unwrap());
    }
    let mut receipts = Vec::new();
    for peer in peers {
        receipts.push(tokio::time::timeout(FIXTURE, peer).await.unwrap().unwrap());
    }
    for device in &devices {
        device.close(TOTAL).await.unwrap();
    }
    assert_eq!(isolated, (false, 1, false));
    assert_eq!(shared.available(), 3);
    assert_eq!(crate::classify_error(&conflict), ErrorClass::Conflict);
    for (device, (result, receipt)) in devices.iter().zip(results.into_iter().zip(receipts)) {
        assert_eq!(device.active(), 0);
        assert!(receipt.eof);
        assert!(receipt.tail.is_empty());
        class(result, ErrorClass::Cancelled);
    }
    let shared = SharedAdmission::new(1).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = UsbmuxEndpoint::tcp(listener.local_addr().unwrap());
    let a = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        endpoint.clone(),
    )
    .unwrap();
    let b = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        endpoint,
    )
    .unwrap();
    let retained = a
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap();
    let full = b
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap_err();
    drop(retained);
    a.close(TOTAL).await.unwrap();
    b.close(TOTAL).await.unwrap();
    let no_dial = tokio::time::timeout(Duration::from_millis(10), listener.accept())
        .await
        .is_err();
    drop(listener);
    assert!(no_dial);
    assert_eq!(shared.available(), 1);
    assert_eq!(a.active(), 0);
    assert_eq!(b.active(), 0);
    assert_eq!(crate::classify_error(&full), ErrorClass::ResourceLimit);
}

#[tokio::test]
async fn actual_dial_refusal_keeps_os_error_and_unpolled_job_drop_returns_permits() {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let addr = socket.local_addr().unwrap();
    #[cfg(target_os = "macos")]
    drop(socket);
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        UsbmuxEndpoint::tcp(addr),
    )
    .unwrap();
    let abandoned = device
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap();
    drop(abandoned);
    let result = device
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap()
        .await;
    #[cfg(not(target_os = "macos"))]
    drop(socket);
    device.close(TOTAL).await.unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    let error = result.unwrap_err();
    let original = source::<io::Error>(&error).unwrap();
    assert_eq!(original.kind(), io::ErrorKind::ConnectionRefused);
    assert!(original.raw_os_error().is_some());
}

#[tokio::test]
async fn mux_error_and_checked_trust_failure_are_typed_and_never_replayed() {
    for stage in [0, 1, 3] {
        let mut script = Script::value("ok");
        script.stop = stage;
        script.frames[stage] = if stage < 2 {
            frame(
                stage,
                &dictionary([("MessageType", "Result".into()), ("Number", 73u64.into())]),
            )
        } else {
            frame(
                stage,
                &dictionary([
                    ("Error", "InvalidHostID".into()),
                    ("EnableSessionSSL", false.into()),
                ]),
            )
        };
        let (result, receipt) =
            finish(start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await).await;
        assert_eq!(receipt.requests.len(), stage + 1);
        assert!(receipt.tail.is_empty());
        let error = result.unwrap_err();
        if stage < 2 {
            let remote = source::<MuxResponseError>(&error).unwrap();
            assert_eq!(remote.code(), 73);
            assert_eq!(remote.class(), ErrorClass::ServiceFailure);
            // Root registers this additive marker with centralized classification.
            assert_eq!(crate::classify_error(&error), ErrorClass::ServiceFailure);
        } else {
            assert_eq!(crate::classify_error(&error), ErrorClass::PermissionDenied);
            assert_eq!(
                source::<crate::LockdownResponseError>(&error)
                    .unwrap()
                    .operation(),
                "StartSession"
            );
        }
    }
    for stage in [0, 1] {
        let mut script = Script::value("ok");
        script.stop = stage;
        script.frames[stage] = frame(stage, &dictionary([("Number", 0u64.into())]));
        let (result, _) =
            finish(start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await).await;
        class(result, ErrorClass::Protocol);
    }
}

#[tokio::test]
async fn total_clock_includes_unpolled_wait_and_prior_completed_response_stages() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        UsbmuxEndpoint::tcp(listener.local_addr().unwrap()),
    )
    .unwrap();
    let script = Script::value("ok");
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let first = mux_request(&mut peer).await.0;
        tokio::time::sleep(Duration::from_millis(300)).await;
        peer.write_all(&script.frames[0]).await.unwrap();
        let second = mux_request(&mut peer).await.0;
        // This valid reply is later than the original total, but earlier than
        // a wrongly restarted Connect budget. No QueryType may follow it.
        let (mut reader, mut writer) = peer.into_split();
        let mut tail = Vec::new();
        let (closed, _) = tokio::join!(reader.read_to_end(&mut tail), async {
            tokio::time::sleep(Duration::from_millis(700)).await;
            // A late write can fail after the owned client has already closed.
            writer.write_all(&script.frames[1]).await
        });
        (first, second, tail, closed)
    });
    let future = device
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(Duration::from_millis(1000)),
            OperationCancel::new(),
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let result = tokio::time::timeout(FIXTURE, future).await.unwrap();
    let (first, second, tail, closed) = tokio::time::timeout(FIXTURE, server)
        .await
        .unwrap()
        .unwrap();
    device.close(TOTAL).await.unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    let error = class(result, ErrorClass::Timeout);
    assert!(source::<tokio::time::error::Elapsed>(&error).is_some());
    assert_eq!(first["MessageType"].as_string(), Some("ListDevices"));
    assert_eq!(second["MessageType"].as_string(), Some("Connect"));
    assert!(tail.is_empty());
    assert!(
        closed.is_ok() || closed.is_err_and(|error| error.kind() == io::ErrorKind::ConnectionReset)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn unix_endpoint_runs_same_finite_query_and_rejects_nul_without_io() {
    use std::os::unix::ffi::OsStringExt;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("ios-sdk-query-{}-{id}.sock", std::process::id()));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let endpoint = UsbmuxEndpoint::unix(path.clone()).unwrap();
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        endpoint,
    )
    .unwrap();
    let script = Script::value("unix-value");
    let server = tokio::spawn(async move {
        let (peer, _) = listener.accept().await.unwrap();
        let (tx, _) = oneshot::channel();
        serve_peer(peer, script, tx).await
    });
    let result = device
        .prepare_query(
            DeviceQuery::device_name(),
            limits(8192, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap()
        .await;
    let receipt = tokio::time::timeout(FIXTURE, server)
        .await
        .unwrap()
        .unwrap();
    device.close(TOTAL).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    wire(&receipt, Some("DeviceName"), 7);
    assert_eq!(result.unwrap().device_name(), Some("unix-value"));
    let nul = std::ffi::OsString::from_vec(b"owned-private\0socket".to_vec());
    assert!(UsbmuxEndpoint::unix(nul.into()).is_err());
    assert!(UsbmuxEndpoint::unix(PathBuf::new()).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn actual_partially_blocked_start_session_send_is_cancelled_then_drained_and_joined() {
    use std::os::fd::AsRawFd;
    let path = std::env::temp_dir().join(format!(
        "ios-sdk-query-blocked-send-{}.sock",
        std::process::id()
    ));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let shared = SharedAdmission::new(1).unwrap();
    let mut record = Arc::try_unwrap(record()).ok().unwrap();
    record.host_id = "x".repeat(8 * 1024 * 1024);
    let device = ExistingTrustDevice::new(
        UDID.into(),
        Arc::new(record),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        UsbmuxEndpoint::unix(path.clone()).unwrap(),
    )
    .unwrap();
    let (ready_tx, ready) = oneshot::channel();
    let (drain_tx, drain) = oneshot::channel();
    let script = Script::value("ok");
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let receive: libc::c_int = 4096;
        // SAFETY: the live owned peer fd and initialized c_int remain valid;
        // setsockopt copies exactly the supplied c_int bytes.
        let configured = unsafe {
            libc::setsockopt(
                peer.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&receive as *const libc::c_int).cast(),
                std::mem::size_of_val(&receive) as libc::socklen_t,
            )
        };
        let mut requests = Vec::new();
        for stage in 0..3 {
            if stage < 2 {
                requests.push(mux_request(&mut peer).await.0);
            } else {
                requests.push(lockdown_request(&mut peer).await);
            }
            peer.write_all(&script.frames[stage]).await.unwrap();
        }
        let declared = peer.read_u32().await.unwrap() as usize;
        let first = peer.read_u8().await.unwrap();
        let _ = ready_tx.send(());
        let _ = drain.await;
        #[cfg(not(target_os = "macos"))]
        let restored = {
            let receive: libc::c_int = 64 * 1024;
            // SAFETY: same live owned socket and correctly sized value as above.
            unsafe {
                libc::setsockopt(
                    peer.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_RCVBUF,
                    (&receive as *const libc::c_int).cast(),
                    std::mem::size_of_val(&receive) as libc::socklen_t,
                )
            }
        };
        #[cfg(target_os = "macos")]
        let restored = 0;
        let mut tail = Vec::new();
        peer.read_to_end(&mut tail).await.unwrap();
        (configured, restored, requests, declared, first, tail.len())
    });
    let cancel = OperationCancel::new();
    let job = tokio::spawn(
        device
            .prepare_query(
                DeviceQuery::device_name(),
                limits(8192, 256),
                OperationOptions::new(Duration::from_secs(10)),
                cancel.clone(),
            )
            .unwrap(),
    );
    tokio::time::timeout(FIXTURE, ready).await.unwrap().unwrap();
    cancel.cancel();
    let result = tokio::time::timeout(FIXTURE, job).await.unwrap().unwrap();
    let _ = drain_tx.send(());
    let (configured, restored, requests, declared, first, tail) =
        tokio::time::timeout(Duration::from_secs(20), server)
            .await
            .unwrap()
            .unwrap();
    device.close(TOTAL).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    class(result, ErrorClass::Cancelled);
    assert_eq!(configured, 0);
    assert_eq!(restored, 0);
    assert_eq!(requests.len(), 3);
    assert_eq!(first, b'<');
    assert!(tail + 1 < declared, "send completed instead of blocking");
}

#[tokio::test]
async fn real_tls_fatal_alert_preserves_rustls_cause_with_no_get_value_or_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        UsbmuxEndpoint::tcp(listener.local_addr().unwrap()),
    )
    .unwrap();
    let script = Script::value("ok").tls();
    let server = tokio::spawn(async move {
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut requests = Vec::new();
        for stage in 0..4 {
            if stage < 2 {
                requests.push(mux_request(&mut peer).await.0);
            } else {
                requests.push(lockdown_request(&mut peer).await);
            }
            peer.write_all(&script.frames[stage]).await.unwrap();
        }
        let mut header = [0; 5];
        peer.read_exact(&mut header).await.unwrap();
        let length = usize::from(u16::from_be_bytes(header[3..].try_into().unwrap()));
        let mut hello = vec![0; length];
        peer.read_exact(&mut hello).await.unwrap();
        peer.write_all(&[21, 3, 3, 0, 2, 2, 40]).await.unwrap();
        let mut tail = Vec::new();
        peer.read_to_end(&mut tail).await.unwrap();
        (requests, header, tail)
    });
    let job = tokio::spawn(
        device
            .prepare_query(
                DeviceQuery::device_name(),
                limits(8192, 256),
                OperationOptions::new(TOTAL),
                OperationCancel::new(),
            )
            .unwrap(),
    );
    let result = tokio::time::timeout(FIXTURE, job).await.unwrap().unwrap();
    let (requests, hello, _) = tokio::time::timeout(FIXTURE, server)
        .await
        .unwrap()
        .unwrap();
    device.close(TOTAL).await.unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    assert_eq!(requests.len(), 4);
    assert_eq!(hello[0], 22);
    let error = result.unwrap_err();
    assert!(source::<rustls::Error>(&error).is_some());
    assert!(source::<tokio::time::error::Elapsed>(&error).is_none());
}

#[tokio::test]
async fn utf8_udid_exact_limit_uses_actual_metadata_and_invalid_selectors_never_dial() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = UsbmuxEndpoint::tcp(listener.local_addr().unwrap());
    let udid = "é".repeat(2048);
    let shared = SharedAdmission::new(1).unwrap();
    let device = ExistingTrustDevice::new(
        udid.clone(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        endpoint.clone(),
    )
    .unwrap();
    let mut script = Script::value("ok");
    script.frames[0] = frame(0, &metadata(vec![entry(&udid, 7, "USB", None)]));
    let (tx, _) = oneshot::channel();
    let server = tokio::spawn(serve(listener, script, tx));
    let result = device
        .prepare_query(
            DeviceQuery::device_name(),
            limits(16384, 256),
            OperationOptions::new(TOTAL),
            OperationCancel::new(),
        )
        .unwrap()
        .await;
    let receipt = tokio::time::timeout(FIXTURE, server)
        .await
        .unwrap()
        .unwrap();
    device.close(TOTAL).await.unwrap();
    assert_eq!(device.active(), 0);
    assert_eq!(shared.available(), 1);
    wire(&receipt, Some("DeviceName"), 7);
    assert_eq!(result.unwrap().device_name(), Some("ok"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = UsbmuxEndpoint::tcp(listener.local_addr().unwrap());
    let mut errors = Vec::new();
    for udid in [
        String::new(),
        "owned-private\0id".into(),
        "é".repeat(2048) + "x",
    ] {
        errors.push(
            ExistingTrustDevice::new(
                udid,
                record(),
                SessionOwner::new(shared.clone(), 1).unwrap(),
                endpoint.clone(),
            )
            .unwrap_err(),
        );
    }
    let no_dial = tokio::time::timeout(Duration::from_millis(10), listener.accept())
        .await
        .is_err();
    drop(listener);
    assert!(no_dial);
    assert_eq!(shared.available(), 1);
    assert_eq!(
        errors
            .iter()
            .map(|error| crate::classify_error(error))
            .collect::<Vec<_>>(),
        [
            ErrorClass::InvalidInput,
            ErrorClass::InvalidInput,
            ErrorClass::ResourceLimit
        ]
    );
    let port_zero = UsbmuxEndpoint::tcp("127.0.0.1:0".parse().unwrap());
    let invalid = ExistingTrustDevice::new(
        UDID.into(),
        record(),
        SessionOwner::new(shared.clone(), 1).unwrap(),
        port_zero,
    )
    .unwrap_err();
    assert_eq!(crate::classify_error(&invalid), ErrorClass::InvalidInput);
    assert_eq!(shared.available(), 1);
}

#[tokio::test]
async fn last_non_usb_match_and_optional_list_message_type_have_explicit_checked_contract() {
    for message in [
        None,
        Some(plist::Value::String("ListDevices".into())),
        Some(plist::Value::String("Attached".into())),
        Some(plist::Value::Boolean(false)),
    ] {
        let mut list = metadata(vec![
            entry(UDID, 4, "Network", None),
            entry("other-device", 6, "USB", None),
            entry(UDID, 5, "Network", Some(0)),
        ]);
        let valid = match message.as_ref() {
            None => true,
            Some(message) => message.as_string() == Some("ListDevices"),
        };
        if let Some(message) = message {
            list.insert("MessageType".into(), message);
        }
        let mut script = Script::value("ok");
        script.frames[0] = frame(0, &list);
        if !valid {
            script.stop = 0;
        }
        let (result, receipt) =
            finish(start(script, DeviceQuery::device_name(), limits(8192, 256), TOTAL).await).await;
        if valid {
            let result = result.unwrap();
            wire(&receipt, Some("DeviceName"), 5);
            assert_eq!(result.metadata().product_id(), Some(0));
        } else {
            class(result, ErrorClass::Protocol);
            assert_eq!(receipt.requests.len(), 1);
        }
    }
}

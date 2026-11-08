use std::{collections::VecDeque, io, sync::Mutex};

use rustls::pki_types::PrivateKeyDer;
use tokio::io::{AsyncRead, AsyncReadExt, DuplexStream};

use super::*;

#[path = "manifest_tests.rs"]
mod manifest_tests;
#[path = "test_assets.rs"]
mod test_assets;

const UDID: &str = "owned-private-doctor-device";
const IMAGE_PORT: u16 = 1234;

#[test]
fn image_mounter_unknown_command_is_an_explicit_unsupported_observation() {
    use ios_core::imagemounter::ImageMounterError;
    for probe in [Probe::DeveloperMode, Probe::DdiManifests] {
        let check = Check::failure(
            probe,
            ImageMounterError::DeviceError("UnknownCommand".into()).into(),
        );
        assert_eq!(check.state, State::Unsupported);
        assert_eq!(check.status, Status::Unknown);
        assert_eq!(check.error_class, Some(ErrorClass::ServiceFailure));
        assert!(check.error.is_some());
    }
    let other = Check::failure(
        Probe::DeveloperMode,
        ImageMounterError::DeviceError("OtherError".into()).into(),
    );
    assert_eq!(other.state, State::Failed);
    assert_eq!(other.status, Status::Failed);
    let other_probe = Check::failure(
        Probe::DdiSignatures,
        ImageMounterError::DeviceError("UnknownCommand".into()).into(),
    );
    assert_eq!(other_probe.state, State::Failed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Devices,
    PairRecord,
    Port(u32, u16),
}

struct FakeSource {
    devices: Vec<DeviceInfo>,
    credentials: Arc<PairRecord>,
    ports: Mutex<VecDeque<(u16, ServiceStream)>>,
    calls: Mutex<Vec<Call>>,
    pair_failure: Option<io::ErrorKind>,
    port_failure: Option<io::ErrorKind>,
    port_delay: Duration,
}

impl FakeSource {
    fn new(ports: Vec<(u16, DuplexStream)>) -> Self {
        Self {
            devices: vec![
                DeviceInfo {
                    device_id: 3,
                    udid: UDID.into(),
                    connection_type: "Network".into(),
                    product_id: 0,
                },
                DeviceInfo {
                    device_id: 7,
                    udid: UDID.into(),
                    connection_type: "USB".into(),
                    product_id: 0,
                },
            ],
            credentials: Arc::new(record()),
            ports: Mutex::new(
                ports
                    .into_iter()
                    .map(|(port, stream)| (port, Box::new(stream) as ServiceStream))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
            pair_failure: None,
            port_failure: None,
            port_delay: Duration::ZERO,
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl Source for FakeSource {
    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>>> {
        self.calls.lock().unwrap().push(Call::Devices);
        Box::pin(async { Ok(self.devices.clone()) })
    }

    fn pair_record<'a>(&'a self, udid: &'a str) -> BoxFuture<'a, Result<Arc<PairRecord>>> {
        assert_eq!(udid, UDID);
        self.calls.lock().unwrap().push(Call::PairRecord);
        Box::pin(async {
            if let Some(kind) = self.pair_failure {
                return Err(io::Error::new(kind, "owned-private-pair-path").into());
            }
            Ok(Arc::clone(&self.credentials))
        })
    }

    fn port(&self, device_id: u32, port: u16) -> BoxFuture<'_, Result<ServiceStream>> {
        self.calls.lock().unwrap().push(Call::Port(device_id, port));
        Box::pin(async move {
            if self.port_delay != Duration::ZERO {
                std::thread::sleep(self.port_delay);
            }
            if let Some(kind) = self.port_failure {
                return Err(io::Error::new(kind, "owned-private-port-source").into());
            }
            let (expected, stream) = self
                .ports
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra device connection");
            assert_eq!(port, expected);
            assert_eq!(
                device_id, 7,
                "doctor must prefer the explicit matching USB entry"
            );
            Ok(stream)
        })
    }
}

fn record() -> PairRecord {
    PairRecord {
        device_certificate: vec![],
        host_certificate: test_assets::TEST_CERT_PEM.to_vec(),
        host_private_key: test_assets::TEST_KEY_PEM.to_vec().into(),
        root_certificate: vec![],
        host_id: "owned-private-host-id".into(),
        system_buid: "owned-private-system-buid".into(),
        wifi_mac_address: None,
    }
}

fn acceptor() -> tokio_rustls::TlsAcceptor {
    acceptor_with_versions(rustls::DEFAULT_VERSIONS)
}

fn acceptor_with_versions(
    versions: &[&'static rustls::SupportedProtocolVersion],
) -> tokio_rustls::TlsAcceptor {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut certificate = std::io::Cursor::new(test_assets::TEST_CERT_PEM);
    let chain = rustls_pemfile::certs(&mut certificate)
        .collect::<io::Result<Vec<_>>>()
        .unwrap();
    let key: PrivateKeyDer<'static> =
        rustls_pemfile::private_key(&mut std::io::Cursor::new(test_assets::TEST_KEY_PEM))
            .unwrap()
            .unwrap();
    let mut config = rustls::ServerConfig::builder_with_protocol_versions(versions)
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    // The synthetic service switches to raw DTX after the handshake. It must
    // not emit encrypted post-handshake session tickets into that raw phase.
    config.send_tls13_tickets = 0;
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

fn dictionary<const N: usize>(fields: [(&str, plist::Value); N]) -> plist::Dictionary {
    fields
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}

async fn request<S: AsyncRead + Unpin>(
    stream: &mut S,
    expected: &str,
    keys: &[&str],
) -> plist::Dictionary {
    let request: plist::Dictionary = recv_lockdown(stream).await.unwrap();
    assert_eq!(request["Request"].as_string(), Some(expected));
    assert_eq!(request.len(), keys.len());
    for key in keys {
        assert!(request.contains_key(key), "missing expected request key");
    }
    request
}

async fn plaintext_session_start(peer: &mut DuplexStream) {
    request(peer, "QueryType", &["Label", "Request"]).await;
    send_lockdown(
        peer,
        &dictionary([("Type", "com.apple.mobile.lockdown".into())]),
    )
    .await
    .unwrap();
    let start = request(
        peer,
        "StartSession",
        &[
            "Label",
            "Request",
            "ProtocolVersion",
            "HostID",
            "SystemBUID",
        ],
    )
    .await;
    assert_eq!(start["HostID"].as_string(), Some("owned-private-host-id"));
    assert_eq!(
        start["SystemBUID"].as_string(),
        Some("owned-private-system-buid")
    );
}

async fn tls_session(mut peer: DuplexStream) -> tokio_rustls::server::TlsStream<DuplexStream> {
    plaintext_session_start(&mut peer).await;
    send_lockdown(
        &mut peer,
        &dictionary([
            ("SessionID", "owned-private-session".into()),
            ("EnableSessionSSL", plist::Value::Boolean(true)),
        ]),
    )
    .await
    .unwrap();
    acceptor().accept(peer).await.unwrap()
}

async fn eof<S: AsyncRead + Unpin>(stream: &mut S) {
    match stream.read(&mut [0; 1]).await {
        Ok(0) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {}
        result => {
            panic!("doctor did not close its owned stream without additional requests: {result:?}")
        }
    }
}

async fn serve_value(peer: DuplexStream, reply: plist::Dictionary) {
    let mut tls = tls_session(peer).await;
    let value = request(&mut tls, "GetValue", &["Label", "Request", "Key"]).await;
    assert_eq!(value["Key"].as_string(), Some("ProductType"));
    send_lockdown(&mut tls, &reply).await.unwrap();
    eof(&mut tls).await;
}

async fn serve_service(peer: DuplexStream, reply: plist::Dictionary) {
    let mut tls = tls_session(peer).await;
    let service = request(&mut tls, "StartService", &["Label", "Request", "Service"]).await;
    assert_eq!(
        service["Service"].as_string(),
        Some(ios_core::imagemounter::protocol::SERVICE_NAME)
    );
    send_lockdown(&mut tls, &reply).await.unwrap();
    eof(&mut tls).await;
}

fn service_reply() -> plist::Dictionary {
    dictionary([
        ("Port", plist::Value::Integer(IMAGE_PORT.into())),
        ("EnableServiceSSL", plist::Value::Boolean(false)),
    ])
}

async fn image_request<S: AsyncRead + Unpin>(
    stream: &mut S,
    command: &str,
    image_type: Option<&str>,
) {
    let request: plist::Dictionary = recv_lockdown(stream).await.unwrap();
    assert_eq!(request["Command"].as_string(), Some(command));
    assert_eq!(request.len(), if image_type.is_some() { 2 } else { 1 });
    if let Some(image_type) = image_type {
        assert_eq!(request["ImageType"].as_string(), Some(image_type));
    }
}

async fn serve_developer(peer: &mut DuplexStream, status: Option<plist::Value>) {
    image_request(peer, "QueryDeveloperModeStatus", None).await;
    let reply = status
        .map(|value| dictionary([("DeveloperModeStatus", value)]))
        .unwrap_or_default();
    send_lockdown(peer, &reply).await.unwrap();
    eof(peer).await;
}

async fn serve_signatures(peer: &mut DuplexStream, fields: [Option<plist::Value>; 2]) {
    for (image_type, signature) in ["Developer", "Personalized"].into_iter().zip(fields) {
        image_request(peer, "LookupImage", Some(image_type)).await;
        let reply = signature
            .map(|value| dictionary([("ImageSignature", value)]))
            .unwrap_or_default();
        send_lockdown(peer, &reply).await.unwrap();
    }
    eof(peer).await;
}

fn safe(check: &Check) {
    let output = format!(
        "{} {check:?} {check:#?}",
        serde_json::to_string(check).unwrap()
    );
    assert!(
        !output.contains("owned-private"),
        "private fixture data leaked into report"
    );
    assert!(!output.contains("-----BEGIN"));
    assert!(!output.contains("fd00"));
}

fn machine_failure_matches_report(check: Check) {
    if check.status != Status::Failed {
        return;
    }
    let expected = check.error_class.unwrap();
    let mut assessment = Assessment {
        target: ios_core::redaction::diagnostic_id(UDID).to_string(),
        transport: "usbmux_existing_trust",
        checks: vec![check],
    };
    let primary = assessment
        .take_first_failed_error()
        .unwrap()
        .context("selected device diagnostics found a failed check")
        .context("outer safe diagnostic context");
    let message = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "doctor",
        &primary,
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&message).unwrap();
    assert_eq!(
        json["error"]["class"],
        serde_json::to_value(expected).unwrap()
    );
    assert_eq!(json["error"]["confirmation_required"], false);
    assert!(!message.contains(UDID));
    assert!(!message.contains("owned-private"));
}

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("owned doctor fixture did not finish")
}

async fn cancel_probe<T>(
    work: impl Future<Output = Result<T>>,
    signal: impl Future<Output = io::Result<()>>,
) -> Result<T> {
    let cancel = super::super::ProbeCancel::new();
    super::super::cancellable(
        super::super::until_cancelled(work, &cancel),
        signal,
        &cancel,
    )
    .await
}

fn has_elapsed(error: &anyhow::Error) -> bool {
    let mut current = Some(error.as_ref() as &(dyn Error + 'static));
    for _ in 0..32 {
        let Some(error) = current else {
            return false;
        };
        if error.is::<tokio::time::error::Elapsed>() {
            return true;
        }
        current = if let Some(error) = error.downcast_ref::<io::Error>() {
            error
                .get_ref()
                .map(|source| source as &(dyn Error + 'static))
                .or_else(|| error.source())
        } else {
            error.source()
        };
    }
    false
}

#[tokio::test]
async fn explicit_device_assessment_uses_six_fresh_readonly_sessions_and_no_tunnel_route() {
    let (trust, trust_peer) = tokio::io::duplex(65_536);
    let (developer, developer_peer) = tokio::io::duplex(65_536);
    let (image_developer, mut image_developer_peer) = tokio::io::duplex(65_536);
    let (ddi, ddi_peer) = tokio::io::duplex(65_536);
    let (image_ddi, mut image_ddi_peer) = tokio::io::duplex(65_536);
    let (mounted, mounted_peer) = tokio::io::duplex(65_536);
    let (image_mounted, mut image_mounted_peer) = tokio::io::duplex(65_536);
    let (usable, usable_peer) = tokio::io::duplex(65_536);
    let (manifest, manifest_peer) = tokio::io::duplex(65_536);
    let (image_manifest, mut image_manifest_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![
        (LOCKDOWN_PORT, trust),
        (LOCKDOWN_PORT, developer),
        (IMAGE_PORT, image_developer),
        (LOCKDOWN_PORT, ddi),
        (IMAGE_PORT, image_ddi),
        (LOCKDOWN_PORT, mounted),
        (IMAGE_PORT, image_mounted),
        (LOCKDOWN_PORT, usable),
        (LOCKDOWN_PORT, manifest),
        (IMAGE_PORT, image_manifest),
    ]);
    let (assessment, (), (), (), (), (), (), (), (), (), ()) = within(async {
        tokio::join!(
            inspect(&source, UDID, Duration::from_secs(2)),
            serve_value(
                trust_peer,
                dictionary([("Value", "owned-private-product-type".into())])
            ),
            serve_service(developer_peer, service_reply()),
            serve_developer(
                &mut image_developer_peer,
                Some(plist::Value::Boolean(false))
            ),
            serve_service(ddi_peer, service_reply()),
            serve_signatures(
                &mut image_ddi_peer,
                [
                    Some(plist::Value::Array(vec![])),
                    Some(plist::Value::Array(vec![]))
                ]
            ),
            serve_service(mounted_peer, service_reply()),
            async {
                image_request(&mut image_mounted_peer, "CopyDevices", None).await;
                send_lockdown(
                    &mut image_mounted_peer,
                    &dictionary([("EntryList", plist::Value::Array(vec![]))]),
                )
                .await
                .unwrap();
                eof(&mut image_mounted_peer).await;
            },
            async {
                let mut tls = tls_session(usable_peer).await;
                let query = request(&mut tls, "GetValue", &["Label", "Request", "Key"]).await;
                assert_eq!(query["Key"].as_string(), Some("ProductVersion"));
                send_lockdown(&mut tls, &dictionary([("Value", "17.0".into())]))
                    .await
                    .unwrap();
                eof(&mut tls).await;
            },
            serve_service(manifest_peer, service_reply()),
            async {
                image_request(&mut image_manifest_peer, "CopyDevices", None).await;
                send_lockdown(
                    &mut image_manifest_peer,
                    &dictionary([("EntryList", plist::Value::Array(vec![]))]),
                )
                .await
                .unwrap();
                eof(&mut image_manifest_peer).await;
            }
        )
    })
    .await;
    assert!(!assessment.failed());
    assert_eq!(assessment.checks.len(), 8);
    assert_eq!(assessment.checks[0].value, Some(true));
    assert_eq!(assessment.checks[1].value, Some(false));
    assert_eq!(assessment.checks[1].state, State::Available);
    assert_eq!(assessment.checks[2].value, Some(false));
    assert_eq!(assessment.checks[2].state, State::Available);
    assert!(assessment.checks[2].detail.contains("may still be mounted"));
    assert_eq!(assessment.checks[5].value, Some(false));
    assert_eq!(assessment.checks[6].status, Status::Unknown);
    assert_eq!(assessment.checks[6].state, State::Unsupported);
    assert_eq!(assessment.checks[7].id, "ddi_manifests");
    assert_eq!(assessment.checks[7].value, Some(false));
    assert!(assessment.checks[3..5]
        .iter()
        .all(|check| check.state == State::NotProbed));
    for check in &assessment.checks {
        safe(check);
    }
    let output = serde_json::to_string(&assessment).unwrap();
    assert!(!output.contains(UDID));
    assert!(output.contains("id:"));
    assert_eq!(
        source.calls(),
        vec![
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Port(7, IMAGE_PORT),
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Port(7, IMAGE_PORT),
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Port(7, IMAGE_PORT),
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Port(7, IMAGE_PORT)
        ]
    );
    assert!(source.ports.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mounted_image_observation_preserves_missing_empty_personalized_and_malformed_replies() {
    let replies = [
        (plist::Dictionary::new(), None, false),
        (
            dictionary([("EntryList", plist::Value::Array(vec![]))]),
            Some(false),
            false,
        ),
        (
            dictionary([(
                "EntryList",
                plist::Value::Array(vec![plist::Value::Dictionary(dictionary([(
                    "DiskImageType",
                    "Personalized".into(),
                )]))]),
            )]),
            Some(true),
            false,
        ),
        (
            dictionary([(
                "EntryList",
                plist::Value::Array(vec![plist::Value::Dictionary(plist::Dictionary::new())]),
            )]),
            None,
            false,
        ),
        (
            dictionary([("EntryList", "private-malformed-data".into())]),
            None,
            true,
        ),
    ];
    for (reply, expected, failed) in replies {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let (image, mut image_peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
        let (check, (), ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::DdiMounted, Duration::from_secs(2)),
                serve_service(lockdown_peer, service_reply()),
                async {
                    image_request(&mut image_peer, "CopyDevices", None).await;
                    send_lockdown(&mut image_peer, &reply).await.unwrap();
                    eof(&mut image_peer).await;
                }
            )
        })
        .await;
        assert_eq!(check.value, expected);
        assert_eq!(check.status == Status::Failed, failed);
        safe(&check);
        assert!(source.ports.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn target_ddi_usability_requires_checked_readonly_dtx_reply_over_existing_service_tls() {
    use ios_core::services::dtx::{encode_dtx, read_dtx_frame, DtxPayload};
    use tokio::io::AsyncWriteExt;
    for (version, malformed, legacy_tls13) in [
        ("16.7.1", false, false),
        ("16.7.1", true, false),
        ("13.7", false, false),
        ("13.7", false, true),
    ] {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let (instruments, instruments_peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, instruments)]);
        let (check, (), ()) = within(async { tokio::join!(
            probe_once(&source, UDID, Probe::DdiUsable, Duration::from_secs(2)),
            async {
                let mut tls = tls_session(lockdown_peer).await;
                let query = request(&mut tls, "GetValue", &["Label", "Request", "Key"]).await;
                assert_eq!(query["Key"].as_string(), Some("ProductVersion"));
                send_lockdown(&mut tls, &dictionary([("Value", version.into())])).await.unwrap();
                let service = request(&mut tls, "StartService", &["Label", "Request", "Service"]).await;
                assert_eq!(service["Service"].as_string(), Some(if version.starts_with("13.") { ios_core::instruments::SERVICE_LEGACY } else { ios_core::instruments::SERVICE_IOS14 }));
                send_lockdown(&mut tls, &dictionary([
                    ("Port", plist::Value::Integer(IMAGE_PORT.into())),
                    ("EnableServiceSSL", plist::Value::Boolean(true)),
                ])).await.unwrap();
                eof(&mut tls).await;
            },
            async {
                // Cover the handshake-only legacy transport explicitly at
                // TLS 1.2. TLS 1.3's post-handshake/raw transition is not proven
                // by this fixture; its earlier native failure remains logged.
                let acceptor = if version.starts_with("13.") && !legacy_tls13 {
                    acceptor_with_versions(&[&rustls::version::TLS12])
                } else {
                    acceptor()
                };
                let tls = acceptor.accept(instruments_peer).await.unwrap_or_else(|error| {
                    panic!("service TLS accept failed for version={version}, malformed={malformed}: {error:?}")
                });
                if legacy_tls13 {
                    assert_eq!(tls.get_ref().1.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_3));
                    let mut original = tls.into_inner().0;
                    let mut remaining = Vec::new();
                    original.read_to_end(&mut remaining).await.unwrap();
                    assert!(remaining.is_empty(), "unsupported TLS transition must send no raw DTX");
                    return;
                }
                if version.starts_with("13.") {
                    assert_eq!(tls.get_ref().1.protocol_version(), Some(rustls::ProtocolVersion::TLSv1_2));
                }
                let mut tls: ServiceStream = if version.starts_with("13.") { Box::new(tls.into_inner().0) } else { Box::new(tls) };
                let channel = read_dtx_frame(&mut tls).await.unwrap();
                assert!(matches!(channel.payload, DtxPayload::MethodInvocation { ref selector, .. } if selector == "_requestChannelWithCode:identifier:"));
                tls.write_all(&encode_dtx(channel.identifier, 1, channel.channel_code, false, 3, &ios_core::archive_null(), &[])).await.unwrap();
                let operation = read_dtx_frame(&mut tls).await.unwrap();
                assert!(matches!(operation.payload, DtxPayload::MethodInvocation { ref selector, ref args } if selector == "runningProcesses" && args.is_empty()));
                let payload = if malformed { ios_core::archive_string("private-invalid-response") } else { ios_core::archive_array(vec![]) };
                tls.write_all(&encode_dtx(operation.identifier, 1, operation.channel_code, false, 3, &payload, &[])).await.unwrap();
                eof(&mut tls).await;
            }
        ) }).await;
        assert_eq!(
            check.status,
            if legacy_tls13 {
                Status::Unknown
            } else if malformed {
                Status::Failed
            } else {
                Status::Ok
            }
        );
        assert_eq!(
            check.value,
            if malformed || legacy_tls13 {
                None
            } else {
                Some(true)
            }
        );
        if legacy_tls13 {
            assert_eq!(check.error_class, Some(ErrorClass::Unsupported));
        }
        safe(&check);
        assert!(source.ports.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn explicit_plaintext_session_supports_readonly_trust_and_image_query() {
    for probe in [Probe::Session, Probe::DeveloperMode] {
        let (lockdown, mut peer) = tokio::io::duplex(65_536);
        let (image, mut image_peer) = tokio::io::duplex(65_536);
        let ports = if matches!(probe, Probe::Session) {
            vec![(LOCKDOWN_PORT, lockdown)]
        } else {
            vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]
        };
        let source = FakeSource::new(ports);
        let (check, (), ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, probe, Duration::from_secs(2)),
                async {
                    plaintext_session_start(&mut peer).await;
                    send_lockdown(
                        &mut peer,
                        &dictionary([
                            ("SessionID", "owned-private-plain-session".into()),
                            ("EnableSessionSSL", plist::Value::Boolean(false)),
                        ]),
                    )
                    .await
                    .unwrap();
                    if matches!(probe, Probe::Session) {
                        let value =
                            request(&mut peer, "GetValue", &["Label", "Request", "Key"]).await;
                        assert_eq!(value["Key"].as_string(), Some("ProductType"));
                        send_lockdown(
                            &mut peer,
                            &dictionary([("Value", "owned-private-plain-product".into())]),
                        )
                        .await
                        .unwrap();
                    } else {
                        let service =
                            request(&mut peer, "StartService", &["Label", "Request", "Service"])
                                .await;
                        assert_eq!(
                            service["Service"].as_string(),
                            Some(ios_core::imagemounter::protocol::SERVICE_NAME)
                        );
                        send_lockdown(&mut peer, &service_reply()).await.unwrap();
                    }
                    eof(&mut peer).await;
                },
                async {
                    if matches!(probe, Probe::DeveloperMode) {
                        serve_developer(&mut image_peer, Some(plist::Value::Boolean(true))).await;
                    }
                }
            )
        })
        .await;
        assert_eq!(check.state, State::Available);
        assert_eq!(check.value, Some(true));
        safe(&check);
    }
}

#[tokio::test]
async fn image_service_explicit_ssl_true_uses_tls_and_preserves_the_same_query_wire() {
    let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
    let (image, image_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
    let (check, (), ()) = within(async {
        tokio::join!(
            probe_once(&source, UDID, Probe::DeveloperMode, Duration::from_secs(2)),
            serve_service(
                lockdown_peer,
                dictionary([
                    ("Port", plist::Value::Integer(IMAGE_PORT.into())),
                    ("EnableServiceSSL", plist::Value::Boolean(true))
                ])
            ),
            async {
                let mut peer = acceptor().accept(image_peer).await.unwrap();
                image_request(&mut peer, "QueryDeveloperModeStatus", None).await;
                send_lockdown(
                    &mut peer,
                    &dictionary([("DeveloperModeStatus", plist::Value::Boolean(false))]),
                )
                .await
                .unwrap();
                eof(&mut peer).await;
            }
        )
    })
    .await;
    assert_eq!(check.state, State::Available);
    assert_eq!(check.value, Some(false));
}

#[tokio::test]
async fn startsession_missing_ssl_is_metadata_missing_and_malformed_response_is_protocol() {
    for (reply, state, class) in [
        (
            dictionary([("SessionID", "owned-private-missing-ssl".into())]),
            State::MetadataMissing,
            ErrorClass::NotFound,
        ),
        (
            dictionary([
                ("SessionID", "owned-private-bad-ssl".into()),
                ("EnableSessionSSL", "false".into()),
            ]),
            State::Failed,
            ErrorClass::Protocol,
        ),
        (
            dictionary([("Error", plist::Value::Boolean(false))]),
            State::Failed,
            ErrorClass::Protocol,
        ),
        (
            dictionary([
                ("Request", "owned-private-unrelated".into()),
                ("SessionID", "owned-private-session".into()),
                ("EnableSessionSSL", plist::Value::Boolean(false)),
            ]),
            State::Failed,
            ErrorClass::Protocol,
        ),
    ] {
        let (lockdown, mut peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
        let (check, ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::Session, Duration::from_secs(2)),
                async {
                    plaintext_session_start(&mut peer).await;
                    send_lockdown(&mut peer, &reply).await.unwrap();
                    eof(&mut peer).await;
                }
            )
        })
        .await;
        assert_eq!(check.state, state);
        assert_eq!(check.error_class, Some(class));
        assert_eq!(check.value, None);
        safe(&check);
    }
}

#[tokio::test]
async fn developer_mode_missing_is_unknown_and_malformed_is_protocol_not_disabled() {
    for (status, expected_state) in [
        (None, State::MetadataMissing),
        (
            Some(plist::Value::String("owned-private-invalid".into())),
            State::Failed,
        ),
        (Some(plist::Value::Boolean(true)), State::Available),
    ] {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let (image, mut image_peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
        let (check, (), ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::DeveloperMode, Duration::from_secs(2)),
                serve_service(lockdown_peer, service_reply()),
                serve_developer(&mut image_peer, status)
            )
        })
        .await;
        assert_eq!(check.state, expected_state);
        if expected_state == State::Available {
            assert_eq!(check.value, Some(true));
        } else {
            assert_eq!(check.value, None);
        }
        if expected_state == State::Failed {
            assert_eq!(check.error_class, Some(ErrorClass::Protocol));
        }
        safe(&check);
    }
}

#[tokio::test]
async fn ddi_signatures_preserve_missing_metadata_and_only_report_observed_presence() {
    for (fields, expected) in [
        ([None, Some(plist::Value::Array(vec![]))], None),
        ([Some(plist::Value::Data(vec![1, 2])), None], Some(true)),
        (
            [
                Some(plist::Value::Data(vec![])),
                Some(plist::Value::Array(vec![])),
            ],
            Some(false),
        ),
    ] {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let (image, mut image_peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
        let (check, (), ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::DdiSignatures, Duration::from_secs(2)),
                serve_service(lockdown_peer, service_reply()),
                serve_signatures(&mut image_peer, fields)
            )
        })
        .await;
        assert_eq!(check.value, expected);
        assert_eq!(
            check.state,
            if expected.is_some() {
                State::Available
            } else {
                State::MetadataMissing
            }
        );
        safe(&check);
    }
}

#[tokio::test]
async fn explicit_startservice_missing_and_permission_errors_keep_original_typed_response() {
    for (code, state, class) in [
        (
            "InvalidService",
            State::ServiceUnavailable,
            ErrorClass::ServiceMissing,
        ),
        (
            "PasswordProtected",
            State::PermissionDenied,
            ErrorClass::PermissionDenied,
        ),
        (
            "owned-private-InvalidService",
            State::Failed,
            ErrorClass::ServiceFailure,
        ),
    ] {
        let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
        let (check, ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::DeveloperMode, Duration::from_secs(2)),
                serve_service(lockdown_peer, dictionary([("Error", code.into())]))
            )
        })
        .await;
        assert_eq!(check.state, state);
        assert_eq!(check.error_class, Some(class));
        assert_eq!(
            LockdownResponseError::in_error_chain(check.error.as_ref().unwrap().as_ref())
                .unwrap()
                .code(),
            code
        );
        assert_eq!(
            source
                .calls()
                .iter()
                .filter(|call| matches!(call, Call::Port(_, _)))
                .count(),
            1
        );
        safe(&check);
        machine_failure_matches_report(check);
    }
}

#[tokio::test]
async fn startsession_denial_never_attempts_pairing_tls_or_service_requests() {
    for (code, state) in [
        ("InvalidHostID", State::PermissionDenied),
        ("PasswordProtected", State::PermissionDenied),
        ("owned-private-denial", State::Failed),
    ] {
        let (lockdown, mut peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
        let (check, ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::Session, Duration::from_secs(2)),
                async {
                    plaintext_session_start(&mut peer).await;
                    send_lockdown(&mut peer, &dictionary([("Error", code.into())]))
                        .await
                        .unwrap();
                    eof(&mut peer).await;
                }
            )
        })
        .await;
        assert_eq!(check.state, state);
        assert_eq!(
            check
                .error
                .as_ref()
                .unwrap()
                .downcast_ref::<RemoteResponseError>()
                .unwrap()
                .code,
            code
        );
        safe(&check);
        machine_failure_matches_report(check);
    }
}

#[tokio::test]
async fn trust_requires_completed_getvalue_and_does_not_reinterpret_missing_or_unknown_errors() {
    for (reply, state) in [
        (dictionary([]), State::MetadataMissing),
        (
            dictionary([("Error", "MissingValue".into())]),
            State::MetadataMissing,
        ),
        (
            dictionary([("Error", "GetProhibited".into())]),
            State::PermissionDenied,
        ),
        (
            dictionary([("Value", plist::Value::Boolean(false))]),
            State::Failed,
        ),
        (
            dictionary([("Error", "owned-private-MissingValue".into())]),
            State::Failed,
        ),
    ] {
        let (lockdown, peer) = tokio::io::duplex(65_536);
        let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
        let (check, ()) = within(async {
            tokio::join!(
                probe_once(&source, UDID, Probe::Session, Duration::from_secs(2)),
                serve_value(peer, reply)
            )
        })
        .await;
        assert_eq!(check.state, state);
        assert_eq!(check.value, None);
        safe(&check);
        machine_failure_matches_report(check);
    }
}

#[tokio::test]
async fn absent_device_pair_metadata_and_host_permission_have_distinct_states_without_contacts() {
    let mut missing_device = FakeSource::new(vec![]);
    missing_device.devices.clear();
    let check = probe_once(
        &missing_device,
        UDID,
        Probe::Session,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(check.state, State::Unreachable);
    assert_eq!(check.error_class, Some(ErrorClass::DeviceNotFound));
    assert_eq!(missing_device.calls(), vec![Call::Devices]);
    for (kind, state) in [
        (io::ErrorKind::NotFound, State::MetadataMissing),
        (io::ErrorKind::PermissionDenied, State::PermissionDenied),
    ] {
        let mut source = FakeSource::new(vec![]);
        source.pair_failure = Some(kind);
        let check = probe_once(&source, UDID, Probe::Session, Duration::from_secs(1)).await;
        assert_eq!(check.state, state);
        assert_eq!(source.calls(), vec![Call::Devices, Call::PairRecord]);
        safe(&check);
    }
    let mut source = FakeSource::new(vec![]);
    source.port_failure = Some(io::ErrorKind::ConnectionRefused);
    let check = probe_once(&source, UDID, Probe::Session, Duration::from_secs(1)).await;
    assert_eq!(check.state, State::Unreachable);
    assert!(check
        .error
        .as_ref()
        .unwrap()
        .downcast_ref::<io::Error>()
        .is_some());
    safe(&check);
}

#[tokio::test]
async fn zero_overflow_and_unsafe_identifiers_do_not_poll_sources() {
    for (udid, timeout, class) in [
        (UDID, Duration::ZERO, ErrorClass::Timeout),
        (UDID, Duration::MAX, ErrorClass::InvalidInput),
        (
            "../owned-private",
            Duration::from_secs(1),
            ErrorClass::InvalidInput,
        ),
        ("", Duration::from_secs(1), ErrorClass::InvalidInput),
    ] {
        let source = FakeSource::new(vec![]);
        let check = probe_once(&source, udid, Probe::Session, timeout).await;
        assert_eq!(check.error_class, Some(class));
        assert!(source.calls().is_empty());
        if class == ErrorClass::Timeout {
            assert!(has_elapsed(check.error.as_ref().unwrap()));
        }
        safe(&check);
    }
}

#[tokio::test]
async fn late_port_return_is_closed_before_querytype_and_retains_native_elapsed() {
    let (lockdown, mut peer) = tokio::io::duplex(65_536);
    let mut source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
    source.port_delay = Duration::from_millis(20);
    let (check, ()) = within(async {
        tokio::join!(
            probe_once(&source, UDID, Probe::Session, Duration::from_millis(5)),
            eof(&mut peer)
        )
    })
    .await;
    assert_eq!(check.error_class, Some(ErrorClass::Timeout));
    assert!(has_elapsed(check.error.as_ref().unwrap()));
    assert_eq!(
        source.calls(),
        vec![
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT)
        ]
    );
}

#[tokio::test]
async fn original_native_error_returned_in_crossing_poll_keeps_priority_over_elapsed() {
    let mut source = FakeSource::new(vec![]);
    source.port_delay = Duration::from_millis(20);
    source.port_failure = Some(io::ErrorKind::PermissionDenied);
    let check = probe_once(&source, UDID, Probe::Session, Duration::from_millis(5)).await;
    assert_eq!(check.state, State::PermissionDenied);
    assert_eq!(check.error_class, Some(ErrorClass::PermissionDenied));
    let error = check.error.as_ref().unwrap();
    assert_eq!(
        error.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert!(!has_elapsed(error));
    safe(&check);
}

#[tokio::test]
async fn stalled_querytype_send_shares_total_budget_and_closes_partial_frame() {
    let (lockdown, mut peer) = tokio::io::duplex(1);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
    // Keep the peer unread until the actual operation has returned: QueryType
    // can accept one header byte, then its write must wait rather than recv.
    let check = within(probe_once(
        &source,
        UDID,
        Probe::Session,
        Duration::from_millis(30),
    ))
    .await;
    assert_eq!(check.error_class, Some(ErrorClass::Timeout));
    assert!(has_elapsed(check.error.as_ref().unwrap()));
    let mut prefix = Vec::new();
    within(peer.read_to_end(&mut prefix)).await.unwrap();
    assert_eq!(prefix.len(), 1);
    assert_eq!(
        source.calls(),
        vec![
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT)
        ]
    );
}

#[tokio::test]
async fn stalled_getvalue_expires_closes_tls_and_an_independent_fresh_probe_is_usable() {
    let (first, peer) = tokio::io::duplex(65_536);
    let (second, second_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, first), (LOCKDOWN_PORT, second)]);
    let (expired, ()) = within(async {
        tokio::join!(
            probe_once(&source, UDID, Probe::Session, Duration::from_millis(500)),
            async {
                let mut tls = tls_session(peer).await;
                request(&mut tls, "GetValue", &["Label", "Request", "Key"]).await;
                eof(&mut tls).await;
            }
        )
    })
    .await;
    assert_eq!(expired.state, State::Unreachable);
    assert_eq!(expired.error_class, Some(ErrorClass::Timeout));
    assert!(has_elapsed(expired.error.as_ref().unwrap()));
    let (fresh, ()) = within(async {
        tokio::join!(
            probe_once(&source, UDID, Probe::Session, Duration::from_secs(2)),
            serve_value(
                second_peer,
                dictionary([("Value", "owned-private-fresh-product".into())])
            )
        )
    })
    .await;
    assert_eq!(fresh.state, State::Available);
    assert_eq!(
        source
            .calls()
            .iter()
            .filter(|call| **call == Call::Port(7, LOCKDOWN_PORT))
            .count(),
        2
    );
}

#[tokio::test]
async fn pinned_cancellation_closes_inflight_protocol_and_preclosed_signal_skips_sources() {
    let source = FakeSource::new(vec![]);
    let work =
        async { Ok(probe_once(&source, UDID, Probe::Session, Duration::from_secs(2)).await) };
    let error = cancel_probe(work, async { Ok(()) }).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ErrorClass::Cancelled
    );
    assert!(source.calls().is_empty());
    let (lockdown, mut peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown)]);
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    let signal = async { cancelled.await.map_err(io::Error::other) };
    let work =
        async { Ok(probe_once(&source, UDID, Probe::Session, Duration::from_secs(30)).await) };
    let (error, ()) = within(async {
        tokio::join!(cancel_probe(work, signal), async {
            request(&mut peer, "QueryType", &["Label", "Request"]).await;
            cancel.send(()).unwrap();
            eof(&mut peer).await;
        })
    })
    .await;
    let error = error.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ErrorClass::Cancelled
    );
    assert_eq!(
        source.calls(),
        vec![
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT)
        ]
    );
}

#[tokio::test]
async fn pinned_cancellation_drops_an_active_image_query_and_stops_the_assessment() {
    let (lockdown, lockdown_peer) = tokio::io::duplex(65_536);
    let (image, mut image_peer) = tokio::io::duplex(65_536);
    let source = FakeSource::new(vec![(LOCKDOWN_PORT, lockdown), (IMAGE_PORT, image)]);
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    let signal = async { cancelled.await.map_err(io::Error::other) };
    let work = async {
        Ok(probe_once(&source, UDID, Probe::DeveloperMode, Duration::from_secs(30)).await)
    };
    let (result, (), ()) = within(async {
        tokio::join!(
            cancel_probe(work, signal),
            serve_service(lockdown_peer, service_reply()),
            async {
                image_request(&mut image_peer, "QueryDeveloperModeStatus", None).await;
                cancel.send(()).unwrap();
                eof(&mut image_peer).await;
            }
        )
    })
    .await;
    assert_eq!(
        ios_core::classify_error(result.unwrap_err().as_ref()),
        ErrorClass::Cancelled
    );
    assert_eq!(
        source.calls(),
        vec![
            Call::Devices,
            Call::PairRecord,
            Call::Port(7, LOCKDOWN_PORT),
            Call::Port(7, IMAGE_PORT)
        ]
    );
}

#[tokio::test]
async fn signal_registration_error_keeps_native_io_without_polling_device_work() {
    let source = FakeSource::new(vec![]);
    let work =
        async { Ok(probe_once(&source, UDID, Probe::Session, Duration::from_secs(2)).await) };
    let signal = async {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owned-private-signal-source",
        ))
    };
    let error = cancel_probe(work, signal).await.unwrap_err();
    assert_eq!(
        ios_core::classify_error(error.as_ref()),
        ErrorClass::PermissionDenied
    );
    assert!(error.chain().any(|cause| cause.is::<io::Error>()));
    assert!(source.calls().is_empty());
}

#[test]
fn host_schema_stays_one_and_explicit_target_adds_separate_safe_device_observations() {
    let host = super::super::Report {
        schema_version: 1,
        scope: "host",
        os: "test",
        arch: "test",
        version: "test",
        checks: vec![super::super::Check::new(
            "device_readiness",
            Status::Unknown,
            "not tested",
        )],
    };
    let host_json = serde_json::to_value(&host).unwrap();
    assert_eq!(host_json["schema_version"], 1);
    assert_eq!(host_json["scope"], "host");
    assert_eq!(host_json.as_object().unwrap().len(), 6);
    assert!(host_json.get("device").is_none());
    let assessment = Assessment {
        target: ios_core::redaction::diagnostic_id(UDID).to_string(),
        transport: "usbmux_existing_trust",
        checks: vec![
            Check::observed(Probe::DeveloperMode, Some(false)),
            Check::not_probed(
                "rsd_services",
                "explicit route not supplied",
                "provide existing route",
            ),
        ],
    };
    let mut report = super::super::TargetReport::new(host, assessment);
    let target_json = serde_json::to_value(&report).unwrap();
    assert_eq!(target_json["schema_version"], 2);
    assert_eq!(target_json["scope"], "host_and_device");
    assert_eq!(target_json["checks"], host_json["checks"]);
    assert_eq!(target_json["device"]["checks"][0]["state"], "available");
    assert_eq!(target_json["device"]["checks"][0]["value"], false);
    assert!(!target_json.to_string().contains(UDID));
    assert!(!report.failed());
    report.device.checks.push(Check::failure(
        Probe::Session,
        io::Error::new(io::ErrorKind::PermissionDenied, "owned-private-source").into(),
    ));
    report.device.checks.push(Check::failure(
        Probe::DdiSignatures,
        io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "owned-private-later-source",
        )
        .into(),
    ));
    assert!(report.failed());
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("owned-private"));
    let primary = report
        .device
        .take_first_failed_error()
        .unwrap()
        .context("selected device diagnostics found a failed check");
    assert_eq!(
        ios_core::classify_error(primary.as_ref()),
        ErrorClass::PermissionDenied
    );
    assert_eq!(
        primary.downcast_ref::<io::Error>().unwrap().kind(),
        io::ErrorKind::PermissionDenied
    );
    let message = crate::output::operation_failure_message(
        crate::output::ErrorFormat::Json,
        "doctor",
        &primary.context("outer safe diagnostic context"),
    )
    .unwrap();
    let json: serde_json::Value = serde_json::from_str(&message).unwrap();
    assert_eq!(json["error"]["class"], "permission_denied");
    assert!(!message.contains("owned-private"));
}

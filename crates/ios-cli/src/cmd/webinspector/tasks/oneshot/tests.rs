//! Actual finite command orchestration over an owned, bounded plist peer.
//! Device admission and output are private injected sources, never real devices.
use super::*;
use ios_core::{classify_error, DeviceInfo, ErrorClass, OperationCancelled};
use plist::{Dictionary, Value};
use serde_json::Value as JsonValue;
use source::SourceFuture;
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context as TaskContext, Poll},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

const BUNDLE: &str = "com.apple.mobilesafari";
const APP: &str = "PID:42";
const URL: &str = "https://example.invalid/";
const TITLE: &str = "Synthetic page";
const BOUND: Duration = Duration::from_secs(4);

async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned finite command fixture stalled")
}

fn pending_signal() -> impl Future<Output = std::io::Result<()>> {
    std::future::pending()
}

fn eval() -> Command<'static> {
    Command::Eval {
        app_id: None,
        bundle_id: BUNDLE.into(),
        page_selector: "page-7",
        expression: "6*7",
    }
}

fn budget(command: &Command<'_>) -> Budget {
    Budget::new(Duration::from_secs(3), command.phase()).unwrap()
}

#[derive(Clone, Copy, Default)]
enum FlushFault {
    #[default]
    None,
    TeardownIo,
    TeardownStall,
    TeardownDelay,
}

struct OwnedStream {
    stream: DuplexStream,
    dropped: Arc<AtomicBool>,
    fault: FlushFault,
    frame: Vec<u8>,
    teardown: bool,
    delay: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Drop for OwnedStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl AsyncRead for OwnedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for OwnedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if let Poll::Ready(Ok(count)) = &result {
            assert!(self.frame.len() + count < 1024 * 1024);
            self.frame.extend_from_slice(&bytes[..*count]);
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        if !self.frame.is_empty() {
            let message: Dictionary =
                plist::from_bytes(&self.frame[4..]).expect("complete fixture frame before flush");
            self.teardown = message.get("__selector").and_then(Value::as_string)
                == Some("_rpc_forwardDidClose:");
            self.frame.clear();
        }
        if self.teardown {
            match self.fault {
                FlushFault::TeardownIo => {
                    return Poll::Ready(Err(std::io::Error::from(
                        std::io::ErrorKind::PermissionDenied,
                    )))
                }
                FlushFault::TeardownStall => return Poll::Pending,
                FlushFault::TeardownDelay => {
                    let delay = self.delay.get_or_insert_with(|| {
                        Box::pin(tokio::time::sleep(Duration::from_millis(30)))
                    });
                    if delay.as_mut().poll(cx).is_pending() {
                        return Poll::Pending;
                    }
                }
                FlushFault::None => {}
            }
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

struct FixtureSource {
    stream: Mutex<Option<ServiceStream>>,
    devices: Vec<DeviceInfo>,
    lists: AtomicUsize,
    connects: Mutex<Vec<String>>,
    connect_delay: Duration,
    list_error: bool,
    connect_error: bool,
    uses_rsd: bool,
    dropped: Arc<AtomicBool>,
}

impl Source for FixtureSource {
    fn list_devices(&self) -> SourceFuture<'_, Vec<DeviceInfo>> {
        Box::pin(async {
            self.lists.fetch_add(1, Ordering::SeqCst);
            if self.list_error {
                return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into());
            }
            Ok(self.devices.clone())
        })
    }
    fn connect<'a>(&'a self, udid: &'a str, _budget: Budget) -> SourceFuture<'a, Connection> {
        Box::pin(async move {
            self.connects.lock().unwrap().push(udid.to_owned());
            if self.connect_error {
                return Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into());
            }
            tokio::time::sleep(self.connect_delay).await;
            Ok(Connection {
                device: None,
                stream: self
                    .stream
                    .lock()
                    .unwrap()
                    .take()
                    .expect("single owned connection"),
                uses_rsd: self.uses_rsd,
            })
        })
    }
}

fn fixture(fault: FlushFault) -> (FixtureSource, DuplexStream) {
    let (stream, peer) = tokio::io::duplex(16 * 1024);
    let dropped = Arc::new(AtomicBool::new(false));
    let stream: ServiceStream = Box::new(OwnedStream {
        stream,
        dropped: dropped.clone(),
        fault,
        frame: Vec::new(),
        teardown: false,
        delay: None,
    });
    (
        FixtureSource {
            stream: Mutex::new(Some(stream)),
            devices: vec![
                DeviceInfo {
                    udid: "owned-network-first".into(),
                    device_id: 42,
                    connection_type: "Network".into(),
                    product_id: 0,
                },
                DeviceInfo {
                    udid: "owned-usb-second".into(),
                    device_id: 43,
                    connection_type: "USB".into(),
                    product_id: 1234,
                },
            ],
            lists: AtomicUsize::new(0),
            connects: Mutex::new(Vec::new()),
            connect_delay: Duration::ZERO,
            list_error: false,
            connect_error: false,
            uses_rsd: false,
            dropped,
        },
        peer,
    )
}

fn strings(entries: &[(&str, &str)]) -> Dictionary {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Value::String((*value).into())))
        .collect()
}

fn envelope(selector: &str, argument: Dictionary) -> Value {
    Value::Dictionary(Dictionary::from_iter([
        ("__selector".to_owned(), Value::String(selector.into())),
        ("__argument".to_owned(), Value::Dictionary(argument)),
    ]))
}

async fn read_frame(stream: &mut DuplexStream) -> Result<Option<Dictionary>> {
    let mut prefix = [0; 4];
    if stream.read(&mut prefix[..1]).await? == 0 {
        return Ok(None);
    }
    stream.read_exact(&mut prefix[1..]).await?;
    let length = u32::from_be_bytes(prefix) as usize;
    assert!(length <= 1024 * 1024);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await?;
    Ok(Some(plist::from_bytes(&body)?))
}

async fn write_frame(stream: &mut DuplexStream, message: Value) -> Result<()> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &message)?;
    stream
        .write_all(&u32::try_from(body.len())?.to_be_bytes())
        .await?;
    stream.write_all(&body).await?;
    Ok(())
}

async fn send_json(stream: &mut DuplexStream, message: JsonValue) -> Result<()> {
    let mut argument = strings(&[("WIRApplicationIdentifierKey", APP)]);
    argument.insert(
        "WIRMessageDataKey".to_owned(),
        Value::Data(serde_json::to_vec(&message)?),
    );
    write_frame(stream, envelope("_rpc_applicationSentData:", argument)).await
}

fn application() -> Dictionary {
    let mut app = strings(&[
        ("WIRApplicationIdentifierKey", APP),
        ("WIRApplicationBundleIdentifierKey", BUNDLE),
        ("WIRApplicationNameKey", "Safari"),
        (
            "WIRAutomationAvailabilityKey",
            "WIRAutomationAvailabilityAvailable",
        ),
    ]);
    for (key, value) in [
        ("WIRIsApplicationActiveKey", true),
        ("WIRIsApplicationProxyKey", false),
        ("WIRIsApplicationReadyKey", true),
    ] {
        app.insert(key.to_owned(), Value::Boolean(value));
    }
    app
}

#[derive(Default)]
struct Trace {
    selectors: Vec<String>,
    methods: Vec<String>,
    session: Option<String>,
    connection: Option<String>,
    teardowns: usize,
}

#[derive(Default)]
struct PeerPolicy {
    eval_error: bool,
    stop_error: bool,
    eval_stall: Option<Arc<tokio::sync::Notify>>,
}

async fn peer(mut stream: DuplexStream, policy: PeerPolicy) -> Result<Trace> {
    let mut trace = Trace::default();
    let mut setup = false;
    while let Some(message) = read_frame(&mut stream).await? {
        let selector = message["__selector"].as_string().unwrap();
        let argument = message["__argument"].as_dictionary().unwrap();
        trace.selectors.push(selector.into());
        match selector {
            "_rpc_reportIdentifier:" => {
                trace.connection = argument["WIRConnectionIdentifierKey"]
                    .as_string()
                    .map(ToOwned::to_owned);
                write_frame(
                    &mut stream,
                    envelope(
                        "_rpc_reportCurrentState:",
                        strings(&[(
                            "WIRAutomationAvailabilityKey",
                            "WIRAutomationAvailabilityAvailable",
                        )]),
                    ),
                )
                .await?;
            }
            "_rpc_requestApplicationLaunch:" => {
                assert_eq!(
                    argument["WIRApplicationBundleIdentifierKey"].as_string(),
                    Some(BUNDLE)
                );
                write_frame(
                    &mut stream,
                    envelope("_rpc_applicationUpdated:", application()),
                )
                .await?;
            }
            "_rpc_getConnectedApplications:" => {
                write_frame(
                    &mut stream,
                    envelope(
                        "_rpc_reportConnectedApplicationList:",
                        Dictionary::from_iter([(
                            "WIRApplicationDictionaryKey".to_owned(),
                            Value::Dictionary(Dictionary::from_iter([(
                                APP.to_owned(),
                                Value::Dictionary(application()),
                            )])),
                        )]),
                    ),
                )
                .await?;
            }
            "_rpc_forwardAutomationSessionRequest:" => {
                assert_eq!(
                    argument["WIRApplicationIdentifierKey"].as_string(),
                    Some(APP)
                );
                trace.session = argument["WIRSessionIdentifierKey"]
                    .as_string()
                    .map(ToOwned::to_owned);
            }
            "_rpc_forwardGetListing:" => {
                assert_eq!(
                    argument["WIRApplicationIdentifierKey"].as_string(),
                    Some(APP)
                );
                let mut page = strings(&[
                    (
                        "WIRTypeKey",
                        if trace.session.is_some() {
                            "WIRTypeAutomation"
                        } else {
                            "WIRTypeWebPage"
                        },
                    ),
                    ("WIRTitleKey", TITLE),
                    ("WIRURLKey", URL),
                ]);
                page.insert(
                    "WIRPageIdentifierKey".to_owned(),
                    Value::Integer(7u64.into()),
                );
                if let Some(session) = &trace.session {
                    page.insert(
                        "WIRSessionIdentifierKey".to_owned(),
                        Value::String(session.clone()),
                    );
                    if setup {
                        page.insert(
                            "WIRConnectionIdentifierKey".to_owned(),
                            Value::String("owned-automation-connection".into()),
                        );
                    }
                }
                let mut listing = strings(&[("WIRApplicationIdentifierKey", APP)]);
                listing.insert(
                    "WIRListingKey".to_owned(),
                    Value::Dictionary(Dictionary::from_iter([(
                        "page-7".to_owned(),
                        Value::Dictionary(page),
                    )])),
                );
                write_frame(
                    &mut stream,
                    envelope("_rpc_applicationSentListing:", listing),
                )
                .await?;
            }
            "_rpc_forwardSocketSetup:" => {
                assert_eq!(
                    argument["WIRApplicationIdentifierKey"].as_string(),
                    Some(APP)
                );
                assert_eq!(
                    argument["WIRPageIdentifierKey"].as_unsigned_integer(),
                    Some(7)
                );
                assert!(!argument.contains_key("WIRAutomaticallyPause"));
                setup = true;
                if let Some(session) = &trace.session {
                    assert_eq!(argument["WIRSenderKey"].as_string(), Some(session.as_str()));
                } else {
                    trace.session = argument["WIRSenderKey"].as_string().map(ToOwned::to_owned);
                    send_json(&mut stream, json!({"method":"Target.targetCreated","params":{"targetInfo":{"targetId":"owned-target"}}})).await?;
                }
            }
            "_rpc_forwardSocketData:" => {
                assert!(setup);
                let wire: JsonValue =
                    serde_json::from_slice(argument["WIRSocketDataKey"].as_data().unwrap())?;
                let nested = wire["method"] == "Target.sendMessageToTarget";
                let command = if nested {
                    assert_eq!(wire["params"]["targetId"], "owned-target");
                    serde_json::from_str::<JsonValue>(wire["params"]["message"].as_str().unwrap())?
                } else {
                    wire.clone()
                };
                let method = command["method"].as_str().unwrap();
                trace.methods.push(method.into());
                let result = match method {
                    "Runtime.evaluate" => {
                        assert_eq!(command["params"]["expression"], "6*7");
                        if let Some(notify) = &policy.eval_stall {
                            notify.notify_one();
                            assert!(
                                read_frame(&mut stream).await?.is_none(),
                                "invalid client must not issue teardown or retry"
                            );
                            return Ok(trace);
                        }
                        if policy.eval_error {
                            json!({"exceptionDetails":{"text":"owned completed business failure"}})
                        } else {
                            json!({"result":{"type":"number","value":42}})
                        }
                    }
                    "Automation.createBrowsingContext" => json!({"handle":"owned-context"}),
                    "Automation.navigateBrowsingContext" => {
                        assert_eq!(command["params"]["url"], URL);
                        assert_eq!(command["params"]["handle"], "owned-context");
                        json!({})
                    }
                    "Automation.getBrowsingContext" => json!({"context":{"url":URL}}),
                    "Automation.evaluateJavaScriptFunction" => {
                        assert_eq!(
                            command["params"]["function"],
                            "function() { return document.title; }"
                        );
                        json!({"result": serde_json::to_string(TITLE)?})
                    }
                    "Automation.closeBrowsingContext" => {
                        assert_eq!(command["params"]["handle"], "owned-context");
                        json!({})
                    }
                    _ => panic!("unexpected actual finite command: {method}"),
                };
                let reply = if policy.stop_error && method == "Automation.closeBrowsingContext" {
                    json!({"id":command["id"],"error":{"code":-32001,"message":"owned close failure"}})
                } else {
                    json!({"id":command["id"],"result":result})
                };
                if nested {
                    send_json(&mut stream, json!({"id":wire["id"],"result":{}})).await?;
                    send_json(
                        &mut stream,
                        json!({"method":"Target.dispatchMessageFromTarget","params":{
                        "targetId":"owned-target","message":reply.to_string()}}),
                    )
                    .await?;
                } else {
                    send_json(&mut stream, reply).await?;
                }
            }
            "_rpc_forwardDidClose:" => {
                assert_eq!(
                    argument["WIRApplicationIdentifierKey"].as_string(),
                    Some(APP)
                );
                assert_eq!(
                    argument["WIRPageIdentifierKey"].as_unsigned_integer(),
                    Some(7)
                );
                assert_eq!(
                    argument["WIRSenderKey"].as_string(),
                    trace.session.as_deref()
                );
                trace.teardowns += 1;
            }
            _ => panic!("unexpected actual finite selector: {selector}"),
        }
    }
    Ok(trace)
}

#[tokio::test]
async fn lifecycle_oneshot_tabs_preserve_rows_human_output_and_first_default() {
    for json in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let command = Command::Tabs;
        let mut output = Vec::new();
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    budget(&Command::Tabs),
                    json,
                    |line| {
                        output.push(line.to_owned());
                        Ok(())
                    },
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert_eq!(source.lists.load(Ordering::SeqCst), 1);
        assert_eq!(*source.connects.lock().unwrap(), ["owned-network-first"]);
        assert!(source.dropped.load(Ordering::SeqCst));
        assert_eq!(trace.teardowns, 0);
        assert!(!trace
            .selectors
            .iter()
            .any(|selector| selector == "_rpc_forwardSocketSetup:"));
        if json {
            let rows: JsonValue = serde_json::from_str(&output[0]).unwrap();
            assert_eq!(
                rows,
                json!([{"application_id":APP,"bundle_identifier":BUNDLE,"application_name":"Safari",
                "pid":42,"page_id":7,"page_key":"page-7","page_type":"web_page","title":TITLE,"url":URL}])
            );
        } else {
            assert_eq!(
                output,
                [
                    format!("Safari [{BUNDLE}] page=7 {TITLE} {URL}"),
                    format!("  app_id: {APP}"),
                    "  type: web_page".into()
                ]
            );
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_eval_preserves_value_json_page_key_and_teardown() {
    for json in [false, true] {
        let (source, stream) = fixture(FlushFault::None);
        let command = eval();
        let total = budget(&command);
        let mut output = Vec::new();
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    Some("owned-explicit".into()),
                    command,
                    total,
                    json,
                    |line| {
                        output.push(line.to_owned());
                        Ok(())
                    },
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
        assert_eq!(*source.connects.lock().unwrap(), ["owned-explicit"]);
        assert_eq!(trace.methods, ["Runtime.evaluate"]);
        assert_eq!(trace.teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if json {
            assert_eq!(
                serde_json::from_str::<JsonValue>(&output[0]).unwrap(),
                json!({"id":1,"result":{"result":{"type":"number","value":42}}})
            );
        } else {
            assert_eq!(output, ["42"]);
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_launch_preserves_metadata_navigation_and_owned_cleanup() {
    for json in [false, true] {
        let (mut source, stream) = fixture(FlushFault::None);
        source.uses_rsd = json;
        let command = Command::Launch {
            bundle_id: BUNDLE,
            url: Some(URL),
        };
        let total = budget(&command);
        let mut output = Vec::new();
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    total,
                    json,
                    |line| {
                        output.push(line.to_owned());
                        Ok(())
                    },
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        result.unwrap();
        let trace = trace.unwrap();
        assert_eq!(
            trace.methods,
            [
                "Automation.createBrowsingContext",
                "Automation.navigateBrowsingContext",
                "Automation.getBrowsingContext",
                "Automation.evaluateJavaScriptFunction",
                "Automation.closeBrowsingContext"
            ]
        );
        assert_eq!(trace.teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if json {
            assert_eq!(
                serde_json::from_str::<JsonValue>(&output[0]).unwrap(),
                json!({
                    "bundle_id":BUNDLE,"application_id":APP,"application_name":"Safari","pid":42,
                    "page_id":7,"page_key":"page-7","page_type":"automation","url":URL,"title":TITLE,
                    "session_id":trace.session,"inspector_connection_id":trace.connection,
                    "automation_connection_id":"owned-automation-connection","service_name":RSD_SERVICE_NAME,"uses_rsd":true,
                })
            );
        } else {
            assert_eq!(
                output,
                [
                    format!("Launched {BUNDLE} (pid 42)"),
                    "  page: 7".into(),
                    format!("  url: {URL}"),
                    format!("  title: {TITLE}"),
                    format!("  session: {}", trace.session.unwrap()),
                    "  automation connection: owned-automation-connection".into(),
                    format!("  service: {SERVICE_NAME}")
                ]
            );
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_ready_cancel_or_signal_error_precedes_default_selection() {
    for failed_signal in [false, true] {
        let (source, _peer) = fixture(FlushFault::None);
        let error = run(
            &source,
            None,
            Command::Tabs,
            budget(&Command::Tabs),
            true,
            |_| panic!("no output"),
            async {
                if failed_signal {
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                } else {
                    Ok(())
                }
            },
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(source.lists.load(Ordering::SeqCst), 0);
        assert!(source.connects.lock().unwrap().is_empty());
        assert_eq!(
            classify_error(error.as_ref()),
            if failed_signal {
                ErrorClass::PermissionDenied
            } else {
                ErrorClass::Cancelled
            }
        );
    }
}

#[tokio::test]
async fn lifecycle_oneshot_default_selection_failures_precede_connection() {
    for permission_error in [false, true] {
        let (mut source, _peer) = fixture(FlushFault::None);
        source.devices.clear();
        source.list_error = permission_error;
        let error = run(
            &source,
            None,
            Command::Tabs,
            budget(&Command::Tabs),
            true,
            |_| panic!("no output"),
            pending_signal(),
            CLEANUP_BUDGET,
        )
        .await
        .unwrap_err();
        assert_eq!(source.lists.load(Ordering::SeqCst), 1);
        assert!(source.connects.lock().unwrap().is_empty());
        if permission_error {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::PermissionDenied);
            assert!(error.downcast_ref::<std::io::Error>().is_some());
        } else {
            assert_eq!(
                error.to_string(),
                "--udid required: no connected iOS devices found"
            );
        }
    }
    let (mut source, _peer) = fixture(FlushFault::None);
    source.connect_error = true;
    let error = run(
        &source,
        Some("owned".into()),
        Command::Tabs,
        budget(&Command::Tabs),
        true,
        |_| panic!("no output"),
        pending_signal(),
        CLEANUP_BUDGET,
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<std::io::Error>().is_some());
    assert_eq!(source.lists.load(Ordering::SeqCst), 0);
    assert_eq!(source.connects.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn lifecycle_oneshot_connection_and_startup_share_one_absolute_total() {
    let (mut source, mut stream) = fixture(FlushFault::None);
    source.connect_delay = Duration::from_millis(60);
    let total = Budget::new(Duration::from_millis(100), "opened-tabs").unwrap();
    let (result, ()) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                Command::Tabs,
                total,
                true,
                |_| panic!("no output"),
                pending_signal(),
                CLEANUP_BUDGET
            ),
            async {
                let request = read_frame(&mut stream).await.unwrap().unwrap();
                assert_eq!(
                    request["__selector"].as_string(),
                    Some("_rpc_reportIdentifier:")
                );
                tokio::time::sleep(Duration::from_millis(60)).await;
                assert!(read_frame(&mut stream).await.unwrap().is_none());
            }
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .downcast_ref::<tokio::time::error::Elapsed>()
        .is_some());
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_startup_native_eof_and_plist_errors_drop_without_teardown() {
    for malformed in [false, true] {
        let (source, mut stream) = fixture(FlushFault::None);
        let (result, ()) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    Command::Tabs,
                    budget(&Command::Tabs),
                    true,
                    |_| panic!("no output"),
                    pending_signal(),
                    CLEANUP_BUDGET
                ),
                async {
                    let _ = read_frame(&mut stream).await.unwrap().unwrap();
                    if malformed {
                        stream.write_all(&5u32.to_be_bytes()).await.unwrap();
                        stream.write_all(b"wrong").await.unwrap();
                        assert!(read_frame(&mut stream).await.unwrap().is_none());
                    } else {
                        stream.write_all(&[0, 0]).await.unwrap();
                        drop(stream);
                    }
                }
            )
        })
        .await;
        let error = result.unwrap_err();
        assert!(source.dropped.load(Ordering::SeqCst));
        if malformed {
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
            assert!(error.chain().any(|source| source.is::<plist::Error>()));
            assert!(matches!(
                error.downcast_ref::<ios_core::webinspector::WebInspectorError>(),
                Some(ios_core::webinspector::WebInspectorError::Plist(_))
            ));
        } else {
            assert!(error.chain().any(|source| source.is::<std::io::Error>()));
            assert_eq!(classify_error(error.as_ref()), ErrorClass::Io);
            assert!(error.chain().any(|source| source
                .downcast_ref::<std::io::Error>()
                .is_some_and(|source| source.kind() == std::io::ErrorKind::UnexpectedEof)));
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_cancelled_committed_eval_drops_stream_without_retry() {
    let (source, stream) = fixture(FlushFault::None);
    let notify = Arc::new(tokio::sync::Notify::new());
    let command = eval();
    let total = budget(&command);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| panic!("no output"),
                async {
                    notify.notified().await;
                    Ok(())
                },
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    eval_stall: Some(notify.clone()),
                    ..Default::default()
                }
            )
        )
    })
    .await;
    let error = result.unwrap_err();
    assert!(error.downcast_ref::<OperationCancelled>().is_some());
    assert!(source.dropped.load(Ordering::SeqCst));
    assert_eq!(trace.unwrap().teardowns, 0);
}

#[tokio::test]
async fn lifecycle_oneshot_completed_business_failure_keeps_primary_and_cleans_once() {
    let (source, stream) = fixture(FlushFault::None);
    let command = eval();
    let total = budget(&command);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| panic!("no output"),
                pending_signal(),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    eval_error: true,
                    ..Default::default()
                }
            )
        )
    })
    .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .starts_with("webinspector evaluate error:"));
    assert_eq!(trace.unwrap().teardowns, 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_writer_failure_retains_primary_and_secondary_native_sources() {
    for fault in [
        FlushFault::None,
        FlushFault::TeardownIo,
        FlushFault::TeardownStall,
    ] {
        let (source, stream) = fixture(fault);
        let command = eval();
        let total = budget(&command);
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    total,
                    true,
                    |_| Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe).into()),
                    pending_signal(),
                    Duration::from_millis(40)
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(classify_error(error.as_ref()), ErrorClass::Disconnected);
        assert_eq!(trace.unwrap().teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
        if !matches!(fault, FlushFault::None) {
            let dual = error.downcast_ref::<control::CleanupFailure>().unwrap();
            assert!(dual.primary.downcast_ref::<std::io::Error>().is_some());
            assert_eq!(
                classify_error(dual.cleanup.as_ref()),
                if matches!(fault, FlushFault::TeardownIo) {
                    ErrorClass::PermissionDenied
                } else {
                    ErrorClass::Timeout
                }
            );
            if matches!(fault, FlushFault::TeardownStall) {
                assert!(dual
                    .cleanup
                    .chain()
                    .any(|source| source.is::<tokio::time::error::Elapsed>()));
            }
        }
    }
}

#[tokio::test]
async fn lifecycle_oneshot_output_expiry_or_cancel_still_gets_independent_cleanup() {
    for cancelled in [false, true] {
        let (source, stream) = fixture(FlushFault::TeardownDelay);
        let notify = tokio::sync::Notify::new();
        let command = eval();
        let total = budget(&command);
        let mut outputs = 0;
        let (result, trace) = bounded(async {
            tokio::join!(
                run(
                    &source,
                    None,
                    command,
                    total,
                    true,
                    |_| {
                        outputs += 1;
                        if cancelled {
                            notify.notify_one();
                        } else {
                            std::thread::sleep(
                                total
                                    .deadline
                                    .saturating_duration_since(tokio::time::Instant::now())
                                    + Duration::from_millis(2),
                            );
                        }
                        Ok(())
                    },
                    async {
                        notify.notified().await;
                        Ok(())
                    },
                    Duration::from_millis(150)
                ),
                peer(stream, PeerPolicy::default())
            )
        })
        .await;
        let error = result.unwrap_err();
        assert_eq!(outputs, 1);
        assert_eq!(
            classify_error(error.as_ref()),
            if cancelled {
                ErrorClass::Cancelled
            } else {
                ErrorClass::Timeout
            }
        );
        if !cancelled {
            assert!(error
                .downcast_ref::<tokio::time::error::Elapsed>()
                .is_some());
        }
        assert!(error.downcast_ref::<control::CleanupFailure>().is_none());
        assert_eq!(trace.unwrap().teardowns, 1);
        assert!(source.dropped.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn lifecycle_oneshot_cleanup_failure_makes_completed_output_fail() {
    let (source, stream) = fixture(FlushFault::TeardownIo);
    let command = eval();
    let total = budget(&command);
    let mut outputs = 0;
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| {
                    outputs += 1;
                    Ok(())
                },
                pending_signal(),
                CLEANUP_BUDGET
            ),
            peer(stream, PeerPolicy::default())
        )
    })
    .await;
    assert_eq!(
        classify_error(result.unwrap_err().as_ref()),
        ErrorClass::PermissionDenied
    );
    assert_eq!(outputs, 1);
    assert_eq!(trace.unwrap().teardowns, 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_automation_stop_failure_survives_second_cleanup_expiry() {
    let (source, stream) = fixture(FlushFault::TeardownStall);
    let command = Command::Launch {
        bundle_id: BUNDLE,
        url: None,
    };
    let total = budget(&command);
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| Ok(()),
                pending_signal(),
                Duration::from_millis(40)
            ),
            peer(
                stream,
                PeerPolicy {
                    stop_error: true,
                    ..Default::default()
                }
            )
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Protocol);
    let dual = error.downcast_ref::<control::CleanupFailure>().unwrap();
    assert!(dual
        .primary
        .downcast_ref::<ios_core::webinspector::WebInspectorError>()
        .is_some());
    assert_eq!(classify_error(dual.cleanup.as_ref()), ErrorClass::Timeout);
    assert!(dual
        .cleanup
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    assert_eq!(trace.unwrap().teardowns, 1);
    assert!(source.dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lifecycle_oneshot_expired_committed_eval_keeps_elapsed_without_retry() {
    let (source, stream) = fixture(FlushFault::None);
    let command = eval();
    let total = Budget::new(Duration::from_millis(700), command.phase()).unwrap();
    let (result, trace) = bounded(async {
        tokio::join!(
            run(
                &source,
                None,
                command,
                total,
                true,
                |_| panic!("no output"),
                pending_signal(),
                CLEANUP_BUDGET
            ),
            peer(
                stream,
                PeerPolicy {
                    eval_stall: Some(Arc::new(tokio::sync::Notify::new())),
                    ..Default::default()
                }
            )
        )
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(classify_error(error.as_ref()), ErrorClass::Timeout);
    assert!(error
        .chain()
        .any(|source| source.is::<tokio::time::error::Elapsed>()));
    let trace = trace.unwrap();
    assert_eq!(trace.methods, ["Runtime.evaluate"]);
    assert_eq!(trace.teardowns, 0);
    assert!(source.dropped.load(Ordering::SeqCst));
}

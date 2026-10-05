//! Shared owned WebInspector plist peer for finite and interactive frontend checks.
use super::{
    control::Budget,
    source::{Connection, Source, SourceFuture},
};
use anyhow::Result;
use ios_core::{device::ServiceStream, DeviceInfo};
use plist::{Dictionary, Value};
use serde_json::{json, Value as JsonValue};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};

pub(super) const BUNDLE: &str = "com.apple.mobilesafari";
pub(super) const APP: &str = "PID:42";
pub(super) const URL: &str = "https://example.invalid/";
pub(super) const TITLE: &str = "Synthetic page";
pub(super) const BOUND: Duration = Duration::from_secs(4);

pub(super) async fn bounded<T>(operation: impl Future<Output = T>) -> T {
    tokio::time::timeout(BOUND, operation)
        .await
        .expect("owned command fixture stalled")
}

pub(super) fn pending_signal() -> impl Future<Output = std::io::Result<()>> {
    std::future::pending()
}

#[derive(Clone, Copy, Default)]
pub(super) enum FlushFault {
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

pub(super) struct FixtureSource {
    pub(super) stream: Mutex<Option<ServiceStream>>,
    pub(super) devices: Vec<DeviceInfo>,
    pub(super) lists: AtomicUsize,
    pub(super) connects: Mutex<Vec<String>>,
    pub(super) connect_delay: Duration,
    pub(super) list_error: bool,
    pub(super) connect_error: bool,
    pub(super) uses_rsd: bool,
    pub(super) dropped: Arc<AtomicBool>,
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

pub(super) fn fixture(fault: FlushFault) -> (FixtureSource, DuplexStream) {
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

pub(super) fn strings(entries: &[(&str, &str)]) -> Dictionary {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), Value::String((*value).into())))
        .collect()
}

pub(super) fn envelope(selector: &str, argument: Dictionary) -> Value {
    Value::Dictionary(Dictionary::from_iter([
        ("__selector".to_owned(), Value::String(selector.into())),
        ("__argument".to_owned(), Value::Dictionary(argument)),
    ]))
}

pub(super) async fn read_frame(stream: &mut DuplexStream) -> Result<Option<Dictionary>> {
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

pub(super) async fn write_frame(stream: &mut DuplexStream, message: Value) -> Result<()> {
    let mut body = Vec::new();
    plist::to_writer_xml(&mut body, &message)?;
    stream
        .write_all(&u32::try_from(body.len())?.to_be_bytes())
        .await?;
    stream.write_all(&body).await?;
    Ok(())
}

pub(super) async fn send_json(stream: &mut DuplexStream, message: JsonValue) -> Result<()> {
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
pub(super) struct Trace {
    pub(super) selectors: Vec<String>,
    pub(super) methods: Vec<String>,
    pub(super) session: Option<String>,
    pub(super) connection: Option<String>,
    pub(super) teardowns: usize,
}

#[derive(Default)]
pub(super) struct PeerPolicy {
    pub(super) create_stall: Option<Arc<tokio::sync::Notify>>,
    pub(super) context_delay: Duration,
    pub(super) eval_error: bool,
    pub(super) eval_protocol_error: bool,
    pub(super) stop_error: bool,
    pub(super) eval_stall: Option<Arc<tokio::sync::Notify>>,
    pub(super) setup_stall: Option<Arc<tokio::sync::Notify>>,
    pub(super) expressions: Vec<String>,
    pub(super) eval_delay: Duration,
    pub(super) eval_disconnect: bool,
    pub(super) eval_invalid: bool,
}

pub(super) async fn peer(mut stream: DuplexStream, policy: PeerPolicy) -> Result<Trace> {
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
                if let Some(notify) = &policy.setup_stall {
                    notify.notify_one();
                    assert!(
                        read_frame(&mut stream).await?.is_none(),
                        "incomplete attach must not guess teardown"
                    );
                    return Ok(trace);
                }
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
                        let index = trace
                            .methods
                            .iter()
                            .filter(|method| *method == "Runtime.evaluate")
                            .count()
                            - 1;
                        let expression = policy
                            .expressions
                            .get(index)
                            .map(String::as_str)
                            .unwrap_or("6*7");
                        assert_eq!(
                            command["params"],
                            super::super::workflows::runtime_evaluate_params(expression)
                        );
                        if policy.eval_disconnect {
                            return Ok(trace);
                        }
                        if policy.eval_invalid {
                            stream.write_all(&4u32.to_be_bytes()).await?;
                            stream.write_all(b"bad!").await?;
                            assert!(
                                read_frame(&mut stream).await?.is_none(),
                                "invalid client must not retry or teardown"
                            );
                            return Ok(trace);
                        }
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
                            tokio::time::sleep(policy.eval_delay).await;
                            json!({"result":{"type":"number","value":42}})
                        }
                    }
                    "Automation.createBrowsingContext" => {
                        if let Some(notify) = &policy.create_stall {
                            notify.notify_one();
                            assert!(
                                read_frame(&mut stream).await?.is_none(),
                                "abandoned setup must not issue teardown or retry"
                            );
                            return Ok(trace);
                        }
                        json!({"handle":"owned-context"})
                    }
                    "Automation.navigateBrowsingContext" => {
                        assert_eq!(command["params"]["url"], URL);
                        assert_eq!(command["params"]["handle"], "owned-context");
                        json!({})
                    }
                    "Automation.getBrowsingContext" => {
                        if !policy.context_delay.is_zero() {
                            tokio::time::sleep(policy.context_delay).await;
                        }
                        json!({"context":{"url":URL}})
                    }
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
                let reply = if policy.eval_protocol_error && method == "Runtime.evaluate" {
                    json!({"id":command["id"],"error":{"code":-32001,"message":"owned private protocol payload","data":"owned private protocol payload"}})
                } else if policy.stop_error && method == "Automation.closeBrowsingContext" {
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

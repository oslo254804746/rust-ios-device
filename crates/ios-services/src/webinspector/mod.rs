use std::collections::VecDeque;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::json;
use serde_json::Value as JsonValue;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{timeout, Duration, Instant};
use uuid::Uuid;

pub const SERVICE_NAME: &str = "com.apple.webinspector";
pub const RSD_SERVICE_NAME: &str = "com.apple.webinspector.shim.remote";
pub const SAFARI_BUNDLE_ID: &str = "com.apple.mobilesafari";
const MAX_PLIST_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum WebInspectorError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plist error: {0}")]
    Plist(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("timed out waiting for webinspector response after {0:?}")]
    Timeout(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationAvailability {
    NotAvailable,
    Available,
    Unknown(String),
}

impl AutomationAvailability {
    fn from_wire(value: &str) -> Self {
        match value {
            "WIRAutomationAvailabilityNotAvailable" => Self::NotAvailable,
            "WIRAutomationAvailabilityAvailable" => Self::Available,
            other => Self::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WirType {
    Automation,
    Itml,
    JavaScript,
    Page,
    ServiceWorker,
    Web,
    WebPage,
    AutomaticallyPause,
    Unknown(String),
}

impl WirType {
    fn from_wire(value: &str) -> Self {
        match value {
            "WIRTypeAutomation" => Self::Automation,
            "WIRTypeITML" => Self::Itml,
            "WIRTypeJavaScript" => Self::JavaScript,
            "WIRTypePage" => Self::Page,
            "WIRTypeServiceWorker" => Self::ServiceWorker,
            "WIRTypeWeb" => Self::Web,
            "WIRTypeWebPage" => Self::WebPage,
            "WIRAutomaticallyPause" => Self::AutomaticallyPause,
            other => Self::Unknown(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Application {
    pub id: String,
    pub bundle_identifier: String,
    pub pid: u64,
    pub name: String,
    pub availability: AutomationAvailability,
    pub is_active: bool,
    pub is_proxy: bool,
    pub is_ready: bool,
    pub host_application_identifier: Option<String>,
}

impl Application {
    fn from_plist(dict: &plist::Dictionary) -> Result<Self, WebInspectorError> {
        let id = required_string(dict, "WIRApplicationIdentifierKey")?.to_string();
        Ok(Self {
            pid: pid_from_identifier(&id)?,
            id,
            bundle_identifier: required_string(dict, "WIRApplicationBundleIdentifierKey")?
                .to_string(),
            name: required_string(dict, "WIRApplicationNameKey")?.to_string(),
            availability: AutomationAvailability::from_wire(required_string(
                dict,
                "WIRAutomationAvailabilityKey",
            )?),
            is_active: required_bool(dict, "WIRIsApplicationActiveKey")?,
            is_proxy: required_bool(dict, "WIRIsApplicationProxyKey")?,
            is_ready: required_bool(dict, "WIRIsApplicationReadyKey")?,
            host_application_identifier: optional_string(dict, "WIRHostApplicationIdentifierKey")
                .map(ToOwned::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page {
    pub id: u64,
    pub listing_key: String,
    pub page_type: WirType,
    pub title: Option<String>,
    pub url: Option<String>,
    pub automation_is_paired: Option<bool>,
    pub automation_name: Option<String>,
    pub automation_version: Option<String>,
    pub automation_session_id: Option<String>,
    pub automation_connection_id: Option<String>,
}

impl Page {
    fn from_plist(listing_key: &str, dict: &plist::Dictionary) -> Result<Self, WebInspectorError> {
        let id = match dict.get("WIRPageIdentifierKey") {
            Some(value) => plist_integer_to_u64(value).ok_or_else(|| {
                WebInspectorError::Protocol("WIRPageIdentifierKey must be an integer".to_string())
            })?,
            None => listing_key.parse::<u64>().map_err(|_| {
                WebInspectorError::Protocol(format!(
                    "missing WIRPageIdentifierKey and listing key '{listing_key}' is not numeric"
                ))
            })?,
        };

        Ok(Self {
            id,
            listing_key: listing_key.to_string(),
            page_type: WirType::from_wire(required_string(dict, "WIRTypeKey")?),
            title: optional_string(dict, "WIRTitleKey").map(ToOwned::to_owned),
            url: optional_string(dict, "WIRURLKey").map(ToOwned::to_owned),
            automation_is_paired: optional_bool(dict, "WIRAutomationTargetIsPairedKey"),
            automation_name: optional_string(dict, "WIRAutomationTargetNameKey")
                .map(ToOwned::to_owned),
            automation_version: optional_string(dict, "WIRAutomationTargetVersionKey")
                .map(ToOwned::to_owned),
            automation_session_id: optional_string(dict, "WIRSessionIdentifierKey")
                .map(ToOwned::to_owned),
            automation_connection_id: optional_string(dict, "WIRConnectionIdentifierKey")
                .map(ToOwned::to_owned),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplicationPage {
    pub application: Application,
    pub page: Page,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebInspectorEvent {
    CurrentState {
        availability: AutomationAvailability,
    },
    ConnectedApplications {
        applications: Vec<Application>,
    },
    ConnectedDrivers,
    Listing {
        application_id: String,
        pages: Vec<Page>,
    },
    ApplicationUpdated {
        application: Application,
    },
    ApplicationConnected {
        application: Application,
    },
    SocketData {
        application_id: Option<String>,
        message: JsonValue,
    },
    ApplicationDisconnected {
        application_id: String,
    },
}

#[derive(Debug)]
pub struct WebInspectorClient<S> {
    stream: S,
    connection_id: String,
    automation_availability: Option<AutomationAvailability>,
    applications: IndexMap<String, Application>,
    application_pages: IndexMap<String, IndexMap<u64, Page>>,
    pending_events: VecDeque<WebInspectorEvent>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> WebInspectorClient<S> {
    pub fn new(stream: S) -> Self {
        Self::with_connection_id(stream, Uuid::new_v4().to_string().to_uppercase())
    }

    pub fn with_connection_id(stream: S, connection_id: impl Into<String>) -> Self {
        Self {
            stream,
            connection_id: connection_id.into(),
            automation_availability: None,
            applications: IndexMap::new(),
            application_pages: IndexMap::new(),
            pending_events: VecDeque::new(),
        }
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub fn automation_availability(&self) -> Option<AutomationAvailability> {
        self.automation_availability.clone()
    }

    pub fn applications(&self) -> &IndexMap<String, Application> {
        &self.applications
    }

    pub fn application_pages(&self, application_id: &str) -> Option<&IndexMap<u64, Page>> {
        self.application_pages.get(application_id)
    }

    pub fn application_by_bundle(&self, bundle_identifier: &str) -> Option<&Application> {
        self.applications
            .values()
            .find(|application| application.bundle_identifier == bundle_identifier)
    }

    pub fn page(&self, application_id: &str, page_id: u64) -> Option<&Page> {
        self.application_pages
            .get(application_id)
            .and_then(|pages| pages.get(&page_id))
    }

    pub fn automation_page_by_session(
        &self,
        application_id: &str,
        session_id: &str,
    ) -> Option<&Page> {
        self.application_pages
            .get(application_id)
            .and_then(|pages| {
                pages.values().find(|page| {
                    page.page_type == WirType::Automation
                        && page.automation_session_id.as_deref() == Some(session_id)
                })
            })
    }

    pub fn open_pages_snapshot(&self) -> Vec<ApplicationPage> {
        let mut result = Vec::new();
        for (application_id, application) in &self.applications {
            if let Some(pages) = self.application_pages.get(application_id) {
                for page in pages.values() {
                    result.push(ApplicationPage {
                        application: application.clone(),
                        page: page.clone(),
                    });
                }
            }
        }
        result
    }

    pub async fn start(&mut self, timeout_duration: Duration) -> Result<(), WebInspectorError> {
        self.report_identifier().await?;
        let deadline = Instant::now() + timeout_duration;
        loop {
            let event = self
                .next_event_with_timeout(remaining_time(deadline, timeout_duration)?)
                .await?;
            if matches!(event, WebInspectorEvent::CurrentState { .. }) {
                return Ok(());
            }
        }
    }

    /// Discovers all open application pages on the device.
    ///
    /// Sends `_rpc_getConnectedApplications:` and consumes incoming events until no
    /// new event arrives within `idle_timeout`. The timeout signals quiescence (no
    /// more pages are being reported) and is the **expected completion path** — it is
    /// not an error. Returns the accumulated snapshot of open pages.
    pub async fn open_application_pages(
        &mut self,
        idle_timeout: Duration,
    ) -> Result<Vec<ApplicationPage>, WebInspectorError> {
        self.request_connected_applications().await?;
        loop {
            match self.next_event_with_timeout(idle_timeout).await {
                Ok(_) => continue,
                Err(WebInspectorError::Timeout(_)) => return Ok(self.open_pages_snapshot()),
                Err(error) => return Err(error),
            }
        }
    }

    pub async fn report_identifier(&mut self) -> Result<(), WebInspectorError> {
        self.send_message("_rpc_reportIdentifier:", plist::Dictionary::new())
            .await
    }

    pub async fn request_connected_applications(&mut self) -> Result<(), WebInspectorError> {
        self.send_message("_rpc_getConnectedApplications:", plist::Dictionary::new())
            .await
    }

    pub async fn request_listing(&mut self, application_id: &str) -> Result<(), WebInspectorError> {
        self.send_message(
            "_rpc_forwardGetListing:",
            plist::Dictionary::from_iter([(
                "WIRApplicationIdentifierKey".to_string(),
                plist::Value::String(application_id.to_string()),
            )]),
        )
        .await
    }

    pub async fn request_application_launch(
        &mut self,
        bundle_identifier: &str,
    ) -> Result<(), WebInspectorError> {
        self.send_message(
            "_rpc_requestApplicationLaunch:",
            plist::Dictionary::from_iter([(
                "WIRApplicationBundleIdentifierKey".to_string(),
                plist::Value::String(bundle_identifier.to_string()),
            )]),
        )
        .await
    }

    pub async fn request_automation_session(
        &mut self,
        session_id: &str,
        application_id: &str,
    ) -> Result<(), WebInspectorError> {
        self.send_message(
            "_rpc_forwardAutomationSessionRequest:",
            plist::Dictionary::from_iter([
                (
                    "WIRApplicationIdentifierKey".to_string(),
                    plist::Value::String(application_id.to_string()),
                ),
                (
                    "WIRSessionCapabilitiesKey".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "org.webkit.webdriver.webrtc.allow-insecure-media-capture".to_string(),
                            plist::Value::Boolean(true),
                        ),
                        (
                            "org.webkit.webdriver.webrtc.suppress-ice-candidate-filtering"
                                .to_string(),
                            plist::Value::Boolean(false),
                        ),
                    ])),
                ),
                (
                    "WIRSessionIdentifierKey".to_string(),
                    plist::Value::String(session_id.to_string()),
                ),
            ]),
        )
        .await
    }

    pub async fn send_socket_setup(
        &mut self,
        session_id: &str,
        application_id: &str,
        page_id: u64,
        pause: bool,
    ) -> Result<(), WebInspectorError> {
        let mut args = plist::Dictionary::from_iter([
            (
                "WIRApplicationIdentifierKey".to_string(),
                plist::Value::String(application_id.to_string()),
            ),
            (
                "WIRPageIdentifierKey".to_string(),
                plist::Value::Integer(page_id.into()),
            ),
            (
                "WIRSenderKey".to_string(),
                plist::Value::String(session_id.to_string()),
            ),
            (
                "WIRMessageDataTypeChunkSupportedKey".to_string(),
                plist::Value::Integer(0.into()),
            ),
        ]);
        if !pause {
            args.insert(
                "WIRAutomaticallyPause".to_string(),
                plist::Value::Boolean(false),
            );
        }
        self.send_message("_rpc_forwardSocketSetup:", args).await
    }

    pub async fn send_socket_data(
        &mut self,
        session_id: &str,
        application_id: &str,
        page_id: u64,
        message: &JsonValue,
    ) -> Result<(), WebInspectorError> {
        self.send_message(
            "_rpc_forwardSocketData:",
            plist::Dictionary::from_iter([
                (
                    "WIRApplicationIdentifierKey".to_string(),
                    plist::Value::String(application_id.to_string()),
                ),
                (
                    "WIRPageIdentifierKey".to_string(),
                    plist::Value::Integer(page_id.into()),
                ),
                (
                    "WIRSessionIdentifierKey".to_string(),
                    plist::Value::String(session_id.to_string()),
                ),
                (
                    "WIRSenderKey".to_string(),
                    plist::Value::String(session_id.to_string()),
                ),
                (
                    "WIRSocketDataKey".to_string(),
                    plist::Value::Data(serde_json::to_vec(message)?),
                ),
            ]),
        )
        .await
    }

    pub async fn next_event(&mut self) -> Result<WebInspectorEvent, WebInspectorError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(event);
        }
        let plist = recv_plist(&mut self.stream).await?;
        self.handle_message(plist).await
    }

    pub async fn next_event_with_timeout(
        &mut self,
        timeout_duration: Duration,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        timeout(timeout_duration, self.next_event())
            .await
            .map_err(|_| WebInspectorError::Timeout(timeout_duration))?
    }

    async fn next_socket_data_with_timeout(
        &mut self,
        timeout_duration: Duration,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        let deadline = Instant::now() + timeout_duration;
        loop {
            let event = self
                .next_event_with_timeout(remaining_time(deadline, timeout_duration)?)
                .await?;
            if matches!(event, WebInspectorEvent::SocketData { .. }) {
                return Ok(event);
            }
        }
    }

    fn restore_pending_events_front(&mut self, mut events: Vec<WebInspectorEvent>) {
        while let Some(event) = events.pop() {
            self.pending_events.push_front(event);
        }
    }

    async fn handle_message(
        &mut self,
        message: plist::Dictionary,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        let selector = required_string(&message, "__selector")?;
        let argument = message
            .get("__argument")
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| {
                WebInspectorError::Protocol(format!(
                    "webinspector message '{selector}' missing __argument dictionary"
                ))
            })?;

        match selector {
            "_rpc_reportCurrentState:" => {
                let availability = AutomationAvailability::from_wire(required_string(
                    argument,
                    "WIRAutomationAvailabilityKey",
                )?);
                self.automation_availability = Some(availability.clone());
                Ok(WebInspectorEvent::CurrentState { availability })
            }
            "_rpc_reportConnectedApplicationList:" => {
                let applications_dict = argument
                    .get("WIRApplicationDictionaryKey")
                    .and_then(plist::Value::as_dictionary)
                    .ok_or_else(|| {
                        WebInspectorError::Protocol(
                            "connected application list missing WIRApplicationDictionaryKey"
                                .to_string(),
                        )
                    })?;
                let mut applications = IndexMap::new();
                for application in applications_dict.values() {
                    let application = application.as_dictionary().ok_or_else(|| {
                        WebInspectorError::Protocol(
                            "connected application entry was not a dictionary".to_string(),
                        )
                    })?;
                    let application = Application::from_plist(application)?;
                    applications.insert(application.id.clone(), application);
                }

                self.application_pages
                    .retain(|application_id, _| applications.contains_key(application_id));
                self.applications = applications.clone();
                for application_id in applications.keys() {
                    self.request_listing(application_id).await?;
                }

                Ok(WebInspectorEvent::ConnectedApplications {
                    applications: applications.into_values().collect(),
                })
            }
            "_rpc_reportConnectedDriverList:" => Ok(WebInspectorEvent::ConnectedDrivers),
            "_rpc_applicationSentListing:" => {
                let application_id =
                    required_string(argument, "WIRApplicationIdentifierKey")?.to_string();
                let listing = argument
                    .get("WIRListingKey")
                    .and_then(plist::Value::as_dictionary)
                    .ok_or_else(|| {
                        WebInspectorError::Protocol(
                            "application listing missing WIRListingKey dictionary".to_string(),
                        )
                    })?;

                let pages = self
                    .application_pages
                    .entry(application_id.clone())
                    .or_default();
                let mut listed_pages = Vec::with_capacity(listing.len());
                for (listing_key, page) in listing {
                    let page = page.as_dictionary().ok_or_else(|| {
                        WebInspectorError::Protocol(
                            "application page entry was not a dictionary".to_string(),
                        )
                    })?;
                    let page = Page::from_plist(listing_key, page)?;
                    pages.insert(page.id, page.clone());
                    listed_pages.push(page);
                }

                Ok(WebInspectorEvent::Listing {
                    application_id,
                    pages: listed_pages,
                })
            }
            "_rpc_applicationUpdated:" => {
                let application = Application::from_plist(argument)?;
                self.applications
                    .insert(application.id.clone(), application.clone());
                Ok(WebInspectorEvent::ApplicationUpdated { application })
            }
            "_rpc_applicationConnected:" => {
                let application = Application::from_plist(argument)?;
                self.applications
                    .insert(application.id.clone(), application.clone());
                Ok(WebInspectorEvent::ApplicationConnected { application })
            }
            "_rpc_applicationSentData:" => {
                let payload = extract_json_payload(argument, "WIRMessageDataKey")?;
                let application_id =
                    optional_string(argument, "WIRApplicationIdentifierKey").map(ToOwned::to_owned);
                Ok(WebInspectorEvent::SocketData {
                    application_id,
                    message: payload,
                })
            }
            "_rpc_applicationDisconnected:" => {
                let application_id =
                    required_string(argument, "WIRApplicationIdentifierKey")?.to_string();
                self.applications.shift_remove(&application_id);
                self.application_pages.shift_remove(&application_id);
                Ok(WebInspectorEvent::ApplicationDisconnected { application_id })
            }
            other => Err(WebInspectorError::Protocol(format!(
                "unsupported webinspector selector '{other}'"
            ))),
        }
    }

    async fn send_message(
        &mut self,
        selector: &str,
        mut arguments: plist::Dictionary,
    ) -> Result<(), WebInspectorError> {
        arguments.insert(
            "WIRConnectionIdentifierKey".to_string(),
            plist::Value::String(self.connection_id.clone()),
        );
        send_plist(
            &mut self.stream,
            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "__selector".to_string(),
                    plist::Value::String(selector.to_string()),
                ),
                (
                    "__argument".to_string(),
                    plist::Value::Dictionary(arguments),
                ),
            ])),
        )
        .await
    }
}

#[derive(Debug, Clone)]
pub struct InspectorSession {
    application_id: String,
    page_id: u64,
    session_id: String,
    target_id: Option<String>,
    next_transport_id: u64,
    next_command_id: u64,
}

impl InspectorSession {
    pub fn new(application_id: impl Into<String>, page_id: u64) -> Self {
        Self::with_session_id(
            application_id,
            page_id,
            Uuid::new_v4().to_string().to_uppercase(),
        )
    }

    pub fn with_session_id(
        application_id: impl Into<String>,
        page_id: u64,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            application_id: application_id.into(),
            page_id,
            session_id: session_id.into(),
            target_id: None,
            next_transport_id: 1,
            next_command_id: 1,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn application_id(&self) -> &str {
        &self.application_id
    }

    pub fn page_id(&self) -> u64 {
        self.page_id
    }

    pub fn target_id(&self) -> Option<&str> {
        self.target_id.as_deref()
    }

    pub async fn attach<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        wait_for_target: bool,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        client
            .send_socket_setup(&self.session_id, &self.application_id, self.page_id, true)
            .await?;
        if wait_for_target {
            self.wait_for_target(client, timeout_duration).await?;
        }
        Ok(())
    }

    pub async fn next_raw_message<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let event = client
            .next_socket_data_with_timeout(timeout_duration)
            .await?;
        if let WebInspectorEvent::SocketData { message, .. } = event {
            self.observe_message(&message)?;
            return Ok(message);
        }
        unreachable!("next_socket_data_with_timeout only returns socket-data events");
    }

    pub async fn send_command<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
    ) -> Result<u64, WebInspectorError> {
        let params = match params {
            JsonValue::Object(_) => params,
            JsonValue::Null => JsonValue::Object(Default::default()),
            other => {
                return Err(WebInspectorError::Protocol(format!(
                    "webinspector command params must be a JSON object, got {other}"
                )))
            }
        };

        let command_id = self.next_command_id;
        self.next_command_id += 1;

        let payload = if let Some(target_id) = &self.target_id {
            let transport_id = self.next_transport_id;
            self.next_transport_id += 1;
            JsonValue::Object(serde_json::Map::from_iter([
                ("id".to_string(), JsonValue::from(transport_id)),
                (
                    "method".to_string(),
                    JsonValue::String("Target.sendMessageToTarget".to_string()),
                ),
                (
                    "params".to_string(),
                    JsonValue::Object(serde_json::Map::from_iter([
                        ("targetId".to_string(), JsonValue::String(target_id.clone())),
                        (
                            "message".to_string(),
                            JsonValue::String(serde_json::to_string(&JsonValue::Object(
                                serde_json::Map::from_iter([
                                    ("id".to_string(), JsonValue::from(command_id)),
                                    ("method".to_string(), JsonValue::String(method.to_string())),
                                    ("params".to_string(), params),
                                ]),
                            ))?),
                        ),
                    ])),
                ),
            ]))
        } else {
            JsonValue::Object(serde_json::Map::from_iter([
                ("id".to_string(), JsonValue::from(command_id)),
                ("method".to_string(), JsonValue::String(method.to_string())),
                ("params".to_string(), params),
            ]))
        };

        client
            .send_socket_data(
                &self.session_id,
                &self.application_id,
                self.page_id,
                &payload,
            )
            .await?;
        Ok(command_id)
    }

    pub async fn send_command_and_wait<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let command_id = self.send_command(client, method, params).await?;
        self.wait_for_response(client, command_id, timeout_duration)
            .await
    }

    pub async fn send_bridge_message<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        message: &JsonValue,
    ) -> Result<(), WebInspectorError> {
        let payload = if let Some(target_id) = &self.target_id {
            let transport_id = self.next_transport_id;
            self.next_transport_id += 1;
            json!({
                "id": transport_id,
                "method": "Target.sendMessageToTarget",
                "params": {
                    "targetId": target_id,
                    "message": serde_json::to_string(message)?,
                }
            })
        } else {
            message.clone()
        };

        client
            .send_socket_data(
                &self.session_id,
                &self.application_id,
                self.page_id,
                &payload,
            )
            .await
    }

    pub fn bridge_message(
        &mut self,
        message: &JsonValue,
    ) -> Result<Option<JsonValue>, WebInspectorError> {
        self.observe_message(message)?;
        if self.target_id.is_some()
            && message.get("id").is_some()
            && message.get("method").is_none()
        {
            return Ok(None);
        }
        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.dispatchMessageFromTarget")
        {
            let nested = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("message"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.dispatchMessageFromTarget missing params.message".to_string(),
                    )
                })?;
            let nested: JsonValue = serde_json::from_str(nested)?;
            self.observe_message(&nested)?;
            return Ok(Some(nested));
        }
        Ok(Some(message.clone()))
    }

    async fn wait_for_target<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        let deadline = Instant::now() + timeout_duration;
        let mut skipped = Vec::new();
        while self.target_id.is_none() {
            let event = match client
                .next_socket_data_with_timeout(remaining_time(deadline, timeout_duration)?)
                .await
            {
                Ok(event) => event,
                Err(error) => {
                    client.restore_pending_events_front(skipped);
                    return Err(error);
                }
            };
            let WebInspectorEvent::SocketData { message, .. } = &event else {
                unreachable!("next_socket_data_with_timeout only returns socket-data events");
            };
            self.observe_message(message)?;
            if self.target_id.is_none() {
                skipped.push(event);
            }
        }
        client.restore_pending_events_front(skipped);
        Ok(())
    }

    async fn wait_for_response<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        command_id: u64,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let deadline = Instant::now() + timeout_duration;
        let mut skipped = Vec::new();
        loop {
            let event = match client
                .next_socket_data_with_timeout(remaining_time(deadline, timeout_duration)?)
                .await
            {
                Ok(event) => event,
                Err(error) => {
                    client.restore_pending_events_front(skipped);
                    return Err(error);
                }
            };
            let WebInspectorEvent::SocketData { message, .. } = &event else {
                unreachable!("next_socket_data_with_timeout only returns socket-data events");
            };
            match self.match_response(message, command_id) {
                Ok(Some(response)) => {
                    client.restore_pending_events_front(skipped);
                    return Ok(response);
                }
                Ok(None) => {
                    if self.should_preserve_message(message) {
                        skipped.push(event);
                    }
                }
                Err(error) => {
                    client.restore_pending_events_front(skipped);
                    return Err(error);
                }
            }
        }
    }

    fn observe_message(&mut self, message: &JsonValue) -> Result<(), WebInspectorError> {
        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.targetCreated")
        {
            let target_id = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("targetInfo"))
                .and_then(JsonValue::as_object)
                .and_then(|info| info.get("targetId"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.targetCreated missing params.targetInfo.targetId".to_string(),
                    )
                })?;
            self.target_id = Some(target_id.to_string());
        }

        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.targetDestroyed")
        {
            if let Some(target_id) = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("targetId"))
                .and_then(JsonValue::as_str)
            {
                if self.target_id.as_deref() == Some(target_id) {
                    self.target_id = None;
                }
            }
        }

        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.didCommitProvisionalTarget")
        {
            let target_id = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("newTargetId"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.didCommitProvisionalTarget missing params.newTargetId".to_string(),
                    )
                })?;
            self.target_id = Some(target_id.to_string());
        }

        Ok(())
    }

    fn match_response(
        &mut self,
        message: &JsonValue,
        command_id: u64,
    ) -> Result<Option<JsonValue>, WebInspectorError> {
        if self.target_id.is_none()
            && message
                .get("id")
                .and_then(JsonValue::as_u64)
                .is_some_and(|id| id == command_id)
        {
            return Ok(Some(message.clone()));
        }

        if message
            .get("method")
            .and_then(JsonValue::as_str)
            .is_some_and(|method| method == "Target.dispatchMessageFromTarget")
        {
            let nested = message
                .get("params")
                .and_then(JsonValue::as_object)
                .and_then(|params| params.get("message"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    WebInspectorError::Protocol(
                        "Target.dispatchMessageFromTarget missing params.message".to_string(),
                    )
                })?;
            let nested: JsonValue = serde_json::from_str(nested)?;
            self.observe_message(&nested)?;
            if nested
                .get("id")
                .and_then(JsonValue::as_u64)
                .is_some_and(|id| id == command_id)
            {
                return Ok(Some(nested));
            }
        }

        Ok(None)
    }

    fn should_preserve_message(&self, message: &JsonValue) -> bool {
        !(self.target_id.is_some()
            && message.get("id").is_some()
            && message.get("method").is_none())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum By {
    Id,
    XPath,
    LinkText,
    PartialLinkText,
    Name,
    TagName,
    ClassName,
    CssSelector,
}

impl By {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::XPath => "xpath",
            Self::LinkText => "link text",
            Self::PartialLinkText => "partial link text",
            Self::Name => "name",
            Self::TagName => "tag name",
            Self::ClassName => "class name",
            Self::CssSelector => "css selector",
        }
    }
}

const FIND_NODES_JS: &str = r#"function(strategy,ancestorElement,query,firstResultOnly,timeoutDuration,callback){ancestorElement=ancestorElement||document;switch(strategy){case"id":strategy="css selector";query="[id=\""+escape(query)+"\"]";break;case"name":strategy="css selector";query="[name=\""+escape(query)+"\"]";break;}switch(strategy){case"css selector":case"link text":case"partial link text":case"tag name":case"class name":case"xpath":break;default: throw{name:"InvalidParameter",message:("Unsupported locator strategy: "+strategy+".")};}function escape(string){return string.replace(/\\/g,"\\\\").replace(/"/g,"\\\"");}function tryToFindNode(){try{switch(strategy){case"css selector":if(firstResultOnly)return ancestorElement.querySelector(query)||null;return Array.from(ancestorElement.querySelectorAll(query));case"link text":let linkTextResult=[];for(let link of ancestorElement.getElementsByTagName("a")){if(link.text.trim()==query){linkTextResult.push(link);if(firstResultOnly)break;}}if(firstResultOnly)return linkTextResult[0]||null;return linkTextResult;case"partial link text":let partialLinkResult=[];for(let link of ancestorElement.getElementsByTagName("a")){if(link.text.includes(query)){partialLinkResult.push(link);if(firstResultOnly)break;}}if(firstResultOnly)return partialLinkResult[0]||null;return partialLinkResult;case"tag name":let tagNameResult=ancestorElement.getElementsByTagName(query);if(firstResultOnly)return tagNameResult[0]||null;return Array.from(tagNameResult);case"class name":let classNameResult=ancestorElement.getElementsByClassName(query);if(firstResultOnly)return classNameResult[0]||null;return Array.from(classNameResult);case"xpath":if(firstResultOnly){let xpathResult=document.evaluate(query,ancestorElement,null,XPathResult.FIRST_ORDERED_NODE_TYPE,null);if(!xpathResult)return null;return xpathResult.singleNodeValue;}let xpathResult=document.evaluate(query,ancestorElement,null,XPathResult.ORDERED_NODE_SNAPSHOT_TYPE,null);if(!xpathResult||!xpathResult.snapshotLength)return[];let arrayResult=[];for(let i=0;i<xpathResult.snapshotLength;++i)arrayResult.push(xpathResult.snapshotItem(i));return arrayResult;}}catch(error){ throw{name:"InvalidSelector",message:error.message};}}const pollInterval=50;let pollUntil=performance.now()+timeoutDuration;function pollForNode(){let result=tryToFindNode();if(typeof result==="string"||result instanceof Node||(result instanceof Array&&result.length)){callback(result);return;}let durationRemaining=pollUntil-performance.now();if(durationRemaining<pollInterval){callback(firstResultOnly?null:[]);return;}setTimeout(pollForNode,pollInterval);}pollForNode();}"#;
const CLICK_ELEMENT_JS: &str = r#"function(element) { element.click(); return null; }"#;
const ELEMENT_TEXT_JS: &str =
    r#"function(element) { return element.innerText.replace(/^[^\S\xa0]+|[^\S\xa0]+$/g, ""); }"#;
const ELEMENT_TAG_JS: &str = r#"function(element) { return element.tagName.toLowerCase(); }"#;

#[derive(Debug, Clone)]
pub struct AutomationSession {
    application_id: String,
    bundle_identifier: String,
    session_id: String,
    page_id: Option<u64>,
    top_level_handle: Option<String>,
    implicit_wait_timeout_ms: u64,
    page_load_timeout_ms: u64,
    next_command_id: u64,
}

impl AutomationSession {
    pub fn new(application_id: impl Into<String>, bundle_identifier: impl Into<String>) -> Self {
        Self::with_session_id(
            application_id,
            bundle_identifier,
            Uuid::new_v4().to_string().to_uppercase(),
        )
    }

    pub fn with_session_id(
        application_id: impl Into<String>,
        bundle_identifier: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            application_id: application_id.into(),
            bundle_identifier: bundle_identifier.into(),
            session_id: session_id.into(),
            page_id: None,
            top_level_handle: None,
            implicit_wait_timeout_ms: 0,
            page_load_timeout_ms: 3_000_000,
            next_command_id: 1,
        }
    }

    pub fn with_page(
        application_id: impl Into<String>,
        bundle_identifier: impl Into<String>,
        session_id: impl Into<String>,
        page_id: u64,
    ) -> Self {
        let mut session = Self::with_session_id(application_id, bundle_identifier, session_id);
        session.page_id = Some(page_id);
        session.top_level_handle = Some(String::new());
        session
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn bundle_identifier(&self) -> &str {
        &self.bundle_identifier
    }

    pub fn page_id(&self) -> u64 {
        self.page_id.unwrap_or_default()
    }

    pub fn set_implicit_wait_timeout(&mut self, timeout: Duration) {
        self.implicit_wait_timeout_ms = timeout.as_millis() as u64;
    }

    pub async fn attach<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        if matches!(
            client.automation_availability(),
            Some(AutomationAvailability::NotAvailable)
        ) {
            return Err(WebInspectorError::Protocol(
                "remote automation is not available".to_string(),
            ));
        }
        client
            .request_automation_session(&self.session_id, &self.application_id)
            .await?;
        client.request_listing(&self.application_id).await?;

        let page = self
            .wait_for_automation_page(client, timeout_duration, false)
            .await?;
        self.page_id = Some(page.id);

        client
            .send_socket_setup(&self.session_id, &self.application_id, page.id, true)
            .await?;
        client.request_listing(&self.application_id).await?;
        let page = self
            .wait_for_automation_page(client, timeout_duration, true)
            .await?;
        self.page_id = Some(page.id);
        Ok(())
    }

    pub async fn start_session<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        let response = self
            .send_command_and_wait(
                client,
                "createBrowsingContext",
                JsonValue::Object(Default::default()),
                Duration::from_secs(10),
            )
            .await?;
        let handle = response
            .get("handle")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                WebInspectorError::Protocol(
                    "Automation.createBrowsingContext missing result.handle".to_string(),
                )
            })?
            .to_string();
        self.top_level_handle = Some(handle.clone());
        Ok(handle)
    }

    pub async fn stop_session<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let Some(handle) = self.top_level_handle.clone() else {
            return Ok(());
        };
        let _ = self
            .send_command_and_wait(
                client,
                "closeBrowsingContext",
                json!({ "handle": handle }),
                Duration::from_secs(10),
            )
            .await?;
        self.top_level_handle = None;
        Ok(())
    }

    pub async fn navigate<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        url: &str,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "navigateBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                    "url": url,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn go_back<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "goBackInBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn go_forward<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "goForwardInBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn refresh<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<(), WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let _ = self
            .send_command_and_wait(
                client,
                "reloadBrowsingContext",
                json!({
                    "handle": handle,
                    "pageLoadTimeout": self.page_load_timeout_ms,
                }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(())
    }

    pub async fn current_url<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<Option<String>, WebInspectorError> {
        let context = self.get_browsing_context(client).await?;
        Ok(context
            .get("url")
            .and_then(JsonValue::as_str)
            .map(ToOwned::to_owned))
    }

    pub async fn execute_script<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        script: &str,
        args: &[JsonValue],
    ) -> Result<JsonValue, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let response = self
            .send_command_and_wait(
                client,
                "evaluateJavaScriptFunction",
                json!({
                    "browsingContextHandle": handle,
                    "function": format!("function(){{\n{script}\n}}"),
                    "arguments": args.iter().map(stringify_automation_argument).collect::<Result<Vec<_>, _>>()?,
                }),
                Duration::from_secs(10),
            )
            .await?;
        decode_automation_result(&response)
    }

    pub async fn evaluate_js_function<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        function: &str,
        args: &[JsonValue],
        implicit_callback: bool,
    ) -> Result<JsonValue, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let mut params = serde_json::Map::from_iter([
            (
                "browsingContextHandle".to_string(),
                JsonValue::String(handle),
            ),
            (
                "function".to_string(),
                JsonValue::String(function.to_string()),
            ),
            (
                "arguments".to_string(),
                JsonValue::Array(
                    args.iter()
                        .map(stringify_automation_argument)
                        .collect::<Result<Vec<_>, _>>()?,
                ),
            ),
        ]);
        if implicit_callback {
            params.insert(
                "expectsImplicitCallbackArgument".to_string(),
                JsonValue::Bool(true),
            );
            if self.implicit_wait_timeout_ms > 0 {
                params.insert(
                    "callbackTimeout".to_string(),
                    JsonValue::from(self.implicit_wait_timeout_ms + 1_000),
                );
            }
        }
        let response = self
            .send_command_and_wait(
                client,
                "evaluateJavaScriptFunction",
                JsonValue::Object(params),
                Duration::from_secs(10),
            )
            .await?;
        decode_automation_result(&response)
    }

    pub async fn get_title<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(client, "function() { return document.title; }", &[], false)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn get_page_source<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(
                client,
                "function() { return document.documentElement.outerHTML; }",
                &[],
                false,
            )
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn screenshot_base64<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<String, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let response = self
            .send_command_and_wait(
                client,
                "takeScreenshot",
                json!({
                    "handle": handle,
                    "clipToViewport": true,
                }),
                Duration::from_secs(10),
            )
            .await?;
        response
            .get("data")
            .and_then(JsonValue::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| {
                WebInspectorError::Protocol(
                    "Automation.takeScreenshot missing result.data".to_string(),
                )
            })
    }

    pub async fn find_element<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        by: By,
        value: &str,
    ) -> Result<Option<JsonValue>, WebInspectorError> {
        Ok(self
            .find_elements_internal(client, by, value, true, None)
            .await?
            .into_iter()
            .next())
    }

    pub async fn find_elements<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        by: By,
        value: &str,
        single: bool,
    ) -> Result<Vec<JsonValue>, WebInspectorError> {
        self.find_elements_internal(client, by, value, single, None)
            .await
    }

    pub async fn click_element<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        element: &JsonValue,
    ) -> Result<(), WebInspectorError> {
        let _ = self
            .evaluate_js_function(
                client,
                CLICK_ELEMENT_JS,
                std::slice::from_ref(element),
                false,
            )
            .await?;
        Ok(())
    }

    pub async fn element_text<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        element: &JsonValue,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(
                client,
                ELEMENT_TEXT_JS,
                std::slice::from_ref(element),
                false,
            )
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn element_tag_name<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        element: &JsonValue,
    ) -> Result<String, WebInspectorError> {
        Ok(self
            .evaluate_js_function(client, ELEMENT_TAG_JS, std::slice::from_ref(element), false)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    async fn get_browsing_context<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
    ) -> Result<JsonValue, WebInspectorError> {
        let handle = self.require_top_level_handle()?;
        let response = self
            .send_command_and_wait(
                client,
                "getBrowsingContext",
                json!({ "handle": handle }),
                Duration::from_secs(10),
            )
            .await?;
        Ok(response
            .get("context")
            .cloned()
            .unwrap_or(JsonValue::Object(Default::default())))
    }

    async fn find_elements_internal<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        by: By,
        value: &str,
        single: bool,
        root: Option<JsonValue>,
    ) -> Result<Vec<JsonValue>, WebInspectorError> {
        let (strategy, query) = normalized_locator(by, value);
        let response = self
            .evaluate_js_function(
                client,
                FIND_NODES_JS,
                &[
                    JsonValue::String(strategy),
                    root.unwrap_or(JsonValue::Null),
                    JsonValue::String(query),
                    JsonValue::Bool(single),
                    JsonValue::from(self.implicit_wait_timeout_ms),
                ],
                true,
            )
            .await?;

        Ok(match response {
            JsonValue::Null => Vec::new(),
            JsonValue::Array(values) => values,
            other => vec![other],
        })
    }

    async fn send_command_and_wait<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let command_id = self.send_command(client, method, params).await?;
        self.wait_for_response(client, command_id, timeout_duration)
            .await
    }

    async fn send_command<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        method: &str,
        params: JsonValue,
    ) -> Result<u64, WebInspectorError> {
        let page_id = self.page_id.ok_or_else(|| {
            WebInspectorError::Protocol("automation session has not attached to a page".to_string())
        })?;
        let command_id = self.next_command_id;
        self.next_command_id += 1;
        client
            .send_socket_data(
                &self.session_id,
                &self.application_id,
                page_id,
                &json!({
                    "id": command_id,
                    "method": format!("Automation.{method}"),
                    "params": params,
                }),
            )
            .await?;
        Ok(command_id)
    }

    async fn wait_for_response<S: AsyncRead + AsyncWrite + Unpin>(
        &mut self,
        client: &mut WebInspectorClient<S>,
        command_id: u64,
        timeout_duration: Duration,
    ) -> Result<JsonValue, WebInspectorError> {
        let deadline = Instant::now() + timeout_duration;
        let mut skipped = Vec::new();
        loop {
            let event = match client
                .next_socket_data_with_timeout(remaining_time(deadline, timeout_duration)?)
                .await
            {
                Ok(event) => event,
                Err(error) => {
                    client.restore_pending_events_front(skipped);
                    return Err(error);
                }
            };
            if let WebInspectorEvent::SocketData { message, .. } = &event {
                if message.get("id").and_then(JsonValue::as_u64) == Some(command_id) {
                    if let Some(error) = message.get("error") {
                        client.restore_pending_events_front(skipped);
                        return Err(WebInspectorError::Protocol(format!(
                            "automation command failed: {error}"
                        )));
                    }
                    client.restore_pending_events_front(skipped);
                    return Ok(message
                        .get("result")
                        .cloned()
                        .unwrap_or(JsonValue::Object(Default::default())));
                }
            }
            skipped.push(event);
        }
    }

    async fn wait_for_automation_page<S: AsyncRead + AsyncWrite + Unpin>(
        &self,
        client: &mut WebInspectorClient<S>,
        timeout_duration: Duration,
        require_connection_id: bool,
    ) -> Result<Page, WebInspectorError> {
        let deadline = Instant::now() + timeout_duration;
        loop {
            if let Some(page) =
                client.automation_page_by_session(&self.application_id, &self.session_id)
            {
                if !require_connection_id || page.automation_connection_id.is_some() {
                    return Ok(page.clone());
                }
            }
            let _ = client
                .next_event_with_timeout(remaining_time(deadline, timeout_duration)?)
                .await?;
        }
    }

    fn require_top_level_handle(&self) -> Result<String, WebInspectorError> {
        self.top_level_handle.clone().ok_or_else(|| {
            WebInspectorError::Protocol(
                "automation session has not started a browsing context".to_string(),
            )
        })
    }
}

fn stringify_automation_argument(value: &JsonValue) -> Result<JsonValue, WebInspectorError> {
    Ok(JsonValue::String(serde_json::to_string(value)?))
}

fn decode_automation_result(value: &JsonValue) -> Result<JsonValue, WebInspectorError> {
    match value.get("result") {
        Some(JsonValue::String(result)) => Ok(serde_json::from_str(result)?),
        Some(other) => Ok(other.clone()),
        None => Ok(JsonValue::Null),
    }
}

fn normalized_locator(by: By, value: &str) -> (String, String) {
    match by {
        By::Id => ("css selector".to_string(), format!("[id=\"{value}\"]")),
        By::Name => ("css selector".to_string(), format!("[name=\"{value}\"]")),
        By::ClassName => ("css selector".to_string(), format!(".{value}")),
        By::TagName => ("css selector".to_string(), value.to_string()),
        _ => (by.as_wire().to_string(), value.to_string()),
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), WebInspectorError> {
    let mut payload = Vec::new();
    plist::to_writer_xml(&mut payload, value)
        .map_err(|error| WebInspectorError::Plist(error.to_string()))?;
    stream
        .write_all(&(payload.len() as u32).to_be_bytes())
        .await?;
    stream.write_all(&payload).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, WebInspectorError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_PLIST_SIZE {
        return Err(WebInspectorError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    plist::from_bytes(&payload).map_err(|error| WebInspectorError::Plist(error.to_string()))
}

fn required_string<'a>(
    dict: &'a plist::Dictionary,
    key: &str,
) -> Result<&'a str, WebInspectorError> {
    dict.get(key)
        .and_then(plist::Value::as_string)
        .ok_or_else(|| WebInspectorError::Protocol(format!("missing string field '{key}'")))
}

fn optional_string<'a>(dict: &'a plist::Dictionary, key: &str) -> Option<&'a str> {
    dict.get(key).and_then(plist::Value::as_string)
}

fn required_bool(dict: &plist::Dictionary, key: &str) -> Result<bool, WebInspectorError> {
    optional_bool(dict, key)
        .ok_or_else(|| WebInspectorError::Protocol(format!("missing bool field '{key}'")))
}

fn optional_bool(dict: &plist::Dictionary, key: &str) -> Option<bool> {
    match dict.get(key) {
        Some(plist::Value::Boolean(value)) => Some(*value),
        Some(plist::Value::Integer(value)) => value
            .as_unsigned()
            .map(|value| value != 0)
            .or_else(|| value.as_signed().map(|value| value != 0)),
        _ => None,
    }
}

fn plist_integer_to_u64(value: &plist::Value) -> Option<u64> {
    match value {
        plist::Value::Integer(value) => value
            .as_unsigned()
            .or_else(|| value.as_signed().map(|value| value as u64)),
        _ => None,
    }
}

fn pid_from_identifier(identifier: &str) -> Result<u64, WebInspectorError> {
    identifier
        .rsplit(':')
        .next()
        .ok_or_else(|| {
            WebInspectorError::Protocol(format!(
                "application identifier '{identifier}' does not contain ':'"
            ))
        })?
        .parse::<u64>()
        .map_err(|error| {
            WebInspectorError::Protocol(format!(
                "failed to parse PID from identifier '{identifier}': {error}"
            ))
        })
}

fn extract_json_payload(
    dict: &plist::Dictionary,
    key: &str,
) -> Result<JsonValue, WebInspectorError> {
    match dict.get(key) {
        Some(plist::Value::Data(payload)) => Ok(serde_json::from_slice(payload)?),
        Some(plist::Value::String(payload)) => Ok(serde_json::from_str(payload)?),
        Some(other) => Err(WebInspectorError::Protocol(format!(
            "{key} expected data/string payload, got {other:?}"
        ))),
        None => Err(WebInspectorError::Protocol(format!(
            "missing JSON payload field '{key}'"
        ))),
    }
}

fn remaining_time(deadline: Instant, fallback: Duration) -> Result<Duration, WebInspectorError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(WebInspectorError::Timeout(fallback));
    }
    Ok(deadline.duration_since(now))
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use tokio::io::{duplex, AsyncWriteExt};

    use super::*;

    fn encode_plist(value: &plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, value).expect("plist serialization");
        let mut framed = Vec::with_capacity(payload.len() + 4);
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);
        framed
    }

    #[test]
    fn open_pages_snapshot_only_includes_pages_for_connected_apps() {
        let stream = tokio::io::empty();
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        client.applications.insert(
            "PID:42".into(),
            Application {
                id: "PID:42".into(),
                bundle_identifier: "com.apple.mobilesafari".into(),
                pid: 42,
                name: "Safari".into(),
                availability: AutomationAvailability::Available,
                is_active: true,
                is_proxy: false,
                is_ready: true,
                host_application_identifier: None,
            },
        );
        client.application_pages.insert(
            "PID:42".into(),
            IndexMap::from_iter([(
                7,
                Page {
                    id: 7,
                    listing_key: "page-7".into(),
                    page_type: WirType::WebPage,
                    title: Some("Example".into()),
                    url: Some("https://example.com".into()),
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            )]),
        );
        client.application_pages.insert(
            "PID:99".into(),
            IndexMap::from_iter([(
                9,
                Page {
                    id: 9,
                    listing_key: "orphan".into(),
                    page_type: WirType::WebPage,
                    title: None,
                    url: None,
                    automation_is_paired: None,
                    automation_name: None,
                    automation_version: None,
                    automation_session_id: None,
                    automation_connection_id: None,
                },
            )]),
        );

        let snapshot = client.open_pages_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].application.id, "PID:42");
        assert_eq!(snapshot[0].page.id, 7);
    }

    #[test]
    fn extract_json_payload_accepts_string_payload() {
        let dict = plist::Dictionary::from_iter([(
            "WIRMessageDataKey".to_string(),
            plist::Value::String("{\"id\":1}".into()),
        )]);

        assert_eq!(
            extract_json_payload(&dict, "WIRMessageDataKey").unwrap(),
            json!({ "id": 1 })
        );
    }

    #[test]
    fn pid_from_identifier_rejects_non_numeric_suffix() {
        let err = pid_from_identifier("PID:not-a-number")
            .expect_err("invalid pid suffix must return an error");
        assert!(err
            .to_string()
            .contains("failed to parse PID from identifier 'PID:not-a-number'"));
    }

    #[test]
    fn normalized_locator_rewrites_common_dom_strategies() {
        assert_eq!(
            normalized_locator(By::ClassName, "hero"),
            ("css selector".into(), ".hero".into())
        );
        assert_eq!(
            normalized_locator(By::Id, "main"),
            ("css selector".into(), "[id=\"main\"]".into())
        );
        assert_eq!(
            normalized_locator(By::TagName, "button"),
            ("css selector".into(), "button".into())
        );
    }

    #[test]
    fn remaining_time_errors_after_deadline() {
        let fallback = Duration::from_millis(25);
        let err = remaining_time(Instant::now(), fallback)
            .expect_err("expired deadlines must become timeout errors");
        assert!(matches!(err, WebInspectorError::Timeout(duration) if duration == fallback));
    }

    #[allow(clippy::type_complexity)]
    fn application_listing_message(
        application_id: &str,
        pages: &[(&str, u64, &str, Option<&str>, Option<&str>)],
    ) -> plist::Dictionary {
        let listing = pages
            .iter()
            .map(|(listing_key, page_id, page_type, title, url)| {
                let mut page = plist::Dictionary::from_iter([
                    (
                        "WIRPageIdentifierKey".to_string(),
                        plist::Value::Integer((*page_id).into()),
                    ),
                    (
                        "WIRTypeKey".to_string(),
                        plist::Value::String((*page_type).to_string()),
                    ),
                ]);
                if let Some(title) = title {
                    page.insert(
                        "WIRTitleKey".to_string(),
                        plist::Value::String((*title).to_string()),
                    );
                }
                if let Some(url) = url {
                    page.insert(
                        "WIRURLKey".to_string(),
                        plist::Value::String((*url).to_string()),
                    );
                }
                ((*listing_key).to_string(), plist::Value::Dictionary(page))
            });

        plist::Dictionary::from_iter([
            (
                "__selector".to_string(),
                plist::Value::String("_rpc_applicationSentListing:".into()),
            ),
            (
                "__argument".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    (
                        "WIRApplicationIdentifierKey".to_string(),
                        plist::Value::String(application_id.to_string()),
                    ),
                    (
                        "WIRListingKey".to_string(),
                        plist::Value::Dictionary(plist::Dictionary::from_iter(listing)),
                    ),
                ])),
            ),
        ])
    }

    #[tokio::test]
    async fn recv_plist_rejects_oversized_frames() {
        let (client, mut server) = duplex(64);
        let task = tokio::spawn(async move {
            let mut stream = client;
            recv_plist(&mut stream).await
        });

        server
            .write_all(&((MAX_PLIST_SIZE as u32) + 1).to_be_bytes())
            .await
            .unwrap();

        let err = task.await.unwrap().expect_err("oversized plist must fail");
        assert!(err.to_string().contains(&format!(
            "plist length {} exceeds max {}",
            MAX_PLIST_SIZE + 1,
            MAX_PLIST_SIZE
        )));
    }

    #[tokio::test]
    async fn handle_message_application_disconnected_clears_cached_state() {
        let stream = tokio::io::empty();
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        client.applications.insert(
            "PID:42".into(),
            Application {
                id: "PID:42".into(),
                bundle_identifier: "com.apple.mobilesafari".into(),
                pid: 42,
                name: "Safari".into(),
                availability: AutomationAvailability::Available,
                is_active: true,
                is_proxy: false,
                is_ready: true,
                host_application_identifier: None,
            },
        );
        client
            .application_pages
            .insert("PID:42".into(), IndexMap::new());

        let message = plist::Dictionary::from_iter([
            (
                "__selector".to_string(),
                plist::Value::String("_rpc_applicationDisconnected:".into()),
            ),
            (
                "__argument".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "WIRApplicationIdentifierKey".to_string(),
                    plist::Value::String("PID:42".into()),
                )])),
            ),
        ]);

        let event = client.handle_message(message).await.unwrap();
        assert!(matches!(
            event,
            WebInspectorEvent::ApplicationDisconnected { ref application_id } if application_id == "PID:42"
        ));
        assert!(client.applications().is_empty());
        assert!(client.application_pages("PID:42").is_none());
    }

    #[tokio::test]
    async fn handle_message_application_listing_merges_existing_page_cache() {
        let stream = tokio::io::empty();
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");

        client
            .handle_message(application_listing_message(
                "PID:42",
                &[
                    (
                        "page-1",
                        1,
                        "WIRTypeWebPage",
                        Some("Example"),
                        Some("https://example.com"),
                    ),
                    (
                        "page-2",
                        2,
                        "WIRTypeWebPage",
                        Some("Second"),
                        Some("https://second.example.com"),
                    ),
                ],
            ))
            .await
            .unwrap();

        let event = client
            .handle_message(application_listing_message(
                "PID:42",
                &[(
                    "page-1",
                    1,
                    "WIRTypeWebPage",
                    Some("Updated Example"),
                    Some("https://updated.example.com"),
                )],
            ))
            .await
            .unwrap();

        assert!(matches!(
            event,
            WebInspectorEvent::Listing { ref application_id, ref pages }
                if application_id == "PID:42"
                    && pages.len() == 1
                    && pages[0].id == 1
                    && pages[0].title.as_deref() == Some("Updated Example")
        ));

        let pages = client
            .application_pages("PID:42")
            .expect("application pages must exist after listing");
        assert_eq!(pages.len(), 2);
        assert_eq!(
            pages.get(&1).and_then(|page| page.title.as_deref()),
            Some("Updated Example")
        );
        assert_eq!(
            pages.get(&1).and_then(|page| page.url.as_deref()),
            Some("https://updated.example.com")
        );
        assert_eq!(
            pages.get(&2).and_then(|page| page.title.as_deref()),
            Some("Second")
        );
        assert_eq!(
            pages.get(&2).and_then(|page| page.url.as_deref()),
            Some("https://second.example.com")
        );
    }

    #[tokio::test]
    async fn open_application_pages_returns_snapshot_on_idle_timeout() {
        let (client_stream, mut server_stream) = duplex(16 * 1024);
        let task = tokio::spawn(async move {
            let mut client = WebInspectorClient::with_connection_id(client_stream, "TEST");
            client
                .open_application_pages(Duration::from_millis(50))
                .await
                .unwrap()
        });

        let request = recv_plist(&mut server_stream).await.unwrap();
        assert_eq!(
            request.get("__selector").and_then(plist::Value::as_string),
            Some("_rpc_getConnectedApplications:")
        );

        server_stream
            .write_all(&encode_plist(&plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    (
                        "__selector".to_string(),
                        plist::Value::String("_rpc_reportConnectedApplicationList:".into()),
                    ),
                    (
                        "__argument".to_string(),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([(
                            "WIRApplicationDictionaryKey".to_string(),
                            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                                "PID:42".to_string(),
                                plist::Value::Dictionary(plist::Dictionary::from_iter([
                                    (
                                        "WIRApplicationIdentifierKey".to_string(),
                                        plist::Value::String("PID:42".into()),
                                    ),
                                    (
                                        "WIRApplicationBundleIdentifierKey".to_string(),
                                        plist::Value::String("com.apple.mobilesafari".into()),
                                    ),
                                    (
                                        "WIRApplicationNameKey".to_string(),
                                        plist::Value::String("Safari".into()),
                                    ),
                                    (
                                        "WIRAutomationAvailabilityKey".to_string(),
                                        plist::Value::String(
                                            "WIRAutomationAvailabilityAvailable".into(),
                                        ),
                                    ),
                                    (
                                        "WIRIsApplicationActiveKey".to_string(),
                                        plist::Value::Boolean(true),
                                    ),
                                    (
                                        "WIRIsApplicationProxyKey".to_string(),
                                        plist::Value::Boolean(false),
                                    ),
                                    (
                                        "WIRIsApplicationReadyKey".to_string(),
                                        plist::Value::Boolean(true),
                                    ),
                                ])),
                            )])),
                        )])),
                    ),
                ]),
            )))
            .await
            .unwrap();

        let listing_request = recv_plist(&mut server_stream).await.unwrap();
        assert_eq!(
            listing_request
                .get("__selector")
                .and_then(plist::Value::as_string),
            Some("_rpc_forwardGetListing:")
        );

        server_stream
            .write_all(&encode_plist(&plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    (
                        "__selector".to_string(),
                        plist::Value::String("_rpc_applicationSentListing:".into()),
                    ),
                    (
                        "__argument".to_string(),
                        plist::Value::Dictionary(plist::Dictionary::from_iter([
                            (
                                "WIRApplicationIdentifierKey".to_string(),
                                plist::Value::String("PID:42".into()),
                            ),
                            (
                                "WIRListingKey".to_string(),
                                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                                    "page-7".to_string(),
                                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                                        (
                                            "WIRPageIdentifierKey".to_string(),
                                            plist::Value::Integer(7.into()),
                                        ),
                                        (
                                            "WIRTypeKey".to_string(),
                                            plist::Value::String("WIRTypeWebPage".into()),
                                        ),
                                    ])),
                                )])),
                            ),
                        ])),
                    ),
                ]),
            )))
            .await
            .unwrap();

        let pages = task.await.unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].application.id, "PID:42");
        assert_eq!(pages[0].page.id, 7);
    }

    #[test]
    fn inspector_session_observe_message_updates_target_after_provisional_commit() {
        let mut session = InspectorSession::with_session_id("PID:42", 1, "TEST-SESSION");
        session
            .observe_message(&json!({
                "method": "Target.targetCreated",
                "params": {
                    "targetInfo": {
                        "targetId": "target-1"
                    }
                }
            }))
            .unwrap();
        assert_eq!(session.target_id(), Some("target-1"));

        session
            .observe_message(&json!({
                "method": "Target.didCommitProvisionalTarget",
                "params": {
                    "newTargetId": "target-2"
                }
            }))
            .unwrap();
        assert_eq!(session.target_id(), Some("target-2"));
    }
}

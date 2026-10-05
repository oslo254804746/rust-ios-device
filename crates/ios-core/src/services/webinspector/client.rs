//! WebInspector service state, application/page cache and incoming event routing.
//! Owns a supplied async stream; device dialing and HTTP adapters are callers.

use super::retained::{event_size, MAX_EVENTS, MAX_EVENT_BYTES};
use super::wire::{extract_json_payload, optional_string, recv_plist, required_string, send_plist};
use super::{
    Application, ApplicationPage, AutomationAvailability, Page, WebInspectorError,
    WebInspectorEvent, WirType,
};
use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use std::collections::VecDeque;
use tokio::io::{AsyncRead, AsyncWrite};
#[cfg(test)]
use tokio::time::timeout;
use tokio::time::Duration;
use uuid::Uuid;

pub(super) mod lifecycle;
pub(super) use lifecycle::{await_operation, WebInspectorOperation};
use lifecycle::{checked_deadline, Deadline, WebInspectorState};
mod metadata;
use metadata::{Change, MAX_APPLICATIONS, MAX_LISTING_ENTRIES};

#[derive(Debug)]
pub struct WebInspectorClient<S> {
    stream: S,
    connection_id: String,
    automation_availability: Option<AutomationAvailability>,
    applications: IndexMap<String, Application>,
    application_pages: IndexMap<String, IndexMap<u64, Page>>,
    pending_events: VecDeque<WebInspectorEvent>,
    pending_event_bytes: usize,
    state: WebInspectorState,
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
            pending_event_bytes: 0,
            state: WebInspectorState::default(),
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

    /// Clone the admitted application/page projection. Received metadata is
    /// admitted against a separate flattened JSON budget before caching, so a
    /// large application cannot be amplified by an unbounded number of pages.
    /// These local logical limits do not measure RSS or remote capability.
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

    /// Starts discovery with the legacy startup-error mapping.
    ///
    /// Some closed-stream and response-timeout failures become `Disabled` for
    /// compatibility. That variant alone does not prove a device setting or
    /// permission rejection. Use [`Self::start_preserving_errors`] to retain
    /// the underlying typed startup failure.
    pub async fn start(&mut self, timeout_duration: Duration) -> Result<(), WebInspectorError> {
        self.start_with_error_mapping(timeout_duration, map_handshake_disabled_error)
            .await
    }

    /// Starts discovery without remapping IO or timeout failures to `Disabled`.
    ///
    /// Sends the same identifier report and consumes the same events under one
    /// deadline as [`Self::start`]. IO errors retain their original source;
    /// elapsed timers retain their native source under `Io`, with the reported
    /// budget available through `WebInspectorError::timeout_duration`. A timeout
    /// or EOF does not prove that Web Inspector is disabled. Failure may follow
    /// a partial identifier write; it does not acknowledge device cleanup or
    /// make retrying the supplied stream safe.
    pub async fn start_preserving_errors(
        &mut self,
        timeout_duration: Duration,
    ) -> Result<(), WebInspectorError> {
        self.start_with_error_mapping(timeout_duration, std::convert::identity)
            .await
    }

    async fn start_with_error_mapping(
        &mut self,
        timeout_duration: Duration,
        map_error: fn(WebInspectorError) -> WebInspectorError,
    ) -> Result<(), WebInspectorError> {
        let deadline = self.deadline(Some(timeout_duration))?;
        let mut response_phase = false;
        let result = await_operation(deadline, "start WebInspector discovery", async {
            let mut operation = WebInspectorOperation::with_deadline(self, deadline)?;
            operation
                .client
                .report_identifier()
                .await
                .map_err(map_error)?;
            operation.commit();
            response_phase = true;
            loop {
                let event = operation.client.next_event_raw().await?;
                if matches!(event, WebInspectorEvent::CurrentState { .. }) {
                    return operation.finish(Ok(()));
                }
                tokio::task::yield_now().await;
            }
        })
        .await;
        // The legacy send-stage outer timer was not mapped to Disabled;
        // response-stage failures were. Retain that distinction.
        if response_phase {
            result.map_err(map_error)
        } else {
            result
        }
    }

    /// Discovers all open application pages on the device.
    ///
    /// Sends `_rpc_getConnectedApplications:` and consumes incoming events until no
    /// new event arrives within `idle_timeout`. An empty idle timeout on a usable
    /// connection signals quiescence and returns the accumulated snapshot. A
    /// partial frame, interrupted routing or configured operation-total expiry
    /// remains an error; those failures cannot certify discovery completion.
    pub async fn open_application_pages(
        &mut self,
        idle_timeout: Duration,
    ) -> Result<Vec<ApplicationPage>, WebInspectorError> {
        let total = self.deadline(None)?;
        let request = checked_deadline(idle_timeout)?;
        await_operation(total, "discover WebInspector pages", async {
            await_operation(
                Some(request),
                "request WebInspector applications",
                self.request_connected_applications(),
            )
            .await?;
            loop {
                // Idle quiescence and a configured operation total are separate
                // owners. Only the empty, still usable idle path is success.
                let idle = checked_deadline(idle_timeout)?;
                match await_operation(
                    Some(idle),
                    "receive WebInspector event",
                    self.next_event_raw(),
                )
                .await
                {
                    Ok(_) => tokio::task::yield_now().await,
                    Err(error) if error.timeout_duration().is_some() && self.is_usable() => {
                        return Ok(self.open_pages_snapshot())
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await
    }

    pub async fn report_identifier(&mut self) -> Result<(), WebInspectorError> {
        let deadline = self.deadline(None)?;
        self.send_message(deadline, "_rpc_reportIdentifier:", plist::Dictionary::new())
            .await
    }

    pub async fn request_connected_applications(&mut self) -> Result<(), WebInspectorError> {
        let deadline = self.deadline(None)?;
        self.send_message(
            deadline,
            "_rpc_getConnectedApplications:",
            plist::Dictionary::new(),
        )
        .await
    }

    pub async fn request_listing(&mut self, application_id: &str) -> Result<(), WebInspectorError> {
        let deadline = self.deadline(None)?;
        self.send_message(
            deadline,
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
        let deadline = self.deadline(None)?;
        self.send_message(
            deadline,
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
        let deadline = self.deadline(None)?;
        self.send_message(
            deadline,
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
        let deadline = self.deadline(None)?;
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
        self.send_message(deadline, "_rpc_forwardSocketSetup:", args)
            .await
    }

    pub async fn send_socket_data(
        &mut self,
        session_id: &str,
        application_id: &str,
        page_id: u64,
        message: &JsonValue,
    ) -> Result<(), WebInspectorError> {
        let deadline = self.deadline(None)?;
        self.send_message(
            deadline,
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

    /// Close a debugger forwarding socket before another client attaches to the page.
    pub async fn send_socket_teardown(
        &mut self,
        session_id: &str,
        application_id: &str,
        page_id: u64,
    ) -> Result<(), WebInspectorError> {
        let deadline = self.deadline(None)?;
        self.send_message(
            deadline,
            "_rpc_forwardDidClose:",
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
                    "WIRSenderKey".to_string(),
                    plist::Value::String(session_id.to_string()),
                ),
            ]),
        )
        .await
    }

    /// Read and route one event, including its automatic listing writes.
    /// Empty idle cancellation can preserve reuse; cancellation after accepted
    /// bytes or a decoded event invalidates local state and clears cached values.
    pub async fn next_event(&mut self) -> Result<WebInspectorEvent, WebInspectorError> {
        let deadline = self.deadline(None)?;
        self.next_event_with_deadline(deadline).await
    }

    async fn next_event_with_deadline(
        &mut self,
        deadline: Option<Deadline>,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        let operation = WebInspectorOperation::with_deadline(self, deadline)?;
        let result = await_operation(
            deadline,
            "receive WebInspector event",
            operation.client.next_event_raw(),
        )
        .await;
        operation.finish(result)
    }

    async fn next_event_raw(&mut self) -> Result<WebInspectorEvent, WebInspectorError> {
        self.ensure_usable()?;
        if let Some(event) = self.pending_events.pop_front() {
            self.pending_event_bytes = self
                .pending_event_bytes
                .saturating_sub(event_size(&event, MAX_EVENT_BYTES)?);
            return Ok(event);
        }
        let mut operation = WebInspectorOperation::new(self)?;
        let mut frame = operation.client.begin_frame()?;
        let result = recv_plist(&mut frame.io()).await;
        let plist = frame.finish(result)?;
        operation.commit();
        let result = operation.client.handle_message(plist).await;
        operation.finish(result)
    }

    /// Read one event within the shorter explicit/configured total. Zero rejects
    /// before cached events or IO as legacy Timeout; overflow is Io InvalidInput.
    /// Actual expiry retains Elapsed. Only an empty idle read can remain usable.
    pub async fn next_event_with_timeout(
        &mut self,
        timeout_duration: Duration,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        let deadline = self.deadline(Some(timeout_duration))?;
        self.next_event_with_deadline(deadline).await
    }

    pub(super) async fn next_socket_data_with_timeout(
        &mut self,
        timeout_duration: Duration,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        let deadline = self.deadline(Some(timeout_duration))?;
        let operation = WebInspectorOperation::with_deadline(self, deadline)?;
        let result = await_operation(
            deadline,
            "receive WebInspector socket data",
            operation.client.next_socket_data(),
        )
        .await;
        operation.finish(result)
    }

    pub(super) async fn next_socket_data(
        &mut self,
    ) -> Result<WebInspectorEvent, WebInspectorError> {
        loop {
            let event = self.next_event().await?;
            if matches!(event, WebInspectorEvent::SocketData { .. }) {
                return Ok(event);
            }
            tokio::task::yield_now().await;
        }
    }

    #[cfg(test)]
    pub(super) fn restore_pending_events_front(&mut self, events: Vec<WebInspectorEvent>) {
        self.restore_events(events.into())
            .expect("test events fit local policy");
    }

    fn restore_events(
        &mut self,
        mut events: VecDeque<WebInspectorEvent>,
    ) -> Result<(), WebInspectorError> {
        if !self.is_usable() {
            return Ok(());
        }
        if events.len() > MAX_EVENTS.saturating_sub(self.pending_events.len()) {
            self.invalidate();
            return Err(super::local_limit(
                "retained event count",
                MAX_EVENTS as u64,
            ));
        }
        let mut bytes = self.pending_event_bytes;
        for event in &events {
            match event_size(event, MAX_EVENT_BYTES.saturating_sub(bytes)) {
                Ok(size) => bytes += size,
                Err(error) => {
                    self.invalidate();
                    return Err(error);
                }
            }
        }
        while let Some(event) = events.pop_back() {
            self.pending_events.push_front(event);
        }
        self.pending_event_bytes = bytes;
        Ok(())
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
                self.admit_metadata(Change::Availability(&availability))?;
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
                metadata::count(
                    applications_dict.len(),
                    MAX_APPLICATIONS,
                    "application list entries",
                )?;
                let mut applications = IndexMap::new();
                let mut staged_bytes = 2;
                for application in applications_dict.values() {
                    let application = application.as_dictionary().ok_or_else(|| {
                        WebInspectorError::Protocol(
                            "connected application entry was not a dictionary".to_string(),
                        )
                    })?;
                    let application = Application::from_plist(application)?;
                    staged_bytes = metadata::staged_map_bytes(
                        staged_bytes,
                        applications.len(),
                        &application.id,
                        &application,
                        applications.get(&application.id),
                    )?;
                    applications.insert(application.id.clone(), application);
                }

                self.admit_metadata(Change::Applications(&applications))?;
                self.application_pages
                    .retain(|application_id, _| applications.contains_key(application_id));
                if self.application_pages.is_empty() {
                    self.application_pages = Default::default();
                } else {
                    self.application_pages.shrink_to_fit();
                }
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

                metadata::count(listing.len(), MAX_LISTING_ENTRIES, "listing entries")?;
                let mut listed_pages = Vec::with_capacity(listing.len());
                let mut pages = IndexMap::new();
                let mut staged_map_bytes = 2;
                let mut staged_list_bytes = 2;
                for (listing_key, page) in listing {
                    let page = page.as_dictionary().ok_or_else(|| {
                        WebInspectorError::Protocol(
                            "application page entry was not a dictionary".to_string(),
                        )
                    })?;
                    let page = Page::from_plist(listing_key, page)?;
                    staged_map_bytes = metadata::staged_map_bytes(
                        staged_map_bytes,
                        pages.len(),
                        &page.id,
                        &page,
                        pages.get(&page.id),
                    )?;
                    staged_list_bytes =
                        metadata::staged_list_bytes(staged_list_bytes, listed_pages.len(), &page)?;
                    pages.insert(page.id, page.clone());
                    listed_pages.push(page);
                }

                // A listing is a complete snapshot, not a delta. Replace the
                // cache only after every page has parsed successfully so a
                // malformed update cannot partially corrupt the last snapshot.
                self.admit_metadata(Change::Pages(&application_id, &pages))?;
                self.application_pages.insert(application_id.clone(), pages);

                Ok(WebInspectorEvent::Listing {
                    application_id,
                    pages: listed_pages,
                })
            }
            "_rpc_applicationUpdated:" => {
                let application = Application::from_plist(argument)?;
                self.admit_metadata(Change::Application(&application))?;
                self.applications
                    .insert(application.id.clone(), application.clone());
                Ok(WebInspectorEvent::ApplicationUpdated { application })
            }
            "_rpc_applicationConnected:" => {
                let application = Application::from_plist(argument)?;
                self.admit_metadata(Change::Application(&application))?;
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
                if self.applications.is_empty() {
                    self.applications = Default::default();
                }
                if self.application_pages.is_empty() {
                    self.application_pages = Default::default();
                }
                Ok(WebInspectorEvent::ApplicationDisconnected { application_id })
            }
            other => Err(WebInspectorError::Protocol(format!(
                "unsupported webinspector selector '{other}'"
            ))),
        }
    }

    async fn send_message(
        &mut self,
        deadline: Option<Deadline>,
        selector: &str,
        mut arguments: plist::Dictionary,
    ) -> Result<(), WebInspectorError> {
        arguments.insert(
            "WIRConnectionIdentifierKey".to_string(),
            plist::Value::String(self.connection_id.clone()),
        );
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "__selector".to_string(),
                plist::Value::String(selector.to_string()),
            ),
            (
                "__argument".to_string(),
                plist::Value::Dictionary(arguments),
            ),
        ]));
        await_operation(deadline, "send WebInspector message", async {
            let mut frame = self.begin_frame()?;
            let result = send_plist(&mut frame.io(), &value).await;
            frame.finish(result)
        })
        .await
    }
}

#[cfg(test)]
mod lifecycle_tests;

#[cfg(test)]
mod correlated_tests;

#[cfg(test)]
mod metadata_tests;

fn map_handshake_disabled_error(error: WebInspectorError) -> WebInspectorError {
    match &error {
        WebInspectorError::Io(io_error)
            if matches!(
                io_error.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
            ) =>
        {
            WebInspectorError::Disabled
        }
        _ if error.timeout_duration().is_some() => WebInspectorError::Disabled,
        _ => error,
    }
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;
    use tokio::io::{duplex, AsyncWriteExt};

    use super::*;

    #[tokio::test]
    async fn native_discovery_startup_expiry_keeps_elapsed_for_preserving_and_blocked_legacy() {
        for (preserve, capacity) in [(false, 8), (true, 8), (true, 16 * 1024)] {
            let (stream, _peer) = duplex(capacity);
            let mut client = WebInspectorClient::with_connection_id(stream, "owned-private");
            let duration = Duration::from_millis(5);
            let error = if preserve {
                client.start_preserving_errors(duration).await
            } else {
                client.start(duration).await
            }
            .unwrap_err();
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
            assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
            assert!(!format!("{error:?}").contains("owned-private"));
        }
    }

    #[tokio::test]
    async fn native_discovery_event_expiry_keeps_elapsed_and_page_quiescence_success() {
        let (stream, _peer) = duplex(16 * 1024);
        let mut client = WebInspectorClient::with_connection_id(stream, "owned-private");
        let error = client
            .next_event_with_timeout(Duration::from_millis(5))
            .await
            .unwrap_err();
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        assert!(crate::test_native::has_source::<tokio::time::error::Elapsed>(&error));
        assert!(client
            .open_application_pages(Duration::from_millis(5))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn native_discovery_foreign_io_timeout_is_not_quiescence_or_disabled() {
        use std::{
            io,
            pin::Pin,
            task::{Context, Poll},
        };
        struct TimedRead;
        impl AsyncRead for TimedRead {
            fn poll_read(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "owned-private",
                )))
            }
        }
        impl AsyncWrite for TimedRead {
            fn poll_write(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<io::Result<usize>> {
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
                Poll::Ready(Ok(()))
            }
        }
        for startup in [false, true] {
            let mut client = WebInspectorClient::new(TimedRead);
            let error = if startup {
                client.start(Duration::from_secs(1)).await.unwrap_err()
            } else {
                client
                    .open_application_pages(Duration::from_secs(1))
                    .await
                    .unwrap_err()
            };
            assert!(matches!(error, WebInspectorError::Io(_)));
            assert_eq!(error.timeout_duration(), None);
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        }
    }

    fn encode_plist(value: &plist::Value) -> Vec<u8> {
        let mut payload = Vec::new();
        plist::to_writer_xml(&mut payload, value).expect("plist serialization");
        let mut framed = Vec::with_capacity(payload.len() + 4);
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);
        framed
    }

    #[tokio::test]
    async fn legacy_start_maps_closed_report_write_to_disabled() {
        let (stream, peer) = duplex(16 * 1024);
        drop(peer);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let error = client.start(Duration::from_millis(20)).await.unwrap_err();
        assert!(matches!(error, WebInspectorError::Disabled));
    }

    #[tokio::test]
    async fn legacy_start_maps_eof_after_identifier_report_to_disabled() {
        let (stream, mut peer) = duplex(16 * 1024);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let (result, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(client.start(Duration::from_secs(1)), async move {
                let report = recv_plist(&mut peer).await.unwrap();
                assert_eq!(
                    report.get("__selector").and_then(plist::Value::as_string),
                    Some("_rpc_reportIdentifier:")
                );
                drop(peer);
            })
        })
        .await
        .expect("owned startup/peer must finish");
        assert!(matches!(result, Err(WebInspectorError::Disabled)));
    }

    #[tokio::test]
    async fn legacy_start_maps_idle_after_identifier_report_to_disabled() {
        let (stream, mut peer) = duplex(16 * 1024);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let error = client.start(Duration::from_millis(20)).await.unwrap_err();
        assert!(matches!(error, WebInspectorError::Disabled));
        let report = recv_plist(&mut peer).await.unwrap();
        assert_eq!(
            report.get("__selector").and_then(plist::Value::as_string),
            Some("_rpc_reportIdentifier:")
        );
    }

    #[tokio::test]
    async fn legacy_start_keeps_timeout_while_identifier_write_is_blocked() {
        let (stream, _peer) = duplex(8);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let duration = Duration::from_millis(20);
        let error = client.start(duration).await.unwrap_err();
        assert_eq!(error.timeout_duration(), Some(duration));
    }

    #[tokio::test]
    async fn preserving_start_retains_closed_report_write_source() {
        let (stream, peer) = duplex(16 * 1024);
        drop(peer);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let error = client
            .start_preserving_errors(Duration::from_millis(20))
            .await
            .unwrap_err();
        assert!(
            matches!(&error, WebInspectorError::Io(source) if source.kind() == std::io::ErrorKind::BrokenPipe)
        );
        let source = std::error::Error::source(&error)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert_eq!(source.kind(), std::io::ErrorKind::BrokenPipe);
        assert_eq!(
            crate::classify_error(&error),
            crate::ErrorClass::Disconnected
        );
    }

    #[tokio::test]
    async fn preserving_start_retains_eof_after_identifier_report() {
        let (stream, mut peer) = duplex(16 * 1024);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let (result, ()) = timeout(Duration::from_secs(1), async {
            tokio::join!(
                client.start_preserving_errors(Duration::from_secs(1)),
                async move {
                    let report = recv_plist(&mut peer).await.unwrap();
                    assert_eq!(
                        report.get("__selector").and_then(plist::Value::as_string),
                        Some("_rpc_reportIdentifier:")
                    );
                    drop(peer);
                }
            )
        })
        .await
        .expect("owned startup/peer must finish");
        let error = result.unwrap_err();
        assert!(
            matches!(&error, WebInspectorError::Io(source) if source.kind() == std::io::ErrorKind::UnexpectedEof)
        );
        // The shared classifier does not guess that a generic file/stream EOF
        // proves a device disconnect or a disabled setting.
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Io);
    }

    #[tokio::test]
    async fn preserving_start_retains_idle_timeout_after_identifier_report() {
        let (stream, mut peer) = duplex(16 * 1024);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let duration = Duration::from_millis(20);
        let error = client.start_preserving_errors(duration).await.unwrap_err();
        assert!(error
            .timeout_duration()
            .is_some_and(|value| value <= duration));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
        let report = recv_plist(&mut peer).await.unwrap();
        assert_eq!(
            report.get("__selector").and_then(plist::Value::as_string),
            Some("_rpc_reportIdentifier:")
        );
    }

    #[tokio::test]
    async fn preserving_start_retains_timeout_while_identifier_write_is_blocked() {
        let (stream, _peer) = duplex(8);
        let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
        let duration = Duration::from_millis(20);
        let error = client.start_preserving_errors(duration).await.unwrap_err();
        assert_eq!(error.timeout_duration(), Some(duration));
        assert_eq!(crate::classify_error(&error), crate::ErrorClass::Timeout);
    }

    #[tokio::test]
    async fn both_start_apis_send_the_same_report_and_accept_not_available_state() {
        for preserve in [false, true] {
            let (stream, mut peer) = duplex(16 * 1024);
            let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
            let (result, ()) = timeout(Duration::from_secs(1), async {
                tokio::join!(
                    async {
                        if preserve {
                            client.start_preserving_errors(Duration::from_secs(1)).await
                        } else {
                            client.start(Duration::from_secs(1)).await
                        }
                    },
                    async move {
                        let report = recv_plist(&mut peer).await.unwrap();
                        assert_eq!(
                            report,
                            plist::Dictionary::from_iter([
                                (
                                    "__selector".to_string(),
                                    plist::Value::String("_rpc_reportIdentifier:".into()),
                                ),
                                (
                                    "__argument".to_string(),
                                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                                        "WIRConnectionIdentifierKey".to_string(),
                                        plist::Value::String("TEST".into()),
                                    )])),
                                ),
                            ])
                        );
                        // A valid intermediate event is consumed before the
                        // existing CurrentState completion condition.
                        for (selector, arguments) in [
                            (
                                "_rpc_reportConnectedApplicationList:",
                                plist::Dictionary::from_iter([(
                                    "WIRApplicationDictionaryKey".to_string(),
                                    plist::Value::Dictionary(plist::Dictionary::new()),
                                )]),
                            ),
                            (
                                "_rpc_reportCurrentState:",
                                plist::Dictionary::from_iter([(
                                    "WIRAutomationAvailabilityKey".to_string(),
                                    plist::Value::String(
                                        "WIRAutomationAvailabilityNotAvailable".into(),
                                    ),
                                )]),
                            ),
                        ] {
                            send_plist(
                                &mut peer,
                                &plist::Value::Dictionary(plist::Dictionary::from_iter([
                                    (
                                        "__selector".to_string(),
                                        plist::Value::String(selector.into()),
                                    ),
                                    (
                                        "__argument".to_string(),
                                        plist::Value::Dictionary(arguments),
                                    ),
                                ])),
                            )
                            .await
                            .unwrap();
                        }
                    }
                )
            })
            .await
            .expect("owned startup/peer must finish");
            result.unwrap();
            assert_eq!(
                client.automation_availability(),
                Some(AutomationAvailability::NotAvailable)
            );
            assert!(client.applications().is_empty());
        }
    }

    #[tokio::test]
    async fn both_start_apis_retain_malformed_state_as_protocol_error() {
        for preserve in [false, true] {
            let (stream, mut peer) = duplex(16 * 1024);
            let mut client = WebInspectorClient::with_connection_id(stream, "TEST");
            let (result, ()) = timeout(Duration::from_secs(1), async {
                tokio::join!(
                    async {
                        if preserve {
                            client.start_preserving_errors(Duration::from_secs(1)).await
                        } else {
                            client.start(Duration::from_secs(1)).await
                        }
                    },
                    async move {
                        recv_plist(&mut peer).await.unwrap();
                        send_plist(
                            &mut peer,
                            &plist::Value::Dictionary(plist::Dictionary::from_iter([
                                (
                                    "__selector".to_string(),
                                    plist::Value::String("_rpc_reportCurrentState:".into()),
                                ),
                                (
                                    "__argument".to_string(),
                                    plist::Value::Dictionary(plist::Dictionary::new()),
                                ),
                            ])),
                        )
                        .await
                        .unwrap();
                    }
                )
            })
            .await
            .expect("owned startup/peer must finish");
            let error = result.unwrap_err();
            assert!(matches!(error, WebInspectorError::Protocol(_)));
            assert_eq!(crate::classify_error(&error), crate::ErrorClass::Protocol);
        }
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
    async fn handle_message_application_listing_replaces_existing_page_cache() {
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
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages.get(&1).and_then(|page| page.title.as_deref()),
            Some("Updated Example")
        );
        assert_eq!(
            pages.get(&1).and_then(|page| page.url.as_deref()),
            Some("https://updated.example.com")
        );
        assert!(pages.get(&2).is_none());
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
}

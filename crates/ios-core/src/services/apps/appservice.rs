//! iOS 17+ CoreDevice appservice helpers for running processes and app lifecycle.
//!
//! Appservice is exposed through the CoreDevice feature invocation envelope rather
//! than the legacy InstallationProxy service. Use it for process listing, launching,
//! spawning executables, icon retrieval, and process termination monitoring when the
//! device exposes the appservice features through RSD.

use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};
use bytes::Bytes;
use futures_util::StreamExt;
use indexmap::IndexMap;

const FEATURE_LIST_PROCESSES: &str = "com.apple.coredevice.feature.listprocesses";
const FEATURE_LIST_APPS: &str = "com.apple.coredevice.feature.listapps";
const FEATURE_STREAM_APPS: &str = "com.apple.coredevice.feature.streamapplist";
const FEATURE_STREAM_PROCESSES: &str = "com.apple.coredevice.feature.streamprocesslist";
const FEATURE_LIST_ROOTS: &str = "com.apple.coredevice.feature.listroots";
const FEATURE_LAUNCH_APPLICATION: &str = "com.apple.coredevice.feature.launchapplication";
const FEATURE_SPAWN_EXECUTABLE: &str = "com.apple.coredevice.feature.spawnexecutable";
const FEATURE_FETCH_APP_ICONS: &str = "com.apple.coredevice.feature.fetchappicons";
const FEATURE_MONITOR_PROCESS_TERMINATION: &str =
    "com.apple.coredevice.feature.monitorprocesstermination";
const FEATURE_SEND_SIGNAL: &str = "com.apple.coredevice.feature.sendsignaltoprocess";
const SIGKILL: i64 = 9;

/// Errors returned by CoreDevice appservice operations.
#[derive(Debug, thiserror::Error)]
pub enum AppServiceError {
    /// Underlying XPC transport or encoding error.
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    /// Appservice response did not match the expected protocol shape.
    #[error("protocol error: {0}")]
    Protocol(String),
}

/// Running process entry returned by `list_processes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningAppProcess {
    /// Process identifier.
    pub pid: u64,
    /// Bundle identifier when the process maps to an app.
    pub bundle_id: Option<String>,
    /// Display or executable name reported by CoreDevice.
    pub name: String,
    /// Executable path or name when present in the response.
    pub executable: Option<String>,
    /// Whether CoreDevice classified the process as an application.
    pub is_application: Option<bool>,
}

/// Filters used for CoreDevice app listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListAppsOptions {
    /// Include App Clip bundles.
    pub include_app_clips: bool,
    /// Include removable user apps.
    pub include_removable_apps: bool,
    /// Include hidden apps.
    pub include_hidden_apps: bool,
    /// Include internal Apple apps.
    pub include_internal_apps: bool,
    /// Include default system apps.
    pub include_default_apps: bool,
    /// Request access to application containers.
    pub require_container_access: bool,
    /// Include application group identifiers in each app entry.
    pub include_app_group_identifiers: bool,
    /// Include application container paths in each app entry.
    pub include_container_paths: bool,
}

impl Default for ListAppsOptions {
    fn default() -> Self {
        Self {
            include_app_clips: true,
            include_removable_apps: true,
            include_hidden_apps: true,
            include_internal_apps: true,
            include_default_apps: true,
            require_container_access: false,
            include_app_group_identifiers: false,
            include_container_paths: false,
        }
    }
}

/// Application metadata returned by CoreDevice app listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDeviceAppInfo {
    /// Bundle identifier.
    pub bundle_id: String,
    /// Display name when CoreDevice provides one.
    pub name: Option<String>,
    /// Version string when present.
    pub version: Option<String>,
    /// Whether the app can be removed by the user.
    pub is_removable: Option<bool>,
    /// Whether the app is hidden from normal listing.
    pub is_hidden: Option<bool>,
    /// Whether CoreDevice marks the app as internal.
    pub is_internal: Option<bool>,
    /// Whether the bundle is an App Clip.
    pub is_app_clip: Option<bool>,
}

/// Options for launching an application through CoreDevice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchApplicationOptions {
    /// Command-line arguments passed to the app process.
    pub arguments: Vec<String>,
    /// Environment variables passed to the app process.
    pub environment_variables: IndexMap<String, String>,
    /// Request pseudo-terminal backed standard I/O.
    pub standard_io_uses_pseudoterminals: bool,
    /// Start the process suspended.
    pub start_stopped: bool,
    /// Terminate an existing instance before launching.
    pub terminate_existing: bool,
    /// Optional CoreDevice standard I/O routing identifiers.
    pub standard_io_identifiers: IndexMap<String, String>,
}

impl Default for LaunchApplicationOptions {
    fn default() -> Self {
        Self {
            arguments: Vec::new(),
            environment_variables: IndexMap::new(),
            standard_io_uses_pseudoterminals: true,
            start_stopped: false,
            terminate_existing: false,
            standard_io_identifiers: IndexMap::new(),
        }
    }
}

/// Raw icon payload and scale metadata returned by CoreDevice.
#[derive(Debug, Clone, PartialEq)]
pub struct AppIcon {
    /// Encoded image bytes.
    pub data: Bytes,
    /// Logical icon width.
    pub width: Option<f64>,
    /// Logical icon height.
    pub height: Option<f64>,
    /// Icon scale factor.
    pub scale: Option<f64>,
}

/// Process termination event returned by CoreDevice monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessTermination {
    /// Terminated process identifier.
    pub pid: Option<u64>,
    /// Exit status when the process exited normally.
    pub exit_status: Option<i64>,
    /// Signal number when the process was signaled.
    pub signal: Option<i64>,
    /// Human-readable reason when CoreDevice provides one.
    pub reason: Option<String>,
}

pub use crate::services::coredevice::CoreDeviceEnvelopeMode;

/// Client for CoreDevice appservice feature calls.
pub struct AppServiceClient {
    client: XpcClient,
    device_identifier: String,
    /// RSD feature metadata confirms a modern CoreDevice service; the explicit
    /// mode remains available for devices known to require the old envelope.
    envelope_mode: CoreDeviceEnvelopeMode,
    /// `None` means the RSD feature list was not available. `Some` is an
    /// explicit feature list and is used only for capability routing.
    service_features: Option<Vec<String>>,
}

impl AppServiceClient {
    /// Create an appservice client using the current CoreDevice envelope.
    pub fn new(client: XpcClient, device_identifier: impl Into<String>) -> Self {
        Self::new_with_mode(client, device_identifier, CoreDeviceEnvelopeMode::Modern)
    }

    /// Create an appservice client using the pre-modern CoreDevice envelope.
    ///
    /// Use this only when device/version evidence outside the RSD feature list
    /// shows that the older protocol is required. A failed modern request is
    /// never replayed automatically because appservice operations may mutate
    /// device state.
    pub fn new_legacy(client: XpcClient, device_identifier: impl Into<String>) -> Self {
        Self::new_with_mode(client, device_identifier, CoreDeviceEnvelopeMode::Legacy)
    }

    /// Create an appservice client with an explicit envelope mode.
    pub fn new_with_mode(
        client: XpcClient,
        device_identifier: impl Into<String>,
        envelope_mode: CoreDeviceEnvelopeMode,
    ) -> Self {
        Self {
            client,
            device_identifier: device_identifier.into(),
            envelope_mode,
            service_features: None,
        }
    }

    /// Create an appservice client with the features advertised by RSD.
    ///
    /// Presence of an RSD service entry is the available modern-device
    /// routing signal. An empty feature list remains permissive for feature
    /// checks, but still selects the modern envelope. A device that explicitly
    /// advertises `streamapplist` must be queried through that feature:
    /// iOS 26-era `dtappserviced` does not answer the older `listapps` request.
    pub fn new_with_features<I, S>(
        client: XpcClient,
        device_identifier: impl Into<String>,
        service_features: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            client,
            device_identifier: device_identifier.into(),
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: Some(service_features.into_iter().map(Into::into).collect()),
        }
    }

    /// Return the envelope mode selected for ordinary feature calls.
    pub fn envelope_mode(&self) -> CoreDeviceEnvelopeMode {
        self.envelope_mode
    }

    fn feature_is_advertised(&self, feature: &str) -> bool {
        feature_is_advertised(self.service_features.as_deref(), feature)
    }

    fn feature_is_known_and_missing(&self, feature: &str) -> bool {
        feature_is_known_and_missing(self.service_features.as_deref(), feature)
    }

    fn ensure_feature(&self, feature: &str) -> Result<(), AppServiceError> {
        if self.feature_is_known_and_missing(feature) {
            return Err(AppServiceError::Protocol(format!(
                "CoreDevice feature {feature} is not advertised by RSD"
            )));
        }
        Ok(())
    }

    fn request(&self, feature_identifier: &str, input: XpcValue) -> XpcValue {
        build_request_for_mode(
            self.envelope_mode,
            &self.device_identifier,
            feature_identifier,
            input,
        )
    }

    /// List running processes visible to CoreDevice.
    pub async fn list_processes(&mut self) -> Result<Vec<RunningAppProcess>, AppServiceError> {
        self.ensure_feature(FEATURE_LIST_PROCESSES)?;
        let response = self
            .client
            .call(self.request(
                FEATURE_LIST_PROCESSES,
                XpcValue::Dictionary(IndexMap::new()),
            ))
            .await?;
        parse_processes(&response)
    }

    /// List installed apps using the supplied CoreDevice filters.
    pub async fn list_apps(
        &mut self,
        options: ListAppsOptions,
    ) -> Result<Vec<CoreDeviceAppInfo>, AppServiceError> {
        if self.feature_is_advertised(FEATURE_STREAM_APPS) {
            let mut stream = Box::pin(self.stream_apps(options));
            let mut apps = Vec::new();
            while let Some(app) = stream.next().await {
                apps.push(app?);
            }
            return Ok(apps);
        }

        self.ensure_feature(FEATURE_LIST_APPS)?;
        let response = self
            .client
            .call(self.request(FEATURE_LIST_APPS, build_list_apps_input(options)))
            .await?;
        parse_apps(&response)
    }

    /// Stream installed apps as CoreDevice pushes them over its side channel.
    ///
    /// If RSD supplied a non-empty feature list and it does not contain
    /// `streamapplist`, no request is sent. Unknown/empty feature metadata
    /// remains permissive for older transports where RSD omitted
    /// `Properties.Features`.
    pub fn stream_apps(
        &mut self,
        options: ListAppsOptions,
    ) -> impl futures_core::Stream<Item = Result<CoreDeviceAppInfo, AppServiceError>> + '_ {
        let envelope_mode = self.envelope_mode;
        let feature_missing = self.feature_is_known_and_missing(FEATURE_STREAM_APPS);
        async_stream::try_stream! {
            ensure_modern_stream_mode(envelope_mode, FEATURE_STREAM_APPS)?;
            if feature_missing {
                Err(AppServiceError::Protocol(
                    "CoreDevice streamapplist is not advertised by RSD".to_string(),
                ))?;
            }

            let mut stream = Box::pin(crate::services::coredevice::stream_invoke(
                &mut self.client,
                &self.device_identifier,
                FEATURE_STREAM_APPS,
                build_list_apps_input(options),
            ));
            while let Some(value) = stream.next().await {
                let value = value?;
                let app = parse_app(&value).ok_or_else(|| {
                    AppServiceError::Protocol(
                        "CoreDevice streamapplist yielded an invalid app element".to_string(),
                    )
                })?;
                yield app;
            }
        }
    }

    /// Stream process tokens as CoreDevice pushes them over its side channel.
    ///
    /// As with the upstream client, this is an explicit streaming operation;
    /// callers should select it only when the device advertises
    /// `streamprocesslist`. A non-empty RSD list is checked before any request
    /// is sent, while missing feature metadata remains permissive.
    pub fn stream_processes(
        &mut self,
    ) -> impl futures_core::Stream<Item = Result<RunningAppProcess, AppServiceError>> + '_ {
        let envelope_mode = self.envelope_mode;
        let feature_missing = self.feature_is_known_and_missing(FEATURE_STREAM_PROCESSES);
        async_stream::try_stream! {
            ensure_modern_stream_mode(envelope_mode, FEATURE_STREAM_PROCESSES)?;
            if feature_missing {
                Err(AppServiceError::Protocol(
                    "CoreDevice streamprocesslist is not advertised by RSD".to_string(),
                ))?;
            }

            let mut stream = Box::pin(crate::services::coredevice::stream_invoke(
                &mut self.client,
                &self.device_identifier,
                FEATURE_STREAM_PROCESSES,
                XpcValue::Dictionary(IndexMap::new()),
            ));
            while let Some(value) = stream.next().await {
                let value = value?;
                let process = parse_process(&value).ok_or_else(|| {
                    AppServiceError::Protocol(
                        "CoreDevice streamprocesslist yielded an invalid process element"
                            .to_string(),
                    )
                })?;
                yield process;
            }
        }
    }

    /// Return appservice root descriptors as the raw CoreDevice output value.
    pub async fn list_roots(&mut self) -> Result<XpcValue, AppServiceError> {
        self.ensure_feature(FEATURE_LIST_ROOTS)?;
        let response = self
            .client
            .call(self.request(FEATURE_LIST_ROOTS, build_list_roots_input()))
            .await?;
        parse_output_value(response)
    }

    /// Send SIGKILL to a process.
    pub async fn kill_process(&mut self, pid: u64) -> Result<(), AppServiceError> {
        self.send_signal(pid, SIGKILL).await
    }

    /// Send an arbitrary signal to a process identifier.
    pub async fn send_signal(&mut self, pid: u64, signal: i64) -> Result<(), AppServiceError> {
        self.ensure_feature(FEATURE_SEND_SIGNAL)?;
        let response = self
            .client
            .call(self.request(FEATURE_SEND_SIGNAL, build_send_signal_input(pid, signal)?))
            .await?;
        ensure_no_error(&response)?;
        Ok(())
    }

    /// Launch an app using default CoreDevice options.
    pub async fn launch_application(
        &mut self,
        bundle_id: &str,
    ) -> Result<Option<u64>, AppServiceError> {
        self.ensure_feature(FEATURE_LAUNCH_APPLICATION)?;
        let response = self
            .client
            .call(self.request(
                FEATURE_LAUNCH_APPLICATION,
                build_launch_application_input(bundle_id)?,
            ))
            .await?;
        ensure_no_error(&response)?;
        Ok(parse_pid(response.body.as_ref()))
    }

    /// Launch an app using explicit CoreDevice launch options.
    pub async fn launch_application_with_options(
        &mut self,
        bundle_id: &str,
        options: &LaunchApplicationOptions,
    ) -> Result<Option<u64>, AppServiceError> {
        self.ensure_feature(FEATURE_LAUNCH_APPLICATION)?;
        let response = self
            .client
            .call(self.request(
                FEATURE_LAUNCH_APPLICATION,
                build_launch_application_input_with_options(bundle_id, options)?,
            ))
            .await?;
        ensure_no_error(&response)?;
        Ok(parse_pid(response.body.as_ref()))
    }

    /// Spawn an executable path with command-line arguments.
    pub async fn spawn_executable(
        &mut self,
        executable: &str,
        arguments: &[String],
    ) -> Result<Option<u64>, AppServiceError> {
        self.ensure_feature(FEATURE_SPAWN_EXECUTABLE)?;
        let response = self
            .client
            .call(self.request(
                FEATURE_SPAWN_EXECUTABLE,
                build_spawn_executable_input(executable, arguments)?,
            ))
            .await?;
        ensure_no_error(&response)?;
        Ok(parse_pid(response.body.as_ref()))
    }

    /// Fetch one or more rendered app icons for a bundle.
    #[deprecated(
        note = "CoreDevice icons are served by iconservice; use IconServiceClient::fetch_icon instead"
    )]
    pub async fn fetch_app_icons(
        &mut self,
        bundle_id: &str,
        width: f64,
        height: f64,
        scale: f64,
        allow_placeholder: bool,
    ) -> Result<Vec<AppIcon>, AppServiceError> {
        self.ensure_feature(FEATURE_FETCH_APP_ICONS)?;
        let response = self
            .client
            .call(self.request(
                FEATURE_FETCH_APP_ICONS,
                build_fetch_app_icons_input(bundle_id, width, height, scale, allow_placeholder),
            ))
            .await?;
        parse_app_icons(&response)
    }

    /// Wait for CoreDevice to report a process termination event.
    pub async fn monitor_process_termination(
        &mut self,
        pid: u64,
    ) -> Result<ProcessTermination, AppServiceError> {
        self.ensure_feature(FEATURE_MONITOR_PROCESS_TERMINATION)?;
        let response = self
            .client
            .call(self.request(
                FEATURE_MONITOR_PROCESS_TERMINATION,
                build_monitor_process_termination_input(pid)?,
            ))
            .await?;
        parse_process_termination(&response)
    }
}

fn feature_is_advertised(service_features: Option<&[String]>, feature: &str) -> bool {
    service_features.is_some_and(|features| features.iter().any(|item| item == feature))
}

fn feature_is_known_and_missing(service_features: Option<&[String]>, feature: &str) -> bool {
    // An empty RSD feature list means the service omitted capability metadata,
    // not that it explicitly rejected every feature.
    service_features.is_some_and(|features| {
        !features.is_empty() && !features.iter().any(|item| item == feature)
    })
}

fn ensure_modern_stream_mode(
    envelope_mode: CoreDeviceEnvelopeMode,
    feature: &str,
) -> Result<(), AppServiceError> {
    if envelope_mode == CoreDeviceEnvelopeMode::Legacy {
        return Err(AppServiceError::Protocol(format!(
            "CoreDevice streaming feature {feature} requires the modern DDI envelope"
        )));
    }
    Ok(())
}

fn build_list_apps_input(options: ListAppsOptions) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        (
            "includeAppClips".to_string(),
            XpcValue::Bool(options.include_app_clips),
        ),
        (
            "includeRemovableApps".to_string(),
            XpcValue::Bool(options.include_removable_apps),
        ),
        (
            "includeHiddenApps".to_string(),
            XpcValue::Bool(options.include_hidden_apps),
        ),
        (
            "includeInternalApps".to_string(),
            XpcValue::Bool(options.include_internal_apps),
        ),
        (
            "includeDefaultApps".to_string(),
            XpcValue::Bool(options.include_default_apps),
        ),
        (
            "requireContainerAccess".to_string(),
            XpcValue::Bool(options.require_container_access),
        ),
        (
            "includeAppGroupIdentifiers".to_string(),
            XpcValue::Bool(options.include_app_group_identifiers),
        ),
        (
            "includeContainerPaths".to_string(),
            XpcValue::Bool(options.include_container_paths),
        ),
    ]))
}

fn build_list_roots_input() -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([(
        "rootPoint".to_string(),
        XpcValue::Dictionary(IndexMap::from([(
            "relative".to_string(),
            XpcValue::String("/".to_string()),
        )])),
    )]))
}

fn build_send_signal_input(pid: u64, signal: i64) -> Result<XpcValue, AppServiceError> {
    let pid = process_identifier_to_i64(pid)?;
    Ok(XpcValue::Dictionary(IndexMap::from([
        (
            "process".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "processIdentifier".to_string(),
                XpcValue::Int64(pid),
            )])),
        ),
        ("signal".to_string(), XpcValue::Int64(signal)),
    ])))
}

fn build_spawn_executable_input(
    executable: &str,
    arguments: &[String],
) -> Result<XpcValue, AppServiceError> {
    let platform_specific_options = empty_binary_plist("platformSpecificOptions")?;

    Ok(XpcValue::Dictionary(IndexMap::from([
        (
            "executableItem".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "url".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "_0".to_string(),
                    XpcValue::Dictionary(IndexMap::from([(
                        "relative".to_string(),
                        XpcValue::String(executable.to_string()),
                    )])),
                )])),
            )])),
        ),
        (
            "standardIOIdentifiers".to_string(),
            XpcValue::Dictionary(IndexMap::new()),
        ),
        (
            "options".to_string(),
            XpcValue::Dictionary(IndexMap::from([
                (
                    "arguments".to_string(),
                    XpcValue::Array(
                        arguments
                            .iter()
                            .map(|argument| XpcValue::String(argument.clone()))
                            .collect(),
                    ),
                ),
                (
                    "environmentVariables".to_string(),
                    XpcValue::Dictionary(IndexMap::new()),
                ),
                (
                    "standardIOUsesPseudoterminals".to_string(),
                    XpcValue::Bool(true),
                ),
                ("startStopped".to_string(), XpcValue::Bool(false)),
                (
                    "user".to_string(),
                    XpcValue::Dictionary(IndexMap::from([(
                        "active".to_string(),
                        XpcValue::Bool(true),
                    )])),
                ),
                (
                    "platformSpecificOptions".to_string(),
                    XpcValue::Data(platform_specific_options),
                ),
            ])),
        ),
    ])))
}

fn build_fetch_app_icons_input(
    bundle_id: &str,
    width: f64,
    height: f64,
    scale: f64,
    allow_placeholder: bool,
) -> XpcValue {
    XpcValue::Dictionary(IndexMap::from([
        ("width".to_string(), XpcValue::Double(width)),
        ("height".to_string(), XpcValue::Double(height)),
        ("scale".to_string(), XpcValue::Double(scale)),
        (
            "allowPlaceholder".to_string(),
            XpcValue::Bool(allow_placeholder),
        ),
        (
            "bundleIdentifier".to_string(),
            XpcValue::String(bundle_id.to_string()),
        ),
    ]))
}

fn build_monitor_process_termination_input(pid: u64) -> Result<XpcValue, AppServiceError> {
    let pid = process_identifier_to_i64(pid)?;
    Ok(XpcValue::Dictionary(IndexMap::from([(
        "processToken".to_string(),
        XpcValue::Dictionary(IndexMap::from([(
            "processIdentifier".to_string(),
            XpcValue::Int64(pid),
        )])),
    )])))
}

fn process_identifier_to_i64(pid: u64) -> Result<i64, AppServiceError> {
    i64::try_from(pid).map_err(|_| {
        AppServiceError::Protocol(format!("process id exceeds DTX integer range: {pid}"))
    })
}

fn build_launch_application_input(bundle_id: &str) -> Result<XpcValue, AppServiceError> {
    build_launch_application_input_with_options(bundle_id, &LaunchApplicationOptions::default())
}

fn build_launch_application_input_with_options(
    bundle_id: &str,
    options: &LaunchApplicationOptions,
) -> Result<XpcValue, AppServiceError> {
    let platform_specific_options = empty_binary_plist("platformSpecificOptions")?;

    Ok(XpcValue::Dictionary(IndexMap::from([
        (
            "applicationSpecifier".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "bundleIdentifier".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "_0".to_string(),
                    XpcValue::String(bundle_id.to_string()),
                )])),
            )])),
        ),
        (
            "options".to_string(),
            XpcValue::Dictionary(IndexMap::from([
                (
                    "arguments".to_string(),
                    XpcValue::Array(
                        options
                            .arguments
                            .iter()
                            .map(|argument| XpcValue::String(argument.clone()))
                            .collect(),
                    ),
                ),
                (
                    "environmentVariables".to_string(),
                    string_map_to_xpc_dict(&options.environment_variables),
                ),
                (
                    "standardIOUsesPseudoterminals".to_string(),
                    XpcValue::Bool(options.standard_io_uses_pseudoterminals),
                ),
                (
                    "startStopped".to_string(),
                    XpcValue::Bool(options.start_stopped),
                ),
                (
                    "terminateExisting".to_string(),
                    XpcValue::Bool(options.terminate_existing),
                ),
                (
                    "user".to_string(),
                    XpcValue::Dictionary(IndexMap::from([
                        ("active".to_string(), XpcValue::Bool(true)),
                        (
                            "shortName".to_string(),
                            XpcValue::String("mobile".to_string()),
                        ),
                    ])),
                ),
                (
                    "platformSpecificOptions".to_string(),
                    XpcValue::Data(platform_specific_options),
                ),
            ])),
        ),
        (
            "standardIOIdentifiers".to_string(),
            string_map_to_xpc_dict(&options.standard_io_identifiers),
        ),
    ])))
}

fn string_map_to_xpc_dict(values: &IndexMap<String, String>) -> XpcValue {
    XpcValue::Dictionary(
        values
            .iter()
            .map(|(key, value)| (key.clone(), XpcValue::String(value.clone())))
            .collect(),
    )
}

fn empty_binary_plist(field_name: &str) -> Result<Bytes, AppServiceError> {
    let mut bytes = Vec::new();
    plist::to_writer_binary(
        &mut bytes,
        &plist::Value::Dictionary(plist::Dictionary::new()),
    )
    .map_err(|error| {
        AppServiceError::Protocol(format!("failed to encode {field_name}: {error}"))
    })?;
    Ok(Bytes::from(bytes))
}

#[cfg(test)]
fn build_request(device_identifier: &str, feature_identifier: &str, input: XpcValue) -> XpcValue {
    crate::services::coredevice::build_request(device_identifier, feature_identifier, input)
}

fn build_request_for_mode(
    mode: CoreDeviceEnvelopeMode,
    device_identifier: &str,
    feature_identifier: &str,
    input: XpcValue,
) -> XpcValue {
    match mode {
        CoreDeviceEnvelopeMode::Modern => {
            crate::services::coredevice::build_request(device_identifier, feature_identifier, input)
        }
        CoreDeviceEnvelopeMode::Legacy => crate::services::coredevice::build_legacy_request(
            device_identifier,
            feature_identifier,
            input,
        ),
    }
}

fn parse_processes(response: &XpcMessage) -> Result<Vec<RunningAppProcess>, AppServiceError> {
    let payload = output_ref(response)?;

    let items = process_items(payload).ok_or_else(|| {
        AppServiceError::Protocol(format!("unexpected process list payload: {payload:?}"))
    })?;

    Ok(items.iter().filter_map(parse_process).collect())
}

fn parse_apps(response: &XpcMessage) -> Result<Vec<CoreDeviceAppInfo>, AppServiceError> {
    let payload = output_ref(response)?;
    let items = app_items(payload).ok_or_else(|| {
        AppServiceError::Protocol(format!("unexpected app list payload: {payload:?}"))
    })?;

    Ok(items.iter().filter_map(parse_app).collect())
}

fn parse_app_icons(response: &XpcMessage) -> Result<Vec<AppIcon>, AppServiceError> {
    let payload = output_ref(response)?;
    let items = icon_items(payload).ok_or_else(|| {
        AppServiceError::Protocol(format!("unexpected app icon payload: {payload:?}"))
    })?;

    Ok(items.iter().filter_map(parse_app_icon).collect())
}

fn parse_process_termination(response: &XpcMessage) -> Result<ProcessTermination, AppServiceError> {
    let payload = output_ref(response)?;
    let dict = payload.as_dict().ok_or_else(|| {
        AppServiceError::Protocol(format!(
            "unexpected process termination payload: {payload:?}"
        ))
    })?;

    Ok(ProcessTermination {
        pid: parse_pid(Some(payload)),
        exit_status: integer_field(dict, &["exitStatus", "exitCode", "status"]),
        signal: integer_field(dict, &["signal", "terminationSignal"]),
        reason: string_field(dict, &["reason", "terminationReason", "message"]),
    })
}

fn parse_output_value(response: XpcMessage) -> Result<XpcValue, AppServiceError> {
    crate::services::coredevice::parse_output(response).map_err(AppServiceError::Protocol)
}

fn output_ref(response: &XpcMessage) -> Result<&XpcValue, AppServiceError> {
    ensure_no_error(response)?;
    let body = response
        .body
        .as_ref()
        .ok_or_else(|| AppServiceError::Protocol("missing response body".into()))?;
    Ok(crate::services::coredevice::output(body).unwrap_or(body))
}

fn process_items(value: &XpcValue) -> Option<&[XpcValue]> {
    match value {
        XpcValue::Array(items) => Some(items.as_slice()),
        XpcValue::Dictionary(dict) => {
            for key in ["processTokens", "processes", "items"] {
                if let Some(XpcValue::Array(items)) = dict.get(key) {
                    return Some(items.as_slice());
                }
            }
            None
        }
        _ => None,
    }
}

fn app_items(value: &XpcValue) -> Option<&[XpcValue]> {
    match value {
        XpcValue::Array(items) => Some(items.as_slice()),
        XpcValue::Dictionary(dict) => {
            for key in ["apps", "appTokens", "applications", "items"] {
                if let Some(XpcValue::Array(items)) = dict.get(key) {
                    return Some(items.as_slice());
                }
            }
            None
        }
        _ => None,
    }
}

fn icon_items(value: &XpcValue) -> Option<&[XpcValue]> {
    match value {
        XpcValue::Array(items) => Some(items.as_slice()),
        XpcValue::Dictionary(dict) => {
            for key in ["icons", "appIcons", "items"] {
                if let Some(XpcValue::Array(items)) = dict.get(key) {
                    return Some(items.as_slice());
                }
            }
            if has_icon_data(dict) {
                Some(std::slice::from_ref(value))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_process(value: &XpcValue) -> Option<RunningAppProcess> {
    let dict = value.as_dict()?;
    let pid = dict
        .get("processIdentifier")
        .and_then(as_u64)
        .or_else(|| dict.get("pid").and_then(as_u64))?;
    let executable_url = url_relative(dict.get("executableURL"));
    let name = string_field(
        dict,
        &[
            "localizedName",
            "name",
            "executableDisplayName",
            "bundleIdentifier",
        ],
    )
    .or_else(|| executable_url.as_deref().and_then(file_name))
    .unwrap_or_else(|| pid.to_string());
    let bundle_id = string_field(dict, &["bundleIdentifier", "bundleIdentifierKey"]);
    let executable = executable_url.or_else(|| string_field(dict, &["executableName", "name"]));
    let is_application = dict.get("isApplication").and_then(as_bool);

    Some(RunningAppProcess {
        pid,
        bundle_id,
        name,
        executable,
        is_application,
    })
}

fn parse_app(value: &XpcValue) -> Option<CoreDeviceAppInfo> {
    let dict = value.as_dict()?;
    let bundle_id = string_field(
        dict,
        &["bundleIdentifier", "bundleID", "CFBundleIdentifier"],
    )?;

    Some(CoreDeviceAppInfo {
        bundle_id,
        name: string_field(
            dict,
            &["localizedName", "displayName", "name", "CFBundleName"],
        ),
        version: string_field(dict, &["version", "bundleVersion", "CFBundleVersion"]),
        is_removable: bool_field(dict, &["isRemovable", "removable"]),
        is_hidden: bool_field(dict, &["isHidden", "hidden"]),
        is_internal: bool_field(dict, &["isInternal", "internal"]),
        is_app_clip: bool_field(dict, &["isAppClip", "appClip"]),
    })
}

fn parse_app_icon(value: &XpcValue) -> Option<AppIcon> {
    let dict = value.as_dict()?;
    let data = data_field(dict, &["iconData", "data", "pngData", "bitmapData"])?;

    Some(AppIcon {
        data,
        width: double_field(dict, &["width"]),
        height: double_field(dict, &["height"]),
        scale: double_field(dict, &["scale"]),
    })
}

fn ensure_no_error(response: &XpcMessage) -> Result<(), AppServiceError> {
    if let Some(body) = response.body.as_ref() {
        crate::services::coredevice::ensure_no_error(body).map_err(AppServiceError::Protocol)?;
    }
    Ok(())
}

fn parse_pid(value: Option<&XpcValue>) -> Option<u64> {
    match value {
        Some(XpcValue::Uint64(pid)) => Some(*pid),
        Some(XpcValue::Int64(pid)) if *pid >= 0 => Some(*pid as u64),
        Some(XpcValue::Dictionary(dict)) => {
            for key in ["processIdentifier", "pid"] {
                if let Some(pid) = dict.get(key).and_then(as_u64) {
                    return Some(pid);
                }
            }
            for key in [
                "CoreDevice.output",
                "processToken",
                "process",
                "launchedProcess",
                "executableToken",
            ] {
                if let Some(pid) = parse_pid(dict.get(key)) {
                    return Some(pid);
                }
            }
            None
        }
        _ => None,
    }
}

fn string_field(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        dict.get(*key)
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    })
}

fn bool_field(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| dict.get(*key).and_then(as_bool))
}

fn integer_field(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| dict.get(*key).and_then(as_i64))
}

fn double_field(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| dict.get(*key).and_then(as_f64))
}

fn data_field(dict: &IndexMap<String, XpcValue>, keys: &[&str]) -> Option<Bytes> {
    keys.iter().find_map(|key| match dict.get(*key) {
        Some(XpcValue::Data(data)) => Some(data.clone()),
        _ => None,
    })
}

fn url_relative(value: Option<&XpcValue>) -> Option<String> {
    let dict = value?.as_dict()?;
    dict.get("relative")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

fn file_name(path: &str) -> Option<String> {
    path.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

fn has_icon_data(dict: &IndexMap<String, XpcValue>) -> bool {
    ["iconData", "data", "pngData", "bitmapData"]
        .iter()
        .any(|key| matches!(dict.get(*key), Some(XpcValue::Data(_))))
}

fn as_u64(value: &XpcValue) -> Option<u64> {
    match value {
        XpcValue::Uint64(n) => Some(*n),
        XpcValue::Int64(n) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

fn as_i64(value: &XpcValue) -> Option<i64> {
    match value {
        XpcValue::Int64(n) => Some(*n),
        XpcValue::Uint64(n) => i64::try_from(*n).ok(),
        _ => None,
    }
}

fn as_f64(value: &XpcValue) -> Option<f64> {
    match value {
        XpcValue::Double(n) => Some(*n),
        XpcValue::Int64(n) => Some(*n as f64),
        XpcValue::Uint64(n) => Some(*n as f64),
        _ => None,
    }
}

fn as_bool(value: &XpcValue) -> Option<bool> {
    match value {
        XpcValue::Bool(v) => Some(*v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_wraps_coredevice_envelope() {
        let request = build_request(
            "DEVICE-ID",
            FEATURE_SEND_SIGNAL,
            XpcValue::Dictionary(IndexMap::from([
                ("processIdentifier".to_string(), XpcValue::Uint64(42)),
                ("signal".to_string(), XpcValue::Int64(SIGKILL)),
            ])),
        );

        let dict = request.as_dict().unwrap();
        assert_eq!(
            dict["CoreDevice.featureIdentifier"].as_str(),
            Some(FEATURE_SEND_SIGNAL)
        );
        let device_identifier = dict["CoreDevice.deviceIdentifier"]
            .as_str()
            .expect("modern device identifier should be a string");
        assert!(uuid::Uuid::parse_str(device_identifier).is_ok());
        assert_eq!(
            dict["CoreDevice.CoreDeviceDDIProtocolVersion"],
            XpcValue::Int64(2)
        );
        let version = dict["CoreDevice.coreDeviceVersion"].as_dict().unwrap();
        assert_eq!(version["stringValue"].as_str(), Some("629.3"));
        assert!(dict["CoreDevice.invocationIdentifier"]
            .as_str()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .is_some());
    }

    #[test]
    fn appservice_envelope_mode_routes_every_ordinary_request() {
        for feature in [
            FEATURE_LIST_APPS,
            FEATURE_LIST_PROCESSES,
            FEATURE_LAUNCH_APPLICATION,
        ] {
            let modern = build_request_for_mode(
                CoreDeviceEnvelopeMode::Modern,
                "DEVICE-ID",
                feature,
                XpcValue::Null,
            );
            let legacy = build_request_for_mode(
                CoreDeviceEnvelopeMode::Legacy,
                "DEVICE-ID",
                feature,
                XpcValue::Null,
            );
            let modern = modern.as_dict().unwrap();
            let legacy = legacy.as_dict().unwrap();

            assert_eq!(
                modern["CoreDevice.featureIdentifier"].as_str(),
                Some(feature)
            );
            assert_eq!(
                legacy["CoreDevice.featureIdentifier"].as_str(),
                Some(feature)
            );
            assert_eq!(
                modern["CoreDevice.CoreDeviceDDIProtocolVersion"],
                XpcValue::Int64(2)
            );
            assert_eq!(
                legacy["CoreDevice.CoreDeviceDDIProtocolVersion"],
                XpcValue::Int64(0)
            );
            assert_ne!(
                modern["CoreDevice.deviceIdentifier"],
                legacy["CoreDevice.deviceIdentifier"]
            );
        }
    }

    #[test]
    fn build_send_signal_input_nests_process_identifier() {
        let input = build_send_signal_input(42, SIGKILL).unwrap();
        let dict = input.as_dict().unwrap();
        let process = dict["process"].as_dict().unwrap();

        assert_eq!(process["processIdentifier"], XpcValue::Int64(42));
        assert_eq!(dict["signal"], XpcValue::Int64(SIGKILL));
    }

    #[test]
    fn build_send_signal_input_rejects_pid_above_i64_max() {
        let err = build_send_signal_input(u64::MAX, SIGKILL).unwrap_err();

        assert!(
            matches!(err, AppServiceError::Protocol(message) if message.contains("process id exceeds"))
        );
    }

    #[test]
    fn build_launch_application_input_matches_reference_shape() {
        let input = build_launch_application_input("com.example.App").unwrap();
        let dict = input.as_dict().unwrap();
        let application_specifier = dict["applicationSpecifier"].as_dict().unwrap();
        let bundle_identifier = application_specifier["bundleIdentifier"].as_dict().unwrap();
        let options = dict["options"].as_dict().unwrap();
        let user = options["user"].as_dict().unwrap();

        assert_eq!(bundle_identifier["_0"].as_str(), Some("com.example.App"));
        assert_eq!(options["arguments"], XpcValue::Array(Vec::new()));
        assert_eq!(
            options["environmentVariables"],
            XpcValue::Dictionary(IndexMap::new())
        );
        assert_eq!(
            options["standardIOUsesPseudoterminals"],
            XpcValue::Bool(true)
        );
        assert_eq!(options["startStopped"], XpcValue::Bool(false));
        assert_eq!(options["terminateExisting"], XpcValue::Bool(false));
        assert_eq!(user["active"], XpcValue::Bool(true));
        assert_eq!(user["shortName"].as_str(), Some("mobile"));
        assert_eq!(
            dict["standardIOIdentifiers"],
            XpcValue::Dictionary(IndexMap::new())
        );

        let XpcValue::Data(platform_specific_options) = &options["platformSpecificOptions"] else {
            panic!("platformSpecificOptions should be XPC data");
        };
        let decoded: plist::Value =
            plist::from_bytes(platform_specific_options).expect("binary plist decode");
        assert_eq!(decoded, plist::Value::Dictionary(plist::Dictionary::new()));
    }

    #[test]
    fn build_launch_application_input_accepts_extended_options() {
        let options = LaunchApplicationOptions {
            arguments: vec!["--flag".into(), "value".into()],
            environment_variables: IndexMap::from([("FOO".into(), "bar".into())]),
            start_stopped: true,
            terminate_existing: true,
            standard_io_uses_pseudoterminals: false,
            standard_io_identifiers: IndexMap::from([("standardOutput".into(), "socket-1".into())]),
        };

        let input = build_launch_application_input_with_options("com.example.App", &options)
            .expect("launch input should build");
        let dict = input.as_dict().unwrap();
        let options = dict["options"].as_dict().unwrap();
        let environment = options["environmentVariables"].as_dict().unwrap();
        let stdio = dict["standardIOIdentifiers"].as_dict().unwrap();

        assert_eq!(
            options["arguments"],
            XpcValue::Array(vec![
                XpcValue::String("--flag".into()),
                XpcValue::String("value".into())
            ])
        );
        assert_eq!(environment["FOO"].as_str(), Some("bar"));
        assert_eq!(options["startStopped"], XpcValue::Bool(true));
        assert_eq!(options["terminateExisting"], XpcValue::Bool(true));
        assert_eq!(
            options["standardIOUsesPseudoterminals"],
            XpcValue::Bool(false)
        );
        assert_eq!(stdio["standardOutput"].as_str(), Some("socket-1"));
    }

    #[test]
    fn parse_processes_reads_coredevice_output_envelope() {
        let process = XpcValue::Dictionary(IndexMap::from([
            ("processIdentifier".to_string(), XpcValue::Uint64(99)),
            (
                "bundleIdentifier".to_string(),
                XpcValue::String("com.example.App".into()),
            ),
            (
                "localizedName".to_string(),
                XpcValue::String("Example".into()),
            ),
            (
                "executableName".to_string(),
                XpcValue::String("ExampleBin".into()),
            ),
            ("isApplication".to_string(), XpcValue::Bool(true)),
        ]));
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "processTokens".to_string(),
                    XpcValue::Array(vec![process]),
                )])),
            )]))),
        };

        let parsed = parse_processes(&response).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].pid, 99);
        assert_eq!(parsed[0].bundle_id.as_deref(), Some("com.example.App"));
    }

    #[test]
    fn ensure_no_error_reads_coredevice_error_envelope() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.error".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "localizedDescription".to_string(),
                    XpcValue::String("boom".into()),
                )])),
            )]))),
        };

        let err = ensure_no_error(&response).unwrap_err();
        assert!(matches!(err, AppServiceError::Protocol(message) if message == "boom"));
    }

    #[test]
    fn parse_pid_accepts_coredevice_output_process_token() {
        let pid = parse_pid(Some(&XpcValue::Dictionary(IndexMap::from([(
            "CoreDevice.output".to_string(),
            XpcValue::Dictionary(IndexMap::from([(
                "processToken".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "processIdentifier".to_string(),
                    XpcValue::Uint64(31337),
                )])),
            )])),
        )]))));

        assert_eq!(pid, Some(31337));
    }

    #[test]
    fn build_list_apps_input_matches_reference_shape() {
        let input = build_list_apps_input(ListAppsOptions::default());
        let dict = input.as_dict().unwrap();

        assert_eq!(dict["includeAppClips"], XpcValue::Bool(true));
        assert_eq!(dict["includeRemovableApps"], XpcValue::Bool(true));
        assert_eq!(dict["includeHiddenApps"], XpcValue::Bool(true));
        assert_eq!(dict["includeInternalApps"], XpcValue::Bool(true));
        assert_eq!(dict["includeDefaultApps"], XpcValue::Bool(true));
        assert_eq!(dict["requireContainerAccess"], XpcValue::Bool(false));
        assert_eq!(dict["includeAppGroupIdentifiers"], XpcValue::Bool(false));
        assert_eq!(dict["includeContainerPaths"], XpcValue::Bool(false));
        assert_eq!(dict.len(), 8);
    }

    #[test]
    fn build_list_apps_input_preserves_all_extended_filters() {
        let input = build_list_apps_input(ListAppsOptions {
            include_app_clips: false,
            include_removable_apps: false,
            include_hidden_apps: false,
            include_internal_apps: false,
            include_default_apps: false,
            require_container_access: true,
            include_app_group_identifiers: true,
            include_container_paths: true,
        });
        let dict = input.as_dict().unwrap();

        for key in [
            "includeAppClips",
            "includeRemovableApps",
            "includeHiddenApps",
            "includeInternalApps",
            "includeDefaultApps",
        ] {
            assert_eq!(dict[key], XpcValue::Bool(false));
        }
        assert_eq!(dict["requireContainerAccess"], XpcValue::Bool(true));
        assert_eq!(dict["includeAppGroupIdentifiers"], XpcValue::Bool(true));
        assert_eq!(dict["includeContainerPaths"], XpcValue::Bool(true));
    }

    #[test]
    fn stream_app_route_requires_explicit_rsd_advertisement() {
        let stream_feature = FEATURE_STREAM_APPS.to_string();
        let old_feature = FEATURE_LIST_APPS.to_string();

        assert!(!feature_is_advertised(None, FEATURE_STREAM_APPS));
        assert!(feature_is_advertised(
            Some(std::slice::from_ref(&stream_feature)),
            FEATURE_STREAM_APPS
        ));
        assert!(!feature_is_advertised(
            Some(std::slice::from_ref(&old_feature)),
            FEATURE_STREAM_APPS
        ));
        assert!(!feature_is_known_and_missing(None, FEATURE_STREAM_APPS));
        assert!(!feature_is_known_and_missing(
            Some(&[]),
            FEATURE_STREAM_APPS
        ));
        assert!(feature_is_known_and_missing(
            Some(std::slice::from_ref(&old_feature)),
            FEATURE_STREAM_APPS
        ));
    }

    #[test]
    fn legacy_envelope_rejects_modern_stream_features() {
        let error = ensure_modern_stream_mode(CoreDeviceEnvelopeMode::Legacy, FEATURE_STREAM_APPS)
            .expect_err("legacy CoreDevice clients must not silently send modern streams");

        assert!(matches!(
            error,
            AppServiceError::Protocol(message) if message.contains("requires the modern DDI envelope")
        ));
    }

    #[test]
    fn build_list_roots_input_uses_root_point_relative_slash() {
        let input = build_list_roots_input();
        let dict = input.as_dict().unwrap();
        let root_point = dict["rootPoint"].as_dict().unwrap();

        assert_eq!(root_point["relative"].as_str(), Some("/"));
    }

    #[test]
    fn build_spawn_executable_input_matches_reference_shape() {
        let input = build_spawn_executable_input(
            "/usr/bin/log",
            &[
                "stream".to_string(),
                "--style".to_string(),
                "json".to_string(),
            ],
        )
        .unwrap();
        let dict = input.as_dict().unwrap();
        let executable_item = dict["executableItem"].as_dict().unwrap();
        let url = executable_item["url"].as_dict().unwrap();
        let url_payload = url["_0"].as_dict().unwrap();
        let options = dict["options"].as_dict().unwrap();
        let user = options["user"].as_dict().unwrap();

        assert_eq!(url_payload["relative"].as_str(), Some("/usr/bin/log"));
        assert_eq!(
            options["arguments"],
            XpcValue::Array(vec![
                XpcValue::String("stream".into()),
                XpcValue::String("--style".into()),
                XpcValue::String("json".into())
            ])
        );
        assert_eq!(
            options["environmentVariables"],
            XpcValue::Dictionary(IndexMap::new())
        );
        assert_eq!(
            options["standardIOUsesPseudoterminals"],
            XpcValue::Bool(true)
        );
        assert_eq!(options["startStopped"], XpcValue::Bool(false));
        assert_eq!(user["active"], XpcValue::Bool(true));
        assert_eq!(
            dict["standardIOIdentifiers"],
            XpcValue::Dictionary(IndexMap::new())
        );
    }

    #[test]
    fn build_fetch_app_icons_input_matches_reference_shape() {
        let input = build_fetch_app_icons_input("com.example.App", 60.0, 60.0, 3.0, true);
        let dict = input.as_dict().unwrap();

        assert_eq!(dict["bundleIdentifier"].as_str(), Some("com.example.App"));
        assert_eq!(dict["width"], XpcValue::Double(60.0));
        assert_eq!(dict["height"], XpcValue::Double(60.0));
        assert_eq!(dict["scale"], XpcValue::Double(3.0));
        assert_eq!(dict["allowPlaceholder"], XpcValue::Bool(true));
    }

    #[test]
    fn build_monitor_process_termination_input_nests_process_token() {
        let input = build_monitor_process_termination_input(1234).unwrap();
        let dict = input.as_dict().unwrap();
        let process_token = dict["processToken"].as_dict().unwrap();

        assert_eq!(process_token["processIdentifier"], XpcValue::Int64(1234));
    }

    #[test]
    fn build_monitor_process_termination_input_rejects_pid_above_i64_max() {
        let err = build_monitor_process_termination_input(u64::MAX).unwrap_err();

        assert!(
            matches!(err, AppServiceError::Protocol(message) if message.contains("process id exceeds"))
        );
    }

    #[test]
    fn parse_process_reads_executable_url_relative() {
        let process = XpcValue::Dictionary(IndexMap::from([
            ("processIdentifier".to_string(), XpcValue::Int64(77)),
            (
                "executableURL".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "relative".to_string(),
                    XpcValue::String("/usr/libexec/foo".into()),
                )])),
            ),
        ]));

        let parsed = parse_process(&process).unwrap();

        assert_eq!(parsed.pid, 77);
        assert_eq!(parsed.name, "foo");
        assert_eq!(parsed.executable.as_deref(), Some("/usr/libexec/foo"));
    }

    #[test]
    fn parse_apps_reads_coredevice_output_variants() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "apps".to_string(),
                    XpcValue::Array(vec![XpcValue::Dictionary(IndexMap::from([
                        (
                            "bundleIdentifier".to_string(),
                            XpcValue::String("com.example.App".into()),
                        ),
                        (
                            "localizedName".to_string(),
                            XpcValue::String("Example".into()),
                        ),
                        ("isRemovable".to_string(), XpcValue::Bool(true)),
                    ]))]),
                )])),
            )]))),
        };

        let apps = parse_apps(&response).unwrap();

        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].bundle_id, "com.example.App");
        assert_eq!(apps[0].name.as_deref(), Some("Example"));
        assert_eq!(apps[0].is_removable, Some(true));
    }

    #[test]
    fn parse_app_icons_reads_coredevice_output() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([(
                    "icons".to_string(),
                    XpcValue::Array(vec![XpcValue::Dictionary(IndexMap::from([
                        ("width".to_string(), XpcValue::Double(60.0)),
                        ("height".to_string(), XpcValue::Double(60.0)),
                        ("scale".to_string(), XpcValue::Double(3.0)),
                        (
                            "iconData".to_string(),
                            XpcValue::Data(Bytes::from_static(b"png")),
                        ),
                    ]))]),
                )])),
            )]))),
        };

        let icons = parse_app_icons(&response).unwrap();

        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].width, Some(60.0));
        assert_eq!(icons[0].height, Some(60.0));
        assert_eq!(icons[0].scale, Some(3.0));
        assert_eq!(icons[0].data.as_ref(), b"png");
    }

    #[test]
    fn parse_process_termination_reads_enveloped_process_token() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 1,
            body: Some(XpcValue::Dictionary(IndexMap::from([(
                "CoreDevice.output".to_string(),
                XpcValue::Dictionary(IndexMap::from([
                    (
                        "processToken".to_string(),
                        XpcValue::Dictionary(IndexMap::from([(
                            "processIdentifier".to_string(),
                            XpcValue::Int64(1234),
                        )])),
                    ),
                    ("exitStatus".to_string(), XpcValue::Int64(0)),
                    ("reason".to_string(), XpcValue::String("exited".to_string())),
                ])),
            )]))),
        };

        let termination = parse_process_termination(&response).unwrap();

        assert_eq!(termination.pid, Some(1234));
        assert_eq!(termination.exit_status, Some(0));
        assert_eq!(termination.reason.as_deref(), Some("exited"));
    }
}

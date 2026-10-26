//! Explicit, read-only device probes. Each probe owns a fresh session and budget.
//!
//! QueryType/StartSession/GetValue use the existing lockdown wire structures.
//! StartSession error fields need observation before the legacy response decoder
//! requires SessionID/EnableSessionSSL. Only existing local trust is loaded;
//! neither this module nor the source interface offers pairing or tunnel startup.

use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::{anyhow, Result};
use futures_util::future::BoxFuture;
use ios_core::{
    recv_lockdown, send_lockdown, start_service_preserving_errors, wrap_service_tls,
    GetValueRequest, PairRecord, QueryTypeRequest, StartSessionRequest, LOCKDOWN_PORT,
};
use ios_core::{DeviceInfo, ErrorClass, LockdownError, LockdownResponseError, ServiceStream};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::Instant;

use super::Status;

mod manifest;
pub(super) mod rsd;
pub(crate) mod target_manifest;

pub(super) trait Source: Send + Sync {
    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>>>;
    fn pair_record<'a>(&'a self, udid: &'a str) -> BoxFuture<'a, Result<Arc<PairRecord>>>;
    fn port(&self, device_id: u32, port: u16) -> BoxFuture<'_, Result<ServiceStream>>;
    // Offline sources must not inherit the current host's DDI files.
    fn ddi_search_paths(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
}

pub(super) struct NativeSource;

impl Source for NativeSource {
    fn ddi_search_paths(&self) -> Vec<std::path::PathBuf> {
        manifest::search_paths()
    }
    fn devices(&self) -> BoxFuture<'_, Result<Vec<DeviceInfo>>> {
        Box::pin(async {
            Ok(ios_core::MuxClient::connect()
                .await?
                .list_devices()
                .await?
                .into_iter()
                .map(|device| DeviceInfo {
                    udid: device.serial_number,
                    device_id: device.device_id,
                    connection_type: device.connection_type,
                    product_id: device.product_id,
                })
                .collect())
        })
    }

    fn pair_record<'a>(&'a self, udid: &'a str) -> BoxFuture<'a, Result<Arc<PairRecord>>> {
        Box::pin(async move {
            let udid = udid.to_owned();
            // PairRecord bounds the serialized file to 1 MiB and wipes keys.
            // A running filesystem read cannot be forcibly cancelled; it never
            // opens a device or publishes a result after this future is dropped.
            let record = tokio::task::spawn_blocking(move || PairRecord::load(&udid)).await??;
            Ok(Arc::new(record))
        })
    }

    fn port(&self, device_id: u32, port: u16) -> BoxFuture<'_, Result<ServiceStream>> {
        Box::pin(async move {
            let stream = ios_core::MuxClient::connect()
                .await?
                .connect_to_port(device_id, port)
                .await?;
            Ok(Box::new(stream) as ServiceStream)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Available,
    NotProbed,
    Unreachable,
    PermissionDenied,
    ServiceUnavailable,
    MetadataMissing,
    Failed,
    Unsupported,
}

#[derive(Serialize)]
pub(super) struct Check {
    pub(super) id: &'static str,
    pub(super) status: Status,
    pub(super) state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) value: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error_class: Option<ErrorClass>,
    pub(super) detail: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) hint: Option<&'static str>,
    // Preserve owner-acquired native errors for assessment/review. Neither JSON
    // nor Debug exposes an arbitrary response, identity, credential or path.
    #[serde(skip)]
    error: Option<anyhow::Error>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rsd: Option<rsd::Metadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ddi_manifests: Option<manifest::Metadata>,
}

impl fmt::Debug for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceCheck")
            .field("id", &self.id)
            .field("status", &self.status)
            .field("state", &self.state)
            .field("value", &self.value)
            .field("error_class", &self.error_class)
            .field("error_retained", &self.error.is_some())
            .finish_non_exhaustive()
    }
}

impl Check {
    fn observed(probe: Probe, value: Option<bool>) -> Self {
        Self {
            id: probe.id(),
            status: match value {
                Some(true) => Status::Ok,
                Some(false) => Status::Warning,
                None => Status::Unknown,
            },
            state: if value.is_some() { State::Available } else { State::MetadataMissing },
            value,
            error_class: None,
            detail: match (probe, value) {
                (Probe::Session, Some(true)) => "An existing-trust session completed a read-only ProductType query. This does not establish every service's readiness.",
                (Probe::DeveloperMode, Some(true)) => "The image mounter explicitly reported Developer Mode enabled.",
                (Probe::DeveloperMode, Some(false)) => "The image mounter explicitly reported Developer Mode disabled.",
                (Probe::DeveloperMode, None) => "The image mounter omitted DeveloperModeStatus; enabled/disabled is unknown.",
                (Probe::DdiSignatures, Some(true)) => "LookupImage returned a nonempty Developer or Personalized signature. Mounted state and usability were not established.",
                (Probe::DdiSignatures, Some(false)) => "Both LookupImage replies explicitly contained no nonempty signature. A personalized image may still be mounted; usability was not established.",
                (Probe::DdiSignatures, None) => "At least one LookupImage reply omitted ImageSignature; signature presence is unknown.",
                (Probe::DdiMounted, Some(true)) => "CopyDevices explicitly listed a Developer or Personalized DiskImageType. This establishes reported mounted state; target service usability and Apple signature verification remain unproven.",
                (Probe::DdiMounted, Some(false)) => "CopyDevices returned a complete EntryList without Developer or Personalized DiskImageType; no developer image was reported mounted.",
                (Probe::DdiMounted, None) => "CopyDevices omitted EntryList or an entry omitted DiskImageType. Mounted state is unknown; signatures and local files do not replace this observation.",
                (Probe::DdiUsable, Some(true)) => "The selected existing-trust developer service completed DeviceInfo runningProcesses with a valid DTX reply. This proves that specific read-only operation worked; it does not establish Apple signature validity or every developer service.",
                (Probe::DdiUsable, _) => "The target developer service did not provide a validated read-only operation result; DDI usability remains unknown.",
                (Probe::DdiManifests, Some(true)) => "The selected target returned nonempty personalization manifests for every reported DeveloperDiskImage signature. These opaque bytes are a read-only device observation; Apple cryptographic validity and local BuildManifest matching remain unproven.",
                (Probe::DdiManifests, Some(false)) => "CopyDevices reported no mounted personalized DeveloperDiskImage entry. No personalization manifest query was sent; other DDI evidence remains independent.",
                (Probe::DdiManifests, None) => "Required mounted developer image or personalization manifest metadata was omitted or empty. No manifest validity or negative service capability was inferred.",
                (Probe::Session, _) => "The read-only session did not provide ProductType metadata.",
            },
            hint: None,
            error: None,
            rsd: None,
            ddi_manifests: None,
        }
    }

    fn failure(probe: Probe, error: anyhow::Error) -> Self {
        let class = error_class(&error);
        let state = if error.is::<MetadataMissing>() || class == ErrorClass::NotFound {
            State::MetadataMissing
        } else {
            match class {
                ErrorClass::Timeout
                | ErrorClass::Disconnected
                | ErrorClass::DeviceNotFound
                | ErrorClass::Io => State::Unreachable,
                ErrorClass::PermissionDenied => State::PermissionDenied,
                ErrorClass::ServiceMissing => State::ServiceUnavailable,
                ErrorClass::Unsupported => State::Unsupported,
                _ => State::Failed,
            }
        };
        Self {
            id: probe.id(),
            status: if matches!(state, State::MetadataMissing | State::Unsupported) { Status::Unknown } else { Status::Failed },
            state,
            value: None,
            error_class: Some(class),
            detail: match state {
                State::Unreachable => "The selected device probe could not reach its transport or required local trust data.",
                State::PermissionDenied => "The host or an explicit lockdown response denied access to the selected device probe.",
                State::ServiceUnavailable => "Lockdown explicitly reported that the requested service is unavailable.",
                State::MetadataMissing => "Required local trust or response metadata was absent; no negative capability value was inferred.",
                State::Unsupported => "The selected developer operation or negotiated transport has no verified adapter. Its DDI usability is unknown; no tunnel, mount or pairing was attempted.",
                _ => "The selected device probe failed; its typed error category is reported without private response data.",
            },
            hint: match state {
                State::PermissionDenied | State::MetadataMissing => Some("Check the existing host pairing record and unlock the device. Doctor does not create pairing or change settings."),
                State::Unreachable => Some("Check the selected device's USB/network connection and usbmuxd endpoint; increase --timeout if appropriate."),
                _ => None,
            },
            error: Some(error),
            rsd: None,
            ddi_manifests: None,
        }
    }

    fn not_probed(id: &'static str, detail: &'static str, hint: &'static str) -> Self {
        Self {
            id,
            status: Status::Unknown,
            state: State::NotProbed,
            value: None,
            error_class: None,
            detail,
            hint: Some(hint),
            error: None,
            rsd: None,
            ddi_manifests: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct Assessment {
    target: String,
    transport: &'static str,
    pub(super) checks: Vec<Check>,
}

impl Assessment {
    pub(super) fn failed(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == Status::Failed)
    }

    /// Preserve the first acquired failed device probe's original cause after
    /// the complete safe report has been emitted. Unknown metadata is not a
    /// substitute for the failure that caused the command to fail.
    pub(super) fn take_first_failed_error(&mut self) -> Option<anyhow::Error> {
        self.checks
            .iter_mut()
            .filter(|check| check.status == Status::Failed)
            .find_map(|check| {
                check.error.take().map(|error| {
                    error.context(super::DoctorProbeClass(
                        check.error_class.unwrap_or(ErrorClass::Unknown),
                    ))
                })
            })
    }
}

pub(super) fn validate_target(udid: &str) -> Result<()> {
    if udid.is_empty()
        || udid.len() > 256
        || !udid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,
            "doctor requires an explicit device identifier containing only ASCII letters, digits or hyphens (1..=256 bytes)").into());
    }
    Ok(())
}

pub(super) async fn inspect(source: &dyn Source, udid: &str, timeout: Duration) -> Assessment {
    inspect_base(source, udid, timeout, false).await
}

async fn inspect_base(
    source: &dyn Source,
    udid: &str,
    timeout: Duration,
    existing_rsd: bool,
) -> Assessment {
    let mut checks = Vec::with_capacity(8);
    for probe in [Probe::Session, Probe::DeveloperMode, Probe::DdiSignatures] {
        checks.push(probe_once(source, udid, probe, timeout).await);
    }
    checks.push(Check::not_probed("rsd_services",
        "No explicit existing RSD route was provided. Service discovery was not attempted and no tunnel was activated.",
        "Inspect services through an explicitly authorized existing RSD route; this doctor invocation only uses existing lockdown trust."));
    checks.push(Check::not_probed("device_route",
        "USB/network enumeration and lockdown probes do not establish an RSD route or remoted coexistence.",
        "Provide platform and existing tunnel/RSD route evidence before claiming route readiness."));
    checks.push(probe_once(source, udid, Probe::DdiMounted, timeout).await);
    checks.push(if existing_rsd {
        Check::not_probed(
            "ddi_usable",
            "The explicit existing RSD developer operation is assessed separately.",
            "No new tunnel or pairing is started.",
        )
    } else {
        probe_once(source, udid, Probe::DdiUsable, timeout).await
    });
    checks.push(manifest::inspect(source, udid, timeout).await);
    Assessment {
        target: ios_core::redaction::diagnostic_id(udid).to_string(),
        transport: "usbmux_existing_trust",
        checks,
    }
}

pub(super) async fn inspect_with_rsd(
    source: &dyn Source,
    udid: &str,
    timeout: Duration,
    request: &rsd::Request,
) -> Assessment {
    // Preserve the three existing-trust checks and their order. A failure in
    // that transport does not skip the independently selected existing route.
    let mut assessment = inspect_base(source, udid, timeout, true).await;
    let [services, route] = rsd::inspect(request, udid, timeout).await;
    assessment.checks[3] = services;
    assessment.checks[4] = route;
    assessment.checks[6] = rsd::developer_probe(request, udid, timeout).await;
    assessment
}

#[derive(Clone, Copy)]
enum Probe {
    Session,
    DeveloperMode,
    DdiSignatures,
    DdiMounted,
    DdiUsable,
    DdiManifests,
}

impl Probe {
    fn id(self) -> &'static str {
        match self {
            Self::Session => "trust_session",
            Self::DeveloperMode => "developer_mode",
            Self::DdiSignatures => "ddi_signatures",
            Self::DdiMounted => "ddi_mounted",
            Self::DdiUsable => "ddi_usable",
            Self::DdiManifests => "ddi_manifests",
        }
    }
}

#[derive(Debug)]
struct MetadataMissing;
impl fmt::Display for MetadataMissing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("required doctor probe metadata missing")
    }
}
impl Error for MetadataMissing {}

struct RemoteResponseError {
    operation: &'static str,
    code: String,
}
impl fmt::Display for RemoteResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "doctor lockdown {} rejected ({})",
            self.operation,
            remote_class(self.operation, &self.code)
        )
    }
}
impl fmt::Debug for RemoteResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl Error for RemoteResponseError {}

fn remote_class(operation: &str, code: &str) -> ErrorClass {
    match (operation, code) {
        (
            "StartSession",
            "InvalidHostID" | "UserDeniedPairing" | "PairingDialogResponsePending",
        ) => ErrorClass::PermissionDenied,
        ("StartSession" | "GetValue" | "StartService", "PasswordProtected")
        | ("GetValue", "GetProhibited") => ErrorClass::PermissionDenied,
        ("GetValue", "MissingValue") => ErrorClass::NotFound,
        ("StartService", "InvalidService") => ErrorClass::ServiceMissing,
        _ => ErrorClass::ServiceFailure,
    }
}

fn error_class(error: &anyhow::Error) -> ErrorClass {
    if let Some(error) = error.downcast_ref::<RemoteResponseError>() {
        return remote_class(error.operation, &error.code);
    }
    if error.is::<MetadataMissing>() {
        return ErrorClass::NotFound;
    }
    if let Some(error) = LockdownResponseError::in_error_chain(error.as_ref()) {
        return remote_class(error.operation(), error.code());
    }
    ios_core::classify_error(error.as_ref())
}

fn check_response(response: &plist::Dictionary, operation: &'static str) -> Result<()> {
    if let Some(request) = response.get("Request") {
        if request.as_string() != Some(operation) {
            return Err(LockdownError::Protocol(
                "doctor received an unrelated lockdown response".into(),
            )
            .into());
        }
    }
    if let Some(error) = response.get("Error") {
        let code = error.as_string().ok_or_else(|| {
            LockdownError::Protocol("doctor lockdown Error was not a string".into())
        })?;
        return Err(RemoteResponseError {
            operation,
            code: code.to_owned(),
        }
        .into());
    }
    Ok(())
}

struct Clock {
    deadline: Instant,
}

// The total's outer poll fence also needs a fence at every socket poll: a
// ready send may cross the deadline before the same future reaches recv.
// No expired stream is reused, and errors acquired in a poll retain priority.
struct DeadlineStream {
    stream: ServiceStream,
    deadline: Instant,
}

impl AsyncRead for DeadlineStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}

impl AsyncWrite for DeadlineStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if Instant::now() >= self.deadline {
            return Poll::Pending;
        }
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
#[derive(Debug)]
struct DeadlineReached;
impl fmt::Display for DeadlineReached {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("doctor probe deadline reached")
    }
}
impl Error for DeadlineReached {}

impl Clock {
    fn guard(&self, stream: ServiceStream) -> ServiceStream {
        Box::new(DeadlineStream {
            stream,
            deadline: self.deadline,
        })
    }
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(DeadlineReached.into())
        } else {
            Ok(())
        }
    }

    async fn run<T>(&self, operation: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::pin!(operation);
        let guarded = std::future::poll_fn(|cx| {
            if self.check().is_err() {
                return Poll::Pending;
            }
            match operation.as_mut().poll(cx) {
                Poll::Ready(Err(error)) if error.is::<DeadlineReached>() => Poll::Pending,
                Poll::Ready(Ok(_)) if self.check().is_err() => Poll::Pending,
                result => result,
            }
        });
        tokio::time::timeout_at(self.deadline, guarded)
            .await
            .map_err(|source| std::io::Error::new(std::io::ErrorKind::TimedOut, source))?
    }
}

async fn probe_once(source: &dyn Source, udid: &str, probe: Probe, timeout: Duration) -> Check {
    let result = async {
        validate_target(udid)?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "doctor probe deadline overflow",
            )
        })?;
        let clock = Clock { deadline };
        clock.run(probe_value(source, udid, probe, &clock)).await
    }
    .await;
    match result {
        Ok(value) => Check::observed(probe, value),
        Err(error) => Check::failure(probe, error),
    }
}

async fn session(
    source: &dyn Source,
    udid: &str,
    clock: &Clock,
) -> Result<(ServiceStream, Arc<PairRecord>, u32)> {
    clock.check()?;
    let devices = source.devices().await?;
    let device = devices
        .iter()
        .find(|device| device.udid == udid && device.connection_type == "USB")
        .or_else(|| devices.iter().rev().find(|device| device.udid == udid))
        .ok_or_else(|| ios_core::CoreError::DeviceNotFound(udid.into()))?;
    let device_id = device.device_id;
    drop(devices);
    clock.check()?;
    let pair_record = source.pair_record(udid).await?;
    clock.check()?;
    let mut stream = source.port(device_id, LOCKDOWN_PORT).await?;
    clock.check()?;
    stream = clock.guard(stream);
    send_lockdown(
        &mut stream,
        &QueryTypeRequest {
            label: "ios-rs",
            request: "QueryType",
        },
    )
    .await?;
    clock.check()?;
    let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
    check_response(&response, "QueryType")?;
    if response.get("Type").and_then(plist::Value::as_string) != Some("com.apple.mobile.lockdown") {
        return Err(
            LockdownError::Protocol("doctor target did not identify as lockdown".into()).into(),
        );
    }
    clock.check()?;
    send_lockdown(
        &mut stream,
        &StartSessionRequest {
            label: "ios-rs",
            protocol_version: "2",
            request: "StartSession",
            host_id: pair_record.host_id.clone(),
            system_buid: pair_record.system_buid.clone(),
        },
    )
    .await?;
    clock.check()?;
    let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
    check_response(&response, "StartSession")?;
    match response.get("SessionID") {
        None => return Err(MetadataMissing.into()),
        Some(plist::Value::String(value)) if !value.is_empty() => {}
        Some(_) => {
            return Err(
                LockdownError::Protocol("doctor StartSession SessionID was invalid".into()).into(),
            )
        }
    }
    let use_ssl = match response.get("EnableSessionSSL") {
        Some(plist::Value::Boolean(value)) => *value,
        None => return Err(MetadataMissing.into()),
        Some(_) => {
            return Err(LockdownError::Protocol(
                "doctor StartSession SSL status was not boolean".into(),
            )
            .into())
        }
    };
    clock.check()?;
    let stream = if use_ssl {
        Box::new(wrap_service_tls(stream, &pair_record).await?) as ServiceStream
    } else {
        stream
    };
    Ok((stream, pair_record, device_id))
}

async fn probe_value(
    source: &dyn Source,
    udid: &str,
    probe: Probe,
    clock: &Clock,
) -> Result<Option<bool>> {
    let (mut stream, pair_record, device_id) = session(source, udid, clock).await?;
    if matches!(probe, Probe::Session) {
        clock.check()?;
        send_lockdown(
            &mut stream,
            &GetValueRequest {
                label: "ios-rs",
                request: "GetValue",
                domain: None,
                key: Some("ProductType"),
            },
        )
        .await?;
        clock.check()?;
        let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
        check_response(&response, "GetValue")?;
        return match response.get("Value") {
            None => Ok(None),
            Some(plist::Value::String(value)) if !value.is_empty() => Ok(Some(true)),
            Some(_) => Err(LockdownError::Protocol(
                "doctor ProductType was not a nonempty string".into(),
            )
            .into()),
        };
    }
    let service_name = if matches!(probe, Probe::DdiUsable) {
        clock.check()?;
        send_lockdown(
            &mut stream,
            &GetValueRequest {
                label: "ios-rs",
                request: "GetValue",
                domain: None,
                key: Some("ProductVersion"),
            },
        )
        .await?;
        clock.check()?;
        let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
        check_response(&response, "GetValue")?;
        let version = response
            .get("Value")
            .ok_or(MetadataMissing)?
            .as_string()
            .ok_or_else(|| {
                LockdownError::Protocol("doctor ProductVersion was not a string".into())
            })?;
        let major = version
            .split('.')
            .next()
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                LockdownError::Protocol("doctor ProductVersion major was invalid".into())
            })?;
        match major {
            0 => return Err(LockdownError::Protocol("doctor ProductVersion major was zero".into()).into()),
            1..=13 => ios_core::instruments::SERVICE_LEGACY,
            14..=16 => ios_core::instruments::SERVICE_IOS14,
            _ => return Err(ios_core::CoreError::Unsupported("doctor requires an explicit checked RSD developer-operation adapter for this target".into()).into()),
        }
    } else {
        ios_core::imagemounter::protocol::SERVICE_NAME
    };
    clock.check()?;
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (port, use_ssl) =
        start_service_preserving_errors(&mut reader, &mut writer, service_name).await?;
    drop((reader, writer));
    if port == 0 {
        return Err(
            LockdownError::Protocol("doctor StartService returned port zero".into()).into(),
        );
    }
    clock.check()?;
    let stream = source.port(device_id, port).await?;
    clock.check()?;
    let stream = clock.guard(stream);
    let stream = if use_ssl {
        clock.check()?;
        let tls = wrap_service_tls(stream, &pair_record).await?;
        if service_name == ios_core::instruments::SERVICE_LEGACY {
            if tls.get_ref().1.protocol_version() != Some(rustls::ProtocolVersion::TLSv1_2) {
                return Err(ios_core::CoreError::Unsupported(
                    "legacy handshake-only developer probe supports negotiated TLS 1.2; the TLS 1.3 raw transition is unverified".into(),
                ).into());
            }
            Box::new(ios_core::strip_service_tls(tls)?) as ServiceStream
        } else {
            Box::new(tls) as ServiceStream
        }
    } else {
        stream
    };
    if matches!(probe, Probe::DdiUsable) {
        let mut device_info =
            Box::pin(ios_core::instruments::DeviceInfoClient::connect(stream)).await?;
        clock.check()?;
        // RunningProcesses has a checked array/dictionary decoder. The older
        // attribute helpers default unrelated replies to an empty success.
        let _processes = device_info.running_processes().await?;
        return Ok(Some(true));
    }
    let mut stream = stream;
    if matches!(probe, Probe::DdiMounted) {
        // The legacy copy_devices helper defaults absent EntryList to empty;
        // preserve Unknown in Doctor's read-only protocol observation.
        send_lockdown(
            &mut stream,
            &plist::Dictionary::from_iter([(
                "Command",
                plist::Value::String("CopyDevices".into()),
            )]),
        )
        .await?;
        clock.check()?;
        let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
        check_response(&response, "CopyDevices")?;
        return mounted_value(&response);
    }
    let mut client = ios_core::imagemounter::ImageMounterClient::new(stream);
    clock.check()?;
    match probe {
        Probe::DeveloperMode => Ok(client.query_developer_mode_status_checked().await?),
        Probe::DdiSignatures => {
            let developer = client.lookup_image_signatures_checked("Developer").await?;
            clock.check()?;
            let personalized = client
                .lookup_image_signatures_checked("Personalized")
                .await?;
            let present = |signatures: &Option<Vec<Vec<u8>>>| {
                signatures
                    .as_ref()
                    .is_some_and(|entries| entries.iter().any(|signature| !signature.is_empty()))
            };
            if present(&developer) || present(&personalized) {
                Ok(Some(true))
            } else if developer.is_some() && personalized.is_some() {
                Ok(Some(false))
            } else {
                Ok(None)
            }
        }
        Probe::Session | Probe::DdiMounted | Probe::DdiUsable | Probe::DdiManifests => {
            Err(anyhow!("unreachable doctor probe selection"))
        }
    }
}

fn mounted_value(response: &plist::Dictionary) -> Result<Option<bool>> {
    let Some(entries) = response.get("EntryList") else {
        return Ok(None);
    };
    let entries = entries.as_array().ok_or_else(|| {
        LockdownError::Protocol("doctor CopyDevices EntryList was not an array".into())
    })?;
    let mut missing = false;
    let mut mounted = false;
    for entry in entries {
        let dictionary = entry.as_dictionary().ok_or_else(|| {
            LockdownError::Protocol("doctor CopyDevices entry was not a dictionary".into())
        })?;
        match dictionary.get("DiskImageType") {
            Some(plist::Value::String(image_type)) => {
                mounted |= matches!(image_type.as_str(), "Developer" | "Personalized")
            }
            None => missing = true,
            Some(_) => {
                return Err(LockdownError::Protocol(
                    "doctor CopyDevices DiskImageType was not a string".into(),
                )
                .into())
            }
        }
    }
    Ok(if mounted {
        Some(true)
    } else if missing {
        None
    } else {
        Some(false)
    })
}

#[cfg(test)]
mod tests;

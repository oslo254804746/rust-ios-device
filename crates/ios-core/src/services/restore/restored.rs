//! Bounded restored plist transport, distinct from normal-mode RestoreRemoteServices.
//!
//! Wire evidence: pymobiledevice3 03ba2270, restore/restored_client.py and
//! restore/restore.py::firmware_response_messages. This module does not boot a
//! restore ramdisk, implement FDR, personalize IMG4, or claim a full update.
//! Synchronous SystemImageData can use the bounded ASR data-port transport.

use std::future::Future;
use std::time::Duration;

use plist::{Dictionary, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

use super::state_machine::{
    failed_execution, RestoreExecutionReport, RestoreState, RestoreStateMachine,
};
use super::{error_mapping::native_error, RestoreError, RestoreLifecycleEvent};
use crate::services::plist_frame::{read_plist_frame, write_xml_plist_frame};
use crate::session::OperationCancel;

const MAX_FRAME: usize = 8 * 1024 * 1024;
const MAX_TRANSFER: u64 = 1024 * 1024 * 1024;

/// An already connected restored service on usbmux port 62078. The caller must
/// establish and own the actual recovery-mode transport, not a normal RSD stream.
pub struct RestoredClient<S> {
    stream: S,
    deadline: Instant,
    cancel: OperationCancel,
    version: Value,
    hardware: Dictionary,
    started: bool,
    terminal: bool,
    written: u64,
}

async fn bounded<T>(
    deadline: Instant,
    cancel: &OperationCancel,
    work: impl Future<Output = Result<T, RestoreError>>,
) -> Result<T, RestoreError> {
    if cancel.is_cancelled() {
        return Err(RestoreError::Io(std::io::Error::other(
            crate::OperationCancelled::new(),
        )));
    }
    if Instant::now() >= deadline {
        return Err(RestoreError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "restored operation deadline",
        )));
    }
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(RestoreError::Io(std::io::Error::other(crate::OperationCancelled::new()))),
        result = tokio::time::timeout_at(deadline, work) => result.map_err(|e|
            native_error(std::io::ErrorKind::TimedOut, "restored operation deadline", e))?,
    };
    if result.is_ok() && cancel.is_cancelled() {
        return Err(RestoreError::Io(std::io::Error::other(
            crate::OperationCancelled::new(),
        )));
    }
    if result.is_ok() && Instant::now() >= deadline {
        return Err(RestoreError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "restored operation deadline",
        )));
    }
    result
}

fn request(name: &str) -> Dictionary {
    Dictionary::from_iter([
        ("Request".to_owned(), Value::String(name.into())),
        ("Label".to_owned(), Value::String("rust-ios-device".into())),
    ])
}

impl<S: AsyncRead + AsyncWrite + Unpin> RestoredClient<S> {
    /// QueryType and HardwareInfo share the single caller deadline. ECID is
    /// matched before any mutation. Unknown peers and missing version fail.
    pub async fn connect(
        stream: S,
        expected_ecid: u64,
        total: Duration,
        cancel: OperationCancel,
    ) -> Result<Self, RestoreError> {
        let deadline = Instant::now()
            .checked_add(total)
            .filter(|_| !total.is_zero())
            .ok_or_else(|| RestoreError::Protocol("invalid restored deadline".into()))?;
        let mut client = Self {
            stream,
            deadline,
            cancel,
            version: Value::String(String::new()),
            hardware: Dictionary::new(),
            started: false,
            terminal: false,
            written: 0,
        };
        let query = client.exchange(request("QueryType")).await?;
        if query.get("Type").and_then(Value::as_string) != Some("com.apple.mobile.restored") {
            return Err(RestoreError::Protocol("peer is not restored".into()));
        }
        client.version = query
            .get("RestoreProtocolVersion")
            .filter(|v| {
                v.as_unsigned_integer().is_some() || v.as_string().is_some_and(|s| !s.is_empty())
            })
            .ok_or_else(|| RestoreError::Protocol("restored missing protocol version".into()))?
            .clone();
        let mut query = request("QueryValue");
        query.insert("QueryKey".to_owned(), Value::String("HardwareInfo".into()));
        let response = client.exchange(query).await?;
        client.hardware = response
            .get("HardwareInfo")
            .and_then(Value::as_dictionary)
            .ok_or_else(|| RestoreError::Protocol("restored missing HardwareInfo".into()))?
            .clone();
        if client
            .hardware
            .get("UniqueChipID")
            .and_then(Value::as_unsigned_integer)
            != Some(expected_ecid)
        {
            return Err(RestoreError::Protocol(
                "restored ECID does not match selected device".into(),
            ));
        }
        Ok(client)
    }

    pub fn hardware_info(&self) -> &Dictionary {
        &self.hardware
    }
    pub fn protocol_version(&self) -> &Value {
        &self.version
    }
    /// Bytes successfully flushed to the local transport, not device flash or
    /// signature verification. A failed write never increments this counter.
    pub fn firmware_bytes_written(&self) -> u64 {
        self.written
    }

    fn live(&self) -> Result<(), RestoreError> {
        if self.terminal {
            Err(RestoreError::Protocol(
                "restored transport is terminal".into(),
            ))
        } else {
            Ok(())
        }
    }

    async fn send(&mut self, body: &Dictionary) -> Result<(), RestoreError> {
        self.live()?;
        self.terminal = true;
        let result = bounded(self.deadline, &self.cancel, async {
            write_xml_plist_frame(&mut self.stream, body, MAX_FRAME)
                .await
                .map_err(|e| native_error(std::io::ErrorKind::Other, "write restored plist", e))
        })
        .await;
        if result.is_ok() {
            self.terminal = false;
        }
        result
    }

    async fn receive(&mut self) -> Result<Dictionary, RestoreError> {
        self.live()?;
        self.terminal = true;
        let result = bounded(self.deadline, &self.cancel, async {
            let value: Value = read_plist_frame(&mut self.stream, MAX_FRAME)
                .await
                .map_err(|e| native_error(std::io::ErrorKind::Other, "read restored plist", e))?;
            let dict = value.into_dictionary().ok_or_else(|| {
                RestoreError::Protocol("restored response is not a dictionary".into())
            })?;
            if dict.contains_key("Error") {
                return Err(RestoreError::Protocol("restored returned an error".into()));
            }
            Ok(dict)
        })
        .await;
        if result.is_ok() {
            self.terminal = false;
        }
        result
    }

    async fn exchange(&mut self, request: Dictionary) -> Result<Dictionary, RestoreError> {
        self.send(&request).await?;
        self.receive().await
    }

    /// Send the actual StartRestore message, only for explicit Update options
    /// and an expected protocol version. Caller must supply proven options for
    /// its machine/build; this API does not manufacture a compatible plan.
    pub async fn start_update(
        &mut self,
        expected_version: &Value,
        options: Dictionary,
    ) -> Result<(), RestoreError> {
        self.live()?;
        if self.started
            || expected_version != &self.version
            || options.get("RestoreBehavior").and_then(Value::as_string) != Some("Update")
        {
            return Err(RestoreError::Protocol(
                "invalid or unverified restored Update plan".into(),
            ));
        }
        let mut body = request("StartRestore");
        body.insert("RestoreProtocolVersion".to_owned(), self.version.clone());
        body.insert("RestoreOptions".to_owned(), Value::Dictionary(options));
        self.send(&body).await?;
        self.started = true;
        Ok(())
    }

    /// Receive an actual lifecycle message after StartRestore. A success status
    /// without a StartRestore cannot be accepted through this API.
    pub async fn next_message(&mut self) -> Result<Dictionary, RestoreError> {
        if !self.started {
            return Err(RestoreError::Protocol(
                "StartRestore has not been sent".into(),
            ));
        }
        self.receive().await
    }

    /// Serve a real SystemImageData request through the separately selected ASR
    /// port. `image` must be the selected identity's OS member extracted to a
    /// seekable reader, never a preflight dictionary. Connect and transfer share
    /// this restore's total deadline and cancellation token.
    pub async fn send_system_image<R, DataStream, F, Fut>(
        &mut self,
        incoming: &Dictionary,
        image: &mut R,
        connect_port: F,
    ) -> Result<super::asr::AsrTransferReceipt, RestoreError>
    where
        R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
        DataStream: AsyncRead + AsyncWrite + Unpin,
        F: FnOnce(u16) -> Fut,
        Fut: Future<Output = Result<DataStream, RestoreError>>,
    {
        self.live()?;
        if !self.started
            || incoming.get("MsgType").and_then(Value::as_string) != Some("DataRequestMsg")
            || incoming.get("DataType").and_then(Value::as_string) != Some("SystemImageData")
        {
            return Err(RestoreError::Protocol(
                "unsupported ASR system-image route".into(),
            ));
        }
        let port = match incoming.get("DataPort") {
            None => super::asr::DEFAULT_PORT,
            Some(value) => value
                .as_unsigned_integer()
                .and_then(|value| u16::try_from(value).ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| RestoreError::Protocol("invalid ASR DataPort".into()))?,
        };
        // Once this request is being served, dropping the caller must not make
        // an incomplete ASR transfer look reusable on the control connection.
        self.terminal = true;
        let result = bounded(self.deadline, &self.cancel, async {
            let stream = connect_port(port).await?;
            super::asr::send_image_until(stream, image, self.deadline, self.cancel.clone()).await
        })
        .await;
        if result.is_ok() {
            self.terminal = false;
        }
        result
    }

    /// Respond to a correlated synchronous FirmwareUpdaterData request on the
    /// control transport. DataPort and AsyncDataRequest need a separate owner
    /// and are explicitly rejected. Tickets/header must already be personalized.
    pub async fn send_firmware_response(
        &mut self,
        incoming: &Dictionary,
        mut header: Dictionary,
        firmware: &[u8],
    ) -> Result<(), RestoreError> {
        self.live()?;
        if !self.started
            || incoming.get("MsgType").and_then(Value::as_string) != Some("DataRequestMsg")
            || incoming.get("DataType").and_then(Value::as_string) != Some("FirmwareUpdaterData")
            || incoming.contains_key("DataPort")
        {
            return Err(RestoreError::Protocol(
                "unsupported restored firmware response route".into(),
            ));
        }
        let total = self
            .written
            .checked_add(firmware.len() as u64)
            .filter(|n| *n <= MAX_TRANSFER)
            .ok_or_else(|| {
                RestoreError::Protocol("restored firmware byte budget exceeded".into())
            })?;
        header.remove("FirmwareData");
        let chunk = incoming
            .get("Arguments")
            .and_then(Value::as_dictionary)
            .and_then(|a| a.get("DataChunkSize"))
            .and_then(Value::as_unsigned_integer);
        if let Some(chunk) = chunk.filter(|n| *n > 0) {
            let chunk = chunk.min(256 * 1024) as usize;
            self.send(&Dictionary::from_iter([
                ("FirmwareResponseData".to_owned(), Value::Dictionary(header)),
                (
                    "DataSize".to_owned(),
                    Value::Integer((firmware.len() as u64).into()),
                ),
            ]))
            .await?;
            for bytes in firmware.chunks(chunk) {
                self.send(&Dictionary::from_iter([(
                    "FirmwareResponseData".to_owned(),
                    Value::Data(bytes.to_vec()),
                )]))
                .await?;
                self.written += bytes.len() as u64;
            }
            self.send(&Dictionary::from_iter([
                ("FirmwareResponseData".to_owned(), Value::Data(Vec::new())),
                ("DataDone".to_owned(), Value::Boolean(true)),
            ]))
            .await?;
        } else {
            header.insert("FirmwareData".to_owned(), Value::Data(firmware.to_vec()));
            self.send(&Dictionary::from_iter([(
                "FirmwareResponseData".to_owned(),
                Value::Dictionary(header),
            )]))
            .await?;
            self.written = total;
        }
        Ok(())
    }

    /// Send a plist dictionary response on the restored control stream and count its encoded
    /// payload bytes toward `firmware_bytes_written`.
    pub async fn send_plist_response(
        &mut self,
        body: &Dictionary,
        counted_bytes: u64,
    ) -> Result<(), RestoreError> {
        self.live()?;
        if !self.started {
            return Err(RestoreError::Protocol(
                "StartRestore has not been sent".into(),
            ));
        }
        let total = self
            .written
            .checked_add(counted_bytes)
            .filter(|n| *n <= MAX_TRANSFER)
            .ok_or_else(|| {
                RestoreError::Protocol("restored firmware byte budget exceeded".into())
            })?;
        self.send(body).await?;
        self.written = total;
        Ok(())
    }

    /// Serve a synchronous or asynchronous SystemImageData / RecoveryOSASRImage request through
    /// the separately selected ASR port.
    pub async fn send_system_image_any_request<R, DataStream, F, Fut>(
        &mut self,
        incoming: &Dictionary,
        image: &mut R,
        connect_port: F,
    ) -> Result<super::asr::AsrTransferReceipt, RestoreError>
    where
        R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
        DataStream: AsyncRead + AsyncWrite + Unpin,
        F: FnOnce(u16) -> Fut,
        Fut: Future<Output = Result<DataStream, RestoreError>>,
    {
        self.live()?;
        let msg_type = incoming.get("MsgType").and_then(Value::as_string);
        let data_type = incoming.get("DataType").and_then(Value::as_string);
        if !self.started
            || !matches!(msg_type, Some("DataRequestMsg" | "AsyncDataRequestMsg"))
            || !matches!(data_type, Some("SystemImageData" | "RecoveryOSASRImage"))
        {
            return Err(RestoreError::Protocol(
                "unsupported ASR system-image route".into(),
            ));
        }
        let port = match incoming.get("DataPort") {
            None => super::asr::DEFAULT_PORT,
            Some(value) => value
                .as_unsigned_integer()
                .and_then(|value| u16::try_from(value).ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| RestoreError::Protocol("invalid ASR DataPort".into()))?,
        };
        self.terminal = true;
        let result = bounded(self.deadline, &self.cancel, async {
            let stream = connect_port(port).await?;
            super::asr::send_image_until(stream, image, self.deadline, self.cancel.clone()).await
        })
        .await;
        if result.is_ok() {
            self.terminal = false;
        }
        result
    }
}

/// Convert an incoming plist dictionary from restored into a typed lifecycle event.
pub fn restore_event_from_plist(dict: &Dictionary) -> RestoreLifecycleEvent {
    let msg_type = dict.get("MsgType").and_then(Value::as_string);
    match msg_type {
        Some("ProgressMsg") => RestoreLifecycleEvent::Progress {
            operation: dict
                .get("Operation")
                .and_then(Value::as_string)
                .map(ToString::to_string),
            progress: dict.get("Progress").and_then(Value::as_unsigned_integer),
        },
        Some("StatusMsg") => {
            let Some(code) = dict.get("Status").and_then(Value::as_unsigned_integer) else {
                return RestoreLifecycleEvent::Unknown {
                    msg_type: Some("MalformedStatusMsg".into()),
                    raw: indexmap::IndexMap::new(),
                };
            };
            RestoreLifecycleEvent::Status {
                code,
                message: super::restore_status_message(code).map(ToString::to_string),
                log: dict
                    .get("Log")
                    .and_then(Value::as_string)
                    .map(ToString::to_string),
                finished: code == 0,
            }
        }
        Some("CheckpointMsg") => RestoreLifecycleEvent::Checkpoint {
            name: dict
                .get("Checkpoint")
                .and_then(Value::as_string)
                .map(ToString::to_string),
            raw: indexmap::IndexMap::new(),
        },
        Some("DataRequestMsg") | Some("AsyncDataRequestMsg") => {
            RestoreLifecycleEvent::DataRequest {
                data_type: dict
                    .get("DataType")
                    .and_then(Value::as_string)
                    .map(ToString::to_string),
                data_port: dict.get("DataPort").and_then(Value::as_unsigned_integer),
                async_request: msg_type == Some("AsyncDataRequestMsg"),
                raw: indexmap::IndexMap::new(),
            }
        }
        Some("CrashReportMsg") | Some("RestoredCrash") => RestoreLifecycleEvent::RestoredCrash {
            backtrace: dict
                .get("Backtrace")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_string)
                        .map(ToString::to_string)
                        .collect()
                })
                .unwrap_or_default(),
        },
        _ => RestoreLifecycleEvent::Unknown {
            msg_type: msg_type.map(ToString::to_string),
            raw: indexmap::IndexMap::new(),
        },
    }
}

/// Experimental restored transport loop for an already prepared recovery peer.
///
/// This is not a complete Update executor: the caller must establish the machine/build
/// plan, personalization, recovery transport and proven restore options. Unsupported
/// requests fail rather than being counted as served. CLI execution remains disabled.
pub async fn run_restored_update_loop<S, F>(
    client: &mut RestoredClient<S>,
    state_machine: RestoreStateMachine,
    options: Dictionary,
    firmware_provider: F,
    cancel: &OperationCancel,
) -> Result<RestoreExecutionReport, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut(&str, &Dictionary) -> Result<(Dictionary, Vec<u8>), RestoreError>,
{
    type NoConnector = fn(u16) -> std::future::Ready<Result<tokio::io::DuplexStream, RestoreError>>;
    run_restored_loop_inner(
        client,
        state_machine,
        options,
        firmware_provider,
        cancel,
        None::<(&mut std::io::Cursor<Vec<u8>>, &mut NoConnector)>,
        false,
    )
    .await
}

/// Dispatch synchronous SystemImageData to ASR using the selected OS image.
/// The receipt records local transfer only; unsupported routes still fail and
/// status zero cannot establish a complete Update while routes are missing.
pub async fn run_restored_update_loop_with_system_image<S, F, R, DataStream, C, Fut>(
    client: &mut RestoredClient<S>,
    state_machine: RestoreStateMachine,
    options: Dictionary,
    firmware_provider: F,
    cancel: &OperationCancel,
    image: &mut R,
    connect_port: &mut C,
) -> Result<RestoreExecutionReport, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut(&str, &Dictionary) -> Result<(Dictionary, Vec<u8>), RestoreError>,
    R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
    DataStream: AsyncRead + AsyncWrite + Unpin,
    C: FnMut(u16) -> Fut,
    Fut: Future<Output = Result<DataStream, RestoreError>>,
{
    run_restored_loop_inner(
        client,
        state_machine,
        options,
        firmware_provider,
        cancel,
        Some((image, connect_port)),
        true,
    )
    .await
}

async fn run_restored_loop_inner<S, F, R, DataStream, C, Fut>(
    client: &mut RestoredClient<S>,
    mut state_machine: RestoreStateMachine,
    options: Dictionary,
    mut firmware_provider: F,
    cancel: &OperationCancel,
    mut system_image: Option<(&mut R, &mut C)>,
    incomplete_executor: bool,
) -> Result<RestoreExecutionReport, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut(&str, &Dictionary) -> Result<(Dictionary, Vec<u8>), RestoreError>,
    R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
    DataStream: AsyncRead + AsyncWrite + Unpin,
    C: FnMut(u16) -> Fut,
    Fut: Future<Output = Result<DataStream, RestoreError>>,
{
    let deadline = client.deadline;
    let result = async {
    let protocol_version = client.protocol_version().clone();
    state_machine.finish_tss_phase()?;
    bounded(deadline, cancel, client.start_update(&protocol_version, options)).await?;

    loop {
        if cancel.is_cancelled() {
            client.terminal = true;
            state_machine.cancel();
            return Err(RestoreError::Io(std::io::Error::other(crate::OperationCancelled::new())));
        }

        let incoming = tokio::select! {
            biased;
            _ = cancel.cancelled() => None,
            result = client.next_message() => Some(result),
        };
        let incoming = match incoming {
            None => {
                client.terminal = true;
                state_machine.cancel();
                return Err(RestoreError::Io(std::io::Error::other(crate::OperationCancelled::new())));
            }
            Some(Ok(msg)) => msg,
            Some(Err(err)) => return Err(err),
        };

        let event = restore_event_from_plist(&incoming);

        if let RestoreLifecycleEvent::DataRequest {
            data_type: Some(ref dt),
            data_port,
            async_request,
            ..
        } = event
        {
            if dt == "FirmwareUpdaterData" && data_port.is_none() && !async_request {
                let updater_name = incoming
                    .get("Arguments")
                    .and_then(Value::as_dictionary)
                    .and_then(|args| args.get("UpdaterName"))
                    .and_then(Value::as_string)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| RestoreError::Protocol("missing restored UpdaterName".into()))?;
                let (header, firmware) = firmware_provider(updater_name, &incoming)?;
                if firmware.is_empty() {
                    return Err(RestoreError::Protocol(
                            "restored firmware provider returned no component bytes".into(),
                        ));
                }
                let before = client.firmware_bytes_written();
                let sent = bounded(deadline, cancel,
                    client.send_firmware_response(&incoming, header, &firmware)).await;
                state_machine
                    .record_bytes_transferred((client.firmware_bytes_written() - before) as usize);
                sent?;
                state_machine.record_data_request_served(format!("{dt}:{updater_name}"));
            } else if dt == "SystemImageData" && !async_request && system_image.is_some() {
                let (image, connector) = system_image.as_mut().expect("checked above");
                let receipt = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => None,
                    result = client.send_system_image(&incoming, &mut **image, &mut **connector) => Some(result),
                };
                let receipt = match receipt {
                    None => {
                        client.terminal = true;
                        state_machine.cancel();
                        return Err(RestoreError::Io(std::io::Error::other(
                                crate::OperationCancelled::new(),
                            )));
                    }
                    Some(result) => {
                        result?
                    }
                };
                // Count local image bytes only, not device flash or validation.
                state_machine.record_bytes_transferred(
                    usize::try_from(receipt.image_bytes_sent).unwrap_or(usize::MAX),
                );
                state_machine.record_data_request_served(dt.clone());
            } else {
                client.terminal = true;
                return Err(RestoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        format!(
                            "unserved restored data request: {dt}; no secondary transport owner"
                        ),
                    )));
            }
        } else if matches!(
            event,
            RestoreLifecycleEvent::DataRequest { .. } | RestoreLifecycleEvent::Unknown { .. }
        ) {
            return Err(RestoreError::Protocol(
                    "malformed or unsupported restored lifecycle message".into(),
                ));
        }

        if incomplete_executor
            && matches!(
                event,
                RestoreLifecycleEvent::Status {
                    code: 0,
                    finished: true,
                    ..
                }
            )
        {
            client.terminal = true;
            return Err(RestoreError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "restored status zero cannot establish a complete Update while routes remain unimplemented",
            )));
        }
        state_machine.process_event(&event)?;

        if state_machine.is_terminal() {
            match state_machine.state() {
                RestoreState::Completed => {
                    return Ok(state_machine.generate_report());
                }
                RestoreState::Failed(reason) => {
                    return Err(RestoreError::Protocol(format!("restored execution failed: {:?}", reason)));
                }
                _ => unreachable!(),
            }
        }
    }
    }.await;
    client.terminal = true;
    result.map_err(|error| {
        if crate::classify_error(&error) == crate::ErrorClass::Cancelled {
            state_machine.cancel();
        }
        failed_execution(&state_machine, error)
    })
}

/// Complete set of IPSW / TSS / personalization assets used by `run_restored_protocol_session`
/// to serve synchronous and asynchronous `restored` data requests.
pub struct RestoredProtocolAssets {
    pub build_identity: Dictionary,
    pub ap_img4_ticket: Vec<u8>,
    pub tss_response: Dictionary,
    pub boot_nonce: Option<Vec<u8>>,
    pub restore_version_plist: Vec<u8>,
    pub system_version_plist: Vec<u8>,
    pub global_manifest_plist: Vec<u8>,
    pub fdr_trust_data: Option<Vec<u8>>,
    pub acknowledge_final_status: bool,
}

impl RestoredProtocolAssets {
    pub fn new(
        build_identity: Dictionary,
        ap_img4_ticket: Vec<u8>,
        tss_response: Dictionary,
        boot_nonce: Option<Vec<u8>>,
    ) -> Self {
        Self {
            build_identity,
            ap_img4_ticket,
            tss_response,
            boot_nonce,
            restore_version_plist: b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>ProductBuildVersion</key><string>21F90</string></dict></plist>".to_vec(),
            system_version_plist: b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>ProductVersion</key><string>17.5.1</string></dict></plist>".to_vec(),
            global_manifest_plist: b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>Manifest</key><string>Global</string></dict></plist>".to_vec(),
            fdr_trust_data: None,
            acknowledge_final_status: true,
        }
    }

    fn stitch_component_with_provider<C>(
        &self,
        component_name: &str,
        fourcc_override: Option<[u8; 4]>,
        component_provider: &mut C,
    ) -> Result<Vec<u8>, RestoreError>
    where
        C: FnMut(&str) -> Result<Vec<u8>, RestoreError>,
    {
        let raw = component_provider(component_name)?;
        let stitched = super::img4::stitch_img4_component(
            component_name,
            &raw,
            &self.ap_img4_ticket,
            Some(&self.tss_response),
            self.boot_nonce.as_deref(),
            fourcc_override,
        )?;
        Ok(stitched.img4_bytes)
    }

    fn build_nor_response<C>(
        &self,
        arguments: Option<&Dictionary>,
        component_provider: &mut C,
    ) -> Result<(Dictionary, u64), RestoreError>
    where
        C: FnMut(&str) -> Result<Vec<u8>, RestoreError>,
    {
        let flash_version_1 = arguments
            .and_then(|a| a.get("FlashVersion1"))
            .and_then(Value::as_boolean)
            .unwrap_or(false);

        let llb_bytes = self.stitch_component_with_provider("LLB", None, component_provider)?;
        let mut counted_bytes = llb_bytes.len() as u64;

        let manifest = self
            .build_identity
            .get("Manifest")
            .and_then(Value::as_dictionary)
            .ok_or_else(|| RestoreError::Protocol("BuildIdentity missing Manifest".into()))?;

        let mut firmware_entries = Vec::new();
        for (name, entry) in manifest {
            if matches!(name.as_str(), "LLB" | "SEP" | "RestoreSEP") {
                continue;
            }
            let Some(info) = entry
                .as_dictionary()
                .and_then(|d| d.get("Info"))
                .and_then(Value::as_dictionary)
            else {
                continue;
            };
            let is_fw = info
                .get("IsFirmwarePayload")
                .and_then(Value::as_boolean)
                .unwrap_or(false)
                || info
                    .get("IsSecondaryFirmwarePayload")
                    .and_then(Value::as_boolean)
                    .unwrap_or(false);
            let has_path = info
                .get("Path")
                .and_then(Value::as_string)
                .is_some_and(|p| !p.is_empty());
            if is_fw && has_path {
                firmware_entries.push(name.clone());
            }
        }
        firmware_entries.sort();

        let mut iboot_data: Option<Vec<u8>> = None;
        let mut other_payloads: Vec<(String, Vec<u8>)> = Vec::new();
        for comp_name in firmware_entries {
            let stitched =
                self.stitch_component_with_provider(&comp_name, None, component_provider)?;
            counted_bytes = counted_bytes.saturating_add(stitched.len() as u64);
            if comp_name == "iBoot" {
                iboot_data = Some(stitched);
            } else {
                other_payloads.push((comp_name, stitched));
            }
        }

        let nor_image_value = if flash_version_1 {
            let mut dict = Dictionary::new();
            if let Some(ref iboot) = iboot_data {
                dict.insert("iBoot".to_owned(), Value::Data(iboot.clone()));
                dict.insert("Bootloader".to_owned(), Value::Data(iboot.clone()));
            }
            for (name, bytes) in other_payloads {
                dict.insert(name, Value::Data(bytes));
            }
            Value::Dictionary(dict)
        } else {
            let mut list = Vec::new();
            if let Some(iboot) = iboot_data {
                list.push(Value::Data(iboot));
            }
            for (_name, bytes) in other_payloads {
                list.push(Value::Data(bytes));
            }
            Value::Array(list)
        };

        let mut response = Dictionary::from_iter([
            ("LlbImageData".to_owned(), Value::Data(llb_bytes)),
            ("NorImageData".to_owned(), nor_image_value),
        ]);

        if manifest
            .get("RestoreSEP")
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get("Info"))
            .and_then(Value::as_dictionary)
            .and_then(|i| i.get("Path"))
            .and_then(Value::as_string)
            .is_some_and(|p| !p.is_empty())
        {
            let rsep =
                self.stitch_component_with_provider("RestoreSEP", None, component_provider)?;
            counted_bytes = counted_bytes.saturating_add(rsep.len() as u64);
            response.insert("RestoreSEPImageData".to_owned(), Value::Data(rsep));
        }

        if manifest
            .get("SEP")
            .and_then(Value::as_dictionary)
            .and_then(|d| d.get("Info"))
            .and_then(Value::as_dictionary)
            .and_then(|i| i.get("Path"))
            .and_then(Value::as_string)
            .is_some_and(|p| !p.is_empty())
        {
            let sep = self.stitch_component_with_provider("SEP", None, component_provider)?;
            counted_bytes = counted_bytes.saturating_add(sep.len() as u64);
            response.insert("SEPImageData".to_owned(), Value::Data(sep));
        }

        Ok((response, counted_bytes))
    }
}

/// Run a complete `restored` protocol session aligned with `pymobiledevice3/restore/restore.py`,
/// dispatching both `DataRequestMsg` and `AsyncDataRequestMsg` across `BuildIdentityDict`,
/// `RootTicket`, `KernelCache`, `DeviceTree`, `NORData`, `FDRTrustData`,
/// `FirmwareUpdaterPreflight` / `DeviceRestoreInfoPreflight`, `FirmwareUpdaterData`,
/// `PersonalizedBootObjectV3` / `SourceBootObjectV4` / `SourceBootObjectV5`,
/// `SystemImageRootHash`, `SystemImageCanonicalMetadata`, `FUDData`, `PersonalizedData`,
/// `EANData`, `ReceiptManifest`, `HostSystemTime`, `SystemImageData` (ASR),
/// `RestoreAttestation`, `BBUpdateStatusMsg`, and `StatusMsg` final acknowledgement.
#[allow(clippy::too_many_arguments)]
pub async fn run_restored_protocol_session<S, C, F, R, DataStream, P, Fut>(
    client: &mut RestoredClient<S>,
    mut state_machine: RestoreStateMachine,
    options: Dictionary,
    assets: &RestoredProtocolAssets,
    mut component_provider: C,
    mut firmware_provider: F,
    mut system_image: &mut R,
    mut connect_port: P,
    cancel: &OperationCancel,
) -> Result<RestoreExecutionReport, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    C: FnMut(&str) -> Result<Vec<u8>, RestoreError>,
    F: FnMut(&str, &Dictionary) -> Result<(Dictionary, Vec<u8>), RestoreError>,
    R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
    DataStream: AsyncRead + AsyncWrite + Unpin,
    P: FnMut(u16) -> Fut,
    Fut: Future<Output = Result<DataStream, RestoreError>>,
{
    let deadline = client.deadline;
    let result = async {
        let protocol_version = client.protocol_version().clone();
        if *state_machine.state() == RestoreState::TssRequest {
            state_machine.finish_tss_phase()?;
        }
        bounded(
            deadline,
            cancel,
            client.start_update(&protocol_version, options),
        )
        .await?;

        loop {
            if cancel.is_cancelled() {
                client.terminal = true;
                state_machine.cancel();
                return Err(RestoreError::Io(std::io::Error::other(
                    crate::OperationCancelled::new(),
                )));
            }

            let incoming = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                result = client.next_message() => Some(result),
            };
            let incoming = match incoming {
                None => {
                    client.terminal = true;
                    state_machine.cancel();
                    return Err(RestoreError::Io(std::io::Error::other(
                        crate::OperationCancelled::new(),
                    )));
                }
                Some(Ok(msg)) => msg,
                Some(Err(err)) => return Err(err),
            };

            let msg_type = incoming.get("MsgType").and_then(Value::as_string);
            if msg_type == Some("RestoreAttestation") {
                let body = Dictionary::from_iter([(
                    "RestoreShouldAttest".to_owned(),
                    Value::Boolean(false),
                )]);
                client.send_plist_response(&body, 0).await?;
                state_machine.record_data_request_served("RestoreAttestation".into());
                continue;
            }
            if msg_type == Some("BBUpdateStatusMsg") {
                let accepted = incoming
                    .get("Accepted")
                    .and_then(Value::as_boolean)
                    .unwrap_or(false);
                if !accepted {
                    return Err(RestoreError::Protocol(format!(
                        "BBUpdateStatusMsg rejected baseband update: {:?}",
                        incoming.get("Error")
                    )));
                }
                state_machine.record_data_request_served("BBUpdateStatusMsg".into());
                continue;
            }

            let event = restore_event_from_plist(&incoming);

            if let RestoreLifecycleEvent::DataRequest {
                data_type: Some(ref dt),
                ..
            } = event
            {
                let arguments = incoming.get("Arguments").and_then(Value::as_dictionary);
                match dt.as_str() {
                    "BuildIdentityDict" | "RecoveryOSLocalPolicy" => {
                        let body = Dictionary::from_iter([(
                            "BuildIdentityDict".to_owned(),
                            Value::Dictionary(assets.build_identity.clone()),
                        )]);
                        client.send_plist_response(&body, 0).await?;
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "RootTicket" | "RecoveryOSRootTicketData" => {
                        let len = assets.ap_img4_ticket.len() as u64;
                        let body = Dictionary::from_iter([(
                            "RootTicketData".to_owned(),
                            Value::Data(assets.ap_img4_ticket.clone()),
                        )]);
                        client.send_plist_response(&body, len).await?;
                        state_machine.record_bytes_transferred(len as usize);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "NORData" => {
                        let (body, bytes_count) =
                            assets.build_nor_response(arguments, &mut component_provider)?;
                        client.send_plist_response(&body, bytes_count).await?;
                        state_machine.record_bytes_transferred(bytes_count as usize);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "FDRTrustData" => {
                        let (body, len) = match &assets.fdr_trust_data {
                            Some(data) => (
                                Dictionary::from_iter([(
                                    "FDRTrustData".to_owned(),
                                    Value::Data(data.clone()),
                                )]),
                                data.len() as u64,
                            ),
                            None => (Dictionary::new(), 0u64),
                        };
                        client.send_plist_response(&body, len).await?;
                        state_machine.record_bytes_transferred(len as usize);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "FirmwareUpdaterPreflight" | "DeviceRestoreInfoPreflight" => {
                        let body = Dictionary::new();
                        client.send_plist_response(&body, 0).await?;
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "FirmwareUpdaterData" => {
                        let updater_name = arguments
                            .and_then(|args| args.get("UpdaterName"))
                            .and_then(Value::as_string)
                            .filter(|name| !name.is_empty())
                            .ok_or_else(|| {
                                RestoreError::Protocol("missing restored UpdaterName".into())
                            })?;
                        let (header, firmware) = firmware_provider(updater_name, &incoming)?;
                        if firmware.is_empty() {
                            return Err(RestoreError::Protocol(
                                "restored firmware provider returned no component bytes".into(),
                            ));
                        }
                        let mut sync_incoming = incoming.clone();
                        sync_incoming
                            .insert("MsgType".to_owned(), Value::String("DataRequestMsg".into()));
                        sync_incoming.remove("DataPort");
                        let before = client.firmware_bytes_written();
                        client
                            .send_firmware_response(&sync_incoming, header, &firmware)
                            .await?;
                        state_machine.record_bytes_transferred(
                            (client.firmware_bytes_written() - before) as usize,
                        );
                        state_machine.record_data_request_served(format!("{dt}:{updater_name}"));
                    }
                    "KernelCache" => {
                        let bytes = assets.stitch_component_with_provider(
                            "KernelCache",
                            None,
                            &mut component_provider,
                        )?;
                        let len = bytes.len();
                        let body = Dictionary::from_iter([(
                            "KernelCacheFile".to_owned(),
                            Value::Data(bytes),
                        )]);
                        client.send_plist_response(&body, len as u64).await?;
                        state_machine.record_bytes_transferred(len);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "DeviceTree" => {
                        let bytes = assets.stitch_component_with_provider(
                            "DeviceTree",
                            None,
                            &mut component_provider,
                        )?;
                        let len = bytes.len();
                        let body = Dictionary::from_iter([(
                            "DeviceTreeFile".to_owned(),
                            Value::Data(bytes),
                        )]);
                        client.send_plist_response(&body, len as u64).await?;
                        state_machine.record_bytes_transferred(len);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "SystemImageRootHash" => {
                        let bytes = assets.stitch_component_with_provider(
                            "SystemVolume",
                            None,
                            &mut component_provider,
                        )?;
                        let len = bytes.len();
                        let body = Dictionary::from_iter([(
                            "SystemImageRootHashFile".to_owned(),
                            Value::Data(bytes),
                        )]);
                        client.send_plist_response(&body, len as u64).await?;
                        state_machine.record_bytes_transferred(len);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "SystemImageCanonicalMetadata" => {
                        let bytes = assets.stitch_component_with_provider(
                            "Ap,SystemVolumeCanonicalMetadata",
                            None,
                            &mut component_provider,
                        )?;
                        let len = bytes.len();
                        let body = Dictionary::from_iter([(
                            "SystemImageCanonicalMetadataFile".to_owned(),
                            Value::Data(bytes),
                        )]);
                        client.send_plist_response(&body, len as u64).await?;
                        state_machine.record_bytes_transferred(len);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "PersonalizedBootObjectV3"
                    | "SourceBootObjectV4"
                    | "SourceBootObjectV5"
                    | "BootabilityBundle"
                    | "BootabilityBundleV2"
                    | "RecoveryOSImage"
                    | "RecoveryOSIBootFWFilesImages" => {
                        let image_name = arguments
                            .and_then(|a| a.get("ImageName"))
                            .and_then(Value::as_string)
                            .unwrap_or(dt.as_str());
                        let payload = match image_name {
                            "__GlobalManifest__" => assets.global_manifest_plist.clone(),
                            "__RestoreVersion__" => assets.restore_version_plist.clone(),
                            "__SystemVersion__" => assets.system_version_plist.clone(),
                            other => assets.stitch_component_with_provider(
                                other,
                                None,
                                &mut component_provider,
                            )?,
                        };
                        if payload.is_empty() {
                            client.send_plist_response(&Dictionary::new(), 0).await?;
                        } else {
                            for chunk in payload.chunks(8192) {
                                let chunk_msg = Dictionary::from_iter([(
                                    "FileData".to_owned(),
                                    Value::Data(chunk.to_vec()),
                                )]);
                                client
                                    .send_plist_response(&chunk_msg, chunk.len() as u64)
                                    .await?;
                            }
                            let done_msg = Dictionary::from_iter([(
                                "FileDataDone".to_owned(),
                                Value::Boolean(true),
                            )]);
                            client.send_plist_response(&done_msg, 0).await?;
                            state_machine.record_bytes_transferred(payload.len());
                        }
                        state_machine.record_data_request_served(format!("{dt}:{image_name}"));
                    }
                    "FUDData" => {
                        let is_list = arguments
                            .and_then(|a| a.get("ImageList"))
                            .and_then(Value::as_boolean)
                            .unwrap_or(false);
                        if is_list {
                            let mut names = Vec::new();
                            if let Some(manifest) = assets
                                .build_identity
                                .get("Manifest")
                                .and_then(Value::as_dictionary)
                            {
                                for (name, entry) in manifest {
                                    let is_fud = entry
                                        .as_dictionary()
                                        .and_then(|d| d.get("Info"))
                                        .and_then(Value::as_dictionary)
                                        .and_then(|i| i.get("IsFUDFirmware"))
                                        .and_then(Value::as_boolean)
                                        .unwrap_or(false);
                                    if is_fud {
                                        names.push(Value::String(name.clone()));
                                    }
                                }
                            }
                            let body = Dictionary::from_iter([(
                                "FUDImageList".to_owned(),
                                Value::Array(names),
                            )]);
                            client.send_plist_response(&body, 0).await?;
                            state_machine.record_data_request_served("FUDData:ImageList".into());
                        } else {
                            let image_name = arguments
                                .and_then(|a| a.get("ImageName"))
                                .and_then(Value::as_string)
                                .ok_or_else(|| {
                                    RestoreError::Protocol("FUDData missing ImageName".into())
                                })?;
                            let bytes = assets.stitch_component_with_provider(
                                image_name,
                                None,
                                &mut component_provider,
                            )?;
                            let len = bytes.len();
                            let body = Dictionary::from_iter([
                                ("FUDImageData".to_owned(), Value::Data(bytes)),
                                (
                                    "UpdaterName".to_owned(),
                                    Value::String(image_name.to_owned()),
                                ),
                            ]);
                            client.send_plist_response(&body, len as u64).await?;
                            state_machine.record_bytes_transferred(len);
                            state_machine
                                .record_data_request_served(format!("FUDData:{image_name}"));
                        }
                    }
                    "PersonalizedData" => {
                        let is_list = arguments
                            .and_then(|a| a.get("ImageList"))
                            .and_then(Value::as_boolean)
                            .unwrap_or(false);
                        if is_list {
                            let body = Dictionary::from_iter([(
                                "ImageList".to_owned(),
                                Value::Array(vec![Value::String(String::new())]),
                            )]);
                            client.send_plist_response(&body, 0).await?;
                            state_machine
                                .record_data_request_served("PersonalizedData:ImageList".into());
                        } else {
                            let image_name = arguments
                                .and_then(|a| a.get("ImageName"))
                                .and_then(Value::as_string)
                                .ok_or_else(|| {
                                    RestoreError::Protocol(
                                        "PersonalizedData missing ImageName".into(),
                                    )
                                })?;
                            let bytes = assets.stitch_component_with_provider(
                                image_name,
                                None,
                                &mut component_provider,
                            )?;
                            let len = bytes.len();
                            let body = Dictionary::from_iter([(
                                "ImageData".to_owned(),
                                Value::Data(bytes),
                            )]);
                            client.send_plist_response(&body, len as u64).await?;
                            state_machine.record_bytes_transferred(len);
                            state_machine.record_data_request_served(format!(
                                "PersonalizedData:{image_name}"
                            ));
                        }
                    }
                    "EANData" => {
                        let is_list = arguments
                            .and_then(|a| a.get("EarlyAccessNANDImageList"))
                            .and_then(Value::as_boolean)
                            .unwrap_or(false);
                        if is_list {
                            let mut names = Vec::new();
                            if let Some(manifest) = assets
                                .build_identity
                                .get("Manifest")
                                .and_then(Value::as_dictionary)
                            {
                                for (name, entry) in manifest {
                                    let is_ean = entry
                                        .as_dictionary()
                                        .and_then(|d| d.get("Info"))
                                        .and_then(Value::as_dictionary)
                                        .and_then(|i| i.get("IsEarlyAccessFirmware"))
                                        .and_then(Value::as_boolean)
                                        .unwrap_or(false);
                                    if is_ean {
                                        names.push(Value::String(name.clone()));
                                    }
                                }
                            }
                            let body = Dictionary::from_iter([(
                                "EANImageList".to_owned(),
                                Value::Array(names),
                            )]);
                            client.send_plist_response(&body, 0).await?;
                            state_machine.record_data_request_served("EANData:ImageList".into());
                        } else {
                            let image_name = arguments
                                .and_then(|a| a.get("ImageName"))
                                .and_then(Value::as_string)
                                .ok_or_else(|| {
                                    RestoreError::Protocol("EANData missing ImageName".into())
                                })?;
                            let bytes = assets.stitch_component_with_provider(
                                image_name,
                                None,
                                &mut component_provider,
                            )?;
                            let len = bytes.len();
                            let body =
                                Dictionary::from_iter([("EANData".to_owned(), Value::Data(bytes))]);
                            client.send_plist_response(&body, len as u64).await?;
                            state_machine.record_bytes_transferred(len);
                            state_machine
                                .record_data_request_served(format!("EANData:{image_name}"));
                        }
                    }
                    "ReceiptManifest" => {
                        let bytes = assets.stitch_component_with_provider(
                            "RBM",
                            Some(*b"rmbt"),
                            &mut component_provider,
                        )?;
                        let len = bytes.len();
                        let body = Dictionary::from_iter([(
                            "ReceiptManifest".to_owned(),
                            Value::Data(bytes),
                        )]);
                        client.send_plist_response(&body, len as u64).await?;
                        state_machine.record_bytes_transferred(len);
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "HostSystemTime" => {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default();
                        let body = Dictionary::from_iter([
                            (
                                "SetHostTimeOnDevice".to_owned(),
                                Value::Integer((now.as_secs()).into()),
                            ),
                            (
                                "SetHostTimeOnDeviceMicroSeconds".to_owned(),
                                Value::Integer((now.subsec_micros() as u64).into()),
                            ),
                        ]);
                        client.send_plist_response(&body, 0).await?;
                        state_machine.record_data_request_served(dt.clone());
                    }
                    "SystemImageData" | "RecoveryOSASRImage" => {
                        let receipt = client
                            .send_system_image_any_request(
                                &incoming,
                                &mut system_image,
                                &mut connect_port,
                            )
                            .await?;
                        state_machine.record_bytes_transferred(
                            usize::try_from(receipt.image_bytes_sent).unwrap_or(usize::MAX),
                        );
                        state_machine.record_data_request_served(dt.clone());
                    }
                    other => {
                        client.terminal = true;
                        return Err(RestoreError::Io(std::io::Error::new(
                            std::io::ErrorKind::Unsupported,
                            format!("unserved restored data request: {other}"),
                        )));
                    }
                }
            } else if matches!(
                event,
                RestoreLifecycleEvent::DataRequest { .. } | RestoreLifecycleEvent::Unknown { .. }
            ) {
                return Err(RestoreError::Protocol(
                    "malformed or unsupported restored lifecycle message".into(),
                ));
            }

            if assets.acknowledge_final_status
                && matches!(
                    event,
                    RestoreLifecycleEvent::Status {
                        code: 0,
                        finished: true,
                        ..
                    }
                )
            {
                let ack = Dictionary::from_iter([(
                    "MsgType".to_owned(),
                    Value::String("ReceivedFinalStatusMsg".into()),
                )]);
                client.send_plist_response(&ack, 0).await?;
            }

            state_machine.process_event(&event)?;

            if state_machine.is_terminal() {
                match state_machine.state() {
                    RestoreState::Completed => {
                        return Ok(state_machine.generate_report());
                    }
                    RestoreState::Failed(reason) => {
                        return Err(RestoreError::Protocol(format!(
                            "restored execution failed: {:?}",
                            reason
                        )));
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    .await;
    client.terminal = true;
    result.map_err(|error| {
        if crate::classify_error(&error) == crate::ErrorClass::Cancelled {
            state_machine.cancel();
        }
        failed_execution(&state_machine, error)
    })
}

#[cfg(test)]
mod tests;

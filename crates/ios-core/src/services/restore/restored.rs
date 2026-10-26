//! Bounded restored plist transport, distinct from normal-mode RestoreRemoteServices.
//!
//! Wire evidence: pymobiledevice3 03ba2270, restore/restored_client.py and
//! restore/restore.py::firmware_response_messages. This module does not boot a
//! restore ramdisk, implement ASR/FDR, personalize IMG4, or claim a full update.

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
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(RestoreError::Io(std::io::Error::other(crate::OperationCancelled::new()))),
        result = tokio::time::timeout_at(deadline, work) => result.map_err(|e|
            native_error(std::io::ErrorKind::TimedOut, "restored operation deadline", e))?,
    }
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
        let result = bounded(self.deadline, &self.cancel, async {
            write_xml_plist_frame(&mut self.stream, body, MAX_FRAME)
                .await
                .map_err(|e| native_error(std::io::ErrorKind::Other, "write restored plist", e))
        })
        .await;
        if result.is_err() {
            self.terminal = true;
        }
        result
    }

    async fn receive(&mut self) -> Result<Dictionary, RestoreError> {
        self.live()?;
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
        if result.is_err() {
            self.terminal = true;
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
    mut state_machine: RestoreStateMachine,
    options: Dictionary,
    mut firmware_provider: F,
    cancel: &OperationCancel,
) -> Result<RestoreExecutionReport, RestoreError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnMut(&str, &Dictionary) -> Result<(Dictionary, Vec<u8>), RestoreError>,
{
    let protocol_version = client.protocol_version().clone();
    state_machine.finish_tss_phase()?;
    client.start_update(&protocol_version, options).await?;

    loop {
        if cancel.is_cancelled() {
            state_machine.cancel();
            return Err(failed_execution(
                &state_machine,
                RestoreError::Io(std::io::Error::other(crate::OperationCancelled::new())),
            ));
        }

        let incoming = match client.next_message().await {
            Ok(msg) => msg,
            Err(err) => {
                return Err(failed_execution(&state_machine, err));
            }
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
                let (header, firmware) = firmware_provider(updater_name, &incoming)
                    .map_err(|error| failed_execution(&state_machine, error))?;
                if firmware.is_empty() {
                    return Err(failed_execution(
                        &state_machine,
                        RestoreError::Protocol(
                            "restored firmware provider returned no component bytes".into(),
                        ),
                    ));
                }
                let before = client.firmware_bytes_written();
                let sent = client
                    .send_firmware_response(&incoming, header, &firmware)
                    .await;
                state_machine
                    .record_bytes_transferred((client.firmware_bytes_written() - before) as usize);
                sent.map_err(|error| failed_execution(&state_machine, error))?;
                state_machine.record_data_request_served(format!("{dt}:{updater_name}"));
            } else {
                return Err(failed_execution(
                    &state_machine,
                    RestoreError::Io(std::io::Error::new(
                        std::io::ErrorKind::Unsupported,
                        format!(
                            "unserved restored data request: {dt}; no secondary transport owner"
                        ),
                    )),
                ));
            }
        } else if matches!(
            event,
            RestoreLifecycleEvent::DataRequest { .. } | RestoreLifecycleEvent::Unknown { .. }
        ) {
            return Err(failed_execution(
                &state_machine,
                RestoreError::Protocol(
                    "malformed or unsupported restored lifecycle message".into(),
                ),
            ));
        }

        state_machine.process_event(&event)?;

        if state_machine.is_terminal() {
            match state_machine.state() {
                RestoreState::Completed => {
                    return Ok(state_machine.generate_report());
                }
                RestoreState::Failed(reason) => {
                    return Err(failed_execution(
                        &state_machine,
                        RestoreError::Protocol(format!("restored execution failed: {:?}", reason)),
                    ));
                }
                _ => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests;

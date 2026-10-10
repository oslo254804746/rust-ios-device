//! Experimental restore execution state machine.
//!
//! Models mode transition, nonce query, TSS signing validation,
//! chunked firmware transmission, verification, progress observation,
//! and failure categorization for the Update restore flow.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use indexmap::IndexMap;

use super::{
    preflight::firmware_response_messages, restore_status_message, RestoreError,
    RestoreLifecycleEvent,
};
use crate::xpc::XpcValue;

/// Current lifecycle state of the restore process.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RestoreState {
    /// Initial idle state.
    Idle,
    /// Mode transition phase: device transitioning from normal mode to recovery mode.
    ModeTransition,
    /// Querying runtime AP/SEP nonces and preflight info.
    QueryNonces,
    /// Validating or requesting TSS signatures and tickets.
    TssRequest,
    /// Streaming and transferring chunked firmware components to restored.
    FirmwareTransfer,
    /// Device flashing / writing updated system and peripheral images.
    FlashingUpdate,
    /// Verifying filesystem, boot objects, and cryptographic integrity.
    Verifying,
    /// Restore execution completed successfully (status code 0).
    Completed,
    /// Restore execution failed.
    Failed(RestoreFailure),
}

/// Categorized failure reasons during restore execution.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RestoreFailure {
    /// Restored returned non-zero status code.
    StatusCode {
        code: u64,
        message: Option<String>,
        log: Option<String>,
    },
    /// Restored crash reported with backtrace.
    Crash { backtrace: Vec<String> },
    /// Runtime nonce mismatch (nonce drift) between preflight and execution.
    NonceMismatch {
        component: String,
        expected_hex: String,
        actual_hex: String,
    },
    /// Missing required component or payload.
    MissingComponent { component: String },
    /// Operation or step timed out under budget.
    Timeout { step: String, duration_ms: u64 },
    /// Operation cancelled.
    Cancelled,
    /// Protocol or connection error.
    ProtocolError(String),
}

/// Explicit inventory of protocol, device, and firmware gaps.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RestoreProtocolGaps {
    /// Live TSS signing requirement.
    pub tss_live_signing: String,
    /// Erase / factory reset FDR proxy requirement.
    pub erase_fdr_proxy: String,
    /// DFU recovery stage 1 bootstrapping requirement.
    pub dfu_bootstrap: String,
    /// Wireless restore tunnel persistence limitation.
    pub wireless_tunnel_drop: String,
    /// Unknown peripheral chipset mapping limitation.
    pub unknown_chipset_prefixes: String,
}

impl Default for RestoreProtocolGaps {
    fn default() -> Self {
        Self {
            tss_live_signing: "Live Apple TSS signing requires an active Apple signing window and valid device ECID signature; offline execution uses synthetic/saved SHSH blobs.".into(),
            erase_fdr_proxy: "Erase/Factory restore requires bidirectional FDR (Factory Data Recovery) proxying (FDR_CTRL / FDR_DATA) and disk wiping, not yet exercised in Update flow.".into(),
            dfu_bootstrap: "DFU mode restore requires Low-Level Bootloader / iBoot stage 1 bootstrapping over USB DFU before restoreserviced is accessible.".into(),
            wireless_tunnel_drop: "Wireless / Wi-Fi restores are unsupported because pairing tunnels terminate during recovery reboot, requiring physical USB connection.".into(),
            unknown_chipset_prefixes: "Firmware component prefixes are mapped only for known peripheral updaters (T200, Rose, SE, Vinyl, Savage, Centauri, Baseband).".into(),
        }
    }
}

/// Outcome of processing one lifecycle event or step in the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreStepOutcome {
    /// State unchanged.
    Unchanged,
    /// State transitioned to new state.
    Transitioned {
        previous: RestoreState,
        current: RestoreState,
    },
    /// Data request handled.
    DataRequestHandled { data_type: String },
    /// Late progress message observed after completion.
    LateMessageObserved,
}

/// Comprehensive summary report of a restore execution session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RestoreExecutionReport {
    pub final_state: RestoreState,
    pub operations_observed: Vec<String>,
    pub checkpoints_reached: Vec<String>,
    pub data_requests_served: Vec<String>,
    pub progress_percent: Option<u64>,
    pub events_count: usize,
    pub late_messages_count: usize,
    pub nonce_changes_detected: usize,
    pub total_bytes_transferred: usize,
    pub total_duration_ms: u64,
    pub protocol_gaps: RestoreProtocolGaps,
}

/// Restore execution state machine for the Update restore flow.
pub struct RestoreStateMachine {
    state: RestoreState,
    operations_observed: Vec<String>,
    checkpoints_reached: Vec<String>,
    data_requests_served: Vec<String>,
    progress_percent: Option<u64>,
    events_count: usize,
    late_messages_count: usize,
    nonce_changes_detected: usize,
    known_nonces: HashMap<String, Vec<u8>>,
    tss_tickets: HashMap<String, Vec<u8>>,
    start_time: Instant,
    protocol_gaps: RestoreProtocolGaps,
    max_chunk_size: usize,
    total_data_bytes_transferred: usize,
    max_data_transfer_limit: usize,
}

impl Default for RestoreStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl RestoreStateMachine {
    /// Create a new restore state machine in the `Idle` state.
    pub fn new() -> Self {
        Self {
            state: RestoreState::Idle,
            operations_observed: Vec::new(),
            checkpoints_reached: Vec::new(),
            data_requests_served: Vec::new(),
            progress_percent: None,
            events_count: 0,
            late_messages_count: 0,
            nonce_changes_detected: 0,
            known_nonces: HashMap::new(),
            tss_tickets: HashMap::new(),
            start_time: Instant::now(),
            protocol_gaps: RestoreProtocolGaps::default(),
            max_chunk_size: 256 * 1024,
            total_data_bytes_transferred: 0,
            max_data_transfer_limit: 1024 * 1024 * 1024, // 1 GiB safety limit
        }
    }

    /// Return reference to current state.
    pub fn state(&self) -> &RestoreState {
        &self.state
    }

    /// Return whether state machine reached a terminal state (Completed or Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            RestoreState::Completed | RestoreState::Failed(_)
        )
    }

    /// Transition from Idle to ModeTransition.
    pub fn start_mode_transition(&mut self) -> Result<(), RestoreError> {
        if self.state != RestoreState::Idle {
            return Err(RestoreError::Protocol(format!(
                "cannot start mode transition from state {:?}",
                self.state
            )));
        }
        self.state = RestoreState::ModeTransition;
        Ok(())
    }

    /// Device has confirmed transition to recovery mode.
    pub fn transition_to_recovery(&mut self) -> Result<(), RestoreError> {
        if !matches!(
            self.state,
            RestoreState::Idle | RestoreState::ModeTransition
        ) {
            return Err(RestoreError::Protocol(format!(
                "cannot transition to recovery from state {:?}",
                self.state
            )));
        }
        self.state = RestoreState::QueryNonces;
        Ok(())
    }

    /// Record a runtime nonce for a component. If a previous nonce was known and differs,
    /// increments the nonce drift counter.
    pub fn record_nonce(&mut self, component: &str, nonce: Vec<u8>) {
        if let Some(existing) = self.known_nonces.get(component) {
            if existing != &nonce {
                self.nonce_changes_detected += 1;
            }
        }
        self.known_nonces.insert(component.to_string(), nonce);
    }

    /// Check if a given component's expected nonce matches runtime nonce.
    pub fn verify_nonce_match(&self, component: &str, expected: &[u8]) -> bool {
        match self.known_nonces.get(component) {
            Some(actual) => actual.as_slice() == expected,
            None => true,
        }
    }

    /// Query nonces step completed.
    pub fn receive_nonces(
        &mut self,
        ap_nonce: Option<Vec<u8>>,
        sep_nonce: Option<Vec<u8>>,
    ) -> Result<(), RestoreError> {
        if let Some(ap) = ap_nonce {
            self.record_nonce("AP", ap);
        }
        if let Some(sep) = sep_nonce {
            self.record_nonce("SEP", sep);
        }
        self.state = RestoreState::TssRequest;
        Ok(())
    }

    /// Store a TSS ticket for a component, checking for nonce drift if an expected nonce is provided.
    pub fn store_tss_ticket(
        &mut self,
        component: &str,
        ticket: Vec<u8>,
        expected_nonce: Option<&[u8]>,
    ) -> Result<(), RestoreError> {
        if let Some(expected) = expected_nonce {
            if let Some(actual) = self.known_nonces.get(component) {
                if actual.as_slice() != expected {
                    self.nonce_changes_detected += 1;
                    let failure = RestoreFailure::NonceMismatch {
                        component: component.to_string(),
                        expected_hex: hex::encode(expected),
                        actual_hex: hex::encode(actual),
                    };
                    self.state = RestoreState::Failed(failure);
                    return Err(RestoreError::Protocol(format!(
                        "nonce drift detected for {component}: expected {}, actual {}",
                        hex::encode(expected),
                        hex::encode(actual)
                    )));
                }
            }
        }
        self.tss_tickets.insert(component.to_string(), ticket);
        Ok(())
    }

    /// Finish TSS request phase and move to firmware transfer.
    pub fn finish_tss_phase(&mut self) -> Result<(), RestoreError> {
        if self.state != RestoreState::TssRequest {
            return Err(RestoreError::Protocol(format!(
                "cannot finish TSS phase from state {:?}",
                self.state
            )));
        }
        self.state = RestoreState::FirmwareTransfer;
        Ok(())
    }

    /// Chunk and prepare firmware messages with safety budget checks.
    pub fn prepare_chunked_firmware(
        &mut self,
        header: IndexMap<String, XpcValue>,
        firmware: &[u8],
        requested_chunk_size: usize,
    ) -> Result<Vec<XpcValue>, RestoreError> {
        let new_total = self
            .total_data_bytes_transferred
            .checked_add(firmware.len())
            .ok_or_else(|| RestoreError::Protocol("firmware transfer size overflow".into()))?;
        if new_total > self.max_data_transfer_limit {
            return Err(RestoreError::Protocol(format!(
                "total firmware transfer exceeds safety limit of {} bytes",
                self.max_data_transfer_limit
            )));
        }
        self.total_data_bytes_transferred = new_total;

        let chunk_size = if requested_chunk_size == 0 {
            self.max_chunk_size
        } else {
            requested_chunk_size.min(self.max_chunk_size)
        };

        let iter = firmware_response_messages(header, firmware, chunk_size)?;
        Ok(iter.collect())
    }

    /// Process one incoming restore lifecycle event from restoreserviced.
    pub fn process_event(
        &mut self,
        event: &RestoreLifecycleEvent,
    ) -> Result<RestoreStepOutcome, RestoreError> {
        self.events_count += 1;

        // If already terminal, record late messages without altering terminal state
        if self.is_terminal() {
            self.late_messages_count += 1;
            return Ok(RestoreStepOutcome::LateMessageObserved);
        }

        match event {
            RestoreLifecycleEvent::Progress {
                operation,
                progress,
            } => {
                if let Some(p) = progress {
                    self.progress_percent = Some(*p);
                }
                if let Some(op) = operation {
                    self.operations_observed.push(op.clone());
                    let lower = op.to_lowercase();
                    if lower.contains("flash")
                        || lower.contains("writing")
                        || lower.contains("update")
                    {
                        if matches!(
                            self.state,
                            RestoreState::FirmwareTransfer | RestoreState::TssRequest
                        ) {
                            let prev = self.state.clone();
                            self.state = RestoreState::FlashingUpdate;
                            return Ok(RestoreStepOutcome::Transitioned {
                                previous: prev,
                                current: RestoreState::FlashingUpdate,
                            });
                        }
                    } else if (lower.contains("verify") || lower.contains("checking"))
                        && matches!(
                            self.state,
                            RestoreState::FlashingUpdate | RestoreState::FirmwareTransfer
                        )
                    {
                        let prev = self.state.clone();
                        self.state = RestoreState::Verifying;
                        return Ok(RestoreStepOutcome::Transitioned {
                            previous: prev,
                            current: RestoreState::Verifying,
                        });
                    }
                }
                Ok(RestoreStepOutcome::Unchanged)
            }
            RestoreLifecycleEvent::Status {
                code,
                message,
                log,
                finished,
            } => {
                let prev = self.state.clone();
                if *finished && *code == 0 {
                    self.state = RestoreState::Completed;
                    Ok(RestoreStepOutcome::Transitioned {
                        previous: prev,
                        current: RestoreState::Completed,
                    })
                } else {
                    let failure = RestoreFailure::StatusCode {
                        code: *code,
                        message: message
                            .clone()
                            .or_else(|| restore_status_message(*code).map(ToString::to_string)),
                        log: log.clone(),
                    };
                    self.state = RestoreState::Failed(failure);
                    Ok(RestoreStepOutcome::Transitioned {
                        previous: prev,
                        current: self.state.clone(),
                    })
                }
            }
            RestoreLifecycleEvent::Checkpoint { name, .. } => {
                if let Some(n) = name {
                    self.checkpoints_reached.push(n.clone());
                }
                Ok(RestoreStepOutcome::Unchanged)
            }
            RestoreLifecycleEvent::DataRequest { data_type, .. } => {
                let dt = data_type
                    .clone()
                    .unwrap_or_else(|| "UnknownDataRequest".into());
                self.data_requests_served.push(dt.clone());
                if matches!(
                    self.state,
                    RestoreState::QueryNonces | RestoreState::TssRequest
                ) {
                    let prev = self.state.clone();
                    self.state = RestoreState::FirmwareTransfer;
                    return Ok(RestoreStepOutcome::Transitioned {
                        previous: prev,
                        current: RestoreState::FirmwareTransfer,
                    });
                }
                Ok(RestoreStepOutcome::DataRequestHandled { data_type: dt })
            }
            RestoreLifecycleEvent::RestoredCrash { backtrace } => {
                let prev = self.state.clone();
                self.state = RestoreState::Failed(RestoreFailure::Crash {
                    backtrace: backtrace.clone(),
                });
                Ok(RestoreStepOutcome::Transitioned {
                    previous: prev,
                    current: self.state.clone(),
                })
            }
            RestoreLifecycleEvent::PreviousRestoreLog(log) => {
                self.operations_observed
                    .push(format!("previous_log: {log}"));
                Ok(RestoreStepOutcome::Unchanged)
            }
            RestoreLifecycleEvent::RestoreProtocol { .. }
            | RestoreLifecycleEvent::Unknown { .. } => Ok(RestoreStepOutcome::Unchanged),
        }
    }

    /// Check if the total elapsed time has exceeded budget. If so, transitions to Failed(Timeout).
    pub fn check_timeout(&mut self, max_duration: Duration, current_step: &str) -> bool {
        let elapsed = self.start_time.elapsed();
        if elapsed > max_duration {
            self.state = RestoreState::Failed(RestoreFailure::Timeout {
                step: current_step.to_string(),
                duration_ms: elapsed.as_millis() as u64,
            });
            true
        } else {
            false
        }
    }

    /// Cancel restore execution.
    pub fn cancel(&mut self) {
        self.state = RestoreState::Failed(RestoreFailure::Cancelled);
    }

    /// Generate execution report.
    pub fn generate_report(&self) -> RestoreExecutionReport {
        RestoreExecutionReport {
            final_state: self.state.clone(),
            operations_observed: self.operations_observed.clone(),
            checkpoints_reached: self.checkpoints_reached.clone(),
            data_requests_served: self.data_requests_served.clone(),
            progress_percent: self.progress_percent,
            events_count: self.events_count,
            late_messages_count: self.late_messages_count,
            nonce_changes_detected: self.nonce_changes_detected,
            total_bytes_transferred: self.total_data_bytes_transferred,
            total_duration_ms: self.start_time.elapsed().as_millis() as u64,
            protocol_gaps: self.protocol_gaps.clone(),
        }
    }
}

/// Run the experimental restore event loop over a connected client.
pub async fn run_experimental_restore_loop<S, H>(
    client: &mut super::RestoreServiceClient<S>,
    mut state_machine: RestoreStateMachine,
    mut data_handler: H,
    timeout: Duration,
    cancel_flag: Option<Arc<AtomicBool>>,
) -> Result<RestoreExecutionReport, RestoreError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    H: FnMut(&str, Option<u64>) -> Result<Vec<u8>, RestoreError>,
{
    let deadline = Instant::now() + timeout;

    while !state_machine.is_terminal() {
        if let Some(ref cancel) = cancel_flag {
            if cancel.load(Ordering::SeqCst) {
                state_machine.cancel();
                break;
            }
        }

        let now = Instant::now();
        if now >= deadline {
            state_machine.check_timeout(timeout, "restore_event_loop");
            break;
        }

        let remaining = deadline - now;
        let event = match tokio::time::timeout(remaining, client.next_lifecycle_event()).await {
            Ok(Ok(event)) => event,
            Ok(Err(err)) => {
                state_machine.state =
                    RestoreState::Failed(RestoreFailure::ProtocolError(err.to_string()));
                break;
            }
            Err(_) => {
                state_machine.check_timeout(timeout, "restore_event_timeout");
                break;
            }
        };

        if let RestoreLifecycleEvent::DataRequest {
            data_type: Some(ref dt),
            data_port,
            ..
        } = event
        {
            match data_handler(dt, data_port) {
                Ok(payload) => {
                    let _ = state_machine.prepare_chunked_firmware(
                        IndexMap::new(),
                        &payload,
                        64 * 1024,
                    )?;
                }
                Err(err) => {
                    state_machine.state = RestoreState::Failed(RestoreFailure::MissingComponent {
                        component: format!("{dt}: {err}"),
                    });
                    break;
                }
            }
        }

        state_machine.process_event(&event)?;
    }

    Ok(state_machine.generate_report())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_machine_transitions_through_normal_update_flow() {
        let mut sm = RestoreStateMachine::new();
        assert_eq!(*sm.state(), RestoreState::Idle);

        sm.start_mode_transition().unwrap();
        assert_eq!(*sm.state(), RestoreState::ModeTransition);

        sm.transition_to_recovery().unwrap();
        assert_eq!(*sm.state(), RestoreState::QueryNonces);

        sm.receive_nonces(Some(vec![0xAA, 0xBB]), Some(vec![0xCC, 0xDD]))
            .unwrap();
        assert_eq!(*sm.state(), RestoreState::TssRequest);

        sm.store_tss_ticket("AP", vec![1, 2, 3], Some(&[0xAA, 0xBB]))
            .unwrap();
        sm.finish_tss_phase().unwrap();
        assert_eq!(*sm.state(), RestoreState::FirmwareTransfer);

        let data_req = RestoreLifecycleEvent::DataRequest {
            data_type: Some("SystemImageData".into()),
            data_port: Some(12345),
            async_request: false,
            raw: IndexMap::new(),
        };
        sm.process_event(&data_req).unwrap();

        let progress1 = RestoreLifecycleEvent::Progress {
            operation: Some("Flashing NOR Firmware".into()),
            progress: Some(45),
        };
        sm.process_event(&progress1).unwrap();
        assert_eq!(*sm.state(), RestoreState::FlashingUpdate);

        let progress2 = RestoreLifecycleEvent::Progress {
            operation: Some("Verifying Root Filesystem".into()),
            progress: Some(85),
        };
        sm.process_event(&progress2).unwrap();
        assert_eq!(*sm.state(), RestoreState::Verifying);

        let status_ok = RestoreLifecycleEvent::Status {
            code: 0,
            message: Some("success".into()),
            log: Some("restore completed".into()),
            finished: true,
        };
        sm.process_event(&status_ok).unwrap();
        assert_eq!(*sm.state(), RestoreState::Completed);

        let report = sm.generate_report();
        assert_eq!(report.final_state, RestoreState::Completed);
        assert_eq!(report.progress_percent, Some(85));
        assert_eq!(report.late_messages_count, 0);
        assert_eq!(report.data_requests_served, ["SystemImageData"]);
    }

    #[test]
    fn state_machine_detects_nonce_drift() {
        let mut sm = RestoreStateMachine::new();
        sm.transition_to_recovery().unwrap();
        sm.receive_nonces(Some(vec![0x11, 0x22]), None).unwrap();

        let err = sm
            .store_tss_ticket("AP", vec![1, 2], Some(&[0x99, 0x88]))
            .unwrap_err();
        assert!(err.to_string().contains("nonce drift detected"));
        assert_eq!(sm.nonce_changes_detected, 1);
        assert!(matches!(
            sm.state(),
            RestoreState::Failed(RestoreFailure::NonceMismatch { .. })
        ));
    }

    #[test]
    fn state_machine_enforces_firmware_chunking_and_transfer_limits() {
        let mut sm = RestoreStateMachine::new();
        let payload = vec![0x42; 600 * 1024]; // 600 KiB

        let chunks = sm
            .prepare_chunked_firmware(IndexMap::new(), &payload, 128 * 1024)
            .unwrap();
        // 1 header + 5 data chunks (4 * 128KB + 1 * 88KB) + 1 done = 7 messages
        assert_eq!(chunks.len(), 7);
        assert_eq!(sm.total_data_bytes_transferred, 600 * 1024);

        // Test safety transfer limit
        sm.max_data_transfer_limit = 1024 * 1024; // 1 MiB limit
        let err = sm
            .prepare_chunked_firmware(IndexMap::new(), &payload, 128 * 1024)
            .unwrap_err();
        assert!(err.to_string().contains("safety limit"));
    }

    #[test]
    fn state_machine_records_late_messages_after_completed() {
        let mut sm = RestoreStateMachine::new();
        sm.process_event(&RestoreLifecycleEvent::Status {
            code: 0,
            message: Some("success".into()),
            log: None,
            finished: true,
        })
        .unwrap();
        assert_eq!(*sm.state(), RestoreState::Completed);

        let late_progress = RestoreLifecycleEvent::Progress {
            operation: Some("Late Cleanup".into()),
            progress: Some(100),
        };
        let outcome = sm.process_event(&late_progress).unwrap();
        assert_eq!(outcome, RestoreStepOutcome::LateMessageObserved);
        assert_eq!(sm.late_messages_count, 1);
        assert_eq!(*sm.state(), RestoreState::Completed);
    }

    #[test]
    fn state_machine_handles_crash_and_status_failures() {
        let mut sm = RestoreStateMachine::new();
        let crash = RestoreLifecycleEvent::RestoredCrash {
            backtrace: vec!["frame 0: panic in restored".into()],
        };
        sm.process_event(&crash).unwrap();
        assert_eq!(
            *sm.state(),
            RestoreState::Failed(RestoreFailure::Crash {
                backtrace: vec!["frame 0: panic in restored".into()]
            })
        );

        let mut sm2 = RestoreStateMachine::new();
        let status_err = RestoreLifecycleEvent::Status {
            code: 27,
            message: Some("failed to mount filesystems".into()),
            log: Some("disk corruption".into()),
            finished: false,
        };
        sm2.process_event(&status_err).unwrap();
        assert_eq!(
            *sm2.state(),
            RestoreState::Failed(RestoreFailure::StatusCode {
                code: 27,
                message: Some("failed to mount filesystems".into()),
                log: Some("disk corruption".into()),
            })
        );
    }

    #[test]
    fn state_machine_handles_timeout_and_cancellation() {
        let mut sm = RestoreStateMachine::new();
        sm.cancel();
        assert_eq!(*sm.state(), RestoreState::Failed(RestoreFailure::Cancelled));

        let mut sm2 = RestoreStateMachine::new();
        let timed_out = sm2.check_timeout(Duration::from_millis(0), "query_nonces");
        assert!(timed_out);
        assert!(matches!(
            sm2.state(),
            RestoreState::Failed(RestoreFailure::Timeout { .. })
        ));
    }

    #[test]
    fn protocol_gaps_inventory_is_complete_and_descriptive() {
        let gaps = RestoreProtocolGaps::default();
        assert!(!gaps.tss_live_signing.is_empty());
        assert!(!gaps.erase_fdr_proxy.is_empty());
        assert!(!gaps.dfu_bootstrap.is_empty());
        assert!(!gaps.wireless_tunnel_drop.is_empty());
        assert!(!gaps.unknown_chipset_prefixes.is_empty());
    }
}

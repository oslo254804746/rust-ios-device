//! Recovery / DFU mode transition, personalized ramdisk boot plan, and USB re-enumeration handoff.
//!
//! Aligned with upstream `pymobiledevice3/restore/recovery.py`:
//! - Transitions from Normal Lockdown mode (`enter_recovery`) or DFU stage-1 (`iBSS` -> `iBEC` -> `go`)
//!   into Recovery iBoot (`0x1281`).
//! - Configures `auto-boot=false` + `saveenv` (skipped on macOS variants with `build_major < 20`).
//! - Personalizes and uploads `RestoreLogo` (`setpicture 4`, `bgcolor 0 0 0`), iBoot-loaded
//!   components (`IsLoadedByiBoot && !IsLoadedByiBootStage1`, `firmware`), `RestoreRamDisk`
//!   (`ramdisk`), `RestoreDeviceTree` (`devicetree`), optional `RestoreSEP` (`rsepfirmware`), and
//!   `RestoreKernelCache` (`bootx`).
//! - Re-enumerates into `restored_external` (`com.apple.mobile.restored`) and hands off the connected
//!   `RestoredClient` transport for the subsequent restore state machine.

use std::future::Future;

use super::img4::{stitch_img4_component, StitchedImg4Component};
use super::RestoreError;

/// Device enumeration mode observed across the Recovery / Ramdisk boot pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecoveryDeviceMode {
    /// Normal Lockdown / RSD mode before entering recovery.
    NormalLockdown,
    /// USB DFU mode (`0x1227`) requiring stage-1 iBSS/iBEC bootstrap.
    Dfu,
    /// Recovery iBoot mode (`0x1281`) accepting env vars, IMG4 components, and `bootx`.
    RecoveryIboot,
    /// Booted restore ramdisk exposing `com.apple.mobile.restored` (`restored_external`).
    RestoredExternal,
}

/// Ordered step in a Recovery / Ramdisk boot plan.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecoveryBootStep {
    /// Request normal-mode service to enter recovery and wait for Recovery iBoot enumeration.
    EnterRecoveryAndReenumerate,
    /// Upload DFU stage-1 bootloader (`iBSS` / `iBEC`) and re-enumerate into Recovery iBoot.
    BootstrapDfuComponent {
        component: String,
        fourcc: String,
        command: Option<String>,
        bytes: usize,
    },
    /// Set an iBoot environment variable (`setenv <name> <value>`) and optionally `saveenv`.
    SetEnv {
        name: String,
        value: String,
        save: bool,
    },
    /// Upload a personalized IMG4 component to Recovery iBoot and issue post-upload command(s).
    UploadComponent {
        component: String,
        fourcc: String,
        bytes: usize,
        commands: Vec<String>,
    },
    /// Wait for the restore kernel/ramdisk (`bootx`) to re-enumerate as `restored_external`.
    WaitForRestoredExternal { ecid: u64 },
}

/// Stitched IMG4 payload and associated iBoot commands for one step in the ramdisk boot plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRecoveryUpload {
    pub stitched: StitchedImg4Component,
    pub commands: Vec<String>,
    pub stage1_dfu: bool,
}

/// Complete, deterministic Recovery / Ramdisk boot plan constructed from an IPSW `BuildIdentity`
/// and TSS response before touching any recovery transport.
#[derive(Debug, Clone)]
pub struct RecoveryRamdiskPlan {
    pub initial_mode: RecoveryDeviceMode,
    pub ecid: u64,
    pub set_auto_boot_false: bool,
    pub uploads: Vec<PreparedRecoveryUpload>,
    pub steps: Vec<RecoveryBootStep>,
}

/// Summary receipt produced after executing a `RecoveryRamdiskPlan` over a `RecoveryTransport`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecoveryHandoffReceipt {
    pub initial_mode: RecoveryDeviceMode,
    pub final_mode: RecoveryDeviceMode,
    pub ecid: u64,
    pub reenumerations_completed: usize,
    pub uploaded_components: Vec<String>,
    pub executed_commands: Vec<String>,
    pub total_img4_bytes_sent: usize,
}

fn manifest_component_path<'a>(identity: &'a plist::Dictionary, name: &str) -> Option<&'a str> {
    identity
        .get("Manifest")
        .and_then(plist::Value::as_dictionary)
        .and_then(|m| m.get(name))
        .and_then(plist::Value::as_dictionary)
        .and_then(|c| c.get("Info"))
        .and_then(plist::Value::as_dictionary)
        .and_then(|i| i.get("Path"))
        .and_then(plist::Value::as_string)
        .filter(|p| !p.is_empty())
}

fn i_boot_loaded_components(identity: &plist::Dictionary) -> Vec<(String, String)> {
    let Some(manifest) = identity
        .get("Manifest")
        .and_then(plist::Value::as_dictionary)
    else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for (name, entry) in manifest {
        let Some(info) = entry
            .as_dictionary()
            .and_then(|d| d.get("Info"))
            .and_then(plist::Value::as_dictionary)
        else {
            continue;
        };
        let loaded_by_iboot = info
            .get("IsLoadedByiBoot")
            .and_then(plist::Value::as_boolean)
            .unwrap_or(false);
        let stage1 = info
            .get("IsLoadedByiBootStage1")
            .and_then(plist::Value::as_boolean)
            .unwrap_or(false);
        if loaded_by_iboot && !stage1 {
            if let Some(path) = info
                .get("Path")
                .and_then(plist::Value::as_string)
                .filter(|p| !p.is_empty())
            {
                items.push((name.clone(), path.to_owned()));
            }
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    items
}

/// Build and personalize a complete `RecoveryRamdiskPlan` aligned with
/// `Recovery.boot_ramdisk` in `pymobiledevice3/restore/recovery.py`.
pub fn build_recovery_ramdisk_plan<F>(
    initial_mode: RecoveryDeviceMode,
    ecid: u64,
    identity: &plist::Dictionary,
    ap_img4_ticket: &[u8],
    tss_response: Option<&plist::Dictionary>,
    boot_nonce: Option<&[u8]>,
    mut read_component: F,
) -> Result<RecoveryRamdiskPlan, RestoreError>
where
    F: FnMut(&str, &str) -> Result<Vec<u8>, RestoreError>,
{
    if ap_img4_ticket.is_empty() {
        return Err(RestoreError::Protocol(
            "cannot build recovery ramdisk plan without ApImg4Ticket".into(),
        ));
    }

    let variant = identity
        .get("Info")
        .and_then(plist::Value::as_dictionary)
        .and_then(|info| info.get("Variant"))
        .and_then(plist::Value::as_string)
        .unwrap_or_default();
    let macos_variant = variant.contains("macOS");
    let build_major = identity
        .get("ProductBuildVersion")
        .or_else(|| {
            identity
                .get("Info")
                .and_then(plist::Value::as_dictionary)
                .and_then(|info| info.get("BuildNumber"))
        })
        .and_then(plist::Value::as_string)
        .and_then(|b| {
            let digits: String = b.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse::<u64>().ok()
        })
        .unwrap_or(21);

    let mut uploads = Vec::new();
    let mut steps = Vec::new();

    match initial_mode {
        RecoveryDeviceMode::NormalLockdown => {
            steps.push(RecoveryBootStep::EnterRecoveryAndReenumerate);
        }
        RecoveryDeviceMode::Dfu => {
            for (comp, post_cmd) in [("iBSS", None), ("iBEC", Some("go".to_owned()))] {
                if let Some(path) = manifest_component_path(identity, comp) {
                    let raw = read_component(comp, path)?;
                    let stitched = stitch_img4_component(
                        comp,
                        &raw,
                        ap_img4_ticket,
                        tss_response,
                        boot_nonce,
                        None,
                    )?;
                    steps.push(RecoveryBootStep::BootstrapDfuComponent {
                        component: comp.to_owned(),
                        fourcc: stitched.fourcc_str(),
                        command: post_cmd.clone(),
                        bytes: stitched.img4_bytes.len(),
                    });
                    uploads.push(PreparedRecoveryUpload {
                        stitched,
                        commands: post_cmd.into_iter().collect(),
                        stage1_dfu: true,
                    });
                }
            }
        }
        RecoveryDeviceMode::RecoveryIboot => {}
        RecoveryDeviceMode::RestoredExternal => {
            return Err(RestoreError::Protocol(
                "device is already in RestoredExternal mode".into(),
            ));
        }
    }

    let set_auto_boot_false = !macos_variant || build_major >= 20;
    if set_auto_boot_false {
        steps.push(RecoveryBootStep::SetEnv {
            name: "auto-boot".into(),
            value: "false".into(),
            save: true,
        });
    }

    // 1. RestoreLogo (optional if absent in Manifest)
    if let Some(path) = manifest_component_path(identity, "RestoreLogo") {
        let raw = read_component("RestoreLogo", path)?;
        let stitched = stitch_img4_component(
            "RestoreLogo",
            &raw,
            ap_img4_ticket,
            tss_response,
            boot_nonce,
            None,
        )?;
        let commands = vec!["setpicture 4".to_owned(), "bgcolor 0 0 0".to_owned()];
        steps.push(RecoveryBootStep::UploadComponent {
            component: "RestoreLogo".into(),
            fourcc: stitched.fourcc_str(),
            bytes: stitched.img4_bytes.len(),
            commands: commands.clone(),
        });
        uploads.push(PreparedRecoveryUpload {
            stitched,
            commands,
            stage1_dfu: false,
        });
    }

    // 2. Components with IsLoadedByiBoot && !IsLoadedByiBootStage1
    for (comp_name, path) in i_boot_loaded_components(identity) {
        let raw = read_component(&comp_name, &path)?;
        let stitched = stitch_img4_component(
            &comp_name,
            &raw,
            ap_img4_ticket,
            tss_response,
            boot_nonce,
            None,
        )?;
        let commands = vec!["firmware".to_owned()];
        steps.push(RecoveryBootStep::UploadComponent {
            component: comp_name,
            fourcc: stitched.fourcc_str(),
            bytes: stitched.img4_bytes.len(),
            commands: commands.clone(),
        });
        uploads.push(PreparedRecoveryUpload {
            stitched,
            commands,
            stage1_dfu: false,
        });
    }

    // 3. RestoreRamDisk (required)
    let ramdisk_path = manifest_component_path(identity, "RestoreRamDisk").ok_or_else(|| {
        RestoreError::Protocol("BuildIdentity missing RestoreRamDisk component".into())
    })?;
    let ramdisk_raw = read_component("RestoreRamDisk", ramdisk_path)?;
    let ramdisk_stitched = stitch_img4_component(
        "RestoreRamDisk",
        &ramdisk_raw,
        ap_img4_ticket,
        tss_response,
        boot_nonce,
        None,
    )?;
    let ramdisk_cmds = vec!["ramdisk".to_owned()];
    steps.push(RecoveryBootStep::UploadComponent {
        component: "RestoreRamDisk".into(),
        fourcc: ramdisk_stitched.fourcc_str(),
        bytes: ramdisk_stitched.img4_bytes.len(),
        commands: ramdisk_cmds.clone(),
    });
    uploads.push(PreparedRecoveryUpload {
        stitched: ramdisk_stitched,
        commands: ramdisk_cmds,
        stage1_dfu: false,
    });

    // 4. RestoreDeviceTree (required)
    let dtree_path = manifest_component_path(identity, "RestoreDeviceTree").ok_or_else(|| {
        RestoreError::Protocol("BuildIdentity missing RestoreDeviceTree component".into())
    })?;
    let dtree_raw = read_component("RestoreDeviceTree", dtree_path)?;
    let dtree_stitched = stitch_img4_component(
        "RestoreDeviceTree",
        &dtree_raw,
        ap_img4_ticket,
        tss_response,
        boot_nonce,
        None,
    )?;
    let dtree_cmds = vec!["devicetree".to_owned()];
    steps.push(RecoveryBootStep::UploadComponent {
        component: "RestoreDeviceTree".into(),
        fourcc: dtree_stitched.fourcc_str(),
        bytes: dtree_stitched.img4_bytes.len(),
        commands: dtree_cmds.clone(),
    });
    uploads.push(PreparedRecoveryUpload {
        stitched: dtree_stitched,
        commands: dtree_cmds,
        stage1_dfu: false,
    });

    // 5. RestoreSEP (optional if absent or Path missing)
    if let Some(sep_path) = manifest_component_path(identity, "RestoreSEP") {
        let sep_raw = read_component("RestoreSEP", sep_path)?;
        let sep_stitched = stitch_img4_component(
            "RestoreSEP",
            &sep_raw,
            ap_img4_ticket,
            tss_response,
            boot_nonce,
            None,
        )?;
        let sep_cmds = vec!["rsepfirmware".to_owned()];
        steps.push(RecoveryBootStep::UploadComponent {
            component: "RestoreSEP".into(),
            fourcc: sep_stitched.fourcc_str(),
            bytes: sep_stitched.img4_bytes.len(),
            commands: sep_cmds.clone(),
        });
        uploads.push(PreparedRecoveryUpload {
            stitched: sep_stitched,
            commands: sep_cmds,
            stage1_dfu: false,
        });
    }

    // 6. RestoreKernelCache + bootx (required)
    let kcache_path = manifest_component_path(identity, "RestoreKernelCache").ok_or_else(|| {
        RestoreError::Protocol("BuildIdentity missing RestoreKernelCache component".into())
    })?;
    let kcache_raw = read_component("RestoreKernelCache", kcache_path)?;
    let kcache_stitched = stitch_img4_component(
        "RestoreKernelCache",
        &kcache_raw,
        ap_img4_ticket,
        tss_response,
        boot_nonce,
        None,
    )?;
    let kcache_cmds = vec!["bootx".to_owned()];
    steps.push(RecoveryBootStep::UploadComponent {
        component: "RestoreKernelCache".into(),
        fourcc: kcache_stitched.fourcc_str(),
        bytes: kcache_stitched.img4_bytes.len(),
        commands: kcache_cmds.clone(),
    });
    uploads.push(PreparedRecoveryUpload {
        stitched: kcache_stitched,
        commands: kcache_cmds,
        stage1_dfu: false,
    });

    steps.push(RecoveryBootStep::WaitForRestoredExternal { ecid });

    Ok(RecoveryRamdiskPlan {
        initial_mode,
        ecid,
        set_auto_boot_false,
        uploads,
        steps,
    })
}

/// Abstract recovery/iBoot transport and USB re-enumeration boundary.
pub trait RecoveryReenumerationTransport {
    type RestoredStream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin;

    fn enter_recovery_from_normal(&mut self) -> impl Future<Output = Result<(), RestoreError>>;
    fn wait_for_recovery_iboot(
        &mut self,
        ecid: u64,
    ) -> impl Future<Output = Result<(), RestoreError>>;
    fn send_iboot_buffer(
        &mut self,
        component: &str,
        img4_bytes: &[u8],
    ) -> impl Future<Output = Result<(), RestoreError>>;
    fn send_iboot_command(&mut self, cmd: &str) -> impl Future<Output = Result<(), RestoreError>>;
    fn wait_for_restored_external(
        &mut self,
        ecid: u64,
    ) -> impl Future<Output = Result<Self::RestoredStream, RestoreError>>;
}

/// Execute a `RecoveryRamdiskPlan` across a `RecoveryReenumerationTransport`, performing the
/// Normal/DFU -> Recovery iBoot -> Ramdisk upload -> `restored_external` re-enumeration handoff.
pub async fn execute_recovery_ramdisk_handoff<T: RecoveryReenumerationTransport>(
    plan: &RecoveryRamdiskPlan,
    transport: &mut T,
) -> Result<(T::RestoredStream, RecoveryHandoffReceipt), RestoreError> {
    let mut reenumerations_completed = 0usize;
    let mut uploaded_components = Vec::new();
    let mut executed_commands = Vec::new();
    let mut total_img4_bytes_sent = 0usize;

    match plan.initial_mode {
        RecoveryDeviceMode::NormalLockdown => {
            transport.enter_recovery_from_normal().await?;
            transport.wait_for_recovery_iboot(plan.ecid).await?;
            reenumerations_completed += 1;
        }
        RecoveryDeviceMode::Dfu => {
            for upload in plan.uploads.iter().filter(|u| u.stage1_dfu) {
                transport
                    .send_iboot_buffer(&upload.stitched.component_name, &upload.stitched.img4_bytes)
                    .await?;
                total_img4_bytes_sent += upload.stitched.img4_bytes.len();
                uploaded_components.push(upload.stitched.component_name.clone());
                for cmd in &upload.commands {
                    transport.send_iboot_command(cmd).await?;
                    executed_commands.push(cmd.clone());
                }
            }
            transport.wait_for_recovery_iboot(plan.ecid).await?;
            reenumerations_completed += 1;
        }
        RecoveryDeviceMode::RecoveryIboot => {}
        RecoveryDeviceMode::RestoredExternal => {
            return Err(RestoreError::Protocol(
                "device is already in RestoredExternal mode".into(),
            ));
        }
    }

    if plan.set_auto_boot_false {
        let setenv_cmd = "setenv auto-boot false";
        transport.send_iboot_command(setenv_cmd).await?;
        executed_commands.push(setenv_cmd.to_owned());
        let saveenv_cmd = "saveenv";
        transport.send_iboot_command(saveenv_cmd).await?;
        executed_commands.push(saveenv_cmd.to_owned());
    }

    for upload in plan.uploads.iter().filter(|u| !u.stage1_dfu) {
        transport
            .send_iboot_buffer(&upload.stitched.component_name, &upload.stitched.img4_bytes)
            .await?;
        total_img4_bytes_sent += upload.stitched.img4_bytes.len();
        uploaded_components.push(upload.stitched.component_name.clone());
        for cmd in &upload.commands {
            transport.send_iboot_command(cmd).await?;
            executed_commands.push(cmd.clone());
        }
    }

    let stream = transport.wait_for_restored_external(plan.ecid).await?;
    reenumerations_completed += 1;

    Ok((
        stream,
        RecoveryHandoffReceipt {
            initial_mode: plan.initial_mode,
            final_mode: RecoveryDeviceMode::RestoredExternal,
            ecid: plan.ecid,
            reenumerations_completed,
            uploaded_components,
            executed_commands,
            total_img4_bytes_sent,
        },
    ))
}

/// In-memory simulated recovery transport for deterministic offline testing of
/// Normal/DFU -> Recovery iBoot -> Ramdisk boot -> `restored_external` re-enumeration.
pub struct SimulatedRecoveryTransport<S> {
    pub mode: RecoveryDeviceMode,
    pub expected_ecid: u64,
    pub uploaded_buffers: Vec<(String, Vec<u8>)>,
    pub commands: Vec<String>,
    pub restored_stream: Option<S>,
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> SimulatedRecoveryTransport<S> {
    pub fn new(initial_mode: RecoveryDeviceMode, expected_ecid: u64, restored_stream: S) -> Self {
        Self {
            mode: initial_mode,
            expected_ecid,
            uploaded_buffers: Vec::new(),
            commands: Vec::new(),
            restored_stream: Some(restored_stream),
        }
    }
}

impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> RecoveryReenumerationTransport
    for SimulatedRecoveryTransport<S>
{
    type RestoredStream = S;

    async fn enter_recovery_from_normal(&mut self) -> Result<(), RestoreError> {
        if self.mode != RecoveryDeviceMode::NormalLockdown {
            return Err(RestoreError::Protocol(
                "simulated device is not in NormalLockdown mode".into(),
            ));
        }
        Ok(())
    }

    async fn wait_for_recovery_iboot(&mut self, ecid: u64) -> Result<(), RestoreError> {
        if ecid != self.expected_ecid {
            return Err(RestoreError::Protocol(
                "simulated recovery iBoot ECID mismatch".into(),
            ));
        }
        self.mode = RecoveryDeviceMode::RecoveryIboot;
        Ok(())
    }

    async fn send_iboot_buffer(
        &mut self,
        component: &str,
        img4_bytes: &[u8],
    ) -> Result<(), RestoreError> {
        let parsed = super::img4::parse_img4_envelope(img4_bytes)?;
        if parsed.im4m_ticket.is_empty() {
            return Err(RestoreError::Protocol(format!(
                "component {component} missing IM4M ticket"
            )));
        }
        self.uploaded_buffers
            .push((component.to_owned(), img4_bytes.to_vec()));
        Ok(())
    }

    async fn send_iboot_command(&mut self, cmd: &str) -> Result<(), RestoreError> {
        if self.mode != RecoveryDeviceMode::RecoveryIboot && self.mode != RecoveryDeviceMode::Dfu {
            return Err(RestoreError::Protocol(format!(
                "cannot execute iBoot command '{cmd}' in mode {:?}",
                self.mode
            )));
        }
        self.commands.push(cmd.to_owned());
        Ok(())
    }

    async fn wait_for_restored_external(
        &mut self,
        ecid: u64,
    ) -> Result<Self::RestoredStream, RestoreError> {
        if ecid != self.expected_ecid {
            return Err(RestoreError::Protocol(
                "simulated restored_external ECID mismatch".into(),
            ));
        }
        if self.commands.last().map(String::as_str) != Some("bootx") {
            return Err(RestoreError::Protocol(
                "restored_external enumeration requires prior bootx command".into(),
            ));
        }
        self.mode = RecoveryDeviceMode::RestoredExternal;
        self.restored_stream
            .take()
            .ok_or_else(|| RestoreError::Protocol("simulated restored stream already taken".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_identity() -> plist::Dictionary {
        let comp = |path: &str, iboot: bool, stage1: bool| {
            let mut info = plist::Dictionary::from_iter([(
                "Path".to_owned(),
                plist::Value::String(path.into()),
            )]);
            if iboot {
                info.insert("IsLoadedByiBoot".into(), plist::Value::Boolean(true));
            }
            if stage1 {
                info.insert("IsLoadedByiBootStage1".into(), plist::Value::Boolean(true));
            }
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Info".to_owned(),
                plist::Value::Dictionary(info),
            )]))
        };
        plist::Dictionary::from_iter([
            ("ProductBuildVersion", plist::Value::String("21F90".into())),
            (
                "Info",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("DeviceClass", plist::Value::String("d63ap".into())),
                    ("RestoreBehavior", plist::Value::String("Update".into())),
                    (
                        "Variant",
                        plist::Value::String("Upgrade Install (IPSW)".into()),
                    ),
                ])),
            ),
            (
                "Manifest",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("iBSS", comp("Firmware/dfu/iBSS.im4p", true, true)),
                    ("iBEC", comp("Firmware/dfu/iBEC.im4p", true, true)),
                    (
                        "RestoreLogo",
                        comp("Firmware/all_flash/applelogo.im4p", false, false),
                    ),
                    ("AOP", comp("Firmware/AOP/aop.im4p", true, false)),
                    ("RestoreRamDisk", comp("090-12345.dmg", false, false)),
                    (
                        "RestoreDeviceTree",
                        comp("Firmware/all_flash/DeviceTree.d63ap.im4p", false, false),
                    ),
                    (
                        "RestoreSEP",
                        comp("Firmware/all_flash/sep-firmware.d63ap.im4p", false, false),
                    ),
                    (
                        "RestoreKernelCache",
                        comp("kernelcache.release.iphone14", false, false),
                    ),
                ])),
            ),
        ])
    }

    #[tokio::test]
    async fn normal_and_dfu_modes_boot_ramdisk_and_reenumerate_to_restored_external() {
        for initial_mode in [RecoveryDeviceMode::NormalLockdown, RecoveryDeviceMode::Dfu] {
            let identity = sample_identity();
            let plan = build_recovery_ramdisk_plan(
                initial_mode,
                0x1122_3344,
                &identity,
                b"AP_IMG4_TICKET_BYTES",
                None,
                Some(&[0x01, 0x02, 0x03, 0x04]),
                |name, _path| Ok(format!("RAW_{name}").into_bytes()),
            )
            .unwrap();

            let (client_stream, _peer_stream) = tokio::io::duplex(1024);
            let mut transport =
                SimulatedRecoveryTransport::new(initial_mode, 0x1122_3344, client_stream);
            let (_stream, receipt) = execute_recovery_ramdisk_handoff(&plan, &mut transport)
                .await
                .unwrap();

            assert_eq!(receipt.final_mode, RecoveryDeviceMode::RestoredExternal);
            assert_eq!(receipt.reenumerations_completed, 2);
            assert!(receipt
                .executed_commands
                .contains(&"setenv auto-boot false".to_string()));
            assert!(receipt.executed_commands.contains(&"saveenv".to_string()));
            assert!(receipt.executed_commands.contains(&"ramdisk".to_string()));
            assert!(receipt
                .executed_commands
                .contains(&"devicetree".to_string()));
            assert!(receipt
                .executed_commands
                .contains(&"rsepfirmware".to_string()));
            assert_eq!(
                receipt.executed_commands.last().map(String::as_str),
                Some("bootx")
            );
            if initial_mode == RecoveryDeviceMode::Dfu {
                assert!(receipt
                    .uploaded_components
                    .starts_with(&["iBSS".to_string(), "iBEC".to_string(),]));
            }
        }
    }
}

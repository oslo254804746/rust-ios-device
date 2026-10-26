//! Device-side preflight request construction for normal-mode updater inspection.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};

use bytes::Bytes;
use indexmap::IndexMap;

use super::error_mapping::{native_error, native_io, native_zip};
use super::RestoreError;
use crate::xpc::XpcValue;

#[cfg(test)]
mod native_source_contract_tests;

const MAX_PREFLIGHT_FIRMWARE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PLIST_DEPTH: usize = 64;
const MAX_BUILD_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_IPSW_ENTRIES: usize = 100_000;

/// Device identifiers and requested restore behavior used to select one IPSW identity.
/// All four fields must match; a missing or ambiguous match is an error.
#[derive(Debug, Clone, Copy)]
pub struct IpswIdentitySelector<'a> {
    pub board_id: u64,
    pub chip_id: u64,
    pub device_class: &'a str,
    pub restore_behavior: &'a str,
}

/// A bounded IPSW reader with one unambiguous build identity selected up front.
pub struct IpswPreflight<R: Read + Seek> {
    archive: zip::ZipArchive<R>,
    identity: plist::Dictionary,
    product_version: Option<String>,
    product_build_version: Option<String>,
}

impl<R: Read + Seek> IpswPreflight<R> {
    pub fn open(reader: R, selector: IpswIdentitySelector<'_>) -> Result<Self, RestoreError> {
        if !matches!(selector.restore_behavior, "Update" | "Erase")
            || selector.device_class.is_empty()
        {
            return Err(RestoreError::Protocol(
                "invalid IPSW identity selector".into(),
            ));
        }
        let mut archive = zip::ZipArchive::new(reader).map_err(|e| native_zip("open IPSW", e))?;
        if archive.len() > MAX_IPSW_ENTRIES {
            return Err(RestoreError::Protocol(
                "IPSW has too many ZIP entries".into(),
            ));
        }
        let manifest = read_zip_member(
            &mut archive,
            "BuildManifest.plist",
            MAX_BUILD_MANIFEST_BYTES,
        )?;
        let root = plist::from_bytes::<plist::Value>(&manifest).map_err(|e| {
            native_error(
                std::io::ErrorKind::InvalidInput,
                "parse IPSW BuildManifest.plist",
                e,
            )
        })?;
        let root_dict = root.as_dictionary().ok_or_else(|| {
            RestoreError::Protocol("BuildManifest missing root dictionary".into())
        })?;
        let product_version = root_dict
            .get("ProductVersion")
            .and_then(plist::Value::as_string)
            .map(ToString::to_string);
        let product_build_version = root_dict
            .get("ProductBuildVersion")
            .and_then(plist::Value::as_string)
            .map(ToString::to_string);
        let identities = root_dict
            .get("BuildIdentities")
            .and_then(plist::Value::as_array)
            .ok_or_else(|| {
                RestoreError::Protocol("BuildManifest missing BuildIdentities array".into())
            })?;
        if identities.len() > 256 {
            return Err(RestoreError::Protocol(
                "too many IPSW build identities".into(),
            ));
        }
        let matches = identities
            .iter()
            .filter_map(plist::Value::as_dictionary)
            .filter(|identity| {
                let info = identity.get("Info").and_then(plist::Value::as_dictionary);
                plist_u64(identity, "ApBoardID") == Some(selector.board_id)
                    && plist_u64(identity, "ApChipID") == Some(selector.chip_id)
                    && info
                        .and_then(|info| info.get("DeviceClass"))
                        .and_then(plist::Value::as_string)
                        == Some(selector.device_class)
                    && info
                        .and_then(|info| info.get("RestoreBehavior"))
                        .and_then(plist::Value::as_string)
                        == Some(selector.restore_behavior)
            })
            .collect::<Vec<_>>();
        let preferred_variant = match selector.restore_behavior {
            "Update" => "Upgrade Install (IPSW)",
            _ => "Erase Install (IPSW)",
        };
        let preferred = matches
            .iter()
            .copied()
            .filter(|identity| {
                identity
                    .get("Info")
                    .and_then(plist::Value::as_dictionary)
                    .and_then(|info| info.get("Variant"))
                    .and_then(plist::Value::as_string)
                    == Some(preferred_variant)
            })
            .collect::<Vec<_>>();
        let candidates = if preferred.is_empty() {
            &matches
        } else {
            &preferred
        };
        if candidates.len() != 1 {
            return Err(RestoreError::Protocol(format!(
                "IPSW identity selection found {} candidates for board={:#x}, chip={:#x}, class={}, behavior={}",
                candidates.len(), selector.board_id, selector.chip_id, selector.device_class,
                selector.restore_behavior
            )));
        }
        Ok(Self {
            archive,
            identity: (*candidates[0]).clone(),
            product_version,
            product_build_version,
        })
    }

    pub fn identity(&self) -> &plist::Dictionary {
        &self.identity
    }

    /// Extracted ProductVersion from BuildManifest.plist when present (e.g. "17.5.1").
    pub fn product_version(&self) -> Option<&str> {
        self.product_version.as_deref()
    }

    /// Extracted ProductBuildVersion from BuildManifest.plist when present (e.g. "21F90").
    pub fn product_build_version(&self) -> Option<&str> {
        self.product_build_version.as_deref()
    }

    /// Validate only the shape of observed current-device version strings.
    /// This does not compare current and IPSW versions or establish an allowed
    /// upgrade/downgrade path, signing availability, or machine/build support.
    pub fn verify_target_version(
        &self,
        device_version: Option<&str>,
        device_build: Option<&str>,
    ) -> Result<(), RestoreError> {
        if let Some(device_ver) = device_version {
            if device_ver.trim().is_empty() {
                return Err(RestoreError::Protocol(
                    "target device returned empty ProductVersion".into(),
                ));
            }
        }
        if let Some(device_bld) = device_build {
            if device_bld.trim().is_empty() {
                return Err(RestoreError::Protocol(
                    "target device returned empty BuildVersion".into(),
                ));
            }
        }
        Ok(())
    }

    /// Read every file referenced by the selected identity to EOF. ZIP CRC,
    /// exact uncompressed size, names and aggregate bytes are checked without
    /// retaining whole system images. This checks archive integrity, not Apple's
    /// signatures or the undocumented meaning of every Manifest Digest field.
    pub fn validate_components(
        &mut self,
        max_total: u64,
    ) -> Result<Vec<ComponentInspection>, RestoreError> {
        use sha2::{Digest, Sha256};
        let manifest = self
            .identity
            .get("Manifest")
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| RestoreError::Protocol("identity missing Manifest".into()))?;
        let mut requested = std::collections::BTreeSet::new();
        for (name, component) in manifest {
            let path = component
                .as_dictionary()
                .and_then(|c| c.get("Info"))
                .and_then(plist::Value::as_dictionary)
                .and_then(|i| i.get("Path"))
                .and_then(plist::Value::as_string)
                .ok_or_else(|| {
                    RestoreError::Protocol(format!("component {name} missing Info.Path"))
                })?;
            if !safe_ipsw_path(path) {
                return Err(RestoreError::Protocol("unsafe component path".into()));
            }
            requested.insert(path.to_owned());
        }
        if requested.is_empty() {
            return Err(RestoreError::Protocol("identity has no components".into()));
        }
        let mut entries = HashMap::new();
        for index in 0..self.archive.len() {
            let file = self
                .archive
                .by_index_raw(index)
                .map_err(|e| native_zip("inspect component ZIP entry", e))?;
            if entries.insert(file.name().to_owned(), index).is_some() {
                return Err(RestoreError::Protocol("duplicate IPSW ZIP member".into()));
            }
        }
        let mut total = 0u64;
        let mut results = Vec::new();
        for path in requested {
            let index = entries
                .get(&path)
                .ok_or_else(|| RestoreError::Protocol(format!("missing component {path}")))?;
            let mut file = self
                .archive
                .by_index(*index)
                .map_err(|e| native_zip("open component", e))?;
            let expected = file.size();
            total = total
                .checked_add(expected)
                .filter(|n| *n <= max_total)
                .ok_or_else(|| {
                    RestoreError::Protocol("IPSW component byte budget exceeded".into())
                })?;
            if file.is_dir()
                || file
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(RestoreError::Protocol(
                    "component must be a regular ZIP file".into(),
                ));
            }
            let mut read = 0u64;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = file
                    .read(&mut buffer)
                    .map_err(|e| native_io("read component and verify ZIP CRC", e))?;
                if count == 0 {
                    break;
                }
                read = read
                    .checked_add(count as u64)
                    .filter(|n| *n <= expected)
                    .ok_or_else(|| {
                        RestoreError::Protocol("component exceeds advertised size".into())
                    })?;
                hash.update(&buffer[..count]);
            }
            if read != expected {
                return Err(RestoreError::Protocol("component size mismatch".into()));
            }
            results.push(ComponentInspection {
                path,
                bytes: read,
                sha256: hex::encode(hash.finalize()),
            });
        }
        Ok(results)
    }

    /// Build a payload for one updater. The service uses a fresh connection per updater.
    pub fn payload_for_updater(&mut self, updater: &str) -> Result<XpcValue, RestoreError> {
        build_device_side_preflight_payload(&self.identity, updater, |path| {
            read_zip_member(&mut self.archive, path, MAX_PREFLIGHT_FIRMWARE_BYTES)
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ComponentInspection {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Correlate a saved request/response envelope with the current hardware and
/// nonce. This is metadata correlation only; Apple ticket signature and IMG4
/// personalization are not implemented here. A raw ticket with no saved request
/// cannot establish correlation and is rejected.
pub fn inspect_saved_tss(
    envelope: &plist::Value,
    ecid: u64,
    board: u64,
    chip: u64,
    ap_nonce: &[u8],
    sep_nonce: Option<&[u8]>,
) -> Result<(), RestoreError> {
    let root = envelope
        .as_dictionary()
        .ok_or_else(|| RestoreError::Protocol("saved TSS envelope must be a dictionary".into()))?;
    let request = root.get("Request").and_then(plist::Value::as_dictionary)
        .ok_or_else(|| RestoreError::Protocol("saved TSS must include its original Request; raw tickets cannot establish nonce/device association".into()))?;
    let response = root
        .get("Response")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| RestoreError::Protocol("saved TSS envelope missing Response".into()))?;
    if ap_nonce.is_empty()
        || plist_u64(request, "ApECID") != Some(ecid)
        || plist_u64(request, "ApBoardID") != Some(board)
        || plist_u64(request, "ApChipID") != Some(chip)
        || request.get("ApNonce").and_then(plist::Value::as_data) != Some(ap_nonce)
        || sep_nonce.is_some_and(|nonce| {
            request.get("SepNonce").and_then(plist::Value::as_data) != Some(nonce)
        })
    {
        return Err(RestoreError::Protocol(
            "saved TSS request does not match current device/nonces".into(),
        ));
    }
    if !response
        .get("ApImg4Ticket")
        .and_then(plist::Value::as_data)
        .is_some_and(|ticket| !ticket.is_empty())
    {
        return Err(RestoreError::Protocol(
            "saved TSS response missing AP IMG4 ticket".into(),
        ));
    }
    Ok(())
}

fn plist_u64(dict: &plist::Dictionary, key: &str) -> Option<u64> {
    match dict.get(key)? {
        plist::Value::Integer(value) => value.as_unsigned(),
        plist::Value::String(value) => {
            let value = value.trim();
            if let Some(hex) = value
                .strip_prefix("0x")
                .or_else(|| value.strip_prefix("0X"))
            {
                u64::from_str_radix(hex, 16).ok()
            } else {
                value.parse().ok()
            }
        }
        _ => None,
    }
}

fn read_zip_member<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &str,
    limit: usize,
) -> Result<Vec<u8>, RestoreError> {
    if !safe_ipsw_path(path) {
        return Err(RestoreError::Protocol("unsafe IPSW member path".into()));
    }
    let mut found = None;
    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index_raw(index)
            .map_err(|e| native_zip("inspect IPSW ZIP entry", e))?;
        if !seen.insert(file.name().to_string()) {
            return Err(RestoreError::Protocol("duplicate IPSW ZIP member".into()));
        }
        if file.name() == path {
            found = Some(index);
        }
    }
    let index =
        found.ok_or_else(|| RestoreError::Protocol(format!("missing IPSW member {path}")))?;
    let file = archive
        .by_index(index)
        .map_err(|e| native_zip("open IPSW member", e))?;
    if file.is_dir() || file.size() > limit as u64 {
        return Err(RestoreError::Protocol(format!(
            "IPSW member {path} exceeds size limit"
        )));
    }
    let mut bytes = Vec::with_capacity(file.size() as usize);
    let read_limit = (limit as u64).saturating_add(1);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|e| native_io("read IPSW member", e))?;
    if bytes.len() > limit {
        return Err(RestoreError::Protocol(format!(
            "IPSW member {path} exceeds size limit"
        )));
    }
    Ok(bytes)
}

/// Updater names accepted by the device-side preflight payload builder.
pub const UPDATERS: &[&str] = &[
    "T200", "Rose", "SE", "Vinyl", "Savage", "Centauri", "Baseband",
];

/// Default updaters queried during normal-mode preflight.
/// Baseband is excluded by default because normal-mode preflight fails on-device and its firmware is ~200 MB.
pub const DEFAULT_PREFLIGHT_UPDATERS: &[&str] =
    &["T200", "Rose", "SE", "Vinyl", "Savage", "Centauri"];

/// Record of an updater that failed during device-side preflight collection.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PreflightUpdaterFailure {
    pub updater: String,
    pub error: String,
}

/// Merged result of device-side preflight collection across updaters.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CollectedPreflightRequests {
    pub device_info: std::collections::BTreeMap<String, serde_json::Value>,
    pub device_info_tags: std::collections::BTreeMap<String, serde_json::Value>,
    pub device_info_requests: std::collections::BTreeMap<String, serde_json::Value>,
    pub device_info_failures: std::collections::BTreeMap<String, serde_json::Value>,
    pub ap_parameters: Option<serde_json::Value>,
    pub failed_updaters: Vec<PreflightUpdaterFailure>,
}

/// Typed causes retained independently of the serializable partial summary.
/// The legacy summary API remains available; new callers should use outcomes.
pub struct PreflightNativeFailure {
    pub updater: String,
    pub phase: &'static str,
    pub cause: RestoreError,
}

pub struct PreflightCollection {
    pub summary: CollectedPreflightRequests,
    pub failures: Vec<PreflightNativeFailure>,
}

impl std::fmt::Debug for PreflightNativeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreflightNativeFailure")
            .field("phase", &self.phase)
            .field("updater", &crate::redaction::diagnostic_id(&self.updater))
            .field("class", &crate::classify_error(&self.cause))
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PreflightCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreflightCollection")
            .field("failures", &self.failures)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for PreflightCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "preflight collection has {} failed updater operations",
            self.failures.len()
        )
    }
}
impl std::error::Error for PreflightCollection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.failures
            .first()
            .map(|failure| &failure.cause as &(dyn std::error::Error + 'static))
    }
}

fn retain_failure(
    collected: &mut CollectedPreflightRequests,
    failures: &mut Vec<PreflightNativeFailure>,
    updater: &str,
    phase: &'static str,
    cause: RestoreError,
) {
    collected.failed_updaters.push(PreflightUpdaterFailure {
        updater: updater.to_owned(),
        error: format!("{phase}: {}", crate::redaction::diagnostic_error(&cause)),
    });
    failures.push(PreflightNativeFailure {
        updater: updater.to_owned(),
        phase,
        cause,
    });
}

/// Ask the device to build the TSS requests of each peripheral updater for an IPSW.
///
/// Restoreserviced consumes and aborts the connection after each `getdevicesidepreflightinfo`
/// command, so the caller provides a connection factory `connect_service` that yields a fresh
/// `RestoreServiceClient` for each updater.
pub async fn collect_device_side_preflight_requests<R, F, S, Fut>(
    ipsw: &mut IpswPreflight<R>,
    updaters: &[&str],
    connect_service: F,
    per_updater_timeout: std::time::Duration,
) -> Result<CollectedPreflightRequests, RestoreError>
where
    R: Read + Seek,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<super::RestoreServiceClient<S>, RestoreError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    collect_device_side_preflight_outcomes(ipsw, updaters, connect_service, per_updater_timeout)
        .await
        .map(|outcome| outcome.summary)
}

/// Collect partial results without discarding native failures or timeout objects.
/// Each updater uses one deadline for package preparation, connect and request.
pub async fn collect_device_side_preflight_outcomes<R, F, S, Fut>(
    ipsw: &mut IpswPreflight<R>,
    updaters: &[&str],
    mut connect_service: F,
    per_updater_timeout: std::time::Duration,
) -> Result<PreflightCollection, RestoreError>
where
    R: Read + Seek,
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<super::RestoreServiceClient<S>, RestoreError>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut collected = CollectedPreflightRequests::default();
    let mut failures = Vec::new();
    if updaters.len() > UPDATERS.len()
        || updaters.iter().copied().collect::<HashSet<_>>().len() != updaters.len()
    {
        return Err(RestoreError::Protocol(
            "too many or duplicate preflight updaters".into(),
        ));
    }
    if per_updater_timeout.is_zero() {
        return Err(RestoreError::Protocol(
            "preflight timeout must be positive".into(),
        ));
    }
    for updater in updaters {
        let deadline = tokio::time::Instant::now()
            .checked_add(per_updater_timeout)
            .ok_or_else(|| RestoreError::Protocol("preflight deadline overflow".into()))?;
        let payload = match ipsw.payload_for_updater(updater) {
            Ok(payload) => payload,
            Err(err) => {
                retain_failure(
                    &mut collected,
                    &mut failures,
                    updater,
                    "prepare payload",
                    err,
                );
                continue;
            }
        };

        let client = match tokio::time::timeout_at(deadline, connect_service()).await {
            Ok(Ok(client)) => client,
            Ok(Err(err)) => {
                retain_failure(&mut collected, &mut failures, updater, "connect", err);
                continue;
            }
            Err(elapsed) => {
                retain_failure(
                    &mut collected,
                    &mut failures,
                    updater,
                    "connect",
                    native_error(
                        std::io::ErrorKind::TimedOut,
                        "preflight connect deadline",
                        elapsed,
                    ),
                );
                continue;
            }
        };

        let info = match tokio::time::timeout_at(
            deadline,
            client.get_device_side_preflight_info(Some(payload)),
        )
        .await
        {
            Ok(Ok(info)) => info,
            Ok(Err(err)) => {
                retain_failure(&mut collected, &mut failures, updater, "request", err);
                continue;
            }
            Err(elapsed) => {
                retain_failure(
                    &mut collected,
                    &mut failures,
                    updater,
                    "request",
                    native_error(
                        std::io::ErrorKind::TimedOut,
                        "preflight request deadline",
                        elapsed,
                    ),
                );
                continue;
            }
        };

        for (src_key, target_map) in [
            ("DeviceInfo", &mut collected.device_info),
            ("DeviceInfoTags", &mut collected.device_info_tags),
            ("DeviceInfoRequests", &mut collected.device_info_requests),
        ] {
            if let Some(XpcValue::Dictionary(dict)) = info.get(src_key) {
                if let Some(updater_val) = dict.get(*updater) {
                    target_map.insert(
                        (*updater).to_string(),
                        super::xpc_value_to_json(updater_val),
                    );
                }
            }
        }

        if let Some(XpcValue::Dictionary(failures)) = info.get("DeviceInfoFailures") {
            for (k, v) in failures {
                collected
                    .device_info_failures
                    .insert(k.clone(), super::xpc_value_to_json(v));
            }
        }

        if let Some(ap) = info.get("ApParameters") {
            collected.ap_parameters = Some(super::xpc_value_to_json(ap));
        }
    }

    Ok(PreflightCollection {
        summary: collected,
        failures,
    })
}

fn prefixes(updater: &str) -> Option<&'static [&'static str]> {
    match updater {
        "T200" => Some(&["BMU,"]),
        "Rose" => Some(&["Rap,"]),
        "SE" => Some(&["SE,"]),
        "Vinyl" => Some(&["eUICC,"]),
        "Savage" => Some(&["SEP", "JasmineIR1,", "Yonkers,", "Savage,"]),
        "Centauri" => Some(&["Wireless1,"]),
        "Baseband" => Some(&["Cellular1,"]),
        _ => None,
    }
}

/// Build one `getdevicesidepreflightinfo` payload from a selected IPSW build identity.
///
/// The caller supplies a reader for firmware paths inside the IPSW. Each path is read once,
/// even if several manifest tags refer to it. The returned XPC tree preserves signed 64-bit
/// integer wire types required by restoreserviced. A per-updater limit prevents accidental
/// loading of very large baseband images into a single XPC request.
pub fn build_device_side_preflight_payload<F>(
    identity: &plist::Dictionary,
    updater: &str,
    mut read_firmware: F,
) -> Result<XpcValue, RestoreError>
where
    F: FnMut(&str) -> Result<Vec<u8>, RestoreError>,
{
    let prefixes = prefixes(updater).ok_or_else(|| {
        RestoreError::Protocol(format!("unsupported preflight updater {updater}"))
    })?;
    let manifest = identity
        .get("Manifest")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| {
            RestoreError::Protocol("build identity missing Manifest dictionary".into())
        })?;
    let mut files = IndexMap::new();
    let mut cache = HashMap::<String, Bytes>::new();
    let mut total = 0usize;
    for (tag, entry) in manifest {
        if !prefixes.iter().any(|prefix| tag.starts_with(prefix)) {
            continue;
        }
        let path = entry
            .as_dictionary()
            .and_then(|entry| entry.get("Info"))
            .and_then(plist::Value::as_dictionary)
            .and_then(|info| info.get("Path"))
            .and_then(plist::Value::as_string);
        let Some(path) = path else { continue };
        if !safe_ipsw_path(path) {
            return Err(RestoreError::Protocol(format!(
                "unsafe firmware path for {tag}"
            )));
        }
        let bytes = if let Some(cached) = cache.get(path) {
            cached.clone()
        } else {
            let bytes = read_firmware(path)?;
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| RestoreError::Protocol("preflight firmware size overflow".into()))?;
            if total > MAX_PREFLIGHT_FIRMWARE_BYTES {
                return Err(RestoreError::Protocol(
                    "preflight firmware exceeds 64 MiB per updater".into(),
                ));
            }
            let bytes = Bytes::from(bytes);
            cache.insert(path.to_string(), bytes.clone());
            bytes
        };
        files.insert(tag.clone(), XpcValue::Data(bytes));
    }
    if files.is_empty() {
        return Err(RestoreError::Protocol(format!(
            "no firmware paths found for preflight updater {updater}"
        )));
    }
    Ok(XpcValue::Dictionary(IndexMap::from([
        (
            "BuildIdentity".into(),
            plist_to_xpc(&plist::Value::Dictionary(identity.clone()), 0)?,
        ),
        (updater.into(), XpcValue::Dictionary(files)),
    ])))
}

fn safe_ipsw_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn plist_to_xpc(value: &plist::Value, depth: usize) -> Result<XpcValue, RestoreError> {
    if depth > MAX_PLIST_DEPTH {
        return Err(RestoreError::Protocol(
            "preflight plist nesting exceeds 64".into(),
        ));
    }
    Ok(match value {
        plist::Value::Boolean(value) => XpcValue::Bool(*value),
        plist::Value::Integer(value) => XpcValue::Int64(value.as_signed().ok_or_else(|| {
            RestoreError::Protocol("preflight integer exceeds signed XPC int64".into())
        })?),
        plist::Value::Real(value) => XpcValue::Double(*value),
        plist::Value::Data(value) => XpcValue::Data(Bytes::copy_from_slice(value)),
        plist::Value::String(value) => XpcValue::String(value.clone()),
        plist::Value::Array(values) => XpcValue::Array(
            values
                .iter()
                .map(|value| plist_to_xpc(value, depth + 1))
                .collect::<Result<_, _>>()?,
        ),
        plist::Value::Dictionary(values) => XpcValue::Dictionary(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), plist_to_xpc(value, depth + 1)?)))
                .collect::<Result<_, RestoreError>>()?,
        ),
        _ => {
            return Err(RestoreError::Protocol(
                "unsupported preflight plist value".into(),
            ))
        }
    })
}

/// Iterate firmware updater replies without buffering all firmware chunks.
/// The first message carries the ticket dictionary and total size; a final empty
/// `DataDone` message terminates the stream, even for an empty firmware blob.
/// The caller writes each message before advancing the iterator.
pub fn firmware_response_messages<'a>(
    header: IndexMap<String, XpcValue>,
    firmware: &'a [u8],
    requested_chunk_size: usize,
) -> Result<impl Iterator<Item = XpcValue> + 'a, RestoreError> {
    if requested_chunk_size == 0 {
        return Err(RestoreError::Protocol(
            "DataChunkSize must be positive".into(),
        ));
    }
    let data_size = i64::try_from(firmware.len()).map_err(|error| {
        native_error(
            std::io::ErrorKind::InvalidInput,
            "convert firmware DataSize",
            error,
        )
    })?;
    let chunk_size = requested_chunk_size.min(256 * 1024);
    let header = XpcValue::Dictionary(IndexMap::from([
        ("FirmwareResponseData".into(), XpcValue::Dictionary(header)),
        ("DataSize".into(), XpcValue::Int64(data_size)),
    ]));
    let chunks = firmware.chunks(chunk_size).map(|chunk| {
        XpcValue::Dictionary(IndexMap::from([(
            "FirmwareResponseData".into(),
            XpcValue::Data(Bytes::copy_from_slice(chunk)),
        )]))
    });
    let done = XpcValue::Dictionary(IndexMap::from([
        ("FirmwareResponseData".into(), XpcValue::Data(Bytes::new())),
        ("DataDone".into(), XpcValue::Bool(true)),
    ]));
    Ok(std::iter::once(header)
        .chain(chunks)
        .chain(std::iter::once(done)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    fn ipsw(identities: Vec<plist::Dictionary>, files: &[(&str, &[u8])]) -> Vec<u8> {
        let manifest = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "BuildIdentities",
            plist::Value::Array(
                identities
                    .into_iter()
                    .map(plist::Value::Dictionary)
                    .collect(),
            ),
        )]));
        let mut manifest_bytes = Vec::new();
        plist::to_writer_xml(&mut manifest_bytes, &manifest).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("BuildManifest.plist", options).unwrap();
        writer.write_all(&manifest_bytes).unwrap();
        for (path, data) in files {
            writer.start_file(path, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn selected_identity(behavior: &str, variant: &str, path: &str) -> plist::Dictionary {
        plist::Dictionary::from_iter([
            ("ApBoardID", plist::Value::String("0x10".into())),
            ("ApChipID", plist::Value::Integer(32.into())),
            (
                "Info",
                plist::Value::Dictionary(plist::Dictionary::from_iter([
                    ("DeviceClass", plist::Value::String("d63ap".into())),
                    ("RestoreBehavior", plist::Value::String(behavior.into())),
                    ("Variant", plist::Value::String(variant.into())),
                ])),
            ),
            (
                "Manifest",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "Rap,RTKitOS",
                    entry(path),
                )])),
            ),
        ])
    }

    fn selector<'a>() -> IpswIdentitySelector<'a> {
        IpswIdentitySelector {
            board_id: 16,
            chip_id: 32,
            device_class: "d63ap",
            restore_behavior: "Update",
        }
    }

    #[test]
    fn component_inspection_reads_real_members_and_rejects_missing_or_budget() {
        let identity = selected_identity("Update", "Upgrade Install (IPSW)", "Firmware/rose.bin");
        let mut reader = IpswPreflight::open(
            Cursor::new(ipsw(
                vec![identity.clone()],
                &[("Firmware/rose.bin", b"rose")],
            )),
            selector(),
        )
        .unwrap();
        let components = reader.validate_components(4).unwrap();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].bytes, 4);
        use sha2::{Digest, Sha256};
        assert_eq!(components[0].sha256, hex::encode(Sha256::digest(b"rose")));
        assert!(reader.validate_components(3).is_err());
        let mut missing =
            IpswPreflight::open(Cursor::new(ipsw(vec![identity], &[])), selector()).unwrap();
        assert!(missing.validate_components(1024).is_err());
    }

    #[test]
    fn saved_tss_requires_original_device_nonce_request_and_nonempty_ticket() {
        let request = plist::Dictionary::from_iter([
            ("ApECID", plist::Value::Integer(42.into())),
            ("ApBoardID", plist::Value::Integer(16.into())),
            ("ApChipID", plist::Value::Integer(32.into())),
            ("ApNonce", plist::Value::Data(vec![1, 2])),
            ("SepNonce", plist::Value::Data(vec![3, 4])),
        ]);
        let envelope = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Request", plist::Value::Dictionary(request)),
            (
                "Response",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "ApImg4Ticket",
                    plist::Value::Data(vec![9]),
                )])),
            ),
        ]));
        inspect_saved_tss(&envelope, 42, 16, 32, &[1, 2], Some(&[3, 4])).unwrap();
        assert!(inspect_saved_tss(&envelope, 43, 16, 32, &[1, 2], Some(&[3, 4])).is_err());
        assert!(inspect_saved_tss(&envelope, 42, 16, 32, &[7, 8], Some(&[3, 4])).is_err());
        assert!(
            inspect_saved_tss(&plist::Value::Data(vec![9]), 42, 16, 32, &[1, 2], None).is_err()
        );
    }

    #[test]
    fn ipsw_selects_exact_update_identity_and_reads_firmware() {
        let bytes = ipsw(
            vec![
                selected_identity("Erase", "Erase Install (IPSW)", "Firmware/erase.bin"),
                selected_identity("Update", "Other", "Firmware/other.bin"),
                selected_identity("Update", "Upgrade Install (IPSW)", "Firmware/rose.bin"),
            ],
            &[("Firmware/rose.bin", b"rose")],
        );
        let mut reader = IpswPreflight::open(Cursor::new(bytes), selector()).unwrap();
        let payload = reader.payload_for_updater("Rose").unwrap();
        assert_eq!(
            payload.as_dict().unwrap()["Rose"].as_dict().unwrap()["Rap,RTKitOS"],
            XpcValue::Data(Bytes::from_static(b"rose"))
        );
    }

    #[test]
    fn ipsw_rejects_missing_ambiguous_and_oversized_members() {
        let identity = selected_identity("Update", "Upgrade Install (IPSW)", "Firmware/rose.bin");
        assert!(IpswPreflight::open(
            Cursor::new(ipsw(vec![identity.clone()], &[])),
            IpswIdentitySelector {
                board_id: 17,
                ..selector()
            }
        )
        .is_err());
        assert!(IpswPreflight::open(
            Cursor::new(ipsw(vec![identity.clone(), identity.clone()], &[])),
            selector()
        )
        .is_err());
        let oversized = ipsw(vec![identity], &[("Firmware/rose.bin", b"four")]);
        let mut archive = zip::ZipArchive::new(Cursor::new(oversized)).unwrap();
        assert!(read_zip_member(&mut archive, "Firmware/rose.bin", 3).is_err());
    }

    #[test]
    fn builds_rose_payload_with_shared_file_and_signed_integers() {
        let manifest = plist::Dictionary::from_iter([
            ("Rap,RTKitOS", entry("Firmware/Rose/ftab.bin")),
            ("Rap,SoftwareBinaryDsp1", entry("Firmware/Rose/ftab.bin")),
            ("BMU,FirmwareMap", entry("Firmware/T200/map.plist")),
        ]);
        let identity = plist::Dictionary::from_iter([
            ("Rap,ChipID", plist::Value::Integer(8228.into())),
            ("Enabled", plist::Value::Boolean(true)),
            ("Manifest", plist::Value::Dictionary(manifest)),
        ]);
        let mut reads = Vec::new();
        let payload = build_device_side_preflight_payload(&identity, "Rose", |path| {
            reads.push(path.to_string());
            Ok(vec![1, 2, 3])
        })
        .unwrap();
        assert_eq!(reads, ["Firmware/Rose/ftab.bin"]);
        let payload = payload.as_dict().unwrap();
        let build = payload["BuildIdentity"].as_dict().unwrap();
        assert_eq!(build["Rap,ChipID"], XpcValue::Int64(8228));
        assert_eq!(build["Enabled"], XpcValue::Bool(true));
        let files = payload["Rose"].as_dict().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            files["Rap,RTKitOS"],
            XpcValue::Data(Bytes::from_static(&[1, 2, 3]))
        );
    }

    #[test]
    fn rejects_unsafe_paths_and_oversized_firmware() {
        let identity = |path| {
            plist::Dictionary::from_iter([(
                "Manifest",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "Rap,RTKitOS",
                    entry(path),
                )])),
            )])
        };
        assert!(
            build_device_side_preflight_payload(&identity("../escape"), "Rose", |_| {
                panic!("reader must not run for unsafe path")
            })
            .is_err()
        );
        assert!(
            build_device_side_preflight_payload(&identity("Firmware/Rose/x"), "Rose", |_| {
                Ok(vec![0; MAX_PREFLIGHT_FIRMWARE_BYTES + 1])
            })
            .is_err()
        );
    }

    #[test]
    fn firmware_replies_include_header_chunks_and_done() {
        let replies: Vec<_> = firmware_response_messages(IndexMap::new(), &[1, 2, 3, 4, 5], 2)
            .unwrap()
            .collect();
        assert_eq!(replies.len(), 5);
        assert_eq!(
            replies[0].as_dict().unwrap()["DataSize"],
            XpcValue::Int64(5)
        );
        assert_eq!(
            replies[1].as_dict().unwrap()["FirmwareResponseData"],
            XpcValue::Data(Bytes::from_static(&[1, 2]))
        );
        assert_eq!(
            replies[4].as_dict().unwrap()["DataDone"],
            XpcValue::Bool(true)
        );
        let empty: Vec<_> = firmware_response_messages(IndexMap::new(), &[], usize::MAX)
            .unwrap()
            .collect();
        assert_eq!(empty.len(), 2);
        assert_eq!(empty[0].as_dict().unwrap()["DataSize"], XpcValue::Int64(0));
    }

    #[tokio::test]
    async fn collect_device_side_preflight_requests_merges_updaters_and_handles_failures() {
        let info = plist::Dictionary::from_iter([
            ("DeviceClass", plist::Value::String("d63ap".into())),
            ("RestoreBehavior", plist::Value::String("Update".into())),
        ]);
        let identity = plist::Dictionary::from_iter([
            ("ApBoardID", plist::Value::Integer(16.into())),
            ("ApChipID", plist::Value::Integer(32.into())),
            ("Info", plist::Value::Dictionary(info)),
            (
                "Manifest",
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "Rap,RTKitOS",
                    entry("Firmware/rose.bin"),
                )])),
            ),
        ]);
        let manifest = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "BuildIdentities",
            plist::Value::Array(vec![plist::Value::Dictionary(identity)]),
        )]));
        let mut manifest_bytes = Vec::new();
        plist::to_writer_xml(&mut manifest_bytes, &manifest).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("BuildManifest.plist", options).unwrap();
        writer.write_all(&manifest_bytes).unwrap();
        writer.start_file("Firmware/rose.bin", options).unwrap();
        writer.write_all(b"rose").unwrap();

        let mut ipsw = IpswPreflight::open(
            Cursor::new(writer.finish().unwrap().into_inner()),
            IpswIdentitySelector {
                board_id: 16,
                chip_id: 32,
                device_class: "d63ap",
                restore_behavior: "Update",
            },
        )
        .unwrap();

        let updaters = ["Rose", "T200"];
        let outcome = collect_device_side_preflight_outcomes(
            &mut ipsw,
            &updaters,
            || async {
                Err::<super::super::RestoreServiceClient<tokio::io::DuplexStream>, _>(
                    RestoreError::Protocol("simulated connect error for Rose".into()),
                )
            },
            std::time::Duration::from_millis(100),
        )
        .await
        .unwrap();
        let collected = &outcome.summary;
        assert_eq!(collected.failed_updaters.len(), 2);
        assert_eq!(collected.failed_updaters[0].updater, "Rose");
        assert!(!collected.failed_updaters[0]
            .error
            .contains("simulated connect error"));
        assert!(
            matches!(&outcome.failures[0].cause,RestoreError::Protocol(message) if message == "simulated connect error for Rose")
        );
        assert_eq!(collected.failed_updaters[1].updater, "T200");
        assert!(
            matches!(&outcome.failures[1].cause,RestoreError::Protocol(message) if message.contains("no firmware paths found"))
        );
    }

    fn entry(path: &str) -> plist::Value {
        plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Info",
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Path",
                plist::Value::String(path.into()),
            )])),
        )]))
    }
}

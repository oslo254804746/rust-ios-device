//! Local DDI BuildManifest-to-target matching and component observation.
//!
//! Evaluates local BuildManifest.plist assets against explicit target hardware
//! evidence (board_id, chip_id, security_domain, versions).
//!
//! Observes component files on disk, verifies digests when available, and
//! explicitly reports:
//! - "file exists"
//! - "digest matches"
//! - "signature unverified" (read-only diagnostic, no Apple trust-chain check)
//! - "device availability not probed" (read-only diagnostic, no mount/activation)

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_MANIFEST_PARSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMPONENT_READ_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB bound

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct TargetEvidence {
    #[serde(serialize_with = "serialize_target_id")]
    pub(crate) udid: String,
    pub(crate) board_id: Option<u64>,
    pub(crate) chip_id: Option<u64>,
    pub(crate) security_domain: Option<u64>,
    pub(crate) product_type: Option<String>,
    pub(crate) product_version: Option<String>,
    pub(crate) build_version: Option<String>,
}

fn serialize_target_id<S: serde::Serializer>(udid: &str, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&ios_core::redaction::diagnostic_id(udid).to_string())
}

impl TargetEvidence {
    pub(crate) fn from_identifiers(
        udid: &str,
        ids: &std::collections::HashMap<String, plist::Value>,
    ) -> Self {
        Self {
            udid: udid.to_owned(),
            board_id: ids.get("BoardId").and_then(parse_plist_int),
            chip_id: ids.get("ChipID").and_then(parse_plist_int),
            security_domain: ids.get("SecurityDomain").and_then(parse_plist_int),
            product_type: ids
                .get("ProductType")
                .and_then(plist::Value::as_string)
                .map(String::from),
            product_version: ids
                .get("ProductVersion")
                .and_then(plist::Value::as_string)
                .map(String::from),
            build_version: ids
                .get("BuildVersion")
                .or_else(|| ids.get("ProductBuildVersion"))
                .and_then(plist::Value::as_string)
                .map(String::from),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ComponentState {
    FileExists,
    DigestMatches,
    DigestMismatch,
    MissingFile,
    PermissionDenied,
    TimedOut,
    ReadFailed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ComponentObservation {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) state: ComponentState,
    pub(crate) signature_verified: bool,
    pub(crate) device_availability_probed: bool,
    pub(crate) expected_digest: Option<String>,
    pub(crate) observed_digest: Option<String>,
    pub(crate) size_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MatchOutcome {
    Matched,
    NoMatchingIdentity,
    AmbiguousIdentities,
    MissingFields,
    MissingFile,
    PermissionDenied,
    DigestMismatch,
    VersionMismatch,
    NoManifestFound,
    TimedOut,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct TargetManifestMetadata {
    pub(crate) manifest_path: Option<String>,
    pub(crate) outcome: Option<MatchOutcome>,
    pub(crate) matched_identities: usize,
    pub(crate) total_identities: usize,
    pub(crate) product_version: Option<String>,
    pub(crate) product_build_version: Option<String>,
    pub(crate) components: Vec<ComponentObservation>,
    pub(crate) detail: String,
    pub(crate) hint: Option<String>,
}

impl TargetManifestMetadata {
    #[cfg(test)]
    pub(crate) fn is_ok(&self) -> bool {
        self.outcome == Some(MatchOutcome::Matched)
    }
}

/// Extract integer from a plist value (Integer or hex/decimal String).
fn parse_plist_int(val: &plist::Value) -> Option<u64> {
    match val {
        plist::Value::Integer(n) => n.as_unsigned(),
        plist::Value::String(s) => {
            if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(hex, 16).ok()
            } else {
                s.parse().ok()
            }
        }
        _ => None,
    }
}

/// Identify Cryptex BuildIdentity entries that do not target a device board/chip.
fn is_cryptex_identity(identity_dict: &plist::Dictionary) -> bool {
    identity_dict.get("ApBoardID").is_none()
        && identity_dict.get("ApChipID").is_none()
        && (identity_dict.keys().any(|key| key.starts_with("Cryptex"))
            || identity_dict
                .get("Ap,TargetType")
                .and_then(plist::Value::as_string)
                .is_some_and(|target| target.eq_ignore_ascii_case("ioscryptex"))
            || identity_dict
                .get("Info")
                .and_then(plist::Value::as_dictionary)
                .and_then(|info| info.get("Variant"))
                .and_then(plist::Value::as_string)
                .is_some_and(|variant| variant.contains("Cryptex")))
}

/// Helper to sanitize a relative path from plist (prevent path traversal).
fn safe_relative_path(path_str: &str) -> Option<PathBuf> {
    let path = Path::new(path_str);
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => return None,
        }
    }
    Some(path.to_path_buf())
}

/// Search for BuildManifest.plist files across the given root paths.
pub(crate) fn find_build_manifests(search_paths: &[PathBuf]) -> Vec<PathBuf> {
    discover_build_manifests(search_paths).0
}

fn discover_build_manifests(search_paths: &[PathBuf]) -> (Vec<PathBuf>, Vec<std::io::Error>) {
    let mut discovered = Vec::new();
    let mut errors = Vec::new();
    let mut inspected = 0usize;

    for base in search_paths {
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((base.clone(), 0usize));

        while let Some((dir, depth)) = queue.pop_front() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    errors.push(error);
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };
                inspected += 1;
                if inspected > 4096 {
                    errors.push(std::io::Error::other("DDI directory entry limit reached"));
                    return (discovered, errors);
                }
                let kind = match entry.file_type() {
                    Ok(k) => k,
                    Err(error) => {
                        errors.push(error);
                        continue;
                    }
                };

                if kind.is_dir() && depth < 2 {
                    queue.push_back((entry.path(), depth + 1));
                    continue;
                }

                if kind.is_file() {
                    let file_name = entry.file_name();
                    if file_name
                        .to_string_lossy()
                        .eq_ignore_ascii_case("BuildManifest.plist")
                    {
                        discovered.push(entry.path());
                    }
                }
            }
        }
    }

    discovered.sort();
    (discovered, errors)
}

/// Evaluate target evidence against local DDI BuildManifest assets.
pub(crate) fn evaluate_target_manifest(
    evidence: &TargetEvidence,
    search_paths: &[PathBuf],
) -> TargetManifestMetadata {
    evaluate_target_manifest_until(evidence, search_paths, None)
}

pub(crate) fn evaluate_target_manifest_until(
    evidence: &TargetEvidence,
    search_paths: &[PathBuf],
    deadline: Option<tokio::time::Instant>,
) -> TargetManifestMetadata {
    if checkpoint(deadline).is_err() {
        return TargetManifestMetadata {
            outcome: Some(MatchOutcome::TimedOut),
            detail: "Local DDI observation deadline elapsed before inspection.".into(),
            ..Default::default()
        };
    }
    let (manifests, errors) = discover_build_manifests(search_paths);
    if !errors.is_empty() {
        let denied = errors
            .iter()
            .any(|e| e.kind() == std::io::ErrorKind::PermissionDenied);
        return TargetManifestMetadata {
            outcome: Some(if denied {
                MatchOutcome::PermissionDenied
            } else {
                MatchOutcome::MissingFields
            }),
            detail: format!(
                "Local DDI search was incomplete ({} directory errors, {} manifests found): {}",
                errors.len(),
                manifests.len(),
                errors[0]
            ),
            hint: Some("Check configured DDI directory paths and read permissions.".into()),
            ..Default::default()
        };
    }
    if manifests.is_empty() {
        return TargetManifestMetadata {
            manifest_path: None,
            outcome: Some(MatchOutcome::NoManifestFound),
            matched_identities: 0,
            total_identities: 0,
            product_version: None,
            product_build_version: None,
            components: Vec::new(),
            detail: "No local DDI BuildManifest.plist found in configured search paths.".into(),
            hint: Some(
                "Download or configure a DDI bundle with 'ios ddi download' or set IOS_DDI_PATH."
                    .into(),
            ),
        };
    }

    // A hardware-matched partial observation outranks an unrelated image.
    let mut best_metadata: Option<TargetManifestMetadata> = None;
    let relevance = |metadata: &TargetManifestMetadata| {
        let rank = match metadata.outcome {
            Some(MatchOutcome::TimedOut) => 8,
            Some(MatchOutcome::PermissionDenied) => 7,
            Some(MatchOutcome::MissingFields) => 6,
            Some(MatchOutcome::AmbiguousIdentities) => 5,
            Some(MatchOutcome::DigestMismatch) => 4,
            Some(MatchOutcome::MissingFile) => 3,
            Some(MatchOutcome::VersionMismatch) => 2,
            _ => 1,
        };
        (metadata.matched_identities > 0, rank)
    };

    for manifest_path in &manifests {
        let metadata = evaluate_single_manifest(evidence, manifest_path, deadline);
        if metadata.outcome == Some(MatchOutcome::Matched) {
            return metadata;
        }
        if best_metadata
            .as_ref()
            .map_or(true, |best| relevance(&metadata) > relevance(best))
        {
            best_metadata = Some(metadata);
        }
    }

    best_metadata.unwrap_or_default()
}

fn evaluate_single_manifest(
    evidence: &TargetEvidence,
    manifest_path: &Path,
    deadline: Option<tokio::time::Instant>,
) -> TargetManifestMetadata {
    let manifest_display = manifest_path.to_string_lossy().to_string();

    use std::io::Read;
    let read = (|| {
        let file = std::fs::File::open(manifest_path)?;
        let mut data = Vec::new();
        file.take(MAX_MANIFEST_PARSE_BYTES as u64 + 1)
            .read_to_end(&mut data)?;
        Ok::<_, std::io::Error>(data)
    })();
    let data = match read {
        Ok(d) => {
            if d.len() > MAX_MANIFEST_PARSE_BYTES {
                return TargetManifestMetadata {
                    manifest_path: Some(manifest_display),
                    outcome: Some(MatchOutcome::MissingFields),
                    detail: format!(
                        "BuildManifest exceeds byte limit ({MAX_MANIFEST_PARSE_BYTES} bytes)."
                    ),
                    hint: Some("Verify local DDI BuildManifest file integrity.".into()),
                    ..Default::default()
                };
            }
            d
        }
        Err(err) => {
            let is_perm = err.kind() == std::io::ErrorKind::PermissionDenied;
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(if is_perm {
                    MatchOutcome::PermissionDenied
                } else {
                    MatchOutcome::MissingFile
                }),
                detail: format!("Failed to read BuildManifest: {err}"),
                hint: Some(if is_perm {
                    "Check filesystem read permissions for DDI directories.".into()
                } else {
                    "Verify BuildManifest file existence in DDI directories.".into()
                }),
                ..Default::default()
            };
        }
    };

    let plist_val: plist::Value = match plist::from_bytes(&data) {
        Ok(v) => v,
        Err(err) => {
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::MissingFields),
                detail: format!("Failed to parse BuildManifest as plist: {err}"),
                hint: Some(
                    "Verify that BuildManifest.plist contains valid XML/binary plist data.".into(),
                ),
                ..Default::default()
            };
        }
    };

    let dict = match plist_val.as_dictionary() {
        Some(d) => d,
        None => {
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::MissingFields),
                detail: "BuildManifest root is not a dictionary.".into(),
                ..Default::default()
            };
        }
    };

    let product_version = dict
        .get("ProductVersion")
        .and_then(plist::Value::as_string)
        .map(str::to_string);
    let product_build_version = dict
        .get("ProductBuildVersion")
        .and_then(plist::Value::as_string)
        .map(str::to_string);

    // A SupportedProductTypes list, when supplied, is an additional model
    // constraint. Board/chip identity matching below is still mandatory.
    if let Some(models) = dict.get("SupportedProductTypes") {
        let Some(models) = models.as_array().filter(|models| {
            !models.is_empty()
                && models
                    .iter()
                    .all(|model| model.as_string().is_some_and(|model| !model.is_empty()))
        }) else {
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::MissingFields),
                detail: "SupportedProductTypes is malformed or empty.".into(),
                ..Default::default()
            };
        };
        if !evidence
            .product_type
            .as_deref()
            .is_some_and(|model| models.iter().any(|value| value.as_string() == Some(model)))
        {
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(if evidence.product_type.is_some() {
                    MatchOutcome::NoMatchingIdentity
                } else {
                    MatchOutcome::MissingFields
                }),
                detail:
                    "Target ProductType is absent from SupportedProductTypes or was not observed."
                        .into(),
                ..Default::default()
            };
        }
    }

    // Personalized DDIs are selected by hardware, not by the device build or
    // matching OS major. Upstream request_personalization_manifest selects the
    // board/chip identity; this image-mounter profile is available from iOS17.
    let personalized = dict
        .get("BuildIdentities")
        .and_then(plist::Value::as_array)
        .is_some_and(|ids| {
            ids.iter().any(|id| {
                id.as_dictionary()
                    .and_then(|id| id.get("Manifest"))
                    .and_then(plist::Value::as_dictionary)
                    .is_some_and(|m| m.contains_key("PersonalizedDMG"))
            })
        });
    let release = |version: &str| {
        let parts = version
            .split('.')
            .map(str::parse::<u64>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (parts.len() >= 2 && parts.len() <= 3).then_some(parts)
    };
    let device_release = evidence.product_version.as_deref().and_then(release);
    let version_supported = if personalized {
        device_release.map(|version| version[0] >= 17)
    } else {
        match (device_release, product_version.as_deref().and_then(release)) {
            (Some(device), Some(manifest)) => Some(device[..2] == manifest[..2]),
            _ => None,
        }
    };
    if version_supported == Some(false) {
        let device_ver = evidence.product_version.as_deref().unwrap_or("unobserved");
        let manifest_ver = product_version
            .as_deref()
            .unwrap_or("generic personalized image");
        let detail = format!(
                "DDI version profile ({manifest_ver}) does not match device version ({device_ver}); personalized images require iOS17+, legacy images require the same major/minor. Build equality is not required."
            );
        return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::VersionMismatch),
                matched_identities: 0,
                total_identities: 0,
                product_version,
                product_build_version,
                components: Vec::new(),
                detail,
                hint: Some("Select a hardware-compatible personalized DDI for iOS17+, or a version-specific legacy DDI.".into()),
            };
    }

    let identities = match dict.get("BuildIdentities").and_then(plist::Value::as_array) {
        Some(arr) => arr,
        None => {
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::MissingFields),
                detail: "BuildManifest is missing BuildIdentities array.".into(),
                ..Default::default()
            };
        }
    };

    let total_identities = identities.len();
    let target_board = evidence.board_id;
    let target_chip = evidence.chip_id;

    if target_board.is_none() || target_chip.is_none() {
        return TargetManifestMetadata {
            manifest_path: Some(manifest_display),
            outcome: Some(MatchOutcome::MissingFields),
            total_identities,
            product_version,
            product_build_version,
            detail: "Target BoardId/ChipID were not observed; identity matching is unavailable."
                .into(),
            ..Default::default()
        };
    }

    // Filter identities matching board and chip
    let mut matching_identities = Vec::new();
    let mut unresolved_target_identity = false;
    let mut unresolved_generic_identity = false;

    for identity_val in identities {
        let identity_dict = match identity_val.as_dictionary() {
            Some(d) => d,
            None => {
                unresolved_generic_identity = true;
                continue;
            }
        };

        if is_cryptex_identity(identity_dict) {
            continue;
        }

        let id_board = identity_dict.get("ApBoardID").and_then(parse_plist_int);
        let id_chip = identity_dict.get("ApChipID").and_then(parse_plist_int);
        let id_sec_domain = identity_dict
            .get("ApSecurityDomain")
            .and_then(parse_plist_int);

        // Missing fields are not evidence that this identity is incompatible.
        if matches!((target_board, id_board), (Some(a), Some(b)) if a != b)
            || matches!((target_chip, id_chip), (Some(a), Some(b)) if a != b)
            || matches!((evidence.security_domain, id_sec_domain), (Some(a), Some(b)) if a != b)
        {
            continue;
        }
        if id_board.is_none()
            || id_chip.is_none()
            || (evidence.security_domain.is_some() && id_sec_domain.is_none())
        {
            if id_board.is_some() || id_chip.is_some() {
                unresolved_target_identity = true;
            } else {
                unresolved_generic_identity = true;
            }
            continue;
        }
        let board_matches = target_board == id_board;
        let chip_matches = target_chip == id_chip;
        let sec_matches =
            evidence.security_domain.is_none() || evidence.security_domain == id_sec_domain;
        if board_matches && chip_matches && sec_matches {
            matching_identities.push(identity_dict);
        }
    }

    let unresolved_identity = unresolved_target_identity
        || (matching_identities.is_empty() && unresolved_generic_identity);
    if matching_identities.is_empty() || unresolved_identity {
        return TargetManifestMetadata {
            manifest_path: Some(manifest_display),
            outcome: Some(if unresolved_identity { MatchOutcome::MissingFields } else { MatchOutcome::NoMatchingIdentity }),
            matched_identities: matching_identities.len(),
            total_identities,
            product_version,
            product_build_version,
            components: Vec::new(),
            detail: format!(
                "BuildIdentity selection is unavailable or incompatible (unresolved identity fields: {unresolved_identity}, board_id={:?}, chip_id={:?}, total identities: {total_identities}).",
                target_board, target_chip
            ),
            hint: Some("Verify target device board/chip hardware model matches DDI manifest.".into()),
        };
    }

    if matching_identities.len() > 1 {
        // Identity ambiguity
        return TargetManifestMetadata {
            manifest_path: Some(manifest_display),
            outcome: Some(MatchOutcome::AmbiguousIdentities),
            matched_identities: matching_identities.len(),
            total_identities,
            product_version,
            product_build_version,
            components: Vec::new(),
            detail: format!(
                "Found {} ambiguous matching BuildIdentities for target (board_id={:?}, chip_id={:?}).",
                matching_identities.len(), target_board, target_chip
            ),
            hint: Some("Specify explicit security domain or variant to disambiguate target identity.".into()),
        };
    }

    // Exactly one matching identity
    let identity = matching_identities[0];
    let manifest_dict = match identity
        .get("Manifest")
        .and_then(plist::Value::as_dictionary)
    {
        Some(m) => m,
        None => {
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::MissingFields),
                matched_identities: 1,
                total_identities,
                product_version,
                product_build_version,
                components: Vec::new(),
                detail: "Selected BuildIdentity is missing Manifest dictionary.".into(),
                hint: Some("Check BuildManifest integrity.".into()),
            };
        }
    };

    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let mut component_observations = Vec::new();
    let mut has_missing_file = false;
    let mut has_permission_denied = false;
    let mut has_digest_mismatch = false;
    let mut has_missing_fields = manifest_dict.is_empty();
    let mut has_timeout = false;

    for (comp_name, comp_val) in manifest_dict {
        let comp_dict = match comp_val.as_dictionary() {
            Some(d) => d,
            None => {
                has_missing_fields = true;
                continue;
            }
        };

        let path_str = match comp_dict
            .get("Info")
            .and_then(plist::Value::as_dictionary)
            .and_then(|info| info.get("Path"))
            .and_then(plist::Value::as_string)
        {
            Some(p) if !p.is_empty() => p,
            _ => {
                has_missing_fields = true;
                continue;
            }
        };

        let safe_rel = match safe_relative_path(path_str) {
            Some(p) => p,
            None => {
                component_observations.push(ComponentObservation {
                    name: comp_name.clone(),
                    path: path_str.to_string(),
                    state: ComponentState::MissingFile,
                    signature_verified: false,
                    device_availability_probed: false,
                    expected_digest: None,
                    observed_digest: None,
                    size_bytes: None,
                });
                has_missing_file = true;
                continue;
            }
        };

        // Try direct path or parent path (e.g. if manifest is in Restore/)
        let comp_path = if manifest_dir.join(&safe_rel).is_file() {
            manifest_dir.join(&safe_rel)
        } else if let Some(parent) = manifest_dir
            .parent()
            .filter(|p| p.join(&safe_rel).is_file())
        {
            parent.join(&safe_rel)
        } else {
            manifest_dir.join(&safe_rel)
        };

        let expected_digest_bytes = comp_dict.get("Digest").and_then(plist::Value::as_data);
        let expected_digest_hex = expected_digest_bytes.map(hex_encode);
        if checkpoint(deadline).is_err() {
            component_observations.push(ComponentObservation {
                name: comp_name.clone(),
                path: path_str.to_owned(),
                state: ComponentState::TimedOut,
                signature_verified: false,
                device_availability_probed: false,
                expected_digest: expected_digest_hex,
                observed_digest: None,
                size_bytes: None,
            });
            has_timeout = true;
            break;
        }

        let metadata = match std::fs::symlink_metadata(&comp_path) {
            Ok(m) => m,
            Err(err) => {
                let is_perm = err.kind() == std::io::ErrorKind::PermissionDenied;
                component_observations.push(ComponentObservation {
                    name: comp_name.clone(),
                    path: path_str.to_string(),
                    state: if is_perm {
                        ComponentState::PermissionDenied
                    } else {
                        ComponentState::MissingFile
                    },
                    signature_verified: false,
                    device_availability_probed: false,
                    expected_digest: expected_digest_hex,
                    observed_digest: None,
                    size_bytes: None,
                });
                if is_perm {
                    has_permission_denied = true;
                } else {
                    has_missing_file = true;
                }
                continue;
            }
        };

        let file_size = metadata.len();

        let (state, observed_digest_hex) = if !metadata.file_type().is_file() {
            // Directories, links and device files are not component evidence.
            has_missing_fields = true;
            (ComponentState::ReadFailed, None)
        } else if let Some(expected_bytes) = expected_digest_bytes {
            if file_size > MAX_COMPONENT_READ_BYTES {
                has_missing_fields = true;
                (ComponentState::ReadFailed, None)
            } else {
                match hash_file(&comp_path, expected_bytes.len(), deadline) {
                    Ok(observed_bytes) => {
                        let matches = observed_bytes == expected_bytes;
                        if !matches {
                            has_digest_mismatch = true;
                        }
                        let observed_hex = hex_encode(&observed_bytes);
                        (
                            if matches {
                                ComponentState::DigestMatches
                            } else {
                                ComponentState::DigestMismatch
                            },
                            Some(observed_hex),
                        )
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                        has_permission_denied = true;
                        (ComponentState::PermissionDenied, None)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                        has_timeout = true;
                        (ComponentState::TimedOut, None)
                    }
                    Err(_) => {
                        has_missing_fields = true;
                        (ComponentState::ReadFailed, None)
                    }
                }
            }
        } else {
            (ComponentState::FileExists, None)
        };

        component_observations.push(ComponentObservation {
            name: comp_name.clone(),
            path: path_str.to_string(),
            state,
            signature_verified: false, // Explicitly false: doctor is read-only
            device_availability_probed: false, // Explicitly false: doctor does not mount
            expected_digest: expected_digest_hex,
            observed_digest: observed_digest_hex,
            size_bytes: Some(file_size),
        });
    }

    let (outcome, detail, hint) = if has_timeout {
        (
            MatchOutcome::TimedOut,
            "Local DDI observation timed out; completed component observations are partial.".into(),
            Some(
                "Retry with a larger diagnostic budget or inspect local storage performance."
                    .into(),
            ),
        )
    } else if has_permission_denied {
        (
            MatchOutcome::PermissionDenied,
            "BuildManifest identity matched, but component read permission was denied on disk."
                .into(),
            Some("Check filesystem read permissions for DDI component files.".into()),
        )
    } else if has_missing_file {
        (
            MatchOutcome::MissingFile,
            format!(
                "BuildManifest identity matched, but {} referenced component file(s) are missing.",
                component_observations
                    .iter()
                    .filter(|c| c.state == ComponentState::MissingFile)
                    .count()
            ),
            Some(
                "Ensure complete DDI package files are downloaded alongside BuildManifest.plist."
                    .into(),
            ),
        )
    } else if has_digest_mismatch {
        (
            MatchOutcome::DigestMismatch,
            "BuildManifest identity matched, but one or more component digests do not match on disk.".into(),
            Some("Re-download or verify DDI bundle integrity; files may be corrupted.".into()),
        )
    } else if has_missing_fields
        || evidence.security_domain.is_none()
        || version_supported.is_none()
    {
        (MatchOutcome::MissingFields,
            "Component paths, target SecurityDomain or version-policy evidence are missing; no complete identity observation is available.".into(),
            Some("Inspect the selected DDI manifest and target personalization identifiers.".into()))
    } else {
        (
            MatchOutcome::Matched,
            format!(
                "Matched local DDI BuildManifest for target (board={:?}, chip={:?}): {} component(s) observed (file exists, digest matches, signature unverified, device availability unprobed; build compatibility unverified).",
                target_board, target_chip, component_observations.len()
            ),
            None,
        )
    };

    TargetManifestMetadata {
        manifest_path: Some(manifest_display),
        outcome: Some(outcome),
        matched_identities: 1,
        total_identities,
        product_version,
        product_build_version,
        components: component_observations,
        detail,
        hint,
    }
}

fn checkpoint(deadline: Option<tokio::time::Instant>) -> std::io::Result<()> {
    if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "local DDI observation deadline elapsed",
        ))
    } else {
        Ok(())
    }
}

fn hash_file(
    path: &Path,
    expected_digest_len: usize,
    deadline: Option<tokio::time::Instant>,
) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?.take(MAX_COMPONENT_READ_BYTES + 1);
    let mut read_bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    checkpoint(deadline)?;
    if ![32, 48].contains(&expected_digest_len) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "unsupported component digest algorithm",
        ));
    }

    if expected_digest_len == 48 {
        use sha2::Sha384;
        let mut hasher = Sha384::new();
        loop {
            checkpoint(deadline)?;
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            read_bytes += n as u64;
            if read_bytes > MAX_COMPONENT_READ_BYTES {
                return Err(std::io::Error::other("component read byte budget exceeded"));
            }
            hasher.update(&buffer[..n]);
        }
        Ok(hasher.finalize().to_vec())
    } else {
        let mut hasher = Sha256::new();
        loop {
            checkpoint(deadline)?;
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            read_bytes += n as u64;
            if read_bytes > MAX_COMPONENT_READ_BYTES {
                return Err(std::io::Error::other("component read byte budget exceeded"));
            }
            hasher.update(&buffer[..n]);
        }
        Ok(hasher.finalize().to_vec())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_target() -> TargetEvidence {
        TargetEvidence {
            board_id: Some(12),
            chip_id: Some(0x8110),
            security_domain: Some(1),
            product_version: Some("17.4.1".into()),
            ..Default::default()
        }
    }

    #[test]
    fn incomplete_identity_and_invalid_version_remain_unknown() {
        let root = std::env::temp_dir().join(format!("ddi-incomplete-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("image.dmg"), b"image").unwrap();
        let manifest = create_test_manifest(
            &root,
            12,
            0x8110,
            None,
            None,
            &[("PersonalizedDMG", "image.dmg", None)],
        );
        assert_eq!(
            evaluate_single_manifest(&complete_target(), &manifest, None).outcome,
            Some(MatchOutcome::MissingFields)
        );
        create_test_manifest(
            &root,
            12,
            0x8110,
            Some(1),
            None,
            &[("PersonalizedDMG", "image.dmg", None)],
        );
        for version in ["unavailable", "17.invalid", "17.4.invalid", "17"] {
            let mut target = complete_target();
            target.product_version = Some(version.into());
            assert_eq!(
                evaluate_single_manifest(&target, &manifest, None).outcome,
                Some(MatchOutcome::MissingFields),
                "{version}"
            );
        }
        std::fs::remove_file(manifest).unwrap();
        std::fs::remove_file(root.join("image.dmg")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn related_component_failure_is_not_hidden_by_unrelated_manifest_sort_order() {
        let root = std::env::temp_dir().join(format!("ddi-selection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let a = root.join("a");
        let z = root.join("z");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&z).unwrap();
        for reverse in [false, true] {
            let (related, unrelated) = if reverse { (&z, &a) } else { (&a, &z) };
            create_test_manifest(
                related,
                12,
                0x8110,
                Some(1),
                None,
                &[("PersonalizedDMG", "missing.dmg", None)],
            );
            create_test_manifest(
                unrelated,
                99,
                0x8110,
                Some(1),
                None,
                &[("PersonalizedDMG", "missing.dmg", None)],
            );
            let observed =
                evaluate_target_manifest(&complete_target(), std::slice::from_ref(&root));
            assert_eq!(observed.outcome, Some(MatchOutcome::MissingFile));
            assert_eq!(observed.matched_identities, 1);
        }
        for dir in [&a, &z] {
            std::fs::remove_file(dir.join("BuildManifest.plist")).unwrap();
            std::fs::remove_dir(dir).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn component_directory_without_digest_is_not_a_file_observation() {
        let root = std::env::temp_dir().join(format!("ddi-directory-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("image.dmg")).unwrap();
        let manifest = create_test_manifest(
            &root,
            12,
            0x8110,
            Some(1),
            None,
            &[("PersonalizedDMG", "image.dmg", None)],
        );
        let observed = evaluate_single_manifest(&complete_target(), &manifest, None);
        assert_eq!(observed.outcome, Some(MatchOutcome::MissingFields));
        assert_eq!(observed.components[0].state, ComponentState::ReadFailed);
        std::fs::remove_file(manifest).unwrap();
        std::fs::remove_dir(root.join("image.dmg")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn model_constraints_require_observed_target_and_valid_manifest_models() {
        let root = std::env::temp_dir().join(format!("ddi-models-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = create_test_manifest(
            &root,
            12,
            0x8110,
            Some(1),
            None,
            &[("PersonalizedDMG", "missing.dmg", None)],
        );
        let mut manifest = plist::Value::from_file(&path).unwrap();
        for models in [
            plist::Value::Array(vec![plist::Value::String("iPhone14,2".into())]),
            plist::Value::String("invalid".into()),
        ] {
            manifest
                .as_dictionary_mut()
                .unwrap()
                .insert("SupportedProductTypes".into(), models);
            manifest.to_file_xml(&path).unwrap();
            assert_eq!(
                evaluate_single_manifest(&complete_target(), &path, None).outcome,
                Some(MatchOutcome::MissingFields)
            );
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn generic_personalized_manifest_without_product_version_still_checks_target_minimum() {
        let temp =
            std::env::temp_dir().join(format!("ddi-generic-version-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        std::fs::write(temp.join("image.dmg"), b"image").unwrap();
        let manifest = create_test_manifest(
            &temp,
            12,
            0x8110,
            Some(1),
            None,
            &[("PersonalizedDMG", "image.dmg", None)],
        );
        let mut target = TargetEvidence {
            board_id: Some(12),
            chip_id: Some(0x8110),
            security_domain: Some(1),
            product_version: Some("16.6".into()),
            ..Default::default()
        };
        assert_eq!(
            evaluate_single_manifest(&target, &manifest, None).outcome,
            Some(MatchOutcome::VersionMismatch)
        );
        target.product_version = Some("17.4.1".into());
        assert_eq!(
            evaluate_single_manifest(&target, &manifest, None).outcome,
            Some(MatchOutcome::Matched)
        );
        target.product_version = None;
        assert_eq!(
            evaluate_single_manifest(&target, &manifest, None).outcome,
            Some(MatchOutcome::MissingFields)
        );
        std::fs::remove_file(manifest).unwrap();
        std::fs::remove_file(temp.join("image.dmg")).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }

    #[tokio::test]
    async fn expired_component_observation_is_partial_and_never_a_digest_mismatch() {
        let temp = std::env::temp_dir().join(format!("ddi-partial-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        std::fs::write(temp.join("image.dmg"), b"image").unwrap();
        let manifest = create_test_manifest(
            &temp,
            12,
            0x8110,
            Some(1),
            Some("18.0"),
            &[("PersonalizedDMG", "image.dmg", Some(&[0; 32]))],
        );
        let evidence = TargetEvidence {
            board_id: Some(12),
            chip_id: Some(0x8110),
            security_domain: Some(1),
            product_version: Some("17.4".into()),
            ..Default::default()
        };
        let metadata =
            evaluate_single_manifest(&evidence, &manifest, Some(tokio::time::Instant::now()));
        assert_eq!(metadata.outcome, Some(MatchOutcome::TimedOut));
        assert_eq!(metadata.components[0].state, ComponentState::TimedOut);
        assert!(metadata.components[0].observed_digest.is_none());
        std::fs::remove_file(manifest).unwrap();
        std::fs::remove_file(temp.join("image.dmg")).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }

    #[test]
    fn personalized_image_uses_hardware_across_os_and_build_versions() {
        let temp = std::env::temp_dir().join(format!("ddi-policy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        std::fs::write(temp.join("image.dmg"), b"image").unwrap();
        create_test_manifest(
            &temp,
            12,
            0x8110,
            Some(1),
            Some("18.0"),
            &[("PersonalizedDMG", "image.dmg", None)],
        );
        let mut evidence = TargetEvidence {
            board_id: Some(12),
            chip_id: Some(0x8110),
            security_domain: Some(1),
            product_type: Some("iPhone14,2".into()),
            product_version: Some("17.4.1".into()),
            build_version: Some("21E236".into()),
            ..Default::default()
        };
        assert_eq!(
            evaluate_target_manifest(&evidence, std::slice::from_ref(&temp)).outcome,
            Some(MatchOutcome::Matched)
        );
        evidence.board_id = Some(14);
        assert_eq!(
            evaluate_target_manifest(&evidence, std::slice::from_ref(&temp)).outcome,
            Some(MatchOutcome::NoMatchingIdentity)
        );
        evidence.board_id = Some(12);
        evidence.product_version = Some("16.6".into());
        assert_eq!(
            evaluate_target_manifest(&evidence, std::slice::from_ref(&temp)).outcome,
            Some(MatchOutcome::VersionMismatch)
        );
        std::fs::remove_file(temp.join("BuildManifest.plist")).unwrap();
        std::fs::remove_file(temp.join("image.dmg")).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }

    #[test]
    fn unreadable_search_is_incomplete_instead_of_no_manifest() {
        let file = std::env::temp_dir().join(format!("ddi-not-directory-{}", uuid::Uuid::new_v4()));
        std::fs::write(&file, b"not a directory").unwrap();
        let metadata =
            evaluate_target_manifest(&TargetEvidence::default(), std::slice::from_ref(&file));
        assert_eq!(metadata.outcome, Some(MatchOutcome::MissingFields));
        assert!(metadata.detail.contains("search was incomplete"));
        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn review_regression_unobserved_hardware_is_not_a_wildcard() {
        let temp = std::env::temp_dir().join(format!("doctor-unobserved-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&temp).unwrap();
        create_test_manifest(&temp, 16, 0x8101, Some(1), None, &[]);
        let metadata =
            evaluate_target_manifest(&TargetEvidence::default(), std::slice::from_ref(&temp));
        assert_eq!(metadata.outcome, Some(MatchOutcome::MissingFields));
        std::fs::remove_file(temp.join("BuildManifest.plist")).unwrap();
        std::fs::remove_dir(temp).unwrap();
    }

    fn create_test_manifest(
        dir: &Path,
        board: u64,
        chip: u64,
        sec_domain: Option<u64>,
        product_version: Option<&str>,
        components: &[(&str, &str, Option<&[u8]>)],
    ) -> PathBuf {
        let manifest_path = dir.join("BuildManifest.plist");
        let mut manifest_dict = plist::Dictionary::new();

        if let Some(pv) = product_version {
            manifest_dict.insert("ProductVersion".into(), plist::Value::String(pv.into()));
            manifest_dict.insert(
                "ProductBuildVersion".into(),
                plist::Value::String("21E219".into()),
            );
        }

        let mut identity = plist::Dictionary::new();
        identity.insert("ApBoardID".into(), plist::Value::Integer(board.into()));
        identity.insert("ApChipID".into(), plist::Value::Integer(chip.into()));
        if let Some(sd) = sec_domain {
            identity.insert("ApSecurityDomain".into(), plist::Value::Integer(sd.into()));
        }

        let mut manifest_components = plist::Dictionary::new();
        for (name, path, digest_opt) in components {
            let mut comp = plist::Dictionary::new();
            let mut info = plist::Dictionary::new();
            info.insert("Path".into(), plist::Value::String((*path).into()));
            comp.insert("Info".into(), plist::Value::Dictionary(info));

            if let Some(digest) = digest_opt {
                comp.insert("Digest".into(), plist::Value::Data(digest.to_vec()));
            }
            manifest_components.insert((*name).into(), plist::Value::Dictionary(comp));
        }

        identity.insert(
            "Manifest".into(),
            plist::Value::Dictionary(manifest_components),
        );
        manifest_dict.insert(
            "BuildIdentities".into(),
            plist::Value::Array(vec![plist::Value::Dictionary(identity)]),
        );

        let mut file = std::fs::File::create(&manifest_path).unwrap();
        plist::to_writer_xml(&mut file, &plist::Value::Dictionary(manifest_dict)).unwrap();

        manifest_path
    }

    #[test]
    fn test_evaluate_target_manifest_no_manifest_found() {
        let temp = std::env::temp_dir().join(format!("no_manifest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(16),
            chip_id: Some(0x8101),
            ..Default::default()
        };
        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::NoManifestFound));
        assert!(!meta.is_ok());
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn test_evaluate_target_manifest_matched_with_digests() {
        let temp = std::env::temp_dir().join(format!("match_manifest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        // Write a test component file
        let comp_content = b"sample-personalized-image-data";
        let mut hasher = Sha256::new();
        hasher.update(comp_content);
        let digest = hasher.finalize().to_vec();

        std::fs::write(temp.join("PersonalizedDMG.dmg"), comp_content).unwrap();

        create_test_manifest(
            &temp,
            16,
            0x8101,
            Some(1),
            Some("17.4"),
            &[("PersonalizedDMG", "PersonalizedDMG.dmg", Some(&digest))],
        );

        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(16),
            chip_id: Some(0x8101),
            security_domain: Some(1),
            product_version: Some("17.4".into()),
            ..Default::default()
        };

        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::Matched));
        assert!(meta.is_ok());
        assert_eq!(meta.components.len(), 1);
        assert_eq!(meta.components[0].state, ComponentState::DigestMatches);
        assert!(!meta.components[0].signature_verified);
        assert!(!meta.components[0].device_availability_probed);
        assert!(meta.detail.contains("signature unverified"));
        assert!(meta.detail.contains("device availability unprobed"));

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn test_evaluate_target_manifest_digest_mismatch() {
        let temp = std::env::temp_dir().join(format!("digest_mismatch_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        std::fs::write(temp.join("PersonalizedDMG.dmg"), b"corrupted-image-data").unwrap();
        let wrong_digest = vec![0xaa; 32];

        create_test_manifest(
            &temp,
            16,
            0x8101,
            None,
            Some("17.4"),
            &[(
                "PersonalizedDMG",
                "PersonalizedDMG.dmg",
                Some(&wrong_digest),
            )],
        );

        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(16),
            chip_id: Some(0x8101),
            ..Default::default()
        };

        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::DigestMismatch));
        assert!(!meta.is_ok());
        assert_eq!(meta.components[0].state, ComponentState::DigestMismatch);

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn test_evaluate_target_manifest_missing_component_file() {
        let temp = std::env::temp_dir().join(format!("missing_comp_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        // Do not write PersonalizedDMG.dmg to disk
        create_test_manifest(
            &temp,
            16,
            0x8101,
            None,
            Some("17.4"),
            &[("PersonalizedDMG", "PersonalizedDMG.dmg", None)],
        );

        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(16),
            chip_id: Some(0x8101),
            ..Default::default()
        };

        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::MissingFile));
        assert!(!meta.is_ok());
        assert_eq!(meta.components[0].state, ComponentState::MissingFile);

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn test_evaluate_target_manifest_version_mismatch() {
        let temp = std::env::temp_dir().join(format!("version_mismatch_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        create_test_manifest(&temp, 16, 0x8101, None, Some("16.4"), &[]);

        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(16),
            chip_id: Some(0x8101),
            product_version: Some("17.4".into()),
            ..Default::default()
        };

        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::VersionMismatch));
        assert!(!meta.is_ok());
        assert!(meta.detail.contains("does not match device version"));

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn test_evaluate_target_manifest_ambiguous_identities() {
        let temp = std::env::temp_dir().join(format!("ambiguous_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        // Create manifest with two identical board/chip identities
        let manifest_path = temp.join("BuildManifest.plist");
        let mut manifest_dict = plist::Dictionary::new();

        let mut id1 = plist::Dictionary::new();
        id1.insert("ApBoardID".into(), plist::Value::Integer(16.into()));
        id1.insert("ApChipID".into(), plist::Value::Integer(0x8101.into()));
        id1.insert(
            "Manifest".into(),
            plist::Value::Dictionary(plist::Dictionary::new()),
        );

        let mut id2 = plist::Dictionary::new();
        id2.insert("ApBoardID".into(), plist::Value::Integer(16.into()));
        id2.insert("ApChipID".into(), plist::Value::Integer(0x8101.into()));
        id2.insert(
            "Manifest".into(),
            plist::Value::Dictionary(plist::Dictionary::new()),
        );

        manifest_dict.insert(
            "BuildIdentities".into(),
            plist::Value::Array(vec![
                plist::Value::Dictionary(id1),
                plist::Value::Dictionary(id2),
            ]),
        );

        let mut file = std::fs::File::create(&manifest_path).unwrap();
        plist::to_writer_xml(&mut file, &plist::Value::Dictionary(manifest_dict)).unwrap();

        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(16),
            chip_id: Some(0x8101),
            ..Default::default()
        };

        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::AmbiguousIdentities));
        assert_eq!(meta.matched_identities, 2);

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn test_evaluate_target_manifest_no_matching_identity() {
        let temp = std::env::temp_dir().join(format!("no_match_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        create_test_manifest(&temp, 16, 0x8101, None, None, &[]);

        let evidence = TargetEvidence {
            udid: "00008101-0000000000000000".into(),
            board_id: Some(32),
            chip_id: Some(0x8030),
            ..Default::default()
        };

        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::NoMatchingIdentity));
        assert!(!meta.is_ok());

        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn cryptex_build_identity_does_not_poison_device_identity_evaluation() {
        let temp = std::env::temp_dir().join(format!("cryptex_manifest_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();

        let comp_content = b"sample-personalized-image-data";
        let mut hasher = Sha256::new();
        hasher.update(comp_content);
        let digest = hasher.finalize().to_vec();
        std::fs::write(temp.join("PersonalizedDMG.dmg"), comp_content).unwrap();

        let manifest_path = create_test_manifest(
            &temp,
            12,
            32800,
            Some(1),
            Some("1.0"),
            &[("PersonalizedDMG", "PersonalizedDMG.dmg", Some(&digest))],
        );

        let mut root = plist::Value::from_file(&manifest_path).unwrap();
        let mut cryptex = plist::Dictionary::new();
        cryptex.insert("Ap,SDKPlatform".into(), plist::Value::String("none".into()));
        cryptex.insert(
            "Ap,TargetType".into(),
            plist::Value::String("ioscryptex".into()),
        );
        cryptex.insert(
            "Cryptex1,ChipID".into(),
            plist::Value::String("0xFF10".into()),
        );
        cryptex.insert(
            "Manifest".into(),
            plist::Value::Dictionary(plist::Dictionary::new()),
        );
        root.as_dictionary_mut()
            .unwrap()
            .get_mut("BuildIdentities")
            .unwrap()
            .as_array_mut()
            .unwrap()
            .push(plist::Value::Dictionary(cryptex));
        root.to_file_xml(&manifest_path).unwrap();

        let mut evidence = TargetEvidence {
            udid: "00008020-0000000000000000".into(),
            board_id: Some(12),
            chip_id: Some(32800),
            security_domain: Some(1),
            product_version: Some("18.7.10".into()),
            ..Default::default()
        };

        // Valid components match despite trailing Cryptex identity
        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::Matched));
        assert_eq!(meta.matched_identities, 1);
        assert_eq!(meta.total_identities, 2);

        // Corrupted component digest is reported as DigestMismatch
        std::fs::write(temp.join("PersonalizedDMG.dmg"), b"corrupted").unwrap();
        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::DigestMismatch));

        // Missing component is reported as MissingFile
        std::fs::remove_file(temp.join("PersonalizedDMG.dmg")).unwrap();
        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::MissingFile));

        // Mismatched board/chip is reported as NoMatchingIdentity
        evidence.board_id = Some(99);
        let meta = evaluate_target_manifest(&evidence, std::slice::from_ref(&temp));
        assert_eq!(meta.outcome, Some(MatchOutcome::NoMatchingIdentity));

        let _ = std::fs::remove_dir_all(temp);
    }
}

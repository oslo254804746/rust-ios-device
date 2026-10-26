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
    pub(crate) udid: String,
    pub(crate) board_id: Option<u64>,
    pub(crate) chip_id: Option<u64>,
    pub(crate) security_domain: Option<u64>,
    pub(crate) product_type: Option<String>,
    pub(crate) product_version: Option<String>,
    pub(crate) build_version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ComponentState {
    FileExists,
    DigestMatches,
    DigestMismatch,
    MissingFile,
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
    DigestMismatch,
    VersionMismatch,
    NoManifestFound,
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
    let mut discovered = Vec::new();
    let mut inspected = 0usize;

    for base in search_paths {
        if !base.is_dir() {
            continue;
        }
        let mut queue = std::collections::VecDeque::new();
        queue.push_back((base.clone(), 0usize));

        while let Some((dir, depth)) = queue.pop_front() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                inspected += 1;
                if inspected > 4096 {
                    return discovered;
                }
                let kind = match entry.file_type() {
                    Ok(k) => k,
                    Err(_) => continue,
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

    discovered
}

/// Evaluate target evidence against local DDI BuildManifest assets.
pub(crate) fn evaluate_target_manifest(
    evidence: &TargetEvidence,
    search_paths: &[PathBuf],
) -> TargetManifestMetadata {
    let manifests = find_build_manifests(search_paths);
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

    // Try evaluating discovered manifests until a match or informative failure is found
    let mut last_metadata = None;

    for manifest_path in &manifests {
        let metadata = evaluate_single_manifest(evidence, manifest_path);
        if metadata.outcome == Some(MatchOutcome::Matched) {
            return metadata;
        }
        last_metadata = Some(metadata);
    }

    last_metadata.unwrap_or_default()
}

fn evaluate_single_manifest(
    evidence: &TargetEvidence,
    manifest_path: &Path,
) -> TargetManifestMetadata {
    let manifest_display = manifest_path.to_string_lossy().to_string();

    let data = match std::fs::read(manifest_path) {
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
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::MissingFile),
                detail: format!("Failed to read BuildManifest: {err}"),
                hint: Some("Check filesystem read permissions for DDI directories.".into()),
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

    // Check version match if device version is available
    if let (Some(device_ver), Some(manifest_ver)) = (
        evidence.product_version.as_deref(),
        product_version.as_deref(),
    ) {
        let dev_major = device_ver.split('.').next().unwrap_or("");
        let man_major = manifest_ver.split('.').next().unwrap_or("");
        if !dev_major.is_empty() && !man_major.is_empty() && dev_major != man_major {
            let detail = format!("DDI BuildManifest version ({manifest_ver}) does not match device version ({device_ver}).");
            return TargetManifestMetadata {
                manifest_path: Some(manifest_display),
                outcome: Some(MatchOutcome::VersionMismatch),
                matched_identities: 0,
                total_identities: 0,
                product_version,
                product_build_version,
                components: Vec::new(),
                detail,
                hint: Some("Select a DDI matching the target iOS major version.".into()),
            };
        }
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

    // Filter identities matching board and chip
    let mut matching_identities = Vec::new();

    for identity_val in identities {
        let identity_dict = match identity_val.as_dictionary() {
            Some(d) => d,
            None => continue,
        };

        let id_board = identity_dict.get("ApBoardID").and_then(parse_plist_int);
        let id_chip = identity_dict.get("ApChipID").and_then(parse_plist_int);
        let id_sec_domain = identity_dict
            .get("ApSecurityDomain")
            .and_then(parse_plist_int);

        // Required fields check: identity must have board and chip IDs
        if id_board.is_none() || id_chip.is_none() {
            continue;
        }

        let board_matches = match (target_board, id_board) {
            (Some(tb), Some(ib)) => tb == ib,
            (None, _) => true, // unprobed target accepts candidate
            _ => false,
        };

        let chip_matches = match (target_chip, id_chip) {
            (Some(tc), Some(ic)) => tc == ic,
            (None, _) => true,
            _ => false,
        };

        let sec_matches = match (evidence.security_domain, id_sec_domain) {
            (Some(tsd), Some(isd)) => tsd == isd,
            _ => true,
        };

        if board_matches && chip_matches && sec_matches {
            matching_identities.push(identity_dict);
        }
    }

    if matching_identities.is_empty() {
        return TargetManifestMetadata {
            manifest_path: Some(manifest_display),
            outcome: Some(MatchOutcome::NoMatchingIdentity),
            matched_identities: 0,
            total_identities,
            product_version,
            product_build_version,
            components: Vec::new(),
            detail: format!(
                "No matching BuildIdentity for target (board_id={:?}, chip_id={:?}, total identities in manifest: {total_identities}).",
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
    let mut has_digest_mismatch = false;

    for (comp_name, comp_val) in manifest_dict {
        let comp_dict = match comp_val.as_dictionary() {
            Some(d) => d,
            None => continue,
        };

        let path_str = match comp_dict
            .get("Info")
            .and_then(plist::Value::as_dictionary)
            .and_then(|info| info.get("Path"))
            .and_then(plist::Value::as_string)
        {
            Some(p) => p,
            None => continue,
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

        if !comp_path.is_file() {
            component_observations.push(ComponentObservation {
                name: comp_name.clone(),
                path: path_str.to_string(),
                state: ComponentState::MissingFile,
                signature_verified: false,
                device_availability_probed: false,
                expected_digest: expected_digest_hex,
                observed_digest: None,
                size_bytes: None,
            });
            has_missing_file = true;
            continue;
        }

        let metadata = match std::fs::symlink_metadata(&comp_path) {
            Ok(m) => m,
            Err(_) => {
                component_observations.push(ComponentObservation {
                    name: comp_name.clone(),
                    path: path_str.to_string(),
                    state: ComponentState::MissingFile,
                    signature_verified: false,
                    device_availability_probed: false,
                    expected_digest: expected_digest_hex,
                    observed_digest: None,
                    size_bytes: None,
                });
                has_missing_file = true;
                continue;
            }
        };

        let file_size = metadata.len();

        let (state, observed_digest_hex) = if let Some(expected_bytes) = expected_digest_bytes {
            if file_size > MAX_COMPONENT_READ_BYTES {
                (ComponentState::DigestMismatch, None)
            } else {
                match hash_file(&comp_path, expected_bytes.len()) {
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
                    Err(_) => {
                        has_digest_mismatch = true;
                        (ComponentState::DigestMismatch, None)
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

    let (outcome, detail, hint) = if has_missing_file {
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
    } else {
        (
            MatchOutcome::Matched,
            format!(
                "Matched local DDI BuildManifest for target (board={:?}, chip={:?}): {} component(s) observed (file exists, digest matches, signature unverified, device availability unprobed).",
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

fn hash_file(path: &Path, expected_digest_len: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];

    if expected_digest_len == 48 {
        use sha2::Sha384;
        let mut hasher = Sha384::new();
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(hasher.finalize().to_vec())
    } else {
        let mut hasher = Sha256::new();
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
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
        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
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

        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
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

        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
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

        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
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

        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
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

        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
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

        let meta = evaluate_target_manifest(&evidence, &[temp.clone()]);
        assert_eq!(meta.outcome, Some(MatchOutcome::NoMatchingIdentity));
        assert!(!meta.is_ok());

        let _ = std::fs::remove_dir_all(temp);
    }
}

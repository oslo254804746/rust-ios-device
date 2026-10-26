//! Read-only target personalization-manifest observation.
//!
//! Request/reply fields are the existing ImageMounterClient CopyDevices and
//! QueryPersonalizationManifest wire in services/imagemounter/protocol.rs.
//! The returned ImageSignature is opaque personalized manifest/ticket data,
//! not a local BuildManifest.plist or a verified Apple cryptographic signature.

use super::*;

const MAX_ENTRIES: usize = 128;
const MAX_QUERIES: usize = 32;
const MAX_SIGNATURE_BYTES: usize = 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_MANIFEST_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Default, Serialize)]
pub(crate) struct Metadata {
    pub(crate) mounted_entries: usize,
    pub(crate) developer_entries: usize,
    pub(crate) manifests_returned: usize,
    pub(crate) signature_bytes: usize,
    pub(crate) manifest_bytes: usize,
    pub(crate) complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_manifest: Option<super::target_manifest::TargetManifestMetadata>,
}

pub(super) async fn inspect(source: &dyn Source, udid: &str, timeout: Duration) -> Check {
    let mut metadata = Metadata::default();
    let result = async {
        validate_target(udid)?;
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "doctor manifest probe deadline overflow",
            )
        })?;
        let clock = Clock { deadline };
        clock
            .run(observe(source, udid, &clock, &mut metadata))
            .await
    }
    .await;

    // Observe local DDI BuildManifest matching against the explicit target
    let mut search_paths = Vec::new();
    if let Ok(env_path) = std::env::var("IOS_DDI_PATH") {
        search_paths.push(std::path::PathBuf::from(env_path));
    }
    if let Some(home) = super::super::dirs_home() {
        search_paths.push(home.join(".ios").join("ddi"));
        search_paths.push(home.join(".ios-rs").join("ddi"));
    }
    #[cfg(target_os = "macos")]
    {
        search_paths.push(std::path::PathBuf::from(
            "/Applications/Xcode.app/Contents/Developer/Platforms/iPhoneOS.platform/DeviceSupport",
        ));
    }
    let evidence = super::target_manifest::TargetEvidence {
        udid: udid.to_string(),
        ..Default::default()
    };
    let target_meta = super::target_manifest::evaluate_target_manifest(&evidence, &search_paths);
    metadata.target_manifest = Some(target_meta);

    let mut check = match result {
        Ok(value) => Check::observed(Probe::DdiManifests, value),
        Err(error) => Check::failure(Probe::DdiManifests, error),
    };
    check.ddi_manifests = Some(metadata);
    check
}

fn protocol(message: &'static str) -> anyhow::Error {
    LockdownError::Protocol(message.into()).into()
}

async fn observe(
    source: &dyn Source,
    udid: &str,
    clock: &Clock,
    metadata: &mut Metadata,
) -> Result<Option<bool>> {
    let (stream, pair_record, device_id) = session(source, udid, clock).await?;
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (port, use_ssl) = start_service_preserving_errors(
        &mut reader,
        &mut writer,
        ios_core::imagemounter::protocol::SERVICE_NAME,
    )
    .await?;
    drop((reader, writer));
    if port == 0 {
        return Err(protocol("doctor manifest StartService returned port zero"));
    }
    clock.check()?;
    let stream = clock.guard(source.port(device_id, port).await?);
    clock.check()?;
    let mut stream = if use_ssl {
        Box::new(wrap_service_tls(stream, &pair_record).await?) as ServiceStream
    } else {
        stream
    };
    send_lockdown(
        &mut stream,
        &plist::Dictionary::from_iter([("Command", plist::Value::String("CopyDevices".into()))]),
    )
    .await?;
    clock.check()?;
    // recv_lockdown rejects frames above its 4 MiB bound before allocation.
    let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
    check_response(&response, "CopyDevices")?;
    let Some(queries) = selected_signatures(&response, metadata)? else {
        return Ok(None);
    };
    drop(response);
    if queries.is_empty() {
        metadata.complete = true;
        return Ok(Some(false));
    }
    for signature in queries {
        clock.check()?;
        send_lockdown(
            &mut stream,
            &plist::Dictionary::from_iter([
                (
                    "Command",
                    plist::Value::String("QueryPersonalizationManifest".into()),
                ),
                (
                    "PersonalizedImageType",
                    plist::Value::String("DeveloperDiskImage".into()),
                ),
                (
                    "ImageType",
                    plist::Value::String("DeveloperDiskImage".into()),
                ),
                ("ImageSignature", plist::Value::Data(signature)),
            ]),
        )
        .await?;
        clock.check()?;
        let response: plist::Dictionary = recv_lockdown(&mut stream).await?;
        check_response(&response, "QueryPersonalizationManifest")?;
        let Some(manifest) = response.get("ImageSignature") else {
            return Ok(None);
        };
        let manifest = manifest.as_data().ok_or_else(|| {
            protocol("doctor personalization manifest ImageSignature was not data")
        })?;
        if manifest.is_empty() {
            return Ok(None);
        }
        if manifest.len() > MAX_MANIFEST_BYTES
            || manifest.len() > MAX_TOTAL_MANIFEST_BYTES - metadata.manifest_bytes
        {
            return Err(protocol(
                "doctor personalization manifest byte limit exceeded",
            ));
        }
        metadata.manifests_returned += 1;
        metadata.manifest_bytes += manifest.len();
    }
    clock.check()?;
    metadata.complete = true;
    Ok(Some(true))
}

fn selected_signatures(
    response: &plist::Dictionary,
    metadata: &mut Metadata,
) -> Result<Option<Vec<Vec<u8>>>> {
    let Some(entries) = response.get("EntryList") else {
        return Ok(None);
    };
    let entries = entries
        .as_array()
        .ok_or_else(|| protocol("doctor manifest CopyDevices EntryList was not an array"))?;
    if entries.len() > MAX_ENTRIES {
        return Err(protocol("doctor manifest mounted-entry limit exceeded"));
    }
    metadata.mounted_entries = entries.len();
    let mut selected = Vec::new();
    let mut missing = false;
    for entry in entries {
        let entry = entry
            .as_dictionary()
            .ok_or_else(|| protocol("doctor manifest CopyDevices entry was not a dictionary"))?;
        let Some(image_type) = entry.get("DiskImageType") else {
            missing = true;
            continue;
        };
        let image_type = image_type
            .as_string()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| protocol("doctor manifest DiskImageType was not a nonempty string"))?;
        if image_type != "Personalized" {
            continue;
        }
        let Some(personalized_type) = entry.get("PersonalizedImageType") else {
            missing = true;
            continue;
        };
        let personalized_type = personalized_type
            .as_string()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                protocol("doctor manifest PersonalizedImageType was not a nonempty string")
            })?;
        if personalized_type != "DeveloperDiskImage" {
            continue;
        }
        metadata.developer_entries += 1;
        let Some(signature) = entry.get("ImageSignature") else {
            missing = true;
            continue;
        };
        let signature = signature
            .as_data()
            .ok_or_else(|| protocol("doctor mounted ImageSignature was not data"))?;
        if signature.is_empty() {
            missing = true;
            continue;
        }
        if selected.len() >= MAX_QUERIES {
            return Err(protocol(
                "doctor personalization manifest query limit exceeded",
            ));
        }
        if signature.len() > MAX_SIGNATURE_BYTES - metadata.signature_bytes {
            return Err(protocol("doctor mounted signature byte limit exceeded"));
        }
        metadata.signature_bytes += signature.len();
        selected.push(signature.to_vec());
    }
    // Incomplete metadata cannot establish that every relevant mounted entry
    // was queried. Admit the whole observation before sending any query.
    Ok(if missing { None } else { Some(selected) })
}

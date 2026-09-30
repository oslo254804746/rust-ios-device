//! Device-side preflight request construction for normal-mode updater inspection.

use std::collections::HashMap;

use bytes::Bytes;
use indexmap::IndexMap;

use super::RestoreError;
use crate::xpc::XpcValue;

const MAX_PREFLIGHT_FIRMWARE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PLIST_DEPTH: usize = 64;

/// Updater names accepted by the device-side preflight payload builder.
pub const UPDATERS: &[&str] = &[
    "T200", "Rose", "SE", "Vinyl", "Savage", "Centauri", "Baseband",
];

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
    let data_size = i64::try_from(firmware.len())
        .map_err(|_| RestoreError::Protocol("FirmwareData exceeds signed DataSize".into()))?;
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

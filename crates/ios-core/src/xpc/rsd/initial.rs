//! Legacy initial-directory semantics over the admitted borrowed XPC arena.
//!
//! Only decoding scratch belongs to the bank. The public String/HashMap/Vec
//! directory can escape through into_rsd, Clone and public mutation; no permit
//! is attached to a shorter-lived ConnectedDevice sidecar.
use super::{RsdHandshake, ServiceDescriptor, XpcError};
use crate::xpc::message::borrowed::Arena;
use std::collections::HashMap;

pub(super) fn parse_handshake(tree: Option<&Arena<'_>>) -> Result<RsdHandshake, XpcError> {
    let dictionary = tree
        .and_then(|tree| tree.root().as_dict())
        .ok_or_else(|| XpcError::Tls("RSD: expected XPC dict body".into()))?;
    let message_type = dictionary
        .get("MessageType")
        .and_then(|value| value.as_str())
        .ok_or_else(|| XpcError::Tls("RSD: missing Handshake MessageType".into()))?;
    if message_type != "Handshake" {
        return Err(XpcError::Tls(
            "RSD: unexpected MessageType; expected Handshake".into(),
        ));
    }
    let udid = dictionary
        .get("Properties")
        .and_then(|value| value.as_dict())
        .and_then(|properties| properties.get("UniqueDeviceID"))
        .and_then(|value| value.as_str())
        .ok_or_else(|| XpcError::Tls("RSD: missing UniqueDeviceID".into()))?
        .to_owned();
    let mut services = HashMap::new();
    if let Some(entries) = dictionary.get("Services").and_then(|value| value.as_dict()) {
        tracing::debug!("RSD handshake exposed {} service entries", entries.len());
        for (name, value) in entries.iter() {
            let Some(descriptor) = value.as_dict() else {
                continue;
            };
            let port = descriptor.get("Port").and_then(|value| {
                if let Some(text) = value.as_str() {
                    text.parse::<u16>().ok()
                } else {
                    value.as_uint64().and_then(|port| u16::try_from(port).ok())
                }
            });
            let Some(port) = port else {
                tracing::debug!("RSD service has an invalid or missing Port");
                continue;
            };
            let features = descriptor
                .get("Properties")
                .and_then(|value| value.as_dict())
                .and_then(|properties| properties.get("Features"))
                .and_then(|value| value.as_array())
                .map(|features| {
                    features
                        .iter()
                        .filter_map(|value| value.as_str())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            services.insert(name.to_owned(), ServiceDescriptor { port, features });
        }
    } else if dictionary.get("Services").is_some() {
        tracing::debug!("RSD handshake Services has unexpected XPC type");
    } else {
        tracing::debug!("RSD handshake missing Services key");
    }
    Ok(RsdHandshake { udid, services })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xpc::message::{
        borrowed, decode_message, encode_message, flags, XpcMessage, XpcValue,
    };
    use indexmap::IndexMap;
    use std::sync::Arc;
    use tokio::sync::Semaphore;

    fn dictionary(entries: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
        XpcValue::Dictionary(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    fn handshake(services: Option<XpcValue>) -> XpcMessage {
        let mut body = IndexMap::from_iter([
            ("MessageType".into(), XpcValue::String("Handshake".into())),
            (
                "Properties".into(),
                dictionary([("UniqueDeviceID", XpcValue::String(String::new()))]),
            ),
        ]);
        if let Some(services) = services {
            body.insert("Services".into(), services);
        }
        XpcMessage {
            flags: flags::ALWAYS_SET,
            msg_id: 0,
            body: Some(XpcValue::Dictionary(body)),
        }
    }

    fn compare(message: XpcMessage) -> Result<RsdHandshake, XpcError> {
        let wire = encode_message(&message).unwrap();
        let legacy = super::super::parse_handshake_message(decode_message(wire.clone()).unwrap());
        let bank = Arc::new(Semaphore::new(1024 * 1024));
        let tree = borrowed::decode(&wire[..24], &wire[24..], &bank).unwrap();
        assert!(tree.is_none() || bank.available_permits() < 1024 * 1024);
        let actual = parse_handshake(tree.as_ref());
        match (&actual, &legacy) {
            (Ok(actual), Ok(legacy)) => {
                assert_eq!(actual.udid, legacy.udid);
                assert_eq!(actual.services, legacy.services);
            }
            (Err(actual), Err(legacy)) => {
                assert!(matches!(actual, XpcError::Tls(_)) && matches!(legacy, XpcError::Tls(_)));
                assert_eq!(actual.to_string(), legacy.to_string());
            }
            _ => panic!("initial parser diverged: actual={actual:?}, legacy={legacy:?}"),
        }
        drop(tree);
        assert_eq!(bank.available_permits(), 1024 * 1024);
        // The unchanged public directory remains usable after scratch release.
        actual
    }

    #[test]
    fn borrowed_initial_keeps_empty_uid_and_missing_or_wrong_services() {
        for services in [
            None,
            Some(XpcValue::Null),
            Some(XpcValue::Array(vec![])),
            Some(dictionary([])),
        ] {
            let directory = compare(handshake(services)).unwrap();
            assert!(directory.udid.is_empty());
            assert!(directory.services.is_empty());
        }
    }

    #[test]
    fn borrowed_initial_keeps_zero_ports_skips_bad_entries_and_filters_features() {
        let mut services = IndexMap::new();
        for (name, port) in [
            ("zero", XpcValue::Uint64(0)),
            ("string-zero", XpcValue::String("0".into())),
            ("max", XpcValue::Uint64(65535)),
            ("oversize", XpcValue::Uint64(65536)),
            ("negative", XpcValue::Int64(-1)),
            (
                "bad-string",
                XpcValue::String("invalid-private-port".into()),
            ),
            ("plus", XpcValue::String("+42".into())),
        ] {
            services.insert(
                name.into(),
                dictionary([
                    ("Port", port),
                    (
                        "Properties",
                        dictionary([(
                            "Features",
                            XpcValue::Array(vec![
                                XpcValue::String(String::new()),
                                XpcValue::Bool(false),
                                XpcValue::String("repeat".into()),
                                XpcValue::Null,
                                XpcValue::String("repeat".into()),
                            ]),
                        )]),
                    ),
                ]),
            );
        }
        services.insert("not-dict".into(), XpcValue::Null);
        services.insert("no-port".into(), dictionary([]));
        services.insert(
            "bad-properties".into(),
            dictionary([
                ("Port", XpcValue::Uint64(7)),
                ("Properties", XpcValue::Null),
            ]),
        );
        services.insert(
            "bad-features".into(),
            dictionary([
                ("Port", XpcValue::Uint64(8)),
                (
                    "Properties",
                    dictionary([("Features", XpcValue::String("ignored".into()))]),
                ),
            ]),
        );
        let directory = compare(handshake(Some(XpcValue::Dictionary(services)))).unwrap();
        assert_eq!(directory.services.len(), 6);
        assert_eq!(directory.services["zero"].port, 0);
        assert_eq!(
            directory.services["zero"].features,
            ["", "repeat", "repeat"]
        );
        assert_eq!(directory.services["plus"].port, 42);
        assert!(directory.services["bad-properties"].features.is_empty());
        assert!(directory.services["bad-features"].features.is_empty());
    }

    #[test]
    fn borrowed_initial_preserves_all_semantic_error_priorities() {
        let bodies = [
            None,
            Some(XpcValue::Null),
            Some(dictionary([])),
            Some(dictionary([("MessageType", XpcValue::Bool(false))])),
            Some(dictionary([(
                "MessageType",
                XpcValue::String("other".into()),
            )])),
            Some(dictionary([(
                "MessageType",
                XpcValue::String("Handshake".into()),
            )])),
            Some(dictionary([
                ("MessageType", XpcValue::String("Handshake".into())),
                (
                    "Properties",
                    dictionary([("UniqueDeviceID", XpcValue::Uint64(1))]),
                ),
            ])),
        ];
        for body in bodies {
            assert!(compare(XpcMessage {
                flags: flags::ALWAYS_SET,
                msg_id: 0,
                body
            })
            .is_err());
        }
    }
}

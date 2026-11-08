use super::assets::{boolean, integer, string};
use super::{CryptexError, Result};

/// Build a Cryptex1 request. AP personalization fields are deliberately absent:
/// Cryptex1 identifies the device through its 16-byte chip-ID/ECID value.
pub fn build_cryptex_tss_request(
    identity: &plist::Dictionary,
    chip_instance: &plist::Dictionary,
    nonce: &[u8],
) -> Result<plist::Dictionary> {
    use plist::Value;
    if nonce.is_empty() || nonce.len() > 64 {
        return Err(CryptexError::Protocol(
            "Cryptex signing nonce is empty or oversized".into(),
        ));
    }
    let mut request = plist::Dictionary::new();
    request.insert("@Cryptex1,Ticket".to_string(), Value::Boolean(true));
    request.insert("@HostPlatformInfo".to_string(), Value::String("mac".into()));
    request.insert(
        "@VersionInfo".to_string(),
        Value::String("libauthinstall-1104.0.9".into()),
    );
    request.insert(
        "@UUID".to_string(),
        Value::String(uuid::Uuid::new_v4().to_string().to_uppercase()),
    );
    for key in [
        "Cryptex1,ChipID",
        "Cryptex1,Type",
        "Cryptex1,SubType",
        "Cryptex1,ProductClass",
        "Cryptex1,NonceDomain",
    ] {
        request.insert(key.into(), Value::Integer(integer(identity, key)?.into()));
    }
    request.insert(
        "Cryptex1,UseProductClass".to_string(),
        Value::Boolean(boolean(identity, "Cryptex1,UseProductClass")?),
    );
    for key in ["Cryptex1,Version", "Cryptex1,PreauthorizationVersion"] {
        request.insert(key.into(), Value::String(string(identity, key)?.into()));
    }
    let production = match chip_instance.get("img4_chip_cpro") {
        Some(Value::Boolean(value)) => *value,
        Some(Value::Integer(value)) if value.as_unsigned().is_some_and(|value| value <= 1) => {
            value.as_unsigned() == Some(1)
        }
        _ => {
            return Err(CryptexError::Protocol(
                "invalid img4_chip_cpro production mode".into(),
            ))
        }
    };
    let mut udid = Vec::with_capacity(16);
    udid.extend_from_slice(&integer(chip_instance, "img4_chip_chip")?.to_be_bytes());
    udid.extend_from_slice(&integer(chip_instance, "img4_chip_ecid")?.to_be_bytes());
    request.insert("Cryptex1,UDID".to_string(), Value::Data(udid));
    request.insert("Cryptex1,Nonce".to_string(), Value::Data(nonce.to_vec()));
    request.insert(
        "Cryptex1,ProductionMode".to_string(),
        Value::Boolean(production),
    );
    request.insert(
        "Cryptex1,UniqueTagList".to_string(),
        Value::Data(Vec::new()),
    );

    let manifest = identity
        .get("Manifest")
        .and_then(Value::as_dictionary)
        .ok_or_else(|| CryptexError::Protocol("missing Cryptex component manifest".into()))?;
    for (key, entry) in manifest {
        if !key.starts_with("Cryptex1,") {
            continue;
        }
        let entry = entry
            .as_dictionary()
            .ok_or_else(|| CryptexError::Protocol(format!("invalid {key} component")))?;
        let personalize = entry
            .get("Info")
            .and_then(Value::as_dictionary)
            .and_then(|info| info.get("Personalize"))
            .and_then(Value::as_boolean)
            .unwrap_or(false);
        if !personalize {
            continue;
        }
        let digest = entry
            .get("Digest")
            .and_then(Value::as_data)
            .filter(|digest| !digest.is_empty() && digest.len() <= 64)
            .ok_or_else(|| CryptexError::Protocol(format!("missing or invalid {key} digest")))?;
        request.insert(
            key.clone(),
            Value::Dictionary(plist::Dictionary::from_iter([(
                "Digest".to_string(),
                Value::Data(digest.to_vec()),
            )])),
        );
    }
    for key in [
        "Cryptex1,GenericTrustCache",
        "Cryptex1,CryptexInfoPlist",
        "Cryptex1,GenericVolume",
    ] {
        if !request.contains_key(key) {
            return Err(CryptexError::Protocol(format!(
                "Cryptex manifest does not personalize {key}"
            )));
        }
    }
    Ok(request)
}

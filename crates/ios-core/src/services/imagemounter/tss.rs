//! TSS (Tatsu Signing Server) client for personalized DDI signing.
//!
//! POST XML plist to `https://gs.apple.com/TSS/controller?action=2`
//! Response: `STATUS=0&MESSAGE=SUCCESS&REQUEST_STRING=<plist>...`
//! Extract `ApImg4Ticket` from the response plist.
//!
//! Reference: go-ios/ios/imagemounter/tss.go

use super::protocol::ImageMounterError;

const TSS_URL: &str = "https://gs.apple.com/TSS/controller?action=2";
// Apple Inc. Root, published at https://www.apple.com/appleca/AppleIncRootCertificate.cer
// SHA-256: B0B1730ECBC7FF4505142C49F1295E6EDA6BCAED7E2C68C5BE91B5A11001F024
const APPLE_ROOT_CERT: &[u8] = include_bytes!("apple-inc-root.cer");

/// Get a personalized signing ticket from Apple's TSS server.
///
/// `request_dict` should be a plist dictionary containing the signing request
/// (board ID, chip ID, nonce, manifest entries, etc.)
pub async fn get_tss_ticket(
    request_dict: &plist::Dictionary,
) -> Result<Vec<u8>, ImageMounterError> {
    request_ticket(request_dict, &["ApImg4Ticket"]).await
}

/// Request a Cryptex1 ticket, accepting the AP ticket alias used by some TSS responses.
#[cfg(feature = "cryptex")]
pub async fn get_cryptex_tss_ticket(
    request_dict: &plist::Dictionary,
) -> Result<Vec<u8>, ImageMounterError> {
    request_ticket(request_dict, &["Cryptex1,Ticket", "ApImg4Ticket"]).await
}

async fn request_ticket(
    request_dict: &plist::Dictionary,
    ticket_keys: &[&str],
) -> Result<Vec<u8>, ImageMounterError> {
    let buf = serialize_request(request_dict)?;
    let builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(120));
    let proxy_url = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("HTTP_PROXY"));
    let client = build_client_with_proxy(builder, proxy_url.as_deref().ok())?;
    request_ticket_at(&client, TSS_URL, buf, ticket_keys).await
}

fn serialize_request(request_dict: &plist::Dictionary) -> Result<Vec<u8>, ImageMounterError> {
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(request_dict.clone())).map_err(
        |source| {
            super::native_error(
                std::io::ErrorKind::InvalidInput,
                "serialize DDI TSS request",
                source,
            )
        },
    )?;
    Ok(buf)
}

fn build_client_with_proxy(
    mut builder: reqwest::ClientBuilder,
    proxy_url: Option<&str>,
) -> Result<reqwest::Client, ImageMounterError> {
    let apple_root = reqwest::Certificate::from_der(APPLE_ROOT_CERT)
        .map_err(|source| super::native_http("parse Apple TSS root certificate", source))?;
    builder = builder.add_root_certificate(apple_root);
    if let Some(proxy_url) = proxy_url {
        if let Ok(proxy) = reqwest::Proxy::all(proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    builder
        .build()
        .map_err(|source| super::native_http("build DDI TSS client", source))
}

async fn request_ticket_at(
    client: &reqwest::Client,
    url: &str,
    buf: Vec<u8>,
    ticket_keys: &[&str],
) -> Result<Vec<u8>, ImageMounterError> {
    let mut resp = client
        .post(url)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("User-Agent", "InetURL/1.0")
        .body(buf)
        .send()
        .await
        .map_err(|source| super::native_http("request DDI TSS ticket", source))?;

    if !resp.status().is_success() {
        return Err(ImageMounterError::Tss(format!(
            "TSS returned HTTP {}",
            resp.status()
        )));
    }

    const MAX_TSS_RESPONSE: usize = 16 * 1024 * 1024;
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|source| super::native_http("read DDI TSS response", source))?
    {
        if chunk.len() > MAX_TSS_RESPONSE - body.len() {
            return Err(ImageMounterError::Tss("response exceeds 16 MiB".into()));
        }
        body.extend_from_slice(&chunk);
    }
    parse_ticket_response(&body, ticket_keys)
}

#[cfg(test)]
mod native_source_contract_tests;

fn parse_ticket_response(body: &[u8], ticket_keys: &[&str]) -> Result<Vec<u8>, ImageMounterError> {
    let body = std::str::from_utf8(body).map_err(|source| {
        super::native_error(
            std::io::ErrorKind::InvalidData,
            "decode DDI TSS response text",
            source,
        )
    })?;
    let status = body
        .split('&')
        .find_map(|field| field.strip_prefix("STATUS="));
    if status != Some("0") {
        return Err(ImageMounterError::Tss("TSS did not return STATUS=0".into()));
    }

    // Response format: STATUS=0&MESSAGE=SUCCESS&REQUEST_STRING=<plist>...</plist>
    let plist_start = body
        .find("<?xml")
        .or_else(|| body.find("<plist"))
        .ok_or_else(|| ImageMounterError::Tss("no plist in TSS response".into()))?;

    let plist_xml = &body[plist_start..];
    let val: plist::Value = plist::from_bytes(plist_xml.as_bytes()).map_err(|source| {
        super::native_error(
            std::io::ErrorKind::InvalidData,
            "parse DDI TSS response plist",
            source,
        )
    })?;

    let dict = val
        .as_dictionary()
        .ok_or_else(|| ImageMounterError::Tss("TSS response is not a dictionary".into()))?;

    let ticket = ticket_keys
        .iter()
        .find_map(|key| {
            dict.get(key)
                .and_then(plist::Value::as_data)
                .filter(|ticket| !ticket.is_empty())
        })
        .ok_or_else(|| ImageMounterError::Tss("signing ticket not found in TSS response".into()))?;

    Ok(ticket.to_vec())
}

/// Build a TSS request dictionary from personalization identifiers, nonce, and build manifest identity.
pub fn build_tss_request(
    identifiers: &std::collections::HashMap<String, plist::Value>,
    board_id: u64,
    chip_id: u64,
    ecid: u64,
    nonce: &[u8],
    identity: &plist::Dictionary,
) -> plist::Dictionary {
    let mut req = plist::Dictionary::new();

    // Standard TSS fields (matches go-ios tss.go)
    req.insert("@ApImg4Ticket".to_string(), plist::Value::Boolean(true));
    req.insert("@BBTicket".to_string(), plist::Value::Boolean(true));
    req.insert(
        "@HostPlatformInfo".to_string(),
        plist::Value::String("mac".into()),
    );
    req.insert(
        "@VersionInfo".to_string(),
        plist::Value::String("libauthinstall-1104.0.9".into()),
    );
    req.insert(
        "@UUID".to_string(),
        plist::Value::String(uuid::Uuid::new_v4().to_string().to_uppercase()),
    );

    // Only Ap-prefixed optional identifiers are passed through to TSS.
    for (k, v) in identifiers.iter().filter(|(key, _)| key.starts_with("Ap,")) {
        req.insert(k.clone(), v.clone());
    }

    req.insert("ApBoardID".into(), plist::Value::Integer(board_id.into()));
    req.insert("ApChipID".into(), plist::Value::Integer(chip_id.into()));
    req.insert("ApECID".into(), plist::Value::Integer(ecid.into()));
    req.insert("ApNonce".to_string(), plist::Value::Data(nonce.to_vec()));
    req.insert("ApProductionMode".into(), plist::Value::Boolean(true));
    req.insert(
        "ApSecurityDomain".into(),
        plist::Value::Integer(1u64.into()),
    );
    req.insert("ApSecurityMode".into(), plist::Value::Boolean(true));
    req.insert("SepNonce".to_string(), plist::Value::Data(vec![0; 20]));
    req.insert("UID_MODE".into(), plist::Value::Boolean(false));

    // Follow the DDI manifest's trust-cache rules for each trusted component.
    if let Some(manifest) = identity.get("Manifest").and_then(|v| v.as_dictionary()) {
        let rules = manifest
            .get("LoadableTrustCache")
            .and_then(plist::Value::as_dictionary)
            .and_then(|entry| entry.get("Info"))
            .and_then(plist::Value::as_dictionary)
            .and_then(|info| info.get("RestoreRequestRules"))
            .and_then(plist::Value::as_array);
        for (k, v) in manifest {
            let Some(entry) = v.as_dictionary() else {
                continue;
            };
            if entry
                .get("Info")
                .and_then(plist::Value::as_dictionary)
                .is_none()
                || entry.get("Trusted").and_then(plist::Value::as_boolean) != Some(true)
            {
                continue;
            }
            let mut entry = entry.clone();
            entry.remove("Info");
            if let Some(rules) = rules {
                for rule in rules.iter().filter_map(plist::Value::as_dictionary) {
                    let Some(conditions) =
                        rule.get("Conditions").and_then(plist::Value::as_dictionary)
                    else {
                        continue;
                    };
                    let matched = conditions.iter().all(|(key, value)| {
                        matches!(
                            key.as_str(),
                            "ApRawProductionMode"
                                | "ApCurrentProductionMode"
                                | "ApRawSecurityMode"
                                | "ApRequiresImage4"
                        ) && value.as_boolean() == Some(true)
                    });
                    if matched {
                        if let Some(actions) =
                            rule.get("Actions").and_then(plist::Value::as_dictionary)
                        {
                            for (key, value) in actions {
                                if value.as_unsigned_integer() != Some(255) {
                                    entry.insert(key.clone(), value.clone());
                                }
                            }
                        }
                    }
                }
            }
            if !entry.contains_key("Digest") {
                entry.insert("Digest".into(), plist::Value::Data(Vec::new()));
            }
            req.insert(k.clone(), plist::Value::Dictionary(entry));
        }
    }

    req
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn apple_tss_root_has_official_fingerprint_and_builds_client() {
        use sha2::{Digest, Sha256};
        assert_eq!(
            hex::encode_upper(Sha256::digest(APPLE_ROOT_CERT)),
            "B0B1730ECBC7FF4505142C49F1295E6EDA6BCAED7E2C68C5BE91B5A11001F024"
        );
        build_client_with_proxy(reqwest::Client::builder().no_proxy(), None).unwrap();
    }

    #[test]
    fn personalized_request_uses_device_identity_and_trusted_manifest_rules() {
        use plist::{Dictionary, Value};
        let dict = |entries: Vec<(&str, Value)>| Value::Dictionary(Dictionary::from_iter(entries));
        let rule = dict(vec![
            (
                "Conditions",
                dict(vec![
                    ("ApRawProductionMode", Value::Boolean(true)),
                    ("ApRawSecurityMode", Value::Boolean(true)),
                ]),
            ),
            (
                "Actions",
                dict(vec![
                    ("EPRO", Value::Boolean(true)),
                    ("ESEC", Value::Boolean(true)),
                    ("Ignored", Value::Integer(255u64.into())),
                ]),
            ),
        ]);
        let false_rule = dict(vec![
            (
                "Conditions",
                dict(vec![("ApRequiresImage4", Value::Boolean(false))]),
            ),
            ("Actions", dict(vec![("Wrong", Value::Boolean(true))])),
        ]);
        let info = dict(vec![(
            "RestoreRequestRules",
            Value::Array(vec![rule, false_rule]),
        )]);
        let trusted = dict(vec![
            ("Trusted", Value::Boolean(true)),
            ("Info", info),
            ("Digest", Value::Data(vec![42])),
        ]);
        let image = dict(vec![
            ("Trusted", Value::Boolean(true)),
            ("Info", dict(vec![])),
        ]);
        let untrusted = dict(vec![
            ("Trusted", Value::Boolean(false)),
            ("Info", dict(vec![])),
        ]);
        let identity = Dictionary::from_iter([(
            "Manifest",
            dict(vec![
                ("LoadableTrustCache", trusted),
                ("PersonalizedDMG", image),
                ("Untrusted", untrusted),
                ("NoInfo", dict(vec![("Trusted", Value::Boolean(true))])),
            ]),
        )]);
        let identifiers = std::collections::HashMap::from([
            ("Ap,Optional".to_string(), Value::Boolean(true)),
            ("UniqueChipID".to_string(), Value::Integer(99u64.into())),
        ]);
        let request = build_tss_request(&identifiers, 1, 2, 99, &[3, 4], &identity);
        for (key, value) in [
            ("ApBoardID", 1),
            ("ApChipID", 2),
            ("ApECID", 99),
            ("ApSecurityDomain", 1),
        ] {
            assert_eq!(request[key].as_unsigned_integer(), Some(value));
        }
        assert_eq!(request["Ap,Optional"].as_boolean(), Some(true));
        assert_eq!(request["UID_MODE"].as_boolean(), Some(false));
        assert_eq!(request["ApProductionMode"].as_boolean(), Some(true));
        assert_eq!(request["ApSecurityMode"].as_boolean(), Some(true));
        assert_eq!(request["ApNonce"].as_data(), Some([3, 4].as_slice()));
        assert!(!request.contains_key("UniqueChipID"));
        assert!(!request.contains_key("Untrusted"));
        assert!(!request.contains_key("NoInfo"));
        for name in ["LoadableTrustCache", "PersonalizedDMG"] {
            let entry = request[name].as_dictionary().unwrap();
            assert!(!entry.contains_key("Info"));
            assert!(!entry.contains_key("Ignored"));
            assert!(!entry.contains_key("Wrong"));
            assert_eq!(entry["EPRO"].as_boolean(), Some(true));
            assert_eq!(entry["ESEC"].as_boolean(), Some(true));
        }
        assert_eq!(
            request["PersonalizedDMG"].as_dictionary().unwrap()["Digest"].as_data(),
            Some([].as_slice())
        );
        assert_eq!(
            request["LoadableTrustCache"].as_dictionary().unwrap()["Digest"].as_data(),
            Some([42].as_slice())
        );
    }

    fn reply(status: &str, tickets: &[(&str, &[u8])]) -> Vec<u8> {
        let dictionary = plist::Dictionary::from_iter(
            tickets
                .iter()
                .map(|(name, bytes)| (*name, plist::Value::Data(bytes.to_vec()))),
        );
        let mut bytes = format!("STATUS={status}&MESSAGE=fixture&REQUEST_STRING=").into_bytes();
        plist::to_writer_xml(&mut bytes, &dictionary).unwrap();
        bytes
    }

    #[test]
    fn cryptex_ticket_preferred_with_ap_alias_fallback() {
        let keys = ["Cryptex1,Ticket", "ApImg4Ticket"];
        assert_eq!(
            parse_ticket_response(
                &reply(
                    "0",
                    &[("Cryptex1,Ticket", b"cryptex"), ("ApImg4Ticket", b"ap")]
                ),
                &keys
            )
            .unwrap(),
            b"cryptex"
        );
        assert_eq!(
            parse_ticket_response(&reply("0", &[("ApImg4Ticket", b"fallback")]), &keys).unwrap(),
            b"fallback"
        );
        assert!(
            parse_ticket_response(&reply("94", &[("Cryptex1,Ticket", b"invalid")]), &keys).is_err()
        );
        assert!(parse_ticket_response(&reply("0", &[("Cryptex1,Ticket", b"")]), &keys).is_err());
    }

    #[test]
    fn standard_tss_still_requires_an_ap_ticket() {
        assert_eq!(
            parse_ticket_response(
                &reply("0", &[("ApImg4Ticket", b"legacy")]),
                &["ApImg4Ticket"]
            )
            .unwrap(),
            b"legacy"
        );
        assert!(parse_ticket_response(
            &reply("0", &[("Cryptex1,Ticket", b"wrong-type")]),
            &["ApImg4Ticket"]
        )
        .is_err());
    }
}

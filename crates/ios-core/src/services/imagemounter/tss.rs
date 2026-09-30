//! TSS (Tatsu Signing Server) client for personalized DDI signing.
//!
//! POST XML plist to `https://gs.apple.com/TSS/controller?action=2`
//! Response: `STATUS=0&MESSAGE=SUCCESS&REQUEST_STRING=<plist>...`
//! Extract `ApImg4Ticket` from the response plist.
//!
//! Reference: go-ios/ios/imagemounter/tss.go

use super::protocol::ImageMounterError;

const TSS_URL: &str = "https://gs.apple.com/TSS/controller?action=2";

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
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(request_dict.clone()))
        .map_err(|e| ImageMounterError::Tss(format!("serialize request: {e}")))?;

    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(120));
    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("HTTP_PROXY")) {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    let client = builder
        .build()
        .map_err(|e| ImageMounterError::Tss(format!("build HTTP client: {e}")))?;
    let mut resp = client
        .post(TSS_URL)
        .header("Content-Type", "text/xml; charset=\"utf-8\"")
        .header("User-Agent", "InetURL/1.0")
        .body(buf)
        .send()
        .await
        .map_err(|e| ImageMounterError::Tss(format!("HTTP request failed: {e}")))?;

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
        .map_err(|e| ImageMounterError::Tss(format!("read response: {e}")))?
    {
        if chunk.len() > MAX_TSS_RESPONSE - body.len() {
            return Err(ImageMounterError::Tss("response exceeds 16 MiB".into()));
        }
        body.extend_from_slice(&chunk);
    }
    parse_ticket_response(&body, ticket_keys)
}

fn parse_ticket_response(body: &[u8], ticket_keys: &[&str]) -> Result<Vec<u8>, ImageMounterError> {
    let body = std::str::from_utf8(body)
        .map_err(|_| ImageMounterError::Tss("response is not UTF-8".into()))?;
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
    let val: plist::Value = plist::from_bytes(plist_xml.as_bytes())
        .map_err(|e| ImageMounterError::Tss(format!("parse TSS plist: {e}")))?;

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
        plist::Value::String("libauthinstall-973.40.2".into()),
    );
    req.insert(
        "@UUID".to_string(),
        plist::Value::String(uuid::Uuid::new_v4().to_string().to_uppercase()),
    );

    // Copy personalization identifiers (BoardId, ChipID, etc.)
    for (k, v) in identifiers {
        req.insert(k.clone(), v.clone());
    }

    // Nonce
    req.insert("ApNonce".to_string(), plist::Value::Data(nonce.to_vec()));
    req.insert("SepNonce".to_string(), plist::Value::Data(vec![0; 20]));

    // Copy manifest identity entries
    if let Some(manifest) = identity.get("Manifest").and_then(|v| v.as_dictionary()) {
        for (k, v) in manifest {
            req.insert(k.clone(), v.clone());
        }
    }

    req
}

#[cfg(test)]
mod tests {
    use super::*;

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

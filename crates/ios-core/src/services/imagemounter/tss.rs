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
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(request_dict.clone()))
        .map_err(|e| ImageMounterError::Tss(format!("serialize request: {e}")))?;

    let mut builder = reqwest::Client::builder();
    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY").or_else(|_| std::env::var("HTTP_PROXY")) {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }
    let client = builder.build().unwrap_or_default();
    let resp = client
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

    let body = resp
        .text()
        .await
        .map_err(|e| ImageMounterError::Tss(format!("read response: {e}")))?;

    // Response format: STATUS=0&MESSAGE=SUCCESS&REQUEST_STRING=<plist>...</plist>
    let plist_start = body
        .find("<?xml")
        .or_else(|| body.find("<plist"))
        .ok_or_else(|| {
            ImageMounterError::Tss(format!(
                "no plist in TSS response: {}",
                &body[..body.len().min(200)]
            ))
        })?;

    let plist_xml = &body[plist_start..];
    let val: plist::Value = plist::from_bytes(plist_xml.as_bytes())
        .map_err(|e| ImageMounterError::Tss(format!("parse TSS plist: {e}")))?;

    let dict = val
        .as_dictionary()
        .ok_or_else(|| ImageMounterError::Tss("TSS response is not a dictionary".into()))?;

    // Extract ApImg4Ticket from the response
    let ticket = dict
        .get("ApImg4Ticket")
        .and_then(|v| v.as_data())
        .ok_or_else(|| ImageMounterError::Tss("ApImg4Ticket not found in TSS response".into()))?;

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

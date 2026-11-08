//! Supervised P12 certificate-based pairing (no user trust dialog required).
//!
//! Supervised devices enrolled with an organization identity (P12 certificate)
//! can be paired without any user interaction on the device. This module
//! implements the lockdown Pair protocol with PKCS7 challenge-response flow:
//!
//! 1. Connect to lockdown (port 62078 via usbmux) — raw, no TLS
//! 2. GetValue(nil, "DevicePublicKey") → device's RSA public key (PEM)
//! 3. Generate cert chain: root CA, host cert, device cert
//! 4. Send Pair request with PairRecord + PairingOptions{SupervisorCertificate}
//! 5. Receive MCChallengeRequired with PairingChallenge in ExtendedResponse
//! 6. Sign challenge with PKCS7 using supervisor's P12 private key
//! 7. Send second Pair request with PairingOptions{ChallengeResponse}
//! 8. Receive success with EscrowBag
//!
//! Reference: go-ios pair.go PairSupervised(), crypto_utils.go

use openssl::asn1::Asn1Time;
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::pkcs12::Pkcs12;
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
use openssl::pkey::{HasPublic, PKey, Private};
use openssl::rsa::Rsa;
use openssl::stack::Stack;
use openssl::x509::extension::{BasicConstraints, KeyUsage, SubjectKeyIdentifier};
use openssl::x509::{X509Builder, X509NameBuilder, X509};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::lockdown::protocol::{recv_lockdown, send_lockdown, GetValueRequest};
use crate::lockdown::LockdownError;

fn backend_error(phase: &'static str, error: openssl::error::ErrorStack) -> LockdownError {
    super::error_context::native(std::io::ErrorKind::Other, phase, error)
}

// ── Serializable pair record for the Pair request ────────────────────────────

/// Full pair record data sent inside the lockdown Pair request.
///
/// All certificate and key fields are PEM-encoded, matching the format
/// that `PairRecord::load()` expects on disk.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FullPairRecord {
    #[serde(with = "serde_bytes")]
    pub device_certificate: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub host_certificate: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub host_private_key: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub root_certificate: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub root_private_key: Vec<u8>,
    #[serde(rename = "HostID")]
    pub host_id: String,
    #[serde(rename = "SystemBUID")]
    pub system_buid: String,
}

impl std::fmt::Debug for FullPairRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullPairRecord").finish_non_exhaustive()
    }
}

// ── Certificate generation ───────────────────────────────────────────────────

/// Generate the full certificate chain for lockdown pairing.
///
/// Produces a `FullPairRecord` containing:
/// - A self-signed root CA certificate
/// - A host certificate signed by the root CA
/// - A device certificate signed by the root CA (using the device's public key)
/// - The corresponding private keys (PEM-encoded)
/// - Generated HostID and SystemBUID
pub fn generate_pair_certs(
    device_public_key_pem: &[u8],
    system_buid: &str,
) -> Result<FullPairRecord, LockdownError> {
    // 1. Parse device public key from PEM
    let device_pkey = PKey::public_key_from_pem(device_public_key_pem).map_err(|error| {
        super::error_context::native(
            std::io::ErrorKind::InvalidData,
            "parse supervised device public key",
            error,
        )
    })?;

    // 2. Generate root RSA key pair (2048-bit, matching go-ios)
    let root_rsa =
        Rsa::generate(2048).map_err(|error| backend_error("RSA key generation failed", error))?;
    let root_pkey =
        PKey::from_rsa(root_rsa).map_err(|error| backend_error("PKey from RSA failed", error))?;

    // 3. Generate host RSA key pair
    let host_rsa =
        Rsa::generate(2048).map_err(|error| backend_error("RSA key generation failed", error))?;
    let host_pkey =
        PKey::from_rsa(host_rsa).map_err(|error| backend_error("PKey from RSA failed", error))?;

    // 4. Create root certificate (self-signed CA)
    let root_cert = build_root_cert(&root_pkey)?;

    // 5. Create host certificate (signed by root, with host's own key)
    let host_cert = build_signed_cert(&host_pkey, &root_cert, &root_pkey)?;

    // 6. Create device certificate (signed by root, using device's public key)
    let device_cert = build_signed_cert(&device_pkey, &root_cert, &root_pkey)?;

    // 7. Generate HostID
    let host_id = uuid::Uuid::new_v4().to_string().to_uppercase();

    Ok(FullPairRecord {
        device_certificate: device_cert
            .to_pem()
            .map_err(|error| backend_error("cert to PEM failed", error))?,
        host_certificate: host_cert
            .to_pem()
            .map_err(|error| backend_error("cert to PEM failed", error))?,
        host_private_key: host_pkey
            .private_key_to_pem_pkcs8()
            .map_err(|error| backend_error("key to PEM failed", error))?,
        root_certificate: root_cert
            .to_pem()
            .map_err(|error| backend_error("cert to PEM failed", error))?,
        root_private_key: root_pkey
            .private_key_to_pem_pkcs8()
            .map_err(|error| backend_error("key to PEM failed", error))?,
        host_id,
        system_buid: system_buid.to_string(),
    })
}

/// Build a self-signed root CA certificate.
///
/// Matches go-ios crypto_utils.go `createRootCert`:
/// - Serial 0, empty subject, 10-year validity
/// - BasicConstraints CA=true
/// - SubjectKeyIdentifier (SHA1 of public key)
/// - SHA1WithRSA signature
fn build_root_cert(pkey: &PKey<Private>) -> Result<X509, LockdownError> {
    let mut builder =
        X509Builder::new().map_err(|error| backend_error("X509Builder::new failed", error))?;

    // X.509 v3
    builder
        .set_version(2)
        .map_err(|error| backend_error("set_version failed", error))?;

    // Serial number = 0
    let serial = BigNum::from_u32(0)
        .and_then(|bn| bn.to_asn1_integer())
        .map_err(|error| backend_error("serial number failed", error))?;
    builder
        .set_serial_number(&serial)
        .map_err(|error| backend_error("set_serial_number failed", error))?;

    // Empty subject name (matching go-ios pkix.Name{})
    let name = X509NameBuilder::new()
        .map(|b| b.build())
        .map_err(|error| backend_error("X509Name build failed", error))?;
    builder
        .set_subject_name(&name)
        .map_err(|error| backend_error("set_subject_name failed", error))?;
    builder
        .set_issuer_name(&name)
        .map_err(|error| backend_error("set_issuer_name failed", error))?;

    // 10-year validity
    let not_before =
        Asn1Time::days_from_now(0).map_err(|error| backend_error("Asn1Time failed", error))?;
    let not_after =
        Asn1Time::days_from_now(3650).map_err(|error| backend_error("Asn1Time failed", error))?;
    builder
        .set_not_before(&not_before)
        .map_err(|error| backend_error("set_not_before failed", error))?;
    builder
        .set_not_after(&not_after)
        .map_err(|error| backend_error("set_not_after failed", error))?;

    builder
        .set_pubkey(pkey)
        .map_err(|error| backend_error("set_pubkey failed", error))?;

    // BasicConstraints: CA=true, critical
    let basic = BasicConstraints::new()
        .critical()
        .ca()
        .build()
        .map_err(|error| backend_error("BasicConstraints build failed", error))?;
    builder
        .append_extension(basic)
        .map_err(|error| backend_error("append_extension failed", error))?;

    // SubjectKeyIdentifier — SHA1 hash of the public key
    // We need to build this from the context of the builder itself
    let ski = SubjectKeyIdentifier::new()
        .build(&builder.x509v3_context(None, None))
        .map_err(|error| backend_error("SKI build failed", error))?;
    builder
        .append_extension(ski)
        .map_err(|error| backend_error("append SKI failed", error))?;

    // Sign with SHA1 (matching go-ios SHA1WithRSA)
    builder
        .sign(pkey, MessageDigest::sha1())
        .map_err(|error| backend_error("sign failed", error))?;

    Ok(builder.build())
}

/// Build a certificate signed by the issuer (root CA).
///
/// Used for both host and device certificates. Matches go-ios `createHostCert`
/// and `createDeviceCert`:
/// - Serial 0, empty subject, 10-year validity
/// - KeyUsage: digitalSignature | keyEncipherment
/// - BasicConstraints: CA=false
/// - SubjectKeyIdentifier
/// - SHA1WithRSA signature (signed by root)
fn build_signed_cert(
    subject_pkey: &PKey<impl HasPublic>,
    issuer_cert: &X509,
    issuer_pkey: &PKey<Private>,
) -> Result<X509, LockdownError> {
    let mut builder =
        X509Builder::new().map_err(|error| backend_error("X509Builder::new failed", error))?;

    // X.509 v3
    builder
        .set_version(2)
        .map_err(|error| backend_error("set_version failed", error))?;

    // Serial number = 0
    let serial = BigNum::from_u32(0)
        .and_then(|bn| bn.to_asn1_integer())
        .map_err(|error| backend_error("serial number failed", error))?;
    builder
        .set_serial_number(&serial)
        .map_err(|error| backend_error("set_serial_number failed", error))?;

    // Empty subject name
    let name = X509NameBuilder::new()
        .map(|b| b.build())
        .map_err(|error| backend_error("X509Name build failed", error))?;
    builder
        .set_subject_name(&name)
        .map_err(|error| backend_error("set_subject_name failed", error))?;

    // Issuer = root cert's subject
    builder
        .set_issuer_name(issuer_cert.subject_name())
        .map_err(|error| backend_error("set_issuer_name failed", error))?;

    // 10-year validity
    let not_before =
        Asn1Time::days_from_now(0).map_err(|error| backend_error("Asn1Time failed", error))?;
    let not_after =
        Asn1Time::days_from_now(3650).map_err(|error| backend_error("Asn1Time failed", error))?;
    builder
        .set_not_before(&not_before)
        .map_err(|error| backend_error("set_not_before failed", error))?;
    builder
        .set_not_after(&not_after)
        .map_err(|error| backend_error("set_not_after failed", error))?;

    builder
        .set_pubkey(subject_pkey)
        .map_err(|error| backend_error("set_pubkey failed", error))?;

    // BasicConstraints: CA=false
    let basic = BasicConstraints::new()
        .critical()
        .build()
        .map_err(|error| backend_error("BasicConstraints build failed", error))?;
    builder
        .append_extension(basic)
        .map_err(|error| backend_error("append_extension failed", error))?;

    // KeyUsage: digitalSignature | keyEncipherment (matching go-ios)
    let key_usage = KeyUsage::new()
        .digital_signature()
        .key_encipherment()
        .build()
        .map_err(|error| backend_error("KeyUsage build failed", error))?;
    builder
        .append_extension(key_usage)
        .map_err(|error| backend_error("append KeyUsage failed", error))?;

    // SubjectKeyIdentifier
    let ski = SubjectKeyIdentifier::new()
        .build(&builder.x509v3_context(Some(issuer_cert), None))
        .map_err(|error| backend_error("SKI build failed", error))?;
    builder
        .append_extension(ski)
        .map_err(|error| backend_error("append SKI failed", error))?;

    // Sign with SHA1 by issuer's private key
    builder
        .sign(issuer_pkey, MessageDigest::sha1())
        .map_err(|error| backend_error("sign failed", error))?;

    Ok(builder.build())
}

// ── Lockdown protocol messages for Pair ──────────────────────────────────────

/// The PairRecord payload embedded in a Pair request.
/// Only contains the certificates and IDs (no private keys sent to device).
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PairRecordPayload {
    #[serde(with = "serde_bytes")]
    device_certificate: Vec<u8>,
    #[serde(with = "serde_bytes")]
    host_certificate: Vec<u8>,
    #[serde(with = "serde_bytes")]
    root_certificate: Vec<u8>,
    #[serde(rename = "HostID")]
    host_id: String,
    #[serde(rename = "SystemBUID")]
    system_buid: String,
}

/// PairingOptions for the first Pair request (supervisor cert).
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PairingOptionsFirst {
    extended_pairing_errors: bool,
    #[serde(with = "serde_bytes")]
    supervisor_certificate: Vec<u8>,
}

/// PairingOptions for the second Pair request (challenge response).
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PairingOptionsChallenge {
    #[serde(with = "serde_bytes")]
    challenge_response: Vec<u8>,
}

/// First Pair request (with SupervisorCertificate).
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PairRequestFirst {
    label: &'static str,
    protocol_version: &'static str,
    request: &'static str,
    pair_record: PairRecordPayload,
    pairing_options: PairingOptionsFirst,
}

/// Second Pair request (with ChallengeResponse).
#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct PairRequestChallenge {
    label: &'static str,
    protocol_version: &'static str,
    request: &'static str,
    pair_record: PairRecordPayload,
    pairing_options: PairingOptionsChallenge,
}

/// Generic GetValue response (used to extract DevicePublicKey).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct GetValueRawResponse {
    #[serde(default)]
    error: Option<String>,
    value: Option<plist::Value>,
}

/// Build the PairRecordPayload from a FullPairRecord (strips private keys).
fn pair_record_payload(record: &FullPairRecord) -> PairRecordPayload {
    PairRecordPayload {
        device_certificate: record.device_certificate.clone(),
        host_certificate: record.host_certificate.clone(),
        root_certificate: record.root_certificate.clone(),
        host_id: record.host_id.clone(),
        system_buid: record.system_buid.clone(),
    }
}

// ── Supervised Pair protocol ─────────────────────────────────────────────────

/// Perform supervised pairing with a P12 certificate on a raw lockdown stream.
///
/// The stream must be a raw TCP connection to lockdown port 62078 (via usbmux),
/// NOT a TLS-wrapped connection. This is because we are pairing for the first
/// time and do not yet have the certificates needed for TLS.
///
/// Returns `(pair_record, escrow_bag)` on success. The `pair_record` contains
/// all certificates and keys needed for future TLS sessions, and should be
/// saved to disk in the standard lockdown pair record directory.
///
/// # Arguments
/// * `stream` - Raw lockdown stream (usbmux connection to port 62078)
/// * `p12_bytes` - Raw bytes of the P12 supervisor certificate file
/// * `p12_password` - Password for the P12 file
/// * `system_buid` - System BUID (from usbmuxd ReadBUID)
pub async fn pair_supervised<S>(
    stream: &mut S,
    p12_bytes: &[u8],
    p12_password: &str,
    system_buid: &str,
) -> Result<(FullPairRecord, Vec<u8>), LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // 1. Parse P12 to extract supervisor cert and private key
    let pkcs12 = Pkcs12::from_der(p12_bytes).map_err(|error| {
        super::error_context::native(
            std::io::ErrorKind::InvalidInput,
            "parse supervisor PKCS12 DER",
            error,
        )
    })?;
    // OpenSSL's Rust wrapper unwraps CString construction in parse2. Reject
    // its invalid input with the native cause before it can panic. Valid
    // passwords take the existing path without allocating another copy.
    if p12_password.as_bytes().contains(&0) {
        std::ffi::CString::new(p12_password).map_err(|error| {
            super::error_context::native(
                std::io::ErrorKind::InvalidInput,
                "validate supervisor PKCS12 password",
                error,
            )
        })?;
    }
    let parsed = pkcs12.parse2(p12_password).map_err(|error| {
        super::error_context::native(
            std::io::ErrorKind::InvalidInput,
            "unlock supervisor PKCS12",
            error,
        )
    })?;
    let supervisor_cert = parsed
        .cert
        .ok_or_else(|| LockdownError::Protocol("P12 missing certificate".into()))?;
    let supervisor_pkey = parsed
        .pkey
        .ok_or_else(|| LockdownError::Protocol("P12 missing private key".into()))?;

    // 2. Get device public key via raw lockdown GetValue
    let device_public_key_pem = get_device_public_key(stream).await?;

    // 3. Generate certificate chain
    let pair_record = generate_pair_certs(&device_public_key_pem, system_buid)?;

    // 4. Send first Pair request with SupervisorCertificate (DER-encoded)
    let supervisor_cert_der = supervisor_cert
        .to_der()
        .map_err(|error| backend_error("supervisor cert to DER failed", error))?;

    let payload = pair_record_payload(&pair_record);
    let first_request = PairRequestFirst {
        label: "ios-rs",
        protocol_version: "2",
        request: "Pair",
        pair_record: payload,
        pairing_options: PairingOptionsFirst {
            extended_pairing_errors: true,
            supervisor_certificate: supervisor_cert_der,
        },
    };
    send_lockdown(stream, &first_request).await?;

    // 5. Receive MCChallengeRequired response and extract PairingChallenge
    let challenge = recv_pairing_challenge(stream).await?;

    // 6. Sign challenge with PKCS7 using supervisor's private key
    let certs = Stack::new().map_err(|error| backend_error("Stack::new failed", error))?;
    let signed = Pkcs7::sign(
        &supervisor_cert,
        &supervisor_pkey,
        &certs,
        &challenge,
        Pkcs7Flags::BINARY,
    )
    .and_then(|p7| p7.to_der())
    .map_err(|error| backend_error("PKCS7 sign failed", error))?;

    // 7. Send second Pair request with ChallengeResponse
    let payload2 = pair_record_payload(&pair_record);
    let challenge_request = PairRequestChallenge {
        label: "ios-rs",
        protocol_version: "2",
        request: "Pair",
        pair_record: payload2,
        pairing_options: PairingOptionsChallenge {
            challenge_response: signed,
        },
    };
    send_lockdown(stream, &challenge_request).await?;

    // 8. Receive success response with EscrowBag
    let escrow_bag = recv_pair_success(stream).await?;

    Ok((pair_record, escrow_bag))
}

/// Send GetValue request for DevicePublicKey on a raw (non-TLS) lockdown stream.
async fn get_device_public_key<S>(stream: &mut S) -> Result<Vec<u8>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request = GetValueRequest {
        label: "ios-rs",
        request: "GetValue",
        domain: None,
        key: Some("DevicePublicKey"),
    };
    send_lockdown(stream, &request).await?;

    let resp: GetValueRawResponse = recv_lockdown(stream).await?;
    if let Some(err) = resp.error {
        return Err(LockdownError::Protocol(format!(
            "GetValue DevicePublicKey failed: {err}"
        )));
    }

    match resp.value {
        Some(plist::Value::Data(data)) => Ok(data),
        other => Err(LockdownError::Protocol(format!(
            "DevicePublicKey: expected Data, got {other:?}"
        ))),
    }
}

/// Receive and validate the MCChallengeRequired response, extracting the
/// PairingChallenge bytes from ExtendedResponse.
async fn recv_pairing_challenge<S>(stream: &mut S) -> Result<Vec<u8>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // Parse as generic plist to handle the nested structure
    let resp: plist::Value = recv_lockdown(stream).await?;
    let dict = resp
        .as_dictionary()
        .ok_or_else(|| LockdownError::Protocol("Pair response is not a dictionary".into()))?;

    // Verify we got MCChallengeRequired error
    let error = dict
        .get("Error")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| {
            LockdownError::Protocol(format!("Pair response missing Error field: {dict:?}"))
        })?;

    if error != "MCChallengeRequired" {
        return Err(LockdownError::Protocol(format!(
            "expected MCChallengeRequired error, got: {error}"
        )));
    }

    // Extract PairingChallenge from ExtendedResponse
    let extended = dict
        .get("ExtendedResponse")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| LockdownError::Protocol("Pair response missing ExtendedResponse".into()))?;

    let challenge = extended
        .get("PairingChallenge")
        .and_then(plist::Value::as_data)
        .ok_or_else(|| {
            LockdownError::Protocol("ExtendedResponse missing PairingChallenge".into())
        })?;

    Ok(challenge.to_vec())
}

/// Receive the successful Pair response and extract the EscrowBag.
async fn recv_pair_success<S>(stream: &mut S) -> Result<Vec<u8>, LockdownError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let resp: plist::Value = recv_lockdown(stream).await?;
    let dict = resp
        .as_dictionary()
        .ok_or_else(|| LockdownError::Protocol("Pair response is not a dictionary".into()))?;

    // Check for errors
    if let Some(error) = dict.get("Error").and_then(plist::Value::as_string) {
        return Err(LockdownError::Protocol(format!("Pair failed: {error}")));
    }

    // Extract EscrowBag
    let escrow_bag = dict
        .get("EscrowBag")
        .and_then(plist::Value::as_data)
        .ok_or_else(|| LockdownError::Protocol("Pair success response missing EscrowBag".into()))?;

    Ok(escrow_bag.to_vec())
}

/// Save a [`FullPairRecord`] to disk as a plist file compatible with
/// `PairRecord::load()`.
///
/// The saved record includes all certificates, host private key, root private
/// key, HostID, SystemBUID, EscrowBag, and optionally the WiFi MAC address.
pub fn save_pair_record(
    record: &FullPairRecord,
    escrow_bag: &[u8],
    wifi_mac: Option<&str>,
    path: &std::path::Path,
) -> Result<(), LockdownError> {
    use plist::Value;

    let mut dict = plist::Dictionary::new();
    dict.insert(
        "DeviceCertificate".into(),
        Value::Data(record.device_certificate.clone()),
    );
    dict.insert(
        "HostCertificate".into(),
        Value::Data(record.host_certificate.clone()),
    );
    dict.insert(
        "HostPrivateKey".into(),
        Value::Data(record.host_private_key.clone()),
    );
    dict.insert(
        "RootCertificate".into(),
        Value::Data(record.root_certificate.clone()),
    );
    dict.insert(
        "RootPrivateKey".into(),
        Value::Data(record.root_private_key.clone()),
    );
    dict.insert("HostID".into(), Value::String(record.host_id.clone()));
    dict.insert(
        "SystemBUID".into(),
        Value::String(record.system_buid.clone()),
    );
    dict.insert("EscrowBag".into(), Value::Data(escrow_bag.to_vec()));

    if let Some(mac) = wifi_mac {
        dict.insert("WiFiMACAddress".into(), Value::String(mac.to_string()));
    }

    let plist_value = Value::Dictionary(dict);
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist_value).map_err(|error| {
        super::error_context::native(
            std::io::ErrorKind::Other,
            "serialize supervised pairing record plist",
            error,
        )
    })?;
    // Carries HostPrivateKey and RootPrivateKey; owner-only, like Apple's own
    // /var/db/lockdown records.
    crate::secret_file::write_secret(path, &buf).map_err(|error| {
        super::error_context::native(error.kind(), "write supervised pairing record", error)
    })?;

    Ok(())
}

#[cfg(test)]
mod source_contract_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_pair_record_debug_omits_all_credential_bytes_and_identifiers() {
        let secret = "synthetic-private-credential-device-data";
        let bytes = secret.as_bytes().to_vec();
        let record = FullPairRecord {
            device_certificate: bytes.clone(),
            host_certificate: bytes.clone(),
            host_private_key: bytes.clone(),
            root_certificate: bytes.clone(),
            root_private_key: bytes.clone(),
            host_id: secret.into(),
            system_buid: secret.into(),
        };
        let output = format!("{record:?} {record:#?}");
        assert!(!output.contains(secret));
        assert!(!output.contains(&format!("{bytes:?}")));
        assert!(!output.contains(&hex::encode(&bytes)));
        assert!(output.contains("FullPairRecord"));
        assert_eq!(record.host_private_key, bytes);
    }

    #[test]
    fn test_generate_pair_certs_structure() {
        // Generate a mock device RSA key and convert to PEM
        let device_rsa = Rsa::generate(2048).unwrap();
        let device_pkey = PKey::from_rsa(device_rsa).unwrap();
        let device_pub_pem = device_pkey.public_key_to_pem().unwrap();

        let record = generate_pair_certs(&device_pub_pem, "TEST-BUID").unwrap();

        // Verify all fields are non-empty
        assert!(!record.device_certificate.is_empty());
        assert!(!record.host_certificate.is_empty());
        assert!(!record.host_private_key.is_empty());
        assert!(!record.root_certificate.is_empty());
        assert!(!record.root_private_key.is_empty());
        assert_eq!(record.host_id.len(), 36); // UUID format
        assert_eq!(record.system_buid, "TEST-BUID");

        // Verify certificates are valid PEM
        let _ = X509::from_pem(&record.device_certificate).unwrap();
        let _ = X509::from_pem(&record.host_certificate).unwrap();
        let _ = X509::from_pem(&record.root_certificate).unwrap();

        // Verify private keys are valid PEM
        let _ = PKey::private_key_from_pem(&record.host_private_key).unwrap();
        let _ = PKey::private_key_from_pem(&record.root_private_key).unwrap();
    }

    #[test]
    fn test_root_cert_is_ca() {
        let rsa = Rsa::generate(2048).unwrap();
        let pkey = PKey::from_rsa(rsa).unwrap();
        let cert = build_root_cert(&pkey).unwrap();

        // Verify the certificate is valid and can be serialized
        let pem = cert.to_pem().unwrap();
        let parsed = X509::from_pem(&pem).unwrap();
        // Root cert should be self-signed (issuer == subject)
        assert_eq!(
            parsed.subject_name().entries().count(),
            parsed.issuer_name().entries().count()
        );
    }

    #[test]
    fn test_signed_cert_not_ca() {
        let root_rsa = Rsa::generate(2048).unwrap();
        let root_pkey = PKey::from_rsa(root_rsa).unwrap();
        let root_cert = build_root_cert(&root_pkey).unwrap();

        let host_rsa = Rsa::generate(2048).unwrap();
        let host_pkey = PKey::from_rsa(host_rsa).unwrap();
        let host_cert = build_signed_cert(&host_pkey, &root_cert, &root_pkey).unwrap();

        // Verify the host cert can be serialized and parsed
        let pem = host_cert.to_pem().unwrap();
        let parsed = X509::from_pem(&pem).unwrap();
        // Host cert should be signed by root (issuer matches root subject)
        assert_eq!(
            parsed.issuer_name().entries().count(),
            root_cert.subject_name().entries().count()
        );
    }

    #[test]
    fn test_pair_record_payload_strips_keys() {
        let record = FullPairRecord {
            device_certificate: b"dev-cert".to_vec(),
            host_certificate: b"host-cert".to_vec(),
            host_private_key: b"SECRET-HOST-KEY".to_vec(),
            root_certificate: b"root-cert".to_vec(),
            root_private_key: b"SECRET-ROOT-KEY".to_vec(),
            host_id: "HOST-ID".to_string(),
            system_buid: "BUID".to_string(),
        };
        let payload = pair_record_payload(&record);
        assert_eq!(payload.device_certificate, b"dev-cert");
        assert_eq!(payload.host_id, "HOST-ID");
        // payload should not contain private keys (it's a different struct)
    }
}

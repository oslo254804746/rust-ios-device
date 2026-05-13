//! Helpers for supervised device preparation workflows.
//!
//! Reference: go-ios `mcinstall/prepare.go` and `crypto_utils.go`

use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkcs12::Pkcs12;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use openssl::x509::extension::{BasicConstraints, ExtendedKeyUsage, KeyUsage};
use openssl::x509::{X509NameBuilder, X509};
use uuid::Uuid;

pub const DEFAULT_ORGANIZATION_NAME: &str = "ios-rs";
pub const DEFAULT_LANGUAGE: &str = "en";
pub const DEFAULT_LOCALE: &str = "en_US";
pub const DEFAULT_SKIP_SETUP_KEYS: &[&str] = &[
    "Accessibility",
    "AccessibilityAppearance",
    "ActionButton",
    "AgeAssurance",
    "AgeBasedSafetySettings",
    "Android",
    "Appearance",
    "AppleID",
    "AppStore",
    "Avatar",
    "Biometric",
    "CameraButton",
    "CloudStorage",
    "DeviceProtection",
    "DeviceToDeviceMigration",
    "Diagnostics",
    "Display",
    "EnableLockdownMode",
    "ExpressLanguage",
    "FileVault",
    "iCloudDiagnostics",
    "iCloudStorage",
    "iMessageAndFaceTime",
    "IntendedUser",
    "Intelligence",
    "Keyboard",
    "Language",
    "LanguageAndLocale",
    "Location",
    "LockdownMode",
    "MessagingActivationUsingPhoneNumber",
    "Multitasking",
    "OSShowCase",
    "Passcode",
    "Payment",
    "PreferredLanguage",
    "Privacy",
    "Region",
    "Registration",
    "Restore",
    "RestoreCompleted",
    "Safety",
    "SafetyAndHandling",
    "ScreenSaver",
    "ScreenTime",
    "SIMSetup",
    "Siri",
    "SoftwareUpdate",
    "SpokenLanguage",
    "TapToSetup",
    "TermsOfAddress",
    "Tips",
    "Tone",
    "TOS",
    "TouchID",
    "TrueToneDisplay",
    "TVHomeScreenSync",
    "TVProviderSignIn",
    "TVRoom",
    "UnlockWithWatch",
    "UpdateCompleted",
    "Wallpaper",
    "WatchMigration",
    "WebContentFiltering",
    "Welcome",
    "WiFi",
    "DisplayTone",
    "HomeButtonSensitivity",
    "OnBoarding",
    "Zoom",
];

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("plist error: {0}")]
    Plist(#[from] plist::Error),
}

#[derive(Debug, Clone)]
pub struct SupervisionIdentity {
    pub certificate_der: Vec<u8>,
    pub certificate_pem: Vec<u8>,
    pub private_key_pem: Vec<u8>,
    pub pkcs12_der: Vec<u8>,
}

pub fn generate_supervision_identity(
    common_name: &str,
    password: &str,
) -> Result<SupervisionIdentity, PrepareError> {
    let rsa = Rsa::generate(2048).map_err(crypto_err)?;
    let pkey = PKey::from_rsa(rsa).map_err(crypto_err)?;

    let mut name_builder = X509NameBuilder::new().map_err(crypto_err)?;
    let subject = if common_name.trim().is_empty() {
        DEFAULT_ORGANIZATION_NAME
    } else {
        common_name
    };
    name_builder
        .append_entry_by_nid(Nid::COMMONNAME, subject)
        .map_err(crypto_err)?;
    let name = name_builder.build();

    let mut serial = BigNum::new().map_err(crypto_err)?;
    serial
        .rand(159, MsbOption::MAYBE_ZERO, false)
        .map_err(crypto_err)?;
    let serial = serial.to_asn1_integer().map_err(crypto_err)?;

    let mut builder = X509::builder().map_err(crypto_err)?;
    builder.set_version(2).map_err(crypto_err)?;
    builder.set_serial_number(&serial).map_err(crypto_err)?;
    builder.set_subject_name(&name).map_err(crypto_err)?;
    builder.set_issuer_name(&name).map_err(crypto_err)?;
    builder.set_pubkey(&pkey).map_err(crypto_err)?;

    let not_before = Asn1Time::days_from_now(0).map_err(crypto_err)?;
    let not_after = Asn1Time::days_from_now(3650).map_err(crypto_err)?;
    builder.set_not_before(&not_before).map_err(crypto_err)?;
    builder.set_not_after(&not_after).map_err(crypto_err)?;
    builder
        .append_extension(
            BasicConstraints::new()
                .critical()
                .ca()
                .build()
                .map_err(crypto_err)?,
        )
        .map_err(crypto_err)?;
    builder
        .append_extension(
            KeyUsage::new()
                .digital_signature()
                .key_cert_sign()
                .build()
                .map_err(crypto_err)?,
        )
        .map_err(crypto_err)?;
    builder
        .append_extension(
            ExtendedKeyUsage::new()
                .server_auth()
                .client_auth()
                .build()
                .map_err(crypto_err)?,
        )
        .map_err(crypto_err)?;
    builder
        .sign(&pkey, MessageDigest::sha512())
        .map_err(crypto_err)?;

    let cert = builder.build();
    #[allow(deprecated)]
    let pkcs12 = Pkcs12::builder()
        .build(password, subject, &pkey, &cert)
        .map_err(crypto_err)?;

    Ok(SupervisionIdentity {
        certificate_der: cert.to_der().map_err(crypto_err)?,
        certificate_pem: cert.to_pem().map_err(crypto_err)?,
        private_key_pem: pkey.private_key_to_pem_pkcs8().map_err(crypto_err)?,
        pkcs12_der: pkcs12.to_der().map_err(crypto_err)?,
    })
}

pub fn build_cloud_configuration(
    skip_setup: &[String],
    supervision_certificate_der: Option<&[u8]>,
    organization_name: Option<&str>,
) -> plist::Dictionary {
    let mut cloud = plist::Dictionary::from_iter([
        ("AllowPairing".to_string(), plist::Value::Boolean(true)),
        (
            "SkipSetup".to_string(),
            plist::Value::Array(
                skip_setup
                    .iter()
                    .cloned()
                    .map(plist::Value::String)
                    .collect(),
            ),
        ),
    ]);

    if let Some(cert) = supervision_certificate_der {
        cloud.insert(
            "OrganizationName".to_string(),
            plist::Value::String(
                organization_name
                    .unwrap_or(DEFAULT_ORGANIZATION_NAME)
                    .to_string(),
            ),
        );
        cloud.insert(
            "OrganizationMagic".to_string(),
            plist::Value::String(Uuid::new_v4().to_string()),
        );
        cloud.insert(
            "SupervisorHostCertificates".to_string(),
            plist::Value::Array(vec![plist::Value::Data(cert.to_vec())]),
        );
        cloud.insert("IsSupervised".to_string(), plist::Value::Boolean(true));
        cloud.insert("IsMultiUser".to_string(), plist::Value::Boolean(false));
    }

    cloud
}

pub fn build_initial_profile() -> Result<Vec<u8>, PrepareError> {
    let payload_uuid = Uuid::new_v4().to_string();
    let content_uuid = Uuid::new_v4().to_string();
    let payload = plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "PayloadContent".to_string(),
            plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    (
                        "PayloadDescription".to_string(),
                        plist::Value::String("Configures Restrictions".into()),
                    ),
                    (
                        "PayloadDisplayName".to_string(),
                        plist::Value::String("Restrictions".into()),
                    ),
                    (
                        "PayloadIdentifier".to_string(),
                        plist::Value::String(format!("com.apple.applicationaccess.{content_uuid}")),
                    ),
                    (
                        "PayloadType".to_string(),
                        plist::Value::String("com.apple.applicationaccess".into()),
                    ),
                    (
                        "PayloadUUID".to_string(),
                        plist::Value::String(content_uuid),
                    ),
                    (
                        "PayloadVersion".to_string(),
                        plist::Value::Integer(1.into()),
                    ),
                    (
                        "allowAppInstallation".to_string(),
                        plist::Value::Boolean(true),
                    ),
                    ("allowAppRemoval".to_string(), plist::Value::Boolean(true)),
                    ("allowCamera".to_string(), plist::Value::Boolean(true)),
                    ("allowCloudBackup".to_string(), plist::Value::Boolean(true)),
                    (
                        "allowDiagnosticSubmission".to_string(),
                        plist::Value::Boolean(true),
                    ),
                ]),
            )]),
        ),
        (
            "PayloadDisplayName".to_string(),
            plist::Value::String("Device Preparation".into()),
        ),
        (
            "PayloadIdentifier".to_string(),
            plist::Value::String(format!("com.apple.prepare.{payload_uuid}")),
        ),
        (
            "PayloadRemovalDisallowed".to_string(),
            plist::Value::Boolean(false),
        ),
        (
            "PayloadType".to_string(),
            plist::Value::String("Configuration".into()),
        ),
        (
            "PayloadUUID".to_string(),
            plist::Value::String(payload_uuid),
        ),
        (
            "PayloadVersion".to_string(),
            plist::Value::Integer(1.into()),
        ),
    ]));

    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &payload)?;
    Ok(buf)
}

fn crypto_err(err: openssl::error::ErrorStack) -> PrepareError {
    PrepareError::Crypto(err.to_string())
}

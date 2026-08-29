//! MCInstall client for configuration and supervised device-management tasks.
//!
//! Service: `com.apple.mobile.MCInstall`

#[cfg(feature = "supervised-pair")]
use openssl::pkcs12::Pkcs12;
#[cfg(feature = "supervised-pair")]
use openssl::pkcs7::{Pkcs7, Pkcs7Flags};
#[cfg(feature = "supervised-pair")]
use openssl::stack::Stack;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::{Zeroize, Zeroizing};

pub const SERVICE_NAME: &str = "com.apple.mobile.MCInstall";

service_error!(
    McInstallError,
    #[error("crypto error: {0}")]
    Crypto(String),
    #[error("device has a passcode set; remove it before fetching an unlock token")]
    PasscodeSet,
);

/// MCInstall reports this keybag error when an unlock token cannot be minted
/// because the device already has a lock passcode.
pub const KEYBAG_ERROR_DOMAIN: &str = "DMCKeybagErrorDomain";
pub const KEYBAG_PASSCODE_SET_ERROR_CODE: i64 = 37_002;

/// A passcode-unlock token held in zeroizing memory.
///
/// The token is deliberately not printable and does not expose the inner
/// `Vec<u8>` through `Debug` or `Display`. Pass it to
/// [`McInstallClient::clear_passcode`] with [`Self::as_bytes`].
#[derive(Eq, PartialEq)]
pub struct UnlockToken(Zeroizing<Vec<u8>>);

impl UnlockToken {
    fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// Wrap raw bytes read from a protected token file.
    ///
    /// This constructor does not validate the token with the device; the
    /// MCInstall service performs that validation during `ClearPasscode`.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }

    /// Borrow the raw token bytes for a device request or a protected file write.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    /// Return the token length without exposing its contents.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the device returned an empty token.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl AsRef<[u8]> for UnlockToken {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl std::fmt::Debug for UnlockToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UnlockToken")
            .field("bytes", &"<redacted>")
            .field("len", &self.len())
            .finish()
    }
}

impl std::fmt::Display for UnlockToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "<redacted unlock token: {} bytes>", self.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProfileInfo {
    pub identifier: String,
    pub display_name: String,
    pub description: Option<String>,
    pub is_active: bool,
    pub removal_disallowed: Option<bool>,
    pub status: Option<String>,
    pub uuid: Option<String>,
    pub version: Option<u64>,
}

#[derive(Debug)]
pub struct McInstallClient<S> {
    stream: S,
}

impl<S: AsyncRead + AsyncWrite + Unpin> McInstallClient<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub async fn list_profiles(&mut self) -> Result<Vec<ProfileInfo>, McInstallError> {
        let response = self.get_profile_list_raw().await?;
        parse_profile_list(response)
    }

    pub async fn get_profile_list_raw(&mut self) -> Result<plist::Value, McInstallError> {
        self.send_plist(&Request {
            request_type: "GetProfileList",
        })
        .await?;

        self.recv_plist().await
    }

    pub async fn get_cloud_configuration(&mut self) -> Result<plist::Dictionary, McInstallError> {
        self.send_plist(&Request {
            request_type: "GetCloudConfiguration",
        })
        .await?;

        let response: plist::Value = self.recv_plist().await?;
        parse_cloud_configuration(response)
    }

    pub async fn get_stored_profile_raw(
        &mut self,
        purpose: &str,
    ) -> Result<plist::Value, McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("GetStoredProfile".into()),
            ),
            (
                "Purpose".to_string(),
                plist::Value::String(purpose.to_string()),
            ),
        ]);
        send_plist(&mut self.stream, &plist::Value::Dictionary(request)).await?;
        self.recv_plist().await
    }

    pub async fn flush(&mut self) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([(
            "RequestType".to_string(),
            plist::Value::String("Flush".into()),
        )]);
        send_request(&mut self.stream, request).await
    }

    pub async fn hello_host_identifier(&mut self) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([(
            "RequestType".to_string(),
            plist::Value::String("HelloHostIdentifier".into()),
        )]);
        send_request(&mut self.stream, request).await
    }

    pub async fn set_cloud_configuration(
        &mut self,
        cloud_configuration: plist::Dictionary,
    ) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("SetCloudConfiguration".into()),
            ),
            (
                "CloudConfiguration".to_string(),
                plist::Value::Dictionary(cloud_configuration),
            ),
        ]);
        send_request(&mut self.stream, request).await
    }

    pub async fn install_profile(&mut self, payload: &[u8]) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("InstallProfile".into()),
            ),
            ("Payload".to_string(), plist::Value::Data(payload.to_vec())),
        ]);
        send_request(&mut self.stream, request).await
    }

    pub async fn install_profile_silent(
        &mut self,
        payload: &[u8],
        p12_bytes: &[u8],
        password: &str,
    ) -> Result<(), McInstallError> {
        #[cfg(not(feature = "supervised-pair"))]
        {
            let _ = (payload, p12_bytes, password);
            Err(McInstallError::Crypto(
                "silent profile installation requires ios-core feature 'supervised-pair'".into(),
            ))
        }

        #[cfg(feature = "supervised-pair")]
        {
            self.escalate_with_p12(p12_bytes, password).await?;
            let request = plist::Dictionary::from_iter([
                (
                    "RequestType".to_string(),
                    plist::Value::String("InstallProfileSilent".into()),
                ),
                ("Payload".to_string(), plist::Value::Data(payload.to_vec())),
            ]);
            send_request(&mut self.stream, request).await
        }
    }

    pub async fn remove_profile(&mut self, identifier: &str) -> Result<(), McInstallError> {
        let profile_identifier = match self.get_profile_list_raw().await {
            Ok(value) => build_remove_profile_identifier(&value, identifier)
                .map_err(|err| McInstallError::Protocol(err.to_string()))?
                .unwrap_or_else(|| plist::Value::String(identifier.to_string())),
            Err(_) => plist::Value::String(identifier.to_string()),
        };
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("RemoveProfile".into()),
            ),
            ("ProfileIdentifier".to_string(), profile_identifier),
        ]);
        send_request(&mut self.stream, request).await
    }

    /// Escalate an MCInstall session using a supervisor PKCS#12 identity.
    ///
    /// This is the same challenge/signature path used by silent profile
    /// installation. Device-management operations such as security queries
    /// and passcode clearing must run on this escalated session as well; they
    /// must not duplicate or partially reimplement the P12 protocol.
    pub async fn escalate_with_p12(
        &mut self,
        p12_bytes: &[u8],
        password: &str,
    ) -> Result<(), McInstallError> {
        #[cfg(not(feature = "supervised-pair"))]
        {
            let _ = (p12_bytes, password);
            Err(McInstallError::Crypto(
                "MCInstall P12 escalation requires ios-core feature 'supervised-pair'".into(),
            ))
        }

        #[cfg(feature = "supervised-pair")]
        {
            self.escalate(p12_bytes, password).await
        }
    }

    pub async fn erase_device(
        &mut self,
        preserve_data_plan: bool,
        disallow_proximity_setup: bool,
    ) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("EraseDevice".into()),
            ),
            (
                "PreserveDataPlan".to_string(),
                plist::Value::Boolean(preserve_data_plan),
            ),
            (
                "DisallowProximitySetup".to_string(),
                plist::Value::Boolean(disallow_proximity_setup),
            ),
        ]);
        send_request_allow_eof(&mut self.stream, request).await
    }

    pub async fn escalate_unsupervised(&mut self) -> Result<(), McInstallError> {
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("Escalate".into()),
            ),
            (
                "SupervisorCertificate".to_string(),
                plist::Value::Data(vec![0]),
            ),
        ]);
        send_request(&mut self.stream, request).await
    }

    #[cfg(feature = "supervised-pair")]
    async fn escalate(&mut self, p12_bytes: &[u8], password: &str) -> Result<(), McInstallError> {
        let pkcs12 =
            Pkcs12::from_der(p12_bytes).map_err(|err| McInstallError::Crypto(err.to_string()))?;
        let parsed = pkcs12
            .parse2(password)
            .map_err(|err| McInstallError::Crypto(err.to_string()))?;
        let cert = parsed
            .cert
            .ok_or_else(|| McInstallError::Crypto("P12 missing certificate".into()))?;
        let pkey = parsed
            .pkey
            .ok_or_else(|| McInstallError::Crypto("P12 missing private key".into()))?;

        let mut request = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("Escalate".into()),
            ),
            (
                "SupervisorCertificate".to_string(),
                plist::Value::Data(
                    cert.to_der()
                        .map_err(|err| McInstallError::Crypto(err.to_string()))?,
                ),
            ),
        ]));
        let send_result = send_plist(&mut self.stream, &request).await;
        // The certificate is public, but keeping the request envelope
        // zeroizing is cheap and prevents accidental retention if a future
        // escalation field carries private material.
        zeroize_sensitive_value(&mut request, "request");
        send_result?;

        let mut response = recv_plist(&mut self.stream).await?;
        if let Err(error) = ensure_acknowledged(&response) {
            zeroize_sensitive_dictionary(&mut response);
            return Err(error);
        }
        let challenge = match response.remove("Challenge") {
            Some(plist::Value::Data(bytes)) => Zeroizing::new(bytes),
            Some(mut value) => {
                zeroize_plist_value(&mut value);
                zeroize_sensitive_dictionary(&mut response);
                return Err(McInstallError::Protocol(
                    "MCInstall escalate response Challenge was not data".into(),
                ));
            }
            None => {
                zeroize_sensitive_dictionary(&mut response);
                return Err(McInstallError::Protocol(
                    "MCInstall escalate response missing Challenge".into(),
                ));
            }
        };
        zeroize_sensitive_dictionary(&mut response);
        let certs = Stack::new().map_err(|err| McInstallError::Crypto(err.to_string()))?;
        let signed_request = Zeroizing::new(
            Pkcs7::sign(&cert, &pkey, &certs, &challenge, Pkcs7Flags::BINARY)
                .and_then(|pkcs7| pkcs7.to_der())
                .map_err(|err| McInstallError::Crypto(err.to_string()))?,
        );

        let response_request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("EscalateResponse".into()),
            ),
            (
                "SignedRequest".to_string(),
                plist::Value::Data(signed_request.to_vec()),
            ),
        ]);
        send_request(&mut self.stream, response_request).await?;

        let proceed_request = plist::Dictionary::from_iter([(
            "RequestType".to_string(),
            plist::Value::String("ProceedWithKeybagMigration".into()),
        )]);
        send_request(&mut self.stream, proceed_request).await
    }

    /// Fetch a device passcode-unlock token from an escalated MCInstall
    /// session.
    ///
    /// The device must not currently have a lock passcode. The returned token
    /// is held in zeroizing memory and is intentionally redacted from
    /// `Debug`/`Display`; callers should persist it only with a protected file
    /// writer and pass [`UnlockToken::as_bytes`] when clearing a passcode.
    pub async fn fetch_unlock_token(&mut self) -> Result<UnlockToken, McInstallError> {
        let request = request_dictionary("RequestUnlockToken");
        let mut response = self.send_plist_request(request).await?;
        if let Err(error) = ensure_acknowledged_named(&response, "RequestUnlockToken") {
            if has_error_code(
                &response,
                KEYBAG_ERROR_DOMAIN,
                KEYBAG_PASSCODE_SET_ERROR_CODE,
            ) {
                zeroize_sensitive_dictionary(&mut response);
                return Err(McInstallError::PasscodeSet);
            }
            zeroize_sensitive_dictionary(&mut response);
            return Err(error);
        }

        let token = response
            .remove("UnlockToken")
            .and_then(|value| match value {
                plist::Value::Data(bytes) => Some(UnlockToken::new(bytes)),
                mut value => {
                    // A malformed daemon response may still contain a secret
                    // under the expected key. Clear it before dropping the
                    // value instead of relying on plist's ordinary drop.
                    zeroize_sensitive_value(&mut value, "UnlockToken");
                    None
                }
            })
            .ok_or_else(|| {
                McInstallError::Protocol("MCInstall response missing UnlockToken data".into())
            });
        zeroize_sensitive_dictionary(&mut response);
        token
    }

    /// Return the complete security-information dictionary from an escalated
    /// MCInstall session.
    ///
    /// The dictionary contains passcode presence/compliance, grace-period,
    /// encryption and management fields. Errors contain only status and
    /// redacted ErrorChain details, never the whole response dictionary.
    pub async fn security_info(&mut self) -> Result<plist::Dictionary, McInstallError> {
        let mut response = self
            .send_checked(request_dictionary("SecurityInfo"), "SecurityInfo")
            .await?;
        let info = match response.remove("SecurityInfo") {
            Some(plist::Value::Dictionary(info)) => info,
            Some(mut value) => {
                zeroize_plist_value(&mut value);
                zeroize_sensitive_dictionary(&mut response);
                return Err(McInstallError::Protocol(
                    "MCInstall response missing SecurityInfo dictionary".into(),
                ));
            }
            None => {
                zeroize_sensitive_dictionary(&mut response);
                return Err(McInstallError::Protocol(
                    "MCInstall response missing SecurityInfo dictionary".into(),
                ));
            }
        };
        // Move the dictionary out of the response instead of cloning it. This
        // preserves the upstream complete-dictionary API without leaving a
        // second copy of a daemon-provided sensitive value in the envelope.
        zeroize_sensitive_dictionary(&mut response);
        Ok(info)
    }

    /// Return whether a lock passcode is currently configured on the device.
    ///
    /// This is intentionally based on MCInstall `SecurityInfo` rather than
    /// lockdown's `PasswordProtected`, which reports current lock state and
    /// is false when a passcode device is presently unlocked.
    pub async fn passcode_present(&mut self) -> Result<bool, McInstallError> {
        let mut info = self.security_info().await?;
        let present = info
            .get("PasscodePresent")
            .and_then(plist::Value::as_boolean)
            .ok_or_else(|| {
                McInstallError::Protocol(
                    "SecurityInfo response missing boolean PasscodePresent".into(),
                )
            });
        zeroize_sensitive_dictionary(&mut info);
        present
    }

    /// Clear the device lock passcode using a previously fetched unlock token.
    ///
    /// The token is accepted as any `AsRef<[u8]>` value so callers can pass a
    /// protected [`UnlockToken`] or a zeroizing file buffer without making a
    /// second public copy of the secret.
    pub async fn clear_passcode<T: AsRef<[u8]>>(
        &mut self,
        unlock_token: T,
    ) -> Result<(), McInstallError> {
        let token = unlock_token.as_ref();
        let request = plist::Dictionary::from_iter([
            (
                "RequestType".to_string(),
                plist::Value::String("ClearPasscode".into()),
            ),
            (
                "UnlockToken".to_string(),
                plist::Value::Data(token.to_vec()),
            ),
        ]);
        let mut response = self.send_checked(request, "ClearPasscode").await?;
        zeroize_sensitive_dictionary(&mut response);
        Ok(())
    }

    /// Clear the Screen Time restrictions passcode. This requires only an
    /// escalated supervisor session and does not consume an unlock token.
    pub async fn clear_screen_time_password(&mut self) -> Result<(), McInstallError> {
        let mut response = self
            .send_checked(
                request_dictionary("ClearRestrictionsPassword"),
                "ClearRestrictionsPassword",
            )
            .await?;
        zeroize_sensitive_dictionary(&mut response);
        Ok(())
    }

    async fn send_plist_request(
        &mut self,
        request: plist::Dictionary,
    ) -> Result<plist::Dictionary, McInstallError> {
        let mut value = plist::Value::Dictionary(request);
        let send_result = send_plist(&mut self.stream, &value).await;
        zeroize_sensitive_value(&mut value, "request");
        send_result?;
        recv_plist(&mut self.stream).await
    }

    async fn send_checked(
        &mut self,
        request: plist::Dictionary,
        operation: &str,
    ) -> Result<plist::Dictionary, McInstallError> {
        let mut response = self.send_plist_request(request).await?;
        if let Err(error) = ensure_acknowledged_named(&response, operation) {
            zeroize_sensitive_dictionary(&mut response);
            return Err(error);
        }
        Ok(response)
    }

    async fn send_plist<T: Serialize>(&mut self, value: &T) -> Result<(), McInstallError> {
        let mut encoded = Vec::new();
        plist::to_writer_xml(&mut encoded, value)?;
        let buf = Zeroizing::new(encoded);
        self.stream
            .write_all(&(buf.len() as u32).to_be_bytes())
            .await?;
        self.stream.write_all(&buf).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv_plist<T>(&mut self) -> Result<T, McInstallError>
    where
        T: for<'de> serde::Deserialize<'de>,
    {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
        if len > MAX_PLIST_SIZE {
            return Err(McInstallError::Protocol(format!(
                "plist length {len} exceeds max {MAX_PLIST_SIZE}"
            )));
        }
        let mut encoded = vec![0u8; len];
        self.stream.read_exact(&mut encoded).await?;
        let buf = Zeroizing::new(encoded);
        Ok(plist::from_bytes(&buf)?)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Request {
    request_type: &'static str,
}

fn request_dictionary(request_type: &'static str) -> plist::Dictionary {
    plist::Dictionary::from_iter([(
        "RequestType".to_string(),
        plist::Value::String(request_type.to_string()),
    )])
}

fn parse_profile_list(value: plist::Value) -> Result<Vec<ProfileInfo>, McInstallError> {
    let dict = value.into_dictionary().ok_or_else(|| {
        McInstallError::Protocol("MCInstall response was not a dictionary".into())
    })?;

    let ordered = dict
        .get("OrderedIdentifiers")
        .and_then(plist::Value::as_array)
        .ok_or_else(|| {
            McInstallError::Protocol("MCInstall response missing OrderedIdentifiers".into())
        })?;
    let manifest_root = dict
        .get("ProfileManifest")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| {
            McInstallError::Protocol("MCInstall response missing ProfileManifest".into())
        })?;
    let metadata_root = dict
        .get("ProfileMetadata")
        .and_then(plist::Value::as_dictionary)
        .ok_or_else(|| {
            McInstallError::Protocol("MCInstall response missing ProfileMetadata".into())
        })?;
    let status = dict
        .get("Status")
        .and_then(plist::Value::as_string)
        .map(ToOwned::to_owned);

    let mut profiles = Vec::with_capacity(ordered.len());
    for identifier in ordered {
        let identifier = identifier.as_string().ok_or_else(|| {
            McInstallError::Protocol("OrderedIdentifiers entry was not a string".into())
        })?;
        let manifest = manifest_root
            .get(identifier)
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| {
                McInstallError::Protocol(format!("ProfileManifest missing entry for {identifier}"))
            })?;
        let metadata = metadata_root
            .get(identifier)
            .and_then(plist::Value::as_dictionary)
            .ok_or_else(|| {
                McInstallError::Protocol(format!("ProfileMetadata missing entry for {identifier}"))
            })?;

        profiles.push(ProfileInfo {
            identifier: identifier.to_string(),
            display_name: metadata
                .get("PayloadDisplayName")
                .and_then(plist::Value::as_string)
                .unwrap_or(identifier)
                .to_string(),
            description: metadata
                .get("PayloadDescription")
                .and_then(plist::Value::as_string)
                .map(ToOwned::to_owned),
            is_active: manifest
                .get("IsActive")
                .and_then(plist::Value::as_boolean)
                .unwrap_or(false),
            removal_disallowed: metadata
                .get("PayloadRemovalDisallowed")
                .and_then(plist::Value::as_boolean),
            status: status.clone(),
            uuid: metadata
                .get("PayloadUUID")
                .and_then(plist::Value::as_string)
                .map(ToOwned::to_owned),
            version: metadata
                .get("PayloadVersion")
                .and_then(plist::Value::as_unsigned_integer),
        });
    }
    Ok(profiles)
}

fn parse_cloud_configuration(value: plist::Value) -> Result<plist::Dictionary, McInstallError> {
    value.into_dictionary().ok_or_else(|| {
        McInstallError::Protocol("MCInstall cloud configuration was not a dictionary".into())
    })
}

fn build_remove_profile_identifier(
    value: &plist::Value,
    identifier: &str,
) -> Result<Option<plist::Value>, plist::Error> {
    let metadata = match value
        .as_dictionary()
        .and_then(|dict| dict.get("ProfileMetadata"))
        .and_then(plist::Value::as_dictionary)
        .and_then(|metadata| metadata.get(identifier))
        .and_then(plist::Value::as_dictionary)
    {
        Some(metadata) => metadata,
        None => return Ok(None),
    };
    let payload_uuid = match metadata
        .get("PayloadUUID")
        .and_then(plist::Value::as_string)
    {
        Some(uuid) => uuid,
        None => return Ok(None),
    };
    let payload_version = match metadata
        .get("PayloadVersion")
        .and_then(plist::Value::as_unsigned_integer)
    {
        Some(version) => version,
        None => return Ok(None),
    };

    let profile_identifier = plist::Value::Dictionary(plist::Dictionary::from_iter([
        (
            "PayloadType".to_string(),
            plist::Value::String("Configuration".into()),
        ),
        (
            "PayloadIdentifier".to_string(),
            plist::Value::String(identifier.to_string()),
        ),
        (
            "PayloadUUID".to_string(),
            plist::Value::String(payload_uuid.to_string()),
        ),
        (
            "PayloadVersion".to_string(),
            plist::Value::Integer((payload_version as i64).into()),
        ),
    ]));
    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &profile_identifier)?;
    Ok(Some(plist::Value::Data(buf)))
}

async fn send_request<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    request: plist::Dictionary,
) -> Result<(), McInstallError> {
    let mut value = plist::Value::Dictionary(request);
    let send_result = send_plist(stream, &value).await;
    zeroize_sensitive_value(&mut value, "request");
    send_result?;
    let mut response = recv_plist(stream).await?;
    let result = ensure_acknowledged(&response);
    zeroize_sensitive_dictionary(&mut response);
    result
}

async fn send_request_allow_eof<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    request: plist::Dictionary,
) -> Result<(), McInstallError> {
    let mut value = plist::Value::Dictionary(request);
    let send_result = send_plist(stream, &value).await;
    zeroize_sensitive_value(&mut value, "request");
    send_result?;
    match recv_plist(stream).await {
        Ok(mut response) => {
            let result = ensure_acknowledged(&response);
            zeroize_sensitive_dictionary(&mut response);
            result
        }
        Err(McInstallError::Io(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => Ok(()),
        Err(err) => Err(err),
    }
}

fn ensure_acknowledged(response: &plist::Dictionary) -> Result<(), McInstallError> {
    ensure_acknowledged_named(response, "MCInstall request")
}

fn ensure_acknowledged_named(
    response: &plist::Dictionary,
    operation: &str,
) -> Result<(), McInstallError> {
    let status = response
        .get("Status")
        .and_then(plist::Value::as_string)
        .ok_or_else(|| McInstallError::Protocol("MCInstall response missing Status".into()))?;
    if status != "Acknowledged" {
        let failure = if is_sensitive_operation(operation) {
            describe_failure_without_descriptions(response)
        } else {
            describe_failure(response)
        };
        return Err(McInstallError::Protocol(format!(
            "{operation} failed: {failure}"
        )));
    }
    Ok(())
}

fn is_sensitive_operation(operation: &str) -> bool {
    matches!(
        operation,
        "RequestUnlockToken" | "ClearPasscode" | "EscalateResponse"
    )
}

/// Describe a failure without copying free-form daemon text for an operation
/// that carries or returns secret material. A daemon may put an echoed token
/// in a localized description without including an `UnlockToken` key, so the
/// response-key heuristic used by [`describe_failure`] is not sufficient for
/// these operations.
fn describe_failure_without_descriptions(response: &plist::Dictionary) -> String {
    let status = response
        .get("Status")
        .and_then(plist::Value::as_string)
        .unwrap_or("unknown");
    let mut parts = vec![format!("Status={status}")];

    if let Some(chain) = response.get("ErrorChain").and_then(plist::Value::as_array) {
        for entry in chain {
            let Some(error) = entry.as_dictionary() else {
                continue;
            };
            let domain = error
                .get("ErrorDomain")
                .and_then(plist::Value::as_string)
                .unwrap_or("unknown");
            let code = error
                .get("ErrorCode")
                .and_then(plist_integer_to_i64)
                .map_or_else(|| "unknown".to_string(), |code| code.to_string());
            parts.push(format!("{domain}({code})"));
        }
    }

    if parts.len() == 1 && status != "unknown" {
        parts.push(status.to_string());
    }
    parts.join("; ")
}

/// Format only the actionable, non-secret part of an MCInstall failure.
///
/// Unlock-token and passcode requests may echo sensitive data in their
/// response dictionaries. Never use `Debug` on the complete response here;
/// retain just status, ErrorChain domains/codes and bounded descriptions.
fn describe_failure(response: &plist::Dictionary) -> String {
    let status = response
        .get("Status")
        .and_then(plist::Value::as_string)
        .unwrap_or("unknown");
    let mut parts = vec![format!("Status={status}")];

    if let Some(chain) = response.get("ErrorChain").and_then(plist::Value::as_array) {
        for entry in chain {
            let Some(error) = entry.as_dictionary() else {
                continue;
            };
            let domain = error
                .get("ErrorDomain")
                .and_then(plist::Value::as_string)
                .unwrap_or("unknown");
            let code = error
                .get("ErrorCode")
                .and_then(plist_integer_to_i64)
                .map_or_else(|| "unknown".to_string(), |code| code.to_string());
            let description = error
                .get("LocalizedDescription")
                .and_then(plist::Value::as_string)
                .map(|description| sanitize_failure_description(response, description));
            match description.as_deref() {
                Some(description) if !description.is_empty() => {
                    parts.push(format!("{domain}({code}): {description}"));
                }
                _ => parts.push(format!("{domain}({code})")),
            }
        }
    }

    if parts.len() == 1 {
        if let Some(error) = response.get("Error").and_then(plist::Value::as_string) {
            parts.push(sanitize_failure_description(response, error));
        } else if status != "unknown" {
            parts.push(status.to_string());
        }
    }

    parts.join("; ")
}

fn sanitize_failure_description(response: &plist::Dictionary, description: &str) -> String {
    // If the daemon echoes a secret-bearing field in a failure response, do
    // not include a free-form description that could repeat that value. The
    // domain and numeric code remain available to the caller above.
    if dictionary_contains_sensitive_key(response) {
        return "<redacted>".into();
    }
    const MAX_DESCRIPTION_BYTES: usize = 512;
    if description.len() <= MAX_DESCRIPTION_BYTES {
        description.to_string()
    } else {
        let mut end = MAX_DESCRIPTION_BYTES;
        while !description.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &description[..end])
    }
}

fn dictionary_contains_sensitive_key(dictionary: &plist::Dictionary) -> bool {
    dictionary
        .iter()
        .any(|(key, value)| is_sensitive_key(key) || value_contains_sensitive_key(value))
}

fn value_contains_sensitive_key(value: &plist::Value) -> bool {
    match value {
        plist::Value::Dictionary(dictionary) => dictionary_contains_sensitive_key(dictionary),
        plist::Value::Array(values) => values.iter().any(value_contains_sensitive_key),
        _ => false,
    }
}

fn has_error_code(response: &plist::Dictionary, domain: &str, code: i64) -> bool {
    response
        .get("ErrorChain")
        .and_then(plist::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(plist::Value::as_dictionary)
        .any(|entry| {
            entry
                .get("ErrorDomain")
                .and_then(plist::Value::as_string)
                .is_some_and(|entry_domain| entry_domain == domain)
                && entry
                    .get("ErrorCode")
                    .and_then(plist_integer_to_i64)
                    .is_some_and(|entry_code| entry_code == code)
        })
}

fn plist_integer_to_i64(value: &plist::Value) -> Option<i64> {
    match value {
        plist::Value::Integer(integer) => integer.as_signed().or_else(|| {
            integer
                .as_unsigned()
                .and_then(|value| value.try_into().ok())
        }),
        // A few plist bridges preserve JSON-like numeric values as `real`.
        // Accept only exact integral values in the i64 range.
        plist::Value::Real(value)
            if value.is_finite()
                && value.fract() == 0.0
                && *value >= i64::MIN as f64
                && *value <= i64::MAX as f64 =>
        {
            Some(*value as i64)
        }
        _ => None,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("unlocktoken")
        || key.contains("token")
        || key.contains("passcode")
        || key.contains("password")
        || key.contains("p12")
        || key.contains("privatekey")
        || key.contains("signedrequest")
        || key.contains("supervisorcertificate")
        || key.contains("secret")
}

fn zeroize_sensitive_dictionary(dictionary: &mut plist::Dictionary) {
    for (key, value) in dictionary {
        zeroize_sensitive_value(value, key);
    }
}

fn zeroize_sensitive_value(value: &mut plist::Value, key: &str) {
    match value {
        plist::Value::Data(bytes) if is_sensitive_key(key) => bytes.zeroize(),
        plist::Value::String(text) if is_sensitive_key(key) => text.zeroize(),
        plist::Value::Array(values) => {
            for value in values {
                zeroize_sensitive_value(value, key);
            }
        }
        plist::Value::Dictionary(dictionary) => {
            zeroize_sensitive_dictionary(dictionary);
        }
        _ => {}
    }
}

fn zeroize_plist_value(value: &mut plist::Value) {
    match value {
        plist::Value::Data(bytes) => bytes.zeroize(),
        plist::Value::String(text) => text.zeroize(),
        plist::Value::Array(values) => {
            for value in values {
                zeroize_plist_value(value);
            }
        }
        plist::Value::Dictionary(dictionary) => {
            for value in dictionary.values_mut() {
                zeroize_plist_value(value);
            }
        }
        _ => {}
    }
}

async fn send_plist<S: AsyncWrite + Unpin>(
    stream: &mut S,
    value: &plist::Value,
) -> Result<(), McInstallError> {
    let mut encoded = Vec::new();
    plist::to_writer_xml(&mut encoded, value)?;
    let buf = Zeroizing::new(encoded);
    stream.write_all(&(buf.len() as u32).to_be_bytes()).await?;
    stream.write_all(&buf).await?;
    stream.flush().await?;
    Ok(())
}

async fn recv_plist<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<plist::Dictionary, McInstallError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
    if len > MAX_PLIST_SIZE {
        return Err(McInstallError::Protocol(format!(
            "plist length {len} exceeds max {MAX_PLIST_SIZE}"
        )));
    }
    let mut encoded = vec![0u8; len];
    stream.read_exact(&mut encoded).await?;
    let buf = Zeroizing::new(encoded);
    Ok(plist::from_bytes(&buf)?)
}

#[cfg(test)]
mod tests {
    use crate::test_util::MockStream;

    use super::*;

    #[test]
    fn parses_ordered_profile_list() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(vec![plist::Value::String("com.example.profile".into())]),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "Description".to_string(),
                            plist::Value::String("Example".into()),
                        ),
                        ("IsActive".to_string(), plist::Value::Boolean(true)),
                    ])),
                )])),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "PayloadDisplayName".to_string(),
                            plist::Value::String("Example Profile".into()),
                        ),
                        (
                            "PayloadDescription".to_string(),
                            plist::Value::String("Example description".into()),
                        ),
                        (
                            "PayloadRemovalDisallowed".to_string(),
                            plist::Value::Boolean(false),
                        ),
                        (
                            "PayloadUUID".to_string(),
                            plist::Value::String("1234".into()),
                        ),
                        (
                            "PayloadVersion".to_string(),
                            plist::Value::Integer(1i64.into()),
                        ),
                    ])),
                )])),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));

        let profiles = parse_profile_list(response).unwrap();
        assert_eq!(profiles.len(), 1);
        let profile = &profiles[0];
        assert_eq!(profile.identifier, "com.example.profile");
        assert_eq!(profile.display_name, "Example Profile");
        assert_eq!(profile.description.as_deref(), Some("Example description"));
        assert!(profile.is_active);
        assert_eq!(profile.removal_disallowed, Some(false));
        assert_eq!(profile.status.as_deref(), Some("Acknowledged"));
        assert_eq!(profile.uuid.as_deref(), Some("1234"));
        assert_eq!(profile.version, Some(1));
    }

    #[test]
    fn cloud_configuration_requires_dictionary_response() {
        let err = parse_cloud_configuration(plist::Value::Array(Vec::new()));
        assert!(matches!(
            err,
            Err(McInstallError::Protocol(message)) if message.contains("cloud configuration")
        ));
    }

    #[test]
    fn parses_cloud_configuration_dictionary() {
        let dict = plist::Dictionary::from_iter([(
            "IsSupervised".to_string(),
            plist::Value::Boolean(true),
        )]);
        let parsed = parse_cloud_configuration(plist::Value::Dictionary(dict.clone())).unwrap();
        assert_eq!(
            parsed
                .get("IsSupervised")
                .and_then(plist::Value::as_boolean),
            Some(true)
        );
    }

    #[tokio::test]
    async fn install_profile_sends_payload_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.install_profile(b"<plist/>").await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("InstallProfile")
        );
        assert_eq!(
            dict.get("Payload").and_then(plist::Value::as_data),
            Some(&b"<plist/>"[..])
        );
    }

    #[tokio::test]
    async fn remove_profile_sends_identifier_request() {
        let profile_list = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(Vec::new()),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));
        let remove_response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_responses(vec![profile_list, remove_response]);
        let mut client = McInstallClient::new(&mut stream);

        client.remove_profile("com.example.profile").await.unwrap();

        let first_len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let offset = 4 + first_len;
        let len =
            u32::from_be_bytes(stream.written[offset..offset + 4].try_into().unwrap()) as usize;
        let payload = &stream.written[offset + 4..offset + 4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("RemoveProfile")
        );
        assert_eq!(
            dict.get("ProfileIdentifier")
                .and_then(plist::Value::as_string),
            Some("com.example.profile")
        );
    }

    #[tokio::test]
    async fn remove_profile_uses_metadata_backed_identifier_when_available() {
        let profile_list = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(vec![plist::Value::String("com.example.profile".into())]),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "IsActive".to_string(),
                        plist::Value::Boolean(true),
                    )])),
                )])),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([
                        (
                            "PayloadUUID".to_string(),
                            plist::Value::String("1234-5678".into()),
                        ),
                        (
                            "PayloadVersion".to_string(),
                            plist::Value::Integer(7.into()),
                        ),
                    ])),
                )])),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));
        let remove_response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_responses(vec![profile_list, remove_response]);
        let mut client = McInstallClient::new(&mut stream);

        client.remove_profile("com.example.profile").await.unwrap();

        let first_len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let second_offset = 4 + first_len;
        let second_len = u32::from_be_bytes(
            stream.written[second_offset..second_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let second_payload = &stream.written[second_offset + 4..second_offset + 4 + second_len];
        let second_request: plist::Dictionary = plist::from_bytes(second_payload).unwrap();
        let profile_identifier = second_request
            .get("ProfileIdentifier")
            .and_then(plist::Value::as_data)
            .expect("metadata-backed profile identifier should be plist data");
        let identifier_plist = plist::Value::from_reader(std::io::Cursor::new(profile_identifier))
            .unwrap()
            .into_dictionary()
            .unwrap();
        assert_eq!(
            identifier_plist
                .get("PayloadIdentifier")
                .and_then(plist::Value::as_string),
            Some("com.example.profile")
        );
        assert_eq!(
            identifier_plist
                .get("PayloadUUID")
                .and_then(plist::Value::as_string),
            Some("1234-5678")
        );
        assert_eq!(
            identifier_plist
                .get("PayloadVersion")
                .and_then(plist::Value::as_unsigned_integer),
            Some(7)
        );
        assert_eq!(
            identifier_plist
                .get("PayloadType")
                .and_then(plist::Value::as_string),
            Some("Configuration")
        );
    }

    #[tokio::test]
    async fn get_profile_list_raw_preserves_unparsed_fields() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "OrderedIdentifiers".to_string(),
                plist::Value::Array(vec![plist::Value::String("com.example.profile".into())]),
            ),
            (
                "ProfileManifest".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "IsActive".to_string(),
                        plist::Value::Boolean(true),
                    )])),
                )])),
            ),
            (
                "ProfileMetadata".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "com.example.profile".to_string(),
                    plist::Value::Dictionary(plist::Dictionary::from_iter([(
                        "PayloadDisplayName".to_string(),
                        plist::Value::String("Example".into()),
                    )])),
                )])),
            ),
            (
                "Unhandled".to_string(),
                plist::Value::String("preserved".into()),
            ),
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        let raw = client.get_profile_list_raw().await.unwrap();
        let dict = raw.as_dictionary().unwrap();
        assert_eq!(dict["Unhandled"].as_string(), Some("preserved"));
    }

    #[tokio::test]
    async fn get_stored_profile_raw_includes_requested_purpose() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
            (
                "ProfileData".to_string(),
                plist::Value::Data(b"<plist/>".to_vec()),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        let raw = client
            .get_stored_profile_raw("PostSetupInstallation")
            .await
            .unwrap();
        let dict = raw.as_dictionary().unwrap();
        assert_eq!(dict["Status"].as_string(), Some("Acknowledged"));
        assert_eq!(dict["ProfileData"].as_data(), Some(&b"<plist/>"[..]));

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let sent: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            sent.get("RequestType").and_then(plist::Value::as_string),
            Some("GetStoredProfile")
        );
        assert_eq!(
            sent.get("Purpose").and_then(plist::Value::as_string),
            Some("PostSetupInstallation")
        );
    }

    #[tokio::test]
    async fn flush_sends_flush_request() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.flush().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("Flush")
        );
    }

    #[tokio::test]
    async fn hello_host_identifier_sends_request_type() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.hello_host_identifier().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("HelloHostIdentifier")
        );
    }

    #[tokio::test]
    async fn set_cloud_configuration_sends_payload() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);
        let cloud_configuration = plist::Dictionary::from_iter([
            ("AllowPairing".to_string(), plist::Value::Boolean(true)),
            (
                "SkipSetup".to_string(),
                plist::Value::Array(vec![plist::Value::String("WiFi".into())]),
            ),
        ]);

        client
            .set_cloud_configuration(cloud_configuration.clone())
            .await
            .unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("SetCloudConfiguration")
        );
        assert_eq!(
            dict.get("CloudConfiguration")
                .and_then(plist::Value::as_dictionary),
            Some(&cloud_configuration)
        );
    }

    #[tokio::test]
    async fn escalate_unsupervised_uses_zero_byte_certificate() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.escalate_unsupervised().await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("Escalate")
        );
        assert_eq!(
            dict.get("SupervisorCertificate")
                .and_then(plist::Value::as_data),
            Some(&b"\x00"[..])
        );
    }

    #[tokio::test]
    async fn erase_device_sends_expected_flags() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        client.erase_device(true, false).await.unwrap();

        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let payload = &stream.written[4..4 + len];
        let dict: plist::Dictionary = plist::from_bytes(payload).unwrap();
        assert_eq!(
            dict.get("RequestType").and_then(plist::Value::as_string),
            Some("EraseDevice")
        );
        assert_eq!(
            dict.get("PreserveDataPlan")
                .and_then(plist::Value::as_boolean),
            Some(true)
        );
        assert_eq!(
            dict.get("DisallowProximitySetup")
                .and_then(plist::Value::as_boolean),
            Some(false)
        );
    }

    fn sent_dictionary(stream: &MockStream) -> plist::Dictionary {
        let len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        plist::from_bytes(&stream.written[4..4 + len]).unwrap()
    }

    fn acknowledged_response() -> plist::Value {
        plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Status".to_string(),
            plist::Value::String("Acknowledged".into()),
        )]))
    }

    #[tokio::test]
    async fn fetch_unlock_token_sends_exact_request_and_redacts_token_debug() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
            (
                "UnlockToken".to_string(),
                plist::Value::Data(vec![0xde, 0xad, 0xbe, 0xef]),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        let token = client.fetch_unlock_token().await.unwrap();
        assert_eq!(token.as_bytes(), &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(token.len(), 4);
        assert!(!token.is_empty());
        assert!(format!("{token:?}").contains("redacted"));
        assert!(!format!("{token:?}").contains("de"));
        assert!(token.to_string().contains("redacted"));

        let request = sent_dictionary(&stream);
        assert_eq!(
            request.get("RequestType").and_then(plist::Value::as_string),
            Some("RequestUnlockToken")
        );
        assert_eq!(request.len(), 1);
    }

    #[tokio::test]
    async fn security_info_and_passcode_present_parse_response_fields() {
        let info = plist::Dictionary::from_iter([
            ("PasscodePresent".to_string(), plist::Value::Boolean(true)),
            (
                "PasscodeCompliant".to_string(),
                plist::Value::Boolean(false),
            ),
        ]);
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            ),
            (
                "SecurityInfo".to_string(),
                plist::Value::Dictionary(info.clone()),
            ),
        ]));
        let mut stream = MockStream::with_responses(vec![response.clone(), response]);
        let mut client = McInstallClient::new(&mut stream);

        assert_eq!(client.security_info().await.unwrap(), info);
        assert!(client.passcode_present().await.unwrap());

        let first = sent_dictionary(&stream);
        assert_eq!(
            first.get("RequestType").and_then(plist::Value::as_string),
            Some("SecurityInfo")
        );
        let first_len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let second_offset = first_len + 4;
        let second_len = u32::from_be_bytes(
            stream.written[second_offset..second_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let second: plist::Dictionary =
            plist::from_bytes(&stream.written[second_offset + 4..second_offset + 4 + second_len])
                .unwrap();
        assert_eq!(
            second.get("RequestType").and_then(plist::Value::as_string),
            Some("SecurityInfo")
        );
    }

    #[tokio::test]
    async fn clear_passcode_sends_token_and_clear_screen_time_uses_distinct_request() {
        let mut stream =
            MockStream::with_responses(vec![acknowledged_response(), acknowledged_response()]);
        let mut client = McInstallClient::new(&mut stream);
        client.clear_passcode(&[1, 2, 3, 4]).await.unwrap();
        client.clear_screen_time_password().await.unwrap();

        let first = sent_dictionary(&stream);
        assert_eq!(
            first.get("RequestType").and_then(plist::Value::as_string),
            Some("ClearPasscode")
        );
        assert_eq!(
            first.get("UnlockToken").and_then(plist::Value::as_data),
            Some(&[1, 2, 3, 4][..])
        );

        let first_len = u32::from_be_bytes(stream.written[..4].try_into().unwrap()) as usize;
        let second_offset = first_len + 4;
        let second_len = u32::from_be_bytes(
            stream.written[second_offset..second_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let second: plist::Dictionary =
            plist::from_bytes(&stream.written[second_offset + 4..second_offset + 4 + second_len])
                .unwrap();
        assert_eq!(
            second.get("RequestType").and_then(plist::Value::as_string),
            Some("ClearRestrictionsPassword")
        );
        assert_eq!(second.len(), 1);
    }

    #[tokio::test]
    async fn fetch_unlock_token_maps_keybag_error_without_secret_leak() {
        let chain_entry = plist::Dictionary::from_iter([
            (
                "ErrorDomain".to_string(),
                plist::Value::String(KEYBAG_ERROR_DOMAIN.into()),
            ),
            (
                "ErrorCode".to_string(),
                plist::Value::Integer(KEYBAG_PASSCODE_SET_ERROR_CODE.into()),
            ),
            (
                "LocalizedDescription".to_string(),
                plist::Value::String("passcode is present".into()),
            ),
        ]);
        let secret = vec![0xca, 0xfe, 0xba, 0xbe];
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Status".to_string(), plist::Value::String("Error".into())),
            (
                "ErrorChain".to_string(),
                plist::Value::Array(vec![plist::Value::Dictionary(chain_entry)]),
            ),
            (
                "UnlockToken".to_string(),
                plist::Value::Data(secret.clone()),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = McInstallClient::new(&mut stream);

        let error = client.fetch_unlock_token().await.unwrap_err();
        assert!(matches!(error, McInstallError::PasscodeSet));
        let rendered = error.to_string();
        assert!(!rendered.contains("cafe"));
        assert!(!rendered.contains("passcode is present"));
    }

    #[test]
    fn error_chain_description_is_structured_and_bounded_without_response_debug() {
        let response = plist::Dictionary::from_iter([
            (
                "Status".to_string(),
                plist::Value::String("Rejected".into()),
            ),
            (
                "ErrorChain".to_string(),
                plist::Value::Array(vec![plist::Value::Dictionary(
                    plist::Dictionary::from_iter([
                        (
                            "ErrorDomain".to_string(),
                            plist::Value::String("TestDomain".into()),
                        ),
                        ("ErrorCode".to_string(), plist::Value::Integer(42.into())),
                        (
                            "LocalizedDescription".to_string(),
                            plist::Value::String("not authorized".into()),
                        ),
                    ]),
                )]),
            ),
        ]);
        let error = ensure_acknowledged(&response).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("TestDomain(42): not authorized"));
        assert!(!rendered.contains("ErrorChain"));
    }

    #[test]
    fn secret_operation_errors_do_not_copy_free_form_daemon_text() {
        let response = plist::Dictionary::from_iter([
            (
                "Status".to_string(),
                plist::Value::String("Rejected".into()),
            ),
            (
                "ErrorChain".to_string(),
                plist::Value::Array(vec![plist::Value::Dictionary(
                    plist::Dictionary::from_iter([
                        (
                            "ErrorDomain".to_string(),
                            plist::Value::String("TestDomain".into()),
                        ),
                        ("ErrorCode".to_string(), plist::Value::Integer(42.into())),
                        (
                            "LocalizedDescription".to_string(),
                            plist::Value::String("unlock token=do-not-print".into()),
                        ),
                    ]),
                )]),
            ),
        ]);
        let error = ensure_acknowledged_named(&response, "RequestUnlockToken").unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("TestDomain(42)"));
        assert!(!rendered.contains("do-not-print"));
    }
}

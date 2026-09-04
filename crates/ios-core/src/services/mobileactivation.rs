//! MobileActivation service and Apple activation HTTP helpers.
//!
//! The device side of activation is a length-prefixed XML plist protocol on
//! `com.apple.mobileactivationd`.  Apple activation itself is a separate HTTPS
//! exchange; keeping the two layers separate makes it possible to test the
//! device protocol without contacting Apple and prevents a service client from
//! accidentally treating a local command as an online activation.

use std::collections::BTreeMap;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use zeroize::Zeroize;

pub const SERVICE_NAME: &str = "com.apple.mobileactivationd";
pub const ACTIVATION_URL: &str = "https://albert.apple.com/deviceservices/deviceActivation";
pub const DRM_HANDSHAKE_URL: &str = "https://albert.apple.com/deviceservices/drmHandshake";
pub const ACTIVATION_USER_AGENT: &str = "iOS Device Activator (MobileActivation-592.103.2)";
pub const ITUNES_HAS_CONNECTED_KEY: &str = "iTunesHasConnected";
pub const ACTIVATION_STATE_ACKNOWLEDGED_KEY: &str = "ActivationStateAcknowledged";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_PLIST_SIZE: usize = 8 * 1024 * 1024;
const MAX_HTTP_BODY_SIZE: usize = 16 * 1024 * 1024;

/// Default delay between nonce/session probes while waiting for a fresh
/// Tunnel1 session.  The delay avoids a busy loop in mobileactivationd while
/// remaining short enough for normal activation.
pub const DEFAULT_ACTIVATION_POLL_INTERVAL: Duration = Duration::from_millis(250);
const MIN_ACTIVATION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_ACTIVATION_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Extract the payload used by the pre-session lockdown activation protocol.
///
/// Modern responses are passed as the raw plist bytes to
/// `HandleActivationInfoWithSessionRequest`; older servers return a wrapper
/// containing `iphone-activation` or `device-activation` and require the
/// lockdown `Activate` request instead.  Returning `None` deliberately leaves
/// modern records to the session path, where mobileactivationd can provide the
/// authoritative protocol error.  Callers that need to distinguish a
/// malformed legacy wrapper should check [`has_legacy_activation_wrapper`].
pub fn extract_legacy_activation_record(bytes: &[u8]) -> Option<plist::Value> {
    let value = plist::from_bytes::<plist::Value>(bytes).ok()?;
    let root = value.as_dictionary()?;
    for key in ["iphone-activation", "device-activation"] {
        if let Some(record) = root
            .get(key)
            .and_then(plist::Value::as_dictionary)
            .and_then(|dict| dict.get("activation-record"))
        {
            return Some(record.clone());
        }
    }
    None
}

/// Whether a serialized activation response explicitly uses the legacy
/// wrapper shape.  This is separate from [`extract_legacy_activation_record`]
/// so callers can reject a malformed legacy wrapper instead of accidentally
/// routing it as a modern session record.
pub fn has_legacy_activation_wrapper(bytes: &[u8]) -> bool {
    let value = match plist::from_bytes::<plist::Value>(bytes) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let Some(root) = value.as_dictionary() else {
        return false;
    };
    ["iphone-activation", "device-activation"]
        .iter()
        .any(|key| root.contains_key(key))
}

service_error!(MobileActivationError, between {
    /// The operation exceeded its configured deadline.
    #[error("mobile activation operation timed out after {0:?}")]
    Timeout(Duration),
    /// An activation endpoint or HTTP response was rejected.
    #[error("activation HTTP error: {0}")]
    Http(String),
});

/// A client for one `mobileactivationd` service connection.
#[derive(Debug)]
pub struct MobileActivationClient<S> {
    stream: S,
    timeout: Duration,
}

impl<S: AsyncRead + AsyncWrite + Unpin> MobileActivationClient<S> {
    pub fn new(stream: S) -> Self {
        Self::with_timeout(stream, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(stream: S, timeout: Duration) -> Self {
        Self { stream, timeout }
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Request the Tunnel1 session information used for the DRM handshake.
    pub async fn request_session_info(
        &mut self,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let response = self
            .send_command("CreateTunnel1SessionInfoRequest", None)
            .await?;
        ensure_value_payload(response, "CreateTunnel1SessionInfoRequest")
    }

    /// Wait until mobileactivationd publishes a new Tunnel1 nonce/session.
    ///
    /// pmd3 performs this wait before online activation so the handshake is
    /// tied to a fresh nonce cycle.  The returned dictionary is the complete
    /// daemon response (including `Value`) and is suitable for the DRM
    /// handshake.  A zero timeout performs no request and returns a timeout.
    pub async fn wait_for_activation_session(
        &mut self,
        timeout: Duration,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| {
                MobileActivationError::Protocol(
                    "activation session wait timeout is too large".into(),
                )
            })?;
        self.wait_for_activation_session_until(deadline, DEFAULT_ACTIVATION_POLL_INTERVAL)
            .await
    }

    /// Wait for a fresh Tunnel1 session using an absolute deadline.
    ///
    /// Each request and the inter-request sleep are bounded by `deadline`.
    /// Poll intervals are clamped to a small non-zero range so callers cannot
    /// accidentally create a busy loop or a very slow unbounded wait.
    pub async fn wait_for_activation_session_until(
        &mut self,
        deadline: tokio::time::Instant,
        poll_interval: Duration,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let started = tokio::time::Instant::now();
        let poll_interval = poll_interval
            .max(MIN_ACTIVATION_POLL_INTERVAL)
            .min(MAX_ACTIVATION_POLL_INTERVAL);
        let initial = self.request_session_info_until(deadline, started).await?;
        self.wait_for_activation_session_from_initial(initial, deadline, started, poll_interval)
            .await
    }

    /// Wait for a fresh session while preserving the legacy caller's ability
    /// to fall back when the first session probe is unsupported.  Once a
    /// session has been observed, subsequent errors are returned rather than
    /// silently retrying through the legacy protocol.
    pub async fn try_wait_for_activation_session_until(
        &mut self,
        deadline: tokio::time::Instant,
        poll_interval: Duration,
    ) -> Result<Option<plist::Dictionary>, MobileActivationError> {
        let started = tokio::time::Instant::now();
        let poll_interval = poll_interval
            .max(MIN_ACTIVATION_POLL_INTERVAL)
            .min(MAX_ACTIVATION_POLL_INTERVAL);
        let initial = match self.request_session_info_until(deadline, started).await {
            Ok(initial) => initial,
            Err(error) if is_session_feature_unavailable(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        self.wait_for_activation_session_from_initial(initial, deadline, started, poll_interval)
            .await
            .map(Some)
    }

    async fn wait_for_activation_session_from_initial(
        &mut self,
        initial: plist::Dictionary,
        deadline: tokio::time::Instant,
        started: tokio::time::Instant,
        poll_interval: Duration,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let initial_message = activation_handshake_message(&initial)?;

        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(activation_session_timeout(started, deadline));
            }
            let next_poll = now
                .checked_add(poll_interval)
                .map_or(deadline, |instant| instant.min(deadline));
            tokio::time::sleep_until(next_poll).await;
            if tokio::time::Instant::now() >= deadline {
                return Err(activation_session_timeout(started, deadline));
            }

            let current = self.request_session_info_until(deadline, started).await?;
            let current_message = activation_handshake_message(&current)?;
            if current_message != initial_message {
                return Ok(current);
            }
        }
    }

    /// Request activation info using go-ios's legacy-compatible command name.
    pub async fn request_activation_info(
        &mut self,
        handshake_response: &[u8],
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let response = self
            .request_activation_info_command("CreateActivationInfoRequest", handshake_response)
            .await?;
        ensure_value_payload(response, "CreateActivationInfoRequest")
    }

    /// Request activation info using the modern pmd3 Tunnel1 command.
    pub async fn request_tunnel1_activation_info(
        &mut self,
        handshake_response: &[u8],
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let response = self
            .request_activation_info_command(
                "CreateTunnel1ActivationInfoRequest",
                handshake_response,
            )
            .await?;
        ensure_value_payload(response, "CreateTunnel1ActivationInfoRequest")
    }

    /// Apply the raw XML activation response returned by Apple's endpoint.
    pub async fn handle_activation_info(
        &mut self,
        activation_record: &[u8],
        response_headers: &BTreeMap<String, String>,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        if activation_record.is_empty() {
            return Err(MobileActivationError::Protocol(
                "activation record must not be empty".into(),
            ));
        }
        let headers: plist::Dictionary = response_headers
            .iter()
            .map(|(key, value)| (key.clone(), plist::Value::String(value.clone())))
            .collect();
        let mut fields = plist::Dictionary::from_iter([(
            "Value".to_string(),
            plist::Value::Data(activation_record.to_vec()),
        )]);
        if !headers.is_empty() {
            fields.insert(
                "ActivationResponseHeaders".to_string(),
                plist::Value::Dictionary(headers),
            );
        }
        self.send_command("HandleActivationInfoWithSessionRequest", Some(fields))
            .await
    }

    /// Apply an activation record returned by Apple's activation endpoint.
    ///
    /// This is the high-level service spelling of
    /// [`Self::handle_activation_info`].  Keeping the operation in the core
    /// client prevents callers from accidentally treating an HTTP response as
    /// a successful activation without sending the daemon command.
    pub async fn activate(
        &mut self,
        activation_record: &[u8],
        response_headers: &BTreeMap<String, String>,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        self.handle_activation_info(activation_record, response_headers)
            .await
    }

    /// Ask mobileactivationd to deactivate the device.
    pub async fn deactivate(&mut self) -> Result<plist::Dictionary, MobileActivationError> {
        self.send_command("DeactivateRequest", None).await
    }

    /// Read activation state through mobileactivationd.
    pub async fn activation_state(&mut self) -> Result<plist::Value, MobileActivationError> {
        let response = self.send_command("GetActivationStateRequest", None).await?;
        response.get("Value").cloned().ok_or_else(|| {
            MobileActivationError::Protocol(
                "GetActivationStateRequest response is missing Value".into(),
            )
        })
    }

    async fn request_activation_info_command(
        &mut self,
        command: &str,
        handshake_response: &[u8],
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let options = plist::Dictionary::from_iter([(
            "BasebandWaitCount".to_string(),
            plist::Value::Integer(90i64.into()),
        )]);
        let fields = plist::Dictionary::from_iter([
            (
                "Value".to_string(),
                plist::Value::Data(handshake_response.to_vec()),
            ),
            ("Options".to_string(), plist::Value::Dictionary(options)),
        ]);
        self.send_command(command, Some(fields)).await
    }

    async fn request_session_info_until(
        &mut self,
        deadline: tokio::time::Instant,
        started: tokio::time::Instant,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        if tokio::time::Instant::now() >= deadline {
            return Err(activation_session_timeout(started, deadline));
        }
        tokio::time::timeout_at(deadline, self.request_session_info())
            .await
            .map_err(|_| activation_session_timeout(started, deadline))?
    }

    async fn send_command(
        &mut self,
        command: &str,
        fields: Option<plist::Dictionary>,
    ) -> Result<plist::Dictionary, MobileActivationError> {
        let mut request = plist::Dictionary::from_iter([(
            "Command".to_string(),
            plist::Value::String(command.to_owned()),
        )]);
        if let Some(fields) = fields {
            request.extend(fields);
        }
        let timeout = self.timeout;
        let operation = async {
            super::plist_frame::write_xml_plist_frame(
                &mut self.stream,
                &plist::Value::Dictionary(request),
                MAX_PLIST_SIZE,
            )
            .await
            .map_err(map_frame_error)?;
            let response: plist::Dictionary =
                super::plist_frame::read_plist_frame(&mut self.stream, MAX_PLIST_SIZE)
                    .await
                    .map_err(map_frame_error)?;
            validate_response(response, command)
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| MobileActivationError::Timeout(timeout))?
    }
}

fn activation_handshake_message(
    response: &plist::Dictionary,
) -> Result<&plist::Value, MobileActivationError> {
    response
        .get("Value")
        .and_then(plist::Value::as_dictionary)
        .and_then(|value| value.get("HandshakeRequestMessage"))
        .ok_or_else(|| {
            MobileActivationError::Protocol(
                "session-info response is missing Value.HandshakeRequestMessage".into(),
            )
        })
}

fn is_session_feature_unavailable(error: &MobileActivationError) -> bool {
    matches!(
        error,
        MobileActivationError::Protocol(message)
            if message.starts_with("CreateTunnel1SessionInfoRequest failed")
    )
}

fn activation_session_timeout(
    started: tokio::time::Instant,
    deadline: tokio::time::Instant,
) -> MobileActivationError {
    MobileActivationError::Timeout(deadline.saturating_duration_since(started))
}

fn validate_response(
    response: plist::Dictionary,
    command: &str,
) -> Result<plist::Dictionary, MobileActivationError> {
    if let Some(error) = response.get("Error") {
        return Err(MobileActivationError::Protocol(format!(
            "{command} failed: {}",
            redact_error_value(error)
        )));
    }
    if let Some(chain) = response.get("ErrorChain") {
        if let Some(summary) = summarize_error_chain(chain) {
            return Err(MobileActivationError::Protocol(format!(
                "{command} failed: {summary}"
            )));
        }
    }
    if let Some(status) = response.get("Status").and_then(plist::Value::as_string) {
        if matches!(
            status.to_ascii_lowercase().as_str(),
            "error" | "failed" | "failure" | "rejected"
        ) {
            return Err(MobileActivationError::Protocol(format!(
                "{command} failed with Status={status}"
            )));
        }
    }
    Ok(response)
}

fn ensure_value_payload(
    response: plist::Dictionary,
    command: &str,
) -> Result<plist::Dictionary, MobileActivationError> {
    if !response.contains_key("Value") {
        return Err(MobileActivationError::Protocol(format!(
            "{command} response is missing Value"
        )));
    }
    Ok(response)
}

fn redact_error_value(value: &plist::Value) -> String {
    match value {
        plist::Value::String(value) => format!("<redacted error: {} chars>", value.chars().count()),
        plist::Value::Integer(_) | plist::Value::Real(_) | plist::Value::Boolean(_) => {
            format!("{value:?}")
        }
        plist::Value::Data(data) => format!("<redacted data: {} bytes>", data.len()),
        plist::Value::Array(_) | plist::Value::Dictionary(_) | plist::Value::Uid(_) => {
            "<redacted structured error>".into()
        }
        _ => "<redacted error>".into(),
    }
}

/// Ceiling on the length of a daemon-supplied error domain echoed into error
/// text.
const MAX_ERROR_DOMAIN_CHARS: usize = 128;

/// Error text reaches stderr before any diagnostic redactor runs.  Real
/// activation error domains are short reverse-DNS or identifier-shaped
/// strings; anything longer, or containing separators, whitespace, or
/// non-ASCII bytes, is treated as untrusted payload and masked to a shape
/// summary instead of being echoed.
fn error_domain_token(domain: &str) -> String {
    if !domain.is_empty()
        && domain.len() <= MAX_ERROR_DOMAIN_CHARS
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_'))
    {
        domain.to_string()
    } else {
        format!("<redacted domain: {} chars>", domain.chars().count())
    }
}

fn summarize_error_chain(value: &plist::Value) -> Option<String> {
    let single_entry;
    let entries = if let Some(entries) = value.as_array() {
        entries
    } else if value.as_dictionary().is_some() {
        single_entry = vec![value.clone()];
        &single_entry
    } else {
        return None;
    };
    if entries.is_empty() {
        return None;
    }
    let mut summary = format!("ErrorChain entries={}", entries.len());
    for entry in entries.iter().take(4) {
        if let Some(dict) = entry.as_dictionary() {
            if let Some(domain) = dict.get("ErrorDomain").and_then(plist::Value::as_string) {
                summary.push_str(" domain=");
                summary.push_str(&error_domain_token(domain));
            }
            if let Some(code) = dict.get("ErrorCode") {
                summary.push_str(" code=");
                summary.push_str(&redact_error_value(code));
            }
        }
    }
    Some(summary)
}

fn map_frame_error(error: super::plist_frame::PlistFrameError) -> MobileActivationError {
    match error {
        super::plist_frame::PlistFrameError::Io(error) => MobileActivationError::Io(error),
        super::plist_frame::PlistFrameError::Plist(error) => MobileActivationError::Plist(error),
        super::plist_frame::PlistFrameError::Protocol(error) => {
            MobileActivationError::Protocol(error)
        }
    }
}

/// A response from one of Apple's activation endpoints.
#[derive(Clone)]
pub struct ActivationHttpResponse {
    /// The response body is zeroized when this value is dropped because it
    /// contains an activation record or DRM material.
    pub body: Vec<u8>,
    pub headers: BTreeMap<String, String>,
    pub content_type: Option<String>,
}

impl std::fmt::Debug for ActivationHttpResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivationHttpResponse")
            .field(
                "body",
                &format_args!("<redacted data: {} bytes>", self.body.len()),
            )
            .field(
                "headers",
                &self.headers.keys().map(String::as_str).collect::<Vec<_>>(),
            )
            .field("content_type", &self.content_type)
            .finish()
    }
}

impl Drop for ActivationHttpResponse {
    fn drop(&mut self) {
        self.body.zeroize();
    }
}

impl ActivationHttpResponse {
    pub fn content_type_is(&self, expected: &str) -> bool {
        self.content_type
            .as_deref()
            .and_then(|value| value.split(';').next())
            .map(|value| value.trim().eq_ignore_ascii_case(expected))
            .unwrap_or(false)
    }
}

/// HTTPS client for Apple's activation endpoints.
///
/// The default constructor accepts only the official Apple host. Custom
/// endpoints remain HTTPS-only and require an explicit unsafe opt-in; TLS
/// certificate verification is never disabled and redirects are rejected.
#[derive(Clone)]
pub struct ActivationHttpClient {
    client: reqwest::Client,
    activation_url: String,
    drm_handshake_url: String,
    timeout: Duration,
}

impl std::fmt::Debug for ActivationHttpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActivationHttpClient")
            .field("activation_url", &self.activation_url)
            .field("drm_handshake_url", &self.drm_handshake_url)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl ActivationHttpClient {
    pub fn official() -> Result<Self, MobileActivationError> {
        Self::with_endpoints(ACTIVATION_URL, DRM_HANDSHAKE_URL, false, DEFAULT_TIMEOUT)
    }

    pub fn with_endpoints(
        activation_url: &str,
        drm_handshake_url: &str,
        unsafe_custom_server: bool,
        timeout: Duration,
    ) -> Result<Self, MobileActivationError> {
        validate_endpoint(activation_url, ACTIVATION_URL, unsafe_custom_server)?;
        validate_endpoint(drm_handshake_url, DRM_HANDSHAKE_URL, unsafe_custom_server)?;
        let client = reqwest::Client::builder()
            .user_agent(ACTIVATION_USER_AGENT)
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| MobileActivationError::Http(error.to_string()))?;
        Ok(Self {
            client,
            activation_url: activation_url.to_owned(),
            drm_handshake_url: drm_handshake_url.to_owned(),
            timeout,
        })
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    pub async fn post_drm_handshake(
        &self,
        session_info: &plist::Dictionary,
    ) -> Result<ActivationHttpResponse, MobileActivationError> {
        let mut body = Vec::new();
        plist::to_writer_xml(&mut body, &plist::Value::Dictionary(session_info.clone()))?;
        self.post(
            &self.drm_handshake_url,
            body,
            "application/x-apple-plist",
            "application/xml",
        )
        .await
    }

    pub async fn post_activation_info(
        &self,
        activation_info: &plist::Dictionary,
    ) -> Result<ActivationHttpResponse, MobileActivationError> {
        self.post_activation_info_with_fields(activation_info, &BTreeMap::new())
            .await
    }

    /// Post activation info with the legacy lockdown form fields used by
    /// pre-Tunnel1 activation.  Modern callers should use
    /// [`Self::post_activation_info`] so they cannot accidentally mix the two
    /// protocols.
    pub async fn post_activation_info_with_fields(
        &self,
        activation_info: &plist::Dictionary,
        fields: &BTreeMap<String, String>,
    ) -> Result<ActivationHttpResponse, MobileActivationError> {
        let mut plist_body = Vec::new();
        plist::to_writer_xml(
            &mut plist_body,
            &plist::Value::Dictionary(activation_info.clone()),
        )?;
        let mut body = format!("activation-info={}", form_encode(&plist_body));
        for (key, value) in fields {
            body.push('&');
            body.push_str(&form_encode(key.as_bytes()));
            body.push('=');
            body.push_str(&form_encode(value.as_bytes()));
        }
        self.post(
            &self.activation_url,
            body.into_bytes(),
            "application/x-www-form-urlencoded",
            "*/*",
        )
        .await
    }

    async fn post(
        &self,
        url: &str,
        body: Vec<u8>,
        content_type: &str,
        accept: &str,
    ) -> Result<ActivationHttpResponse, MobileActivationError> {
        if body.len() > MAX_HTTP_BODY_SIZE {
            return Err(MobileActivationError::Http(format!(
                "request body exceeds {} bytes",
                MAX_HTTP_BODY_SIZE
            )));
        }
        let response = self
            .client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .header(reqwest::header::ACCEPT, accept)
            .header(reqwest::header::EXPECT, "100-continue")
            .body(body)
            .send()
            .await
            .map_err(|error| MobileActivationError::Http(error.to_string()))?;
        let status = response.status();
        let content_length = response.content_length();
        if content_length.is_some_and(|length| length > MAX_HTTP_BODY_SIZE as u64) {
            return Err(MobileActivationError::Http(format!(
                "response body exceeds {} bytes",
                MAX_HTTP_BODY_SIZE
            )));
        }
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (canonical_header_name(name.as_str()), value.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        let content_type = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.clone());
        let mut bytes = Vec::with_capacity(
            content_length
                .unwrap_or_default()
                .min(MAX_HTTP_BODY_SIZE as u64) as usize,
        );
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| MobileActivationError::Http(error.to_string()))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_HTTP_BODY_SIZE {
                return Err(MobileActivationError::Http(format!(
                    "response body exceeds {} bytes",
                    MAX_HTTP_BODY_SIZE
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(MobileActivationError::Http(format!(
                "endpoint returned HTTP status {status}"
            )));
        }
        Ok(ActivationHttpResponse {
            body: bytes,
            headers,
            content_type,
        })
    }
}

/// `HeaderMap` exposes lowercase names, while the device-side activation
/// protocol historically receives the canonical HTTP spelling (for example
/// `Content-Type`).  Keep the wire-level plist shape compatible with
/// net/http/requests clients, which preserve canonical header names.
fn canonical_header_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for (index, part) in name.split('-').enumerate() {
        if index != 0 {
            result.push('-');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            result.push_str(&chars.as_str().to_ascii_lowercase());
        }
    }
    result
}

fn validate_endpoint(
    url: &str,
    official_url: &str,
    unsafe_custom_server: bool,
) -> Result<(), MobileActivationError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| MobileActivationError::Http(format!("invalid endpoint URL: {error}")))?;
    if parsed.scheme() != "https" {
        return Err(MobileActivationError::Http(
            "activation endpoints must use HTTPS".into(),
        ));
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(MobileActivationError::Http(
            "activation endpoint must not contain user credentials".into(),
        ));
    }
    if !unsafe_custom_server && url != official_url {
        return Err(MobileActivationError::Http(
            "non-official activation endpoint requires explicit unsafe opt-in".into(),
        ));
    }
    Ok(())
}

fn form_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else if byte == b' ' {
            encoded.push('+');
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use crate::test_util::MockStream;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    use super::*;

    fn request_dict(written: &[u8]) -> plist::Dictionary {
        let len = u32::from_be_bytes(written[..4].try_into().unwrap()) as usize;
        plist::from_bytes(&written[4..4 + len]).unwrap()
    }

    fn response_with_handshake(message: &[u8]) -> plist::Value {
        plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("CreateTunnel1SessionInfoRequest".into()),
            ),
            (
                "Value".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "HandshakeRequestMessage".to_string(),
                    plist::Value::Data(message.to_vec()),
                )])),
            ),
        ]))
    }

    fn request_frame_count(written: &[u8]) -> usize {
        let mut offset = 0;
        let mut count = 0;
        while offset < written.len() {
            let end = offset
                + 4
                + u32::from_be_bytes(written[offset..offset + 4].try_into().unwrap()) as usize;
            assert!(end <= written.len());
            offset = end;
            count += 1;
        }
        count
    }

    #[tokio::test]
    async fn request_session_info_sends_exact_tunnel1_command() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            (
                "Command".to_string(),
                plist::Value::String("CreateTunnel1SessionInfoRequest".into()),
            ),
            (
                "Value".to_string(),
                plist::Value::Dictionary(plist::Dictionary::from_iter([(
                    "HandshakeRequestMessage".to_string(),
                    plist::Value::Data(vec![1, 2, 3]),
                )])),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = MobileActivationClient::new(&mut stream);
        let result = client.request_session_info().await.unwrap();
        assert!(result.contains_key("Value"));
        assert_eq!(
            request_dict(&stream.written)["Command"].as_string(),
            Some("CreateTunnel1SessionInfoRequest")
        );
    }

    #[tokio::test]
    async fn session_info_rejects_success_without_value() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            )])));
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client
            .request_session_info()
            .await
            .expect_err("a missing Value is not a session payload");
        assert!(error.to_string().contains("missing Value"));
    }

    #[tokio::test]
    async fn waits_for_a_new_nonce_with_multiple_session_probes() {
        let mut stream = MockStream::with_responses(vec![
            response_with_handshake(b"old"),
            response_with_handshake(b"old"),
            response_with_handshake(b"new"),
        ]);
        let mut client = MobileActivationClient::new(&mut stream);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        let response = client
            .wait_for_activation_session_until(deadline, Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(
            response["Value"].as_dictionary().unwrap()["HandshakeRequestMessage"].as_data(),
            Some(b"new".as_slice())
        );
        assert_eq!(request_frame_count(&stream.written), 3);
    }

    #[tokio::test]
    async fn zero_activation_session_timeout_does_not_send_a_probe() {
        let mut stream = MockStream::with_response(response_with_handshake(b"old"));
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client
            .wait_for_activation_session(Duration::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(error, MobileActivationError::Timeout(_)));
        assert!(stream.written.is_empty());
    }

    #[tokio::test]
    async fn activation_session_wait_times_out_and_cancellation_is_observable() {
        let mut stream = MockStream::with_responses(vec![response_with_handshake(b"old"); 32]);
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client
            .wait_for_activation_session_until(
                // Windows timer scheduling can overshoot a 10 ms sleep by a
                // full timer tick.  Leave enough room for the test to
                // observe a second probe while still exercising the
                // absolute-deadline timeout path.
                tokio::time::Instant::now() + Duration::from_millis(100),
                Duration::from_millis(10),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, MobileActivationError::Timeout(_)));
        assert!(request_frame_count(&stream.written) >= 2);

        let mut client = MobileActivationClient::new(PendingStream);
        let result = tokio::time::timeout(
            Duration::from_millis(10),
            client.wait_for_activation_session(Duration::from_secs(1)),
        )
        .await;
        assert!(
            result.is_err(),
            "cancellation must stop a pending session request"
        );
    }

    #[tokio::test]
    async fn activation_session_wait_propagates_probe_errors_without_extra_requests() {
        let mut stream = MockStream::with_responses(vec![
            response_with_handshake(b"old"),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Error".to_string(),
                plist::Value::String("session is unavailable".into()),
            )])),
        ]);
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client
            .wait_for_activation_session_until(
                tokio::time::Instant::now() + Duration::from_millis(100),
                Duration::from_millis(10),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("failed"));
        assert_eq!(request_frame_count(&stream.written), 2);
    }

    #[tokio::test]
    async fn try_wait_preserves_legacy_fallback_only_for_initial_probe_failure() {
        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Error".to_string(),
                plist::Value::String("unsupported".into()),
            )])));
        let mut client = MobileActivationClient::new(&mut stream);
        let result = client
            .try_wait_for_activation_session_until(
                tokio::time::Instant::now() + Duration::from_millis(100),
                Duration::from_millis(10),
            )
            .await
            .unwrap();
        assert!(result.is_none());
        assert_eq!(request_frame_count(&stream.written), 1);

        let mut stream = MockStream::with_responses(vec![
            response_with_handshake(b"old"),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Error".to_string(),
                plist::Value::String("poll failed".into()),
            )])),
        ]);
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client
            .try_wait_for_activation_session_until(
                tokio::time::Instant::now() + Duration::from_millis(100),
                Duration::from_millis(10),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("failed"));
        assert_eq!(request_frame_count(&stream.written), 2);
    }

    #[tokio::test]
    async fn try_wait_does_not_turn_timeout_or_malformed_probe_into_legacy_fallback() {
        let mut client = MobileActivationClient::new(PendingStream);
        let error = client
            .try_wait_for_activation_session_until(
                tokio::time::Instant::now() + Duration::from_millis(10),
                Duration::from_millis(10),
            )
            .await
            .expect_err("a stalled probe must remain an error");
        assert!(matches!(error, MobileActivationError::Timeout(_)));

        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Status".to_string(),
                plist::Value::String("Acknowledged".into()),
            )])));
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client
            .try_wait_for_activation_session_until(
                tokio::time::Instant::now() + Duration::from_millis(100),
                Duration::from_millis(10),
            )
            .await
            .expect_err("a malformed successful response must remain an error");
        assert!(error.to_string().contains("missing Value"));
    }

    #[tokio::test]
    async fn activation_info_supports_both_upstream_command_spellings() {
        for command in [
            "CreateActivationInfoRequest",
            "CreateTunnel1ActivationInfoRequest",
        ] {
            let response = plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Value".to_string(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            )]));
            let mut stream = MockStream::with_response(response);
            let mut client = MobileActivationClient::new(&mut stream);
            if command.starts_with("CreateTunnel1") {
                client
                    .request_tunnel1_activation_info(&[9, 8])
                    .await
                    .unwrap();
            } else {
                client.request_activation_info(&[9, 8]).await.unwrap();
            }
            assert_eq!(
                request_dict(&stream.written)["Command"].as_string(),
                Some(command)
            );
            assert_eq!(
                request_dict(&stream.written)["Value"].as_data(),
                Some(&[9, 8][..])
            );
        }
    }

    #[tokio::test]
    async fn handle_deactivate_and_state_requests_are_structured_and_errors_are_redacted() {
        let responses = vec![
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Value".to_string(),
                plist::Value::String("Activated".into()),
            )])),
            plist::Value::Dictionary(plist::Dictionary::from_iter([(
                "Error".to_string(),
                plist::Value::Data(vec![1, 2, 3]),
            )])),
        ];
        let mut stream = MockStream::with_responses(responses);
        let mut client = MobileActivationClient::new(&mut stream);
        assert_eq!(
            client.activation_state().await.unwrap().as_string(),
            Some("Activated")
        );
        let error = client.deactivate().await.unwrap_err();
        assert!(error.to_string().contains("3 bytes"));
        assert!(!error.to_string().contains("[1, 2, 3]"));
    }

    #[tokio::test]
    async fn status_and_error_chain_are_reported_without_secret_descriptions() {
        let response = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("Status".to_string(), plist::Value::String("Error".into())),
            (
                "ErrorChain".to_string(),
                plist::Value::Array(vec![plist::Value::Dictionary(
                    plist::Dictionary::from_iter([
                        (
                            "ErrorDomain".to_string(),
                            plist::Value::String("MobileActivation".into()),
                        ),
                        ("ErrorCode".to_string(), plist::Value::Integer(42.into())),
                        (
                            "ErrorDescription".to_string(),
                            plist::Value::String("private activation token".into()),
                        ),
                    ]),
                )]),
            ),
        ]));
        let mut stream = MockStream::with_response(response);
        let mut client = MobileActivationClient::new(&mut stream);
        let error = client.request_session_info().await.unwrap_err().to_string();
        assert!(error.contains("Status=Error") || error.contains("ErrorChain"));
        assert!(!error.contains("private activation token"));
        assert!(error.contains("MobileActivation"));
        assert!(error.contains("42"));
    }

    #[test]
    fn error_chain_domains_pass_identifier_shaped_values_and_mask_the_rest() {
        let identifier =
            summarize_error_chain(&plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([
                    (
                        "ErrorDomain".to_string(),
                        plist::Value::String("AKAuthenticationError".into()),
                    ),
                    (
                        "ErrorCode".to_string(),
                        plist::Value::Integer((-44101).into()),
                    ),
                ]),
            )]))
            .unwrap();
        assert!(
            identifier.contains("domain=AKAuthenticationError"),
            "{identifier}"
        );
        assert!(identifier.contains("-44101"), "{identifier}");

        let blob = "-----BEGIN CERTIFICATE-----SECRET-MARKER-BLOB-----END CERTIFICATE-----";
        let masked = summarize_error_chain(&plist::Value::Array(vec![plist::Value::Dictionary(
            plist::Dictionary::from_iter([
                ("ErrorDomain".to_string(), plist::Value::String(blob.into())),
                (
                    "ErrorCode".to_string(),
                    plist::Value::String("SECRET-MARKER-CODE".into()),
                ),
            ]),
        )]))
        .unwrap();
        assert!(!masked.contains("SECRET-MARKER-BLOB"), "{masked}");
        assert!(!masked.contains("SECRET-MARKER-CODE"), "{masked}");
        assert!(masked.contains("redacted domain"), "{masked}");
        assert!(masked.contains("redacted error"), "{masked}");

        let oversized = "A".repeat(500) + "SECRETMARKERTAIL";
        let masked = summarize_error_chain(&plist::Value::Array(vec![plist::Value::Dictionary(
            plist::Dictionary::from_iter([(
                "ErrorDomain".to_string(),
                plist::Value::String(oversized.clone()),
            )]),
        )]))
        .unwrap();
        assert!(!masked.contains("SECRETMARKERTAIL"), "{masked}");

        let with_spaces =
            summarize_error_chain(&plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([(
                    "ErrorDomain".to_string(),
                    plist::Value::String("SECRET MARKER WITH SPACES".into()),
                )]),
            )]))
            .unwrap();
        assert!(!with_spaces.contains("SECRET MARKER"), "{with_spaces}");

        // Empty and non-string domains simply contribute nothing readable.
        assert!(
            summarize_error_chain(&plist::Value::Array(vec![plist::Value::Dictionary(
                plist::Dictionary::from_iter([(
                    "ErrorDomain".to_string(),
                    plist::Value::String(String::new()),
                )]),
            )]))
            .unwrap()
            .contains("domain=<redacted domain: 0 chars>")
        );
    }

    #[test]
    fn error_strings_are_never_echoed_verbatim() {
        let summary =
            summarize_error_chain(&plist::Value::Dictionary(plist::Dictionary::from_iter([
                (
                    "ErrorDomain".to_string(),
                    plist::Value::String("AKAuthenticationError".into()),
                ),
                (
                    "ErrorCode".to_string(),
                    plist::Value::String("opaque secret token".into()),
                ),
            ])));
        assert!(!summary.unwrap().contains("opaque secret token"));
    }

    #[test]
    fn legacy_activation_wrapper_extracts_only_the_activation_record() {
        let nested_record = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "Certificate".to_string(),
            plist::Value::String("secret".into()),
        )]));
        let wrapper = plist::Value::Dictionary(plist::Dictionary::from_iter([
            ("activation-record".to_string(), nested_record),
            (
                "ignored".to_string(),
                plist::Value::String("not sent".into()),
            ),
        ]));
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "iphone-activation".to_string(),
            wrapper,
        )]));
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &value).unwrap();
        let extracted = extract_legacy_activation_record(&bytes).unwrap();
        assert_eq!(
            extracted.as_dictionary().unwrap()["Certificate"].as_string(),
            Some("secret")
        );
        assert!(extract_legacy_activation_record(b"not a plist").is_none());
    }

    #[test]
    fn malformed_legacy_wrapper_is_distinguishable_from_modern_record() {
        let value = plist::Value::Dictionary(plist::Dictionary::from_iter([(
            "device-activation".to_string(),
            plist::Value::Dictionary(plist::Dictionary::new()),
        )]));
        let mut bytes = Vec::new();
        plist::to_writer_xml(&mut bytes, &value).unwrap();

        assert!(has_legacy_activation_wrapper(&bytes));
        assert!(extract_legacy_activation_record(&bytes).is_none());
        assert!(!has_legacy_activation_wrapper(b"not a plist"));
    }

    #[tokio::test]
    async fn handle_activation_info_omits_empty_headers_and_serializes_headers() {
        let response = plist::Value::Dictionary(plist::Dictionary::new());
        let mut stream = MockStream::with_response(response);
        let mut client = MobileActivationClient::new(&mut stream);
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".to_string(), "text/xml".to_string());
        client
            .handle_activation_info(b"<plist/>", &headers)
            .await
            .unwrap();
        let request = request_dict(&stream.written);
        assert_eq!(
            request["Command"].as_string(),
            Some("HandleActivationInfoWithSessionRequest")
        );
        assert_eq!(request["Value"].as_data(), Some(b"<plist/>".as_slice()));
        assert_eq!(
            request["ActivationResponseHeaders"]
                .as_dictionary()
                .unwrap()["Content-Type"]
                .as_string(),
            Some("text/xml")
        );

        let mut stream =
            MockStream::with_response(plist::Value::Dictionary(plist::Dictionary::new()));
        let mut client = MobileActivationClient::new(&mut stream);
        client
            .handle_activation_info(b"record", &BTreeMap::new())
            .await
            .unwrap();
        assert!(!request_dict(&stream.written).contains_key("ActivationResponseHeaders"));
    }

    #[test]
    fn endpoint_policy_requires_https_and_explicit_custom_opt_in() {
        assert!(ActivationHttpClient::with_endpoints(
            "http://albert.apple.com/deviceActivation",
            DRM_HANDSHAKE_URL,
            false,
            Duration::from_secs(1)
        )
        .is_err());
        assert!(ActivationHttpClient::with_endpoints(
            "https://example.invalid/deviceActivation",
            DRM_HANDSHAKE_URL,
            false,
            Duration::from_secs(1)
        )
        .is_err());
        assert!(ActivationHttpClient::with_endpoints(
            "https://albert.apple.com/not-the-official-endpoint",
            DRM_HANDSHAKE_URL,
            false,
            Duration::from_secs(1)
        )
        .is_err());
        assert!(ActivationHttpClient::with_endpoints(
            "https://example.invalid/deviceActivation",
            "https://example.invalid/drmHandshake",
            true,
            Duration::from_secs(1)
        )
        .is_ok());
    }

    #[test]
    fn form_encoding_is_ascii_safe_and_preserves_unicode_bytes() {
        let encoded = form_encode("a b/你好".as_bytes());
        assert_eq!(encoded, "a+b%2F%E4%BD%A0%E5%A5%BD");
    }

    #[test]
    fn response_header_names_are_canonicalized_for_mobileactivationd() {
        assert_eq!(canonical_header_name("content-type"), "Content-Type");
        assert_eq!(
            canonical_header_name("x-apple-session-id"),
            "X-Apple-Session-Id"
        );
    }

    #[test]
    fn http_response_debug_redacts_activation_bytes() {
        let response = ActivationHttpResponse {
            body: b"private activation record".to_vec(),
            headers: BTreeMap::new(),
            content_type: Some("text/xml".into()),
        };
        let rendered = format!("{response:?}");
        assert!(!rendered.contains("private activation record"));
        assert!(rendered.contains("redacted"));
    }

    #[test]
    fn content_type_matching_accepts_parameters_and_case_variants() {
        let response = ActivationHttpResponse {
            body: Vec::new(),
            headers: BTreeMap::new(),
            content_type: Some("Application/X-BuddyML; charset=utf-8".into()),
        };
        assert!(response.content_type_is("application/x-buddyml"));
    }

    struct PendingStream;

    impl AsyncRead for PendingStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for PendingStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn device_request_timeout_is_bounded() {
        let mut client =
            MobileActivationClient::with_timeout(PendingStream, Duration::from_millis(5));
        assert!(matches!(
            client.request_session_info().await,
            Err(MobileActivationError::Timeout(_))
        ));
    }
}

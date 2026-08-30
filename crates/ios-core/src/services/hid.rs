//! CoreDevice HID input over RemoteXPC.
//!
//! The wire format in this module follows pymobiledevice3's
//! `remote/core_device/hid_service.py`.  In particular, touchscreen reports
//! are the 58-byte single-contact report used by the universal HID service;
//! they do not contain a contact-id or pressure field.  The public state
//! machine therefore rejects concurrent contacts instead of silently putting
//! fields on the wire that CoreDevice does not understand.

use std::fmt;
use std::str::FromStr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use indexmap::IndexMap;

use crate::device::ResolvedServiceMetadata;
use crate::xpc::{XpcClient, XpcError, XpcMessage, XpcValue};

pub use crate::services::coredevice::CoreDeviceEnvelopeMode;

/// RSD service hosting Indigo button events.
pub const INDIGO_SERVICE_NAME: &str = "com.apple.coredevice.hid.indigo";
/// RSD service hosting universal HID reports.
pub const UNIVERSAL_SERVICE_NAME: &str = "com.apple.coredevice.hid.universalhidservice";
/// Feature used by Indigo button events.
pub const BUTTON_FEATURE: &str = "com.apple.coredevice.feature.remote.hid.button";
/// Feature used by universal HID reports and virtual services.
pub const UNIVERSAL_FEATURE: &str = "com.apple.coredevice.feature.remote.universalhidservice";

const BUTTON_MESSAGE_TYPE: &str = "IndigoButtonEvent";
const UNIVERSAL_MESSAGE_TYPE: &str = "Request";
const SIDE_CHANNEL_REPORT_LIMIT: usize = 4 * 1024;
/// HID reports are deliberately rate limited.  This is a protocol guard, not
/// a timing guarantee: callers should still use an absolute operation
/// deadline around a sequence of reports.
pub const MAX_TOUCH_REPORTS: usize = 256;
/// Bound keyboard input before any report is sent.
pub const MAX_TEXT_LENGTH: usize = 4096;
pub const DEFAULT_TOUCHSCREEN_SERVICE_ID: u64 = 257;
pub const DEFAULT_GESTURE_SERVICE_ID: u64 = 1281;
pub const DEFAULT_KEYBOARD_SERVICE_ID: u64 = 0x0001_0000_2001;
/// Upstream pmd3 names for the static HID surfaces.
pub const DIGITIZER_SURFACE_MAIN_TOUCHSCREEN: u64 = DEFAULT_TOUCHSCREEN_SERVICE_ID;
pub const DIGITIZER_SURFACE_TOUCHSCREEN_GESTURE: u64 = DEFAULT_GESTURE_SERVICE_ID;
pub const KEYBOARD_SURFACE_DEFAULT_SERVICE_ID: u64 = DEFAULT_KEYBOARD_SERVICE_ID;
pub const TOUCHSCREEN_STATE_CONTACT: u8 = 0xc2;
pub const TOUCHSCREEN_STATE_RELEASE: u8 = 0x02;
pub const HID_BUTTON_STATE_DOWN: u64 = 1;
pub const HID_BUTTON_STATE_UP: u64 = 2;
pub const HID_BUTTON_STATE_CANCELED: u64 = 3;
pub const DIGITIZER_REPORT_ID: u8 = 0x13;
pub const TOUCHSCREEN_REPORT_ID: u8 = 0x09;
pub const KEYBOARD_REPORT_ID: u8 = 0x01;

/// Errors returned by CoreDevice HID operations.
#[derive(Debug, thiserror::Error)]
pub enum HidError {
    #[error("xpc error: {0}")]
    Xpc(#[from] XpcError),
    #[error("HID protocol error: {0}")]
    Protocol(String),
    #[error("CoreDevice HID requires the modern envelope; legacy mode is unsupported")]
    LegacyUnsupported,
    #[error("CoreDevice HID does not support an RSD shim service: {0}")]
    ShimUnsupported(String),
    #[error("CoreDevice HID feature is not advertised by RSD: {0}")]
    FeatureMissing(&'static str),
}

/// Button transition encoded by `IndigoButtonEvent.payload.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ButtonState {
    Down = 1,
    Up = 2,
    Canceled = 3,
}

impl FromStr for ButtonState {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "down" | "press" => Ok(Self::Down),
            "up" | "release" => Ok(Self::Up),
            "canceled" | "cancel" => Ok(Self::Canceled),
            _ => Err(format!(
                "button state must be down, up, or canceled; got {value:?}"
            )),
        }
    }
}

impl fmt::Display for ButtonState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Down => "down",
            Self::Up => "up",
            Self::Canceled => "canceled",
        })
    }
}

/// Touch transition.  CoreDevice's report has only contact/release state;
/// move is consequently encoded with the same `0xc2` state as down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

impl TouchPhase {
    pub const fn wire_state(self) -> u8 {
        match self {
            Self::Down | Self::Move => TOUCHSCREEN_STATE_CONTACT,
            Self::Up | Self::Cancel => TOUCHSCREEN_STATE_RELEASE,
        }
    }
}

impl FromStr for TouchPhase {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "down" => Ok(Self::Down),
            "move" | "moved" => Ok(Self::Move),
            "up" | "release" => Ok(Self::Up),
            "cancel" | "canceled" => Ok(Self::Cancel),
            _ => Err(format!(
                "touch phase must be down, move, up, or cancel; got {value:?}"
            )),
        }
    }
}

/// A normalized screen coordinate.  The universal touchscreen report stores
/// each axis as an unsigned 16-bit value, so 0.0 and 1.0 map exactly to 0 and
/// 65535.  NaN, infinities, and values outside the display are rejected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchCoordinate {
    pub x: f64,
    pub y: f64,
}

impl TouchCoordinate {
    pub fn new(x: f64, y: f64) -> Result<Self, HidError> {
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(&x)
            || !(0.0..=1.0).contains(&y)
        {
            return Err(HidError::Protocol(
                "touch coordinates must be finite normalized values in 0.0..=1.0".into(),
            ));
        }
        Ok(Self { x, y })
    }

    fn raw(self) -> (u16, u16) {
        // The range check above makes this conversion lossless with respect to
        // the report's domain and prevents NaN-to-integer implementation traps.
        (
            (self.x * 65535.0).round() as u16,
            (self.y * 65535.0).round() as u16,
        )
    }
}

/// A checked HID usage code.  The keyboard bitmap has 240 usable bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyboardUsage(u8);

impl KeyboardUsage {
    pub const fn new(value: u8) -> Option<Self> {
        if value < 240 {
            Some(Self(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u8 {
        self.0
    }

    pub const fn from_value(value: u16) -> Option<Self> {
        if value < 240 {
            Some(Self(value as u8))
        } else {
            None
        }
    }
}

// Named usages are kept as constants for parity with pmd3 while the wrapper
// type prevents an out-of-range value from entering the 240-bit report.
macro_rules! keyboard_usage_constants {
    ($($name:ident = $value:expr),* $(,)?) => {
        $(pub const $name: KeyboardUsage = KeyboardUsage($value);)*
    };
}

keyboard_usage_constants! {
    KEY_A = 0x04, KEY_B = 0x05, KEY_C = 0x06, KEY_D = 0x07, KEY_E = 0x08,
    KEY_F = 0x09, KEY_G = 0x0a, KEY_H = 0x0b, KEY_I = 0x0c, KEY_J = 0x0d,
    KEY_K = 0x0e, KEY_L = 0x0f, KEY_M = 0x10, KEY_N = 0x11, KEY_O = 0x12,
    KEY_P = 0x13, KEY_Q = 0x14, KEY_R = 0x15, KEY_S = 0x16, KEY_T = 0x17,
    KEY_U = 0x18, KEY_V = 0x19, KEY_W = 0x1a, KEY_X = 0x1b, KEY_Y = 0x1c,
    KEY_Z = 0x1d, KEY_1 = 0x1e, KEY_2 = 0x1f, KEY_3 = 0x20, KEY_4 = 0x21,
    KEY_5 = 0x22, KEY_6 = 0x23, KEY_7 = 0x24, KEY_8 = 0x25, KEY_9 = 0x26,
    KEY_0 = 0x27, KEY_ENTER = 0x28, KEY_ESC = 0x29, KEY_BACKSPACE = 0x2a,
    KEY_TAB = 0x2b, KEY_SPACE = 0x2c, KEY_MINUS = 0x2d, KEY_EQUAL = 0x2e,
    KEY_LBRACKET = 0x2f, KEY_RBRACKET = 0x30, KEY_BACKSLASH = 0x31,
    KEY_SEMICOLON = 0x33, KEY_APOSTROPHE = 0x34, KEY_GRAVE = 0x35,
    KEY_COMMA = 0x36, KEY_DOT = 0x37, KEY_SLASH = 0x38, KEY_CAPS_LOCK = 0x39,
    KEY_F1 = 0x3a, KEY_F2 = 0x3b, KEY_F3 = 0x3c, KEY_F4 = 0x3d,
    KEY_F5 = 0x3e, KEY_F6 = 0x3f, KEY_F7 = 0x40, KEY_F8 = 0x41,
    KEY_F9 = 0x42, KEY_F10 = 0x43, KEY_F11 = 0x44, KEY_F12 = 0x45,
    KEY_RIGHT = 0x4f, KEY_LEFT = 0x50, KEY_DOWN = 0x51, KEY_UP = 0x52,
}

pub const KEY_LEFT_CTRL: KeyboardUsage = KeyboardUsage(0xe0);
pub const KEY_LEFT_SHIFT: KeyboardUsage = KeyboardUsage(0xe1);
pub const KEY_LEFT_ALT: KeyboardUsage = KeyboardUsage(0xe2);
pub const KEY_LEFT_GUI: KeyboardUsage = KeyboardUsage(0xe3);
pub const KEY_RIGHT_CTRL: KeyboardUsage = KeyboardUsage(0xe4);
pub const KEY_RIGHT_SHIFT: KeyboardUsage = KeyboardUsage(0xe5);
pub const KEY_RIGHT_ALT: KeyboardUsage = KeyboardUsage(0xe6);
pub const KEY_RIGHT_GUI: KeyboardUsage = KeyboardUsage(0xe7);

impl TryFrom<u16> for KeyboardUsage {
    type Error = HidError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::from_value(value).ok_or_else(|| {
            HidError::Protocol(format!(
                "keyboard usage {value} is outside the 0..=239 report bitmap"
            ))
        })
    }
}

impl FromStr for KeyboardUsage {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Ok(raw) = value.parse::<u16>() {
            return Self::try_from(raw).map_err(|error| error.to_string());
        }
        let name = value.to_ascii_lowercase();
        let usage = match name.as_str() {
            "enter" | "return" => KEY_ENTER,
            "esc" | "escape" => KEY_ESC,
            "backspace" => KEY_BACKSPACE,
            "tab" => KEY_TAB,
            "space" => KEY_SPACE,
            "caps-lock" | "capslock" => KEY_CAPS_LOCK,
            "left-ctrl" | "left-control" => KEY_LEFT_CTRL,
            "left-shift" => KEY_LEFT_SHIFT,
            "left-alt" | "left-option" => KEY_LEFT_ALT,
            "left-gui" | "left-command" => KEY_LEFT_GUI,
            "right-ctrl" | "right-control" => KEY_RIGHT_CTRL,
            "right-shift" => KEY_RIGHT_SHIFT,
            "right-alt" | "right-option" => KEY_RIGHT_ALT,
            "right-gui" | "right-command" => KEY_RIGHT_GUI,
            _ if value.chars().count() == 1 => ascii_usage(value.chars().next().unwrap())
                .map(|(usage, _)| usage)
                .ok_or_else(|| format!("unsupported keyboard usage {value:?}"))?,
            _ => return Err(format!("unknown keyboard usage {value:?}")),
        };
        Ok(usage)
    }
}

/// Modifier usages are ordinary HID usages, but this type prevents callers
/// from accidentally passing a modifier bitmask as a usage code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum KeyboardModifier {
    LeftControl = 0xe0,
    LeftShift = 0xe1,
    LeftAlt = 0xe2,
    LeftGui = 0xe3,
    RightControl = 0xe4,
    RightShift = 0xe5,
    RightAlt = 0xe6,
    RightGui = 0xe7,
}

impl KeyboardModifier {
    pub const fn usage(self) -> KeyboardUsage {
        KeyboardUsage(self as u8)
    }
}

/// Build the exact 39-byte universal keyboard report used by pmd3.
pub fn build_keyboard_report(usage_codes: &[KeyboardUsage], timestamp: Option<u64>) -> Bytes {
    let mut report = [0u8; 39];
    report[0] = KEYBOARD_REPORT_ID;
    for usage in usage_codes {
        let value = usage.value();
        report[1 + usize::from(value / 8)] |= 1 << (value % 8);
    }
    report[31..37].copy_from_slice(
        &(timestamp.unwrap_or_else(monotonic_timestamp) & ((1 << 48) - 1)).to_le_bytes()[..6],
    );
    Bytes::copy_from_slice(&report)
}

/// Build the exact 19-byte digitizer report used by pmd3.
pub fn build_digitizer_report(x: i32, y: i32, timestamp: Option<u64>) -> Bytes {
    let mut report = [0u8; 19];
    report[0] = DIGITIZER_REPORT_ID;
    report[1..5].copy_from_slice(&x.to_le_bytes());
    report[5..9].copy_from_slice(&y.to_le_bytes());
    report[11..17].copy_from_slice(
        &(timestamp.unwrap_or_else(monotonic_timestamp) & ((1 << 48) - 1)).to_le_bytes()[..6],
    );
    Bytes::copy_from_slice(&report)
}

/// Build the exact 58-byte single-contact touchscreen report used by pmd3.
pub fn build_touchscreen_report(
    phase: TouchPhase,
    x: u16,
    y: u16,
    timestamp: Option<u64>,
) -> Bytes {
    let mut report = [0u8; 58];
    report[0] = TOUCHSCREEN_REPORT_ID;
    report[1] = 0x01;
    report[2] = 0x05;
    report[3] = phase.wire_state();
    report[4..6].copy_from_slice(&x.to_le_bytes());
    report[6..8].copy_from_slice(&y.to_le_bytes());
    report[40] = 0x02;
    report[44..50].copy_from_slice(
        &(timestamp.unwrap_or_else(monotonic_timestamp) & ((1 << 48) - 1)).to_le_bytes()[..6],
    );
    Bytes::copy_from_slice(&report)
}

fn monotonic_timestamp() -> u64 {
    // HID timestamps are only compared by the device, and pmd3 uses a
    // monotonic nanosecond clock truncated to 48 bits.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

fn dict(entries: impl IntoIterator<Item = (&'static str, XpcValue)>) -> XpcValue {
    XpcValue::Dictionary(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn ensure_common(
    mode: CoreDeviceEnvelopeMode,
    features: Option<&[String]>,
    required: &'static str,
) -> Result<(), HidError> {
    if mode == CoreDeviceEnvelopeMode::Legacy {
        return Err(HidError::LegacyUnsupported);
    }
    if features.is_some_and(|items| !items.is_empty() && !items.iter().any(|item| item == required))
    {
        return Err(HidError::FeatureMissing(required));
    }
    Ok(())
}

fn ensure_canonical(metadata: &ResolvedServiceMetadata) -> Result<(), HidError> {
    if metadata.resolved_service_name.ends_with(".shim.remote") {
        return Err(HidError::ShimUnsupported(
            metadata.resolved_service_name.clone(),
        ));
    }
    Ok(())
}

/// Indigo button service client.
pub struct IndigoHidServiceClient {
    client: XpcClient,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

pub type IndigoHIDService = IndigoHidServiceClient;

impl IndigoHidServiceClient {
    pub fn new(client: XpcClient) -> Self {
        Self::new_with_mode(client, CoreDeviceEnvelopeMode::Modern)
    }

    pub fn new_with_features<I, S>(client: XpcClient, features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            client,
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: Some(features.into_iter().map(Into::into).collect()),
        }
    }

    pub fn new_with_mode(client: XpcClient, envelope_mode: CoreDeviceEnvelopeMode) -> Self {
        Self {
            client,
            envelope_mode,
            service_features: None,
        }
    }

    pub fn from_resolved_metadata(
        client: XpcClient,
        metadata: &ResolvedServiceMetadata,
    ) -> Result<Self, HidError> {
        ensure_canonical(metadata)?;
        Ok(Self::new_with_features(client, metadata.features.clone()))
    }

    pub fn supports_buttons(&self) -> bool {
        self.service_features.as_deref().map_or(true, |items| {
            items.is_empty() || items.iter().any(|item| item == BUTTON_FEATURE)
        })
    }

    pub async fn send_button(
        &mut self,
        usage_page: u16,
        usage_code: u16,
        state: ButtonState,
    ) -> Result<(), HidError> {
        ensure_common(
            self.envelope_mode,
            self.service_features.as_deref(),
            BUTTON_FEATURE,
        )?;
        let request = build_button_request(usage_page, usage_code, state);
        // pmd3 deliberately uses send_request here: Indigo button events are
        // one-way and waiting for a synthetic reply can deadlock older daemons.
        self.client.send(request).await?;
        Ok(())
    }
}

/// Universal HID service client.
pub struct UniversalHidServiceClient {
    client: XpcClient,
    envelope_mode: CoreDeviceEnvelopeMode,
    service_features: Option<Vec<String>>,
}

pub type UniversalHIDServiceService = UniversalHidServiceClient;

impl UniversalHidServiceClient {
    pub fn new(client: XpcClient) -> Self {
        Self::new_with_mode(client, CoreDeviceEnvelopeMode::Modern)
    }

    pub fn new_with_features<I, S>(client: XpcClient, features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            client,
            envelope_mode: CoreDeviceEnvelopeMode::Modern,
            service_features: Some(features.into_iter().map(Into::into).collect()),
        }
    }

    pub fn new_with_mode(client: XpcClient, envelope_mode: CoreDeviceEnvelopeMode) -> Self {
        Self {
            client,
            envelope_mode,
            service_features: None,
        }
    }

    pub fn from_resolved_metadata(
        client: XpcClient,
        metadata: &ResolvedServiceMetadata,
    ) -> Result<Self, HidError> {
        ensure_canonical(metadata)?;
        Ok(Self::new_with_features(client, metadata.features.clone()))
    }

    pub fn supports_universal_hid(&self) -> bool {
        self.service_features.as_deref().map_or(true, |items| {
            items.is_empty() || items.iter().any(|item| item == UNIVERSAL_FEATURE)
        })
    }

    pub async fn list_services(&mut self) -> Result<XpcValue, HidError> {
        ensure_common(
            self.envelope_mode,
            self.service_features.as_deref(),
            UNIVERSAL_FEATURE,
        )?;
        let response = self
            .client
            .call(universal_request(dict([(
                "connectedServices",
                XpcValue::Dictionary(IndexMap::new()),
            )])))
            .await?;
        parse_direct_response(response)
    }

    /// Upstream-compatible spelling for [`Self::list_services`].
    pub async fn list_connected_services(&mut self) -> Result<XpcValue, HidError> {
        self.list_services().await
    }

    pub async fn send_report(&mut self, service_id: u64, report: Bytes) -> Result<(), HidError> {
        ensure_common(
            self.envelope_mode,
            self.service_features.as_deref(),
            UNIVERSAL_FEATURE,
        )?;
        if report.is_empty() || report.len() > SIDE_CHANNEL_REPORT_LIMIT {
            return Err(HidError::Protocol(format!(
                "HID report length {} is outside 1..={SIDE_CHANNEL_REPORT_LIMIT}",
                report.len()
            )));
        }
        self.client
            .send(build_send_report_request(service_id, report))
            .await?;
        Ok(())
    }

    pub async fn send_digitizer(
        &mut self,
        x: i32,
        y: i32,
        service_id: u64,
        timestamp: Option<u64>,
    ) -> Result<(), HidError> {
        self.send_report(service_id, build_digitizer_report(x, y, timestamp))
            .await
    }

    pub async fn send_touchscreen(
        &mut self,
        phase: TouchPhase,
        coordinate: TouchCoordinate,
        service_id: u64,
        timestamp: Option<u64>,
    ) -> Result<(), HidError> {
        let (x, y) = coordinate.raw();
        self.send_report(service_id, build_touchscreen_report(phase, x, y, timestamp))
            .await
    }

    pub async fn send_touchscreen_raw(
        &mut self,
        phase: TouchPhase,
        x: u16,
        y: u16,
        service_id: u64,
        timestamp: Option<u64>,
    ) -> Result<(), HidError> {
        self.send_report(service_id, build_touchscreen_report(phase, x, y, timestamp))
            .await
    }

    pub async fn create_keyboard_service(
        &mut self,
        service_id: u64,
        product: &str,
        manufacturer: &str,
        vendor_id: i64,
        product_id: i64,
    ) -> Result<u64, HidError> {
        ensure_common(
            self.envelope_mode,
            self.service_features.as_deref(),
            UNIVERSAL_FEATURE,
        )?;
        if product.is_empty() || manufacturer.is_empty() || vendor_id < 0 || product_id < 0 {
            return Err(HidError::Protocol(
                "keyboard product/manufacturer must be non-empty and IDs non-negative".into(),
            ));
        }
        let service =
            keyboard_service_dictionary(service_id, product, manufacturer, vendor_id, product_id);
        let response = self
            .client
            .call(build_create_keyboard_request(service))
            .await?;
        let output = parse_direct_response(response)?;
        let value = output
            .as_dict()
            .and_then(|items| items.get("serviceID"))
            .and_then(XpcValue::as_uint64);
        Ok(value.unwrap_or(service_id))
    }

    pub async fn send_keyboard(
        &mut self,
        service_id: u64,
        usage_codes: &[KeyboardUsage],
        timestamp: Option<u64>,
    ) -> Result<(), HidError> {
        self.send_report(service_id, build_keyboard_report(usage_codes, timestamp))
            .await
    }

    pub fn keyboard_session(&mut self, service_id: u64) -> KeyboardSession<'_> {
        KeyboardSession {
            service: self,
            service_id,
            pressed: Vec::new(),
            closed: false,
        }
    }

    pub fn touch_session(&mut self, service_id: u64) -> TouchSession<'_> {
        TouchSession {
            service: self,
            service_id,
            active_contact: None,
            last_coordinate: None,
            report_count: 0,
            closed: false,
        }
    }
}

fn universal_request(payload: XpcValue) -> XpcValue {
    dict([
        (
            "featureIdentifier",
            XpcValue::String(UNIVERSAL_FEATURE.into()),
        ),
        (
            "messageType",
            XpcValue::String(UNIVERSAL_MESSAGE_TYPE.into()),
        ),
        ("payload", payload),
    ])
}

fn build_button_request(usage_page: u16, usage_code: u16, state: ButtonState) -> XpcValue {
    dict([
        ("messageType", XpcValue::String(BUTTON_MESSAGE_TYPE.into())),
        (
            "payload",
            dict([
                ("state", XpcValue::Uint64(state as u64)),
                ("usagePage", XpcValue::Uint64(u64::from(usage_page))),
                ("usageCode", XpcValue::Uint64(u64::from(usage_code))),
            ]),
        ),
        ("featureIdentifier", XpcValue::String(BUTTON_FEATURE.into())),
    ])
}

fn build_send_report_request(service_id: u64, report: Bytes) -> XpcValue {
    universal_request(dict([(
        "send",
        dict([
            ("_0", XpcValue::Data(report)),
            ("_1", XpcValue::Uint64(service_id)),
        ]),
    )]))
}

fn build_create_keyboard_request(service: XpcValue) -> XpcValue {
    universal_request(dict([("createService", dict([("_0", service)]))]))
}

fn parse_direct_response(response: XpcMessage) -> Result<XpcValue, HidError> {
    let body = response
        .body
        .ok_or_else(|| HidError::Protocol("HID response is missing a body".into()))?;
    let Some(items) = body.as_dict() else {
        return Err(HidError::Protocol(
            "HID response body is not a dictionary".into(),
        ));
    };
    for key in ["error", "Error", "receivedError"] {
        if let Some(value) = items.get(key) {
            return Err(HidError::Protocol(format!(
                "HID service error: {}",
                bounded_value(value, 512)
            )));
        }
    }
    Ok(body)
}

fn bounded_value(value: &XpcValue, limit: usize) -> String {
    let text = format!("{value:?}");
    if text.len() > limit {
        format!("{}…", &text[..limit.saturating_sub(3)])
    } else {
        text
    }
}

fn keyboard_service_dictionary(
    service_id: u64,
    product: &str,
    manufacturer: &str,
    vendor_id: i64,
    product_id: i64,
) -> XpcValue {
    let pair = dict([
        ("DeviceUsage", XpcValue::Int64(6)),
        ("DeviceUsagePage", XpcValue::Int64(1)),
    ]);
    let storage = dict([
        (
            "Manufacturer",
            dict([("string", XpcValue::String(manufacturer.into()))]),
        ),
        (
            "Product",
            dict([("string", XpcValue::String(product.into()))]),
        ),
        ("ProductID", dict([("int", XpcValue::Int64(product_id))])),
        ("VendorID", dict([("int", XpcValue::Int64(vendor_id))])),
        ("PrimaryUsage", dict([("int", XpcValue::Int64(6))])),
        ("PrimaryUsagePage", dict([("int", XpcValue::Int64(1))])),
        (
            "DeviceUsagePairs",
            XpcValue::Array(vec![dict([("dictionary", pair)])]),
        ),
        (
            "Transport",
            dict([("string", XpcValue::String("USB".into()))]),
        ),
        (
            "ReportDescriptor",
            dict([("data", XpcValue::Data(KEYBOARD_REPORT_DESCRIPTOR.into()))]),
        ),
        (
            "UniversalControlVirtualService",
            dict([("bool", XpcValue::Bool(true))]),
        ),
        ("_ServiceID", dict([("uint", XpcValue::Uint64(service_id))])),
    ]);
    dict([
        (
            "DeviceUsagePairs",
            XpcValue::Array(vec![dict([
                ("DeviceUsage", XpcValue::Int64(6)),
                ("DeviceUsagePage", XpcValue::Int64(1)),
            ])]),
        ),
        ("PrimaryUsage", XpcValue::Uint64(6)),
        ("PrimaryUsagePage", XpcValue::Uint64(1)),
        ("Product", XpcValue::String(product.into())),
        ("ProductID", XpcValue::Int64(product_id)),
        ("VendorID", XpcValue::Int64(vendor_id)),
        ("_CoreDevice_codablePropertyStorage", storage),
        ("_ServiceID", XpcValue::Uint64(service_id)),
    ])
}

const KEYBOARD_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00, 0x25, 0x01,
    0x95, 0x08, 0x75, 0x01, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x05, 0x07, 0x19, 0x00,
    0x29, 0xff, 0x15, 0x00, 0x26, 0xff, 0x00, 0x95, 0x06, 0x75, 0x08, 0x81, 0x00, 0x05, 0x08, 0x19,
    0x01, 0x29, 0x05, 0x15, 0x00, 0x25, 0x01, 0x95, 0x05, 0x75, 0x01, 0x91, 0x02, 0x95, 0x01, 0x75,
    0x03, 0x91, 0x01, 0xc0,
];

/// Checked contact lifecycle independent of XPC, useful for deterministic
/// tests and embedders that batch their own report transport.
#[derive(Debug, Clone, Default)]
pub struct TouchState {
    active_contact: Option<u32>,
}

impl TouchState {
    pub fn active_contact(&self) -> Option<u32> {
        self.active_contact
    }

    pub fn transition(&mut self, contact_id: u32, phase: TouchPhase) -> Result<(), HidError> {
        match (self.active_contact, phase) {
            (None, TouchPhase::Down) => self.active_contact = Some(contact_id),
            (Some(_), TouchPhase::Down) => {
                return Err(HidError::Protocol(
                    "a second touch contact cannot be represented by the single-contact report"
                        .into(),
                ))
            }
            (Some(active), TouchPhase::Move | TouchPhase::Up | TouchPhase::Cancel)
                if active == contact_id =>
            {
                if matches!(phase, TouchPhase::Up | TouchPhase::Cancel) {
                    self.active_contact = None;
                }
            }
            (None, TouchPhase::Move | TouchPhase::Up | TouchPhase::Cancel) => {
                return Err(HidError::Protocol(format!(
                    "touch contact {contact_id} is not active"
                )))
            }
            (Some(active), _) => {
                return Err(HidError::Protocol(format!(
                    "touch contact {contact_id} does not match active contact {active}"
                )))
            }
        }
        Ok(())
    }
}

/// Borrowed RAII touch session.  `Drop` only marks the session closed; it does
/// not perform async I/O. Call `close` when a best-effort release is required.
pub struct TouchSession<'a> {
    service: &'a mut UniversalHidServiceClient,
    service_id: u64,
    active_contact: Option<u32>,
    last_coordinate: Option<TouchCoordinate>,
    report_count: usize,
    closed: bool,
}

impl TouchSession<'_> {
    pub async fn touch(
        &mut self,
        contact_id: u32,
        phase: TouchPhase,
        coordinate: TouchCoordinate,
    ) -> Result<(), HidError> {
        if self.closed {
            return Err(HidError::Protocol("touch session is closed".into()));
        }
        if self.report_count >= MAX_TOUCH_REPORTS {
            return Err(HidError::Protocol(format!(
                "touch report limit {MAX_TOUCH_REPORTS} exceeded"
            )));
        }
        let mut state = TouchState {
            active_contact: self.active_contact,
        };
        state.transition(contact_id, phase)?;
        self.service
            .send_touchscreen(phase, coordinate, self.service_id, None)
            .await?;
        self.active_contact = state.active_contact;
        self.last_coordinate = Some(coordinate);
        self.report_count += 1;
        Ok(())
    }

    pub async fn close(&mut self, timeout: Duration) -> Result<(), HidError> {
        if self.closed {
            return Ok(());
        }
        let Some(contact_id) = self.active_contact else {
            self.closed = true;
            return Ok(());
        };
        let coordinate = self
            .last_coordinate
            .unwrap_or(TouchCoordinate { x: 0.0, y: 0.0 });
        let operation =
            self.service
                .send_touchscreen(TouchPhase::Cancel, coordinate, self.service_id, None);
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| HidError::Protocol("touch session close timed out".into()))??;
        self.active_contact = None;
        self.last_coordinate = None;
        let _ = contact_id;
        self.closed = true;
        Ok(())
    }
}

impl Drop for TouchSession<'_> {
    fn drop(&mut self) {
        self.closed = true;
    }
}

/// Borrowed keyboard session.  Its destructor never sends a report; explicit
/// `close` releases held keys under the caller's deadline.
pub struct KeyboardSession<'a> {
    service: &'a mut UniversalHidServiceClient,
    service_id: u64,
    pressed: Vec<KeyboardUsage>,
    closed: bool,
}

impl KeyboardSession<'_> {
    pub async fn send_key(
        &mut self,
        usage: KeyboardUsage,
        modifiers: &[KeyboardModifier],
    ) -> Result<(), HidError> {
        if self.closed {
            return Err(HidError::Protocol("keyboard session is closed".into()));
        }
        let mut usages = modifiers
            .iter()
            .map(|modifier| modifier.usage())
            .collect::<Vec<_>>();
        usages.push(usage);
        self.service
            .send_keyboard(self.service_id, &usages, None)
            .await?;
        self.pressed = usages;
        self.service
            .send_keyboard(self.service_id, &[], None)
            .await?;
        self.pressed.clear();
        Ok(())
    }

    pub async fn type_text(&mut self, text: &str) -> Result<usize, HidError> {
        if text.len() > MAX_TEXT_LENGTH {
            return Err(HidError::Protocol(format!(
                "keyboard text exceeds {MAX_TEXT_LENGTH} bytes"
            )));
        }
        let mut count = 0;
        for character in text.chars() {
            let (usage, shifted) = ascii_usage(character).ok_or_else(|| {
                HidError::Protocol(format!(
                    "unsupported keyboard character U+{:04X}",
                    character as u32
                ))
            })?;
            let modifiers = if shifted {
                &[KeyboardModifier::LeftShift][..]
            } else {
                &[]
            };
            self.send_key(usage, modifiers).await?;
            count += 1;
        }
        Ok(count)
    }

    pub async fn close(&mut self, timeout: Duration) -> Result<(), HidError> {
        if self.closed {
            return Ok(());
        }
        let operation = self.service.send_keyboard(self.service_id, &[], None);
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| HidError::Protocol("keyboard session close timed out".into()))??;
        self.pressed.clear();
        self.closed = true;
        Ok(())
    }
}

impl Drop for KeyboardSession<'_> {
    fn drop(&mut self) {
        self.closed = true;
    }
}

fn ascii_usage(character: char) -> Option<(KeyboardUsage, bool)> {
    let lower = character.to_ascii_lowercase();
    let usage = match lower {
        'a'..='z' => KeyboardUsage::new(0x04 + (lower as u8 - b'a'))?,
        '1'..='9' => KeyboardUsage::new(0x1e + (lower as u8 - b'1'))?,
        '0' => KeyboardUsage::new(0x27)?,
        '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '(' | ')' => {
            let unshifted = match character {
                '!' => '1',
                '@' => '2',
                '#' => '3',
                '$' => '4',
                '%' => '5',
                '^' => '6',
                '&' => '7',
                '*' => '8',
                '(' => '9',
                ')' => '0',
                _ => unreachable!(),
            };
            KeyboardUsage::new(if unshifted == '0' {
                0x27
            } else {
                0x1e + (unshifted as u8 - b'1')
            })?
        }
        '\n' => KeyboardUsage::new(0x28)?,
        '\t' => KeyboardUsage::new(0x2b)?,
        ' ' => KeyboardUsage::new(0x2c)?,
        '-' | '_' => KeyboardUsage::new(0x2d)?,
        '=' | '+' => KeyboardUsage::new(0x2e)?,
        '[' | '{' => KeyboardUsage::new(0x2f)?,
        ']' | '}' => KeyboardUsage::new(0x30)?,
        '\\' | '|' => KeyboardUsage::new(0x31)?,
        ';' | ':' => KeyboardUsage::new(0x33)?,
        '\'' | '"' => KeyboardUsage::new(0x34)?,
        '`' | '~' => KeyboardUsage::new(0x35)?,
        ',' | '<' => KeyboardUsage::new(0x36)?,
        '.' | '>' => KeyboardUsage::new(0x37)?,
        '/' | '?' => KeyboardUsage::new(0x38)?,
        _ => return None,
    };
    Some((
        usage,
        character.is_ascii_uppercase()
            || matches!(
                character,
                '!' | '@' | '#' | '$' | '%' | '^' | '&' | '*' | '(' | ')'
            )
            || matches!(
                character,
                '_' | '+' | '{' | '}' | '|' | ':' | '"' | '~' | '<' | '>' | '?'
            ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_match_upstream_sizes_and_offsets() {
        assert_eq!(
            build_digitizer_report(0x0102_0304, -2, Some(0x0102_0304_0506)),
            Bytes::from_static(&[
                0x13, 4, 3, 2, 1, 0xfe, 0xff, 0xff, 0xff, 0, 0, 6, 5, 4, 3, 2, 1, 0, 0,
            ])
        );
        let keyboard = build_keyboard_report(
            &[
                KeyboardUsage::new(0).unwrap(),
                KeyboardUsage::new(0xe1).unwrap(),
            ],
            Some(0x0102_0304_0506),
        );
        assert_eq!(keyboard.len(), 39);
        assert_eq!(keyboard[0], 1);
        assert_eq!(keyboard[1], 1);
        assert_eq!(keyboard[1 + 0xe1 / 8] & (1 << (0xe1 % 8)), 1 << (0xe1 % 8));
        assert_eq!(&keyboard[31..37], &[6, 5, 4, 3, 2, 1]);
        let touch =
            build_touchscreen_report(TouchPhase::Down, 0x1234, 0xabcd, Some(0x0102_0304_0506));
        assert_eq!(touch.len(), 58);
        assert_eq!(&touch[..8], &[9, 1, 5, 0xc2, 0x34, 0x12, 0xcd, 0xab]);
        assert_eq!(&touch[40..50], &[2, 0, 0, 0, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn normalized_coordinates_and_state_are_checked() {
        assert_eq!(TouchCoordinate::new(0.0, 1.0).unwrap().raw(), (0, 65535));
        assert!(TouchCoordinate::new(f64::NAN, 0.0).is_err());
        assert!(TouchCoordinate::new(1.1, 0.0).is_err());
        let mut state = TouchState::default();
        state.transition(7, TouchPhase::Down).unwrap();
        assert!(state.transition(8, TouchPhase::Move).is_err());
        assert!(state.transition(8, TouchPhase::Down).is_err());
        state.transition(7, TouchPhase::Up).unwrap();
        assert_eq!(state.active_contact(), None);
    }

    #[test]
    fn ascii_mapping_uses_shift_without_echoing_text() {
        assert!(!ascii_usage('a').unwrap().1);
        assert!(ascii_usage('A').unwrap().1);
        assert_eq!(ascii_usage('?').unwrap().0.value(), 0x38);
        assert!(ascii_usage('é').is_none());
    }

    #[test]
    fn keyboard_service_shape_has_exact_nested_storage() {
        let XpcValue::Dictionary(service) = keyboard_service_dictionary(9, "p", "m", 1, 2) else {
            panic!()
        };
        assert_eq!(service["_ServiceID"], XpcValue::Uint64(9));
        let storage = service["_CoreDevice_codablePropertyStorage"]
            .as_dict()
            .unwrap();
        assert_eq!(
            storage["Transport"],
            dict([("string", XpcValue::String("USB".into()))])
        );
        assert_eq!(
            storage["UniversalControlVirtualService"],
            dict([("bool", XpcValue::Bool(true))])
        );
        let XpcValue::Data(descriptor) =
            storage["ReportDescriptor"].as_dict().unwrap()["data"].clone()
        else {
            panic!()
        };
        assert_eq!(descriptor.as_ref(), KEYBOARD_REPORT_DESCRIPTOR);
    }

    #[test]
    fn feature_and_route_guards_are_fail_closed() {
        assert!(matches!(
            ensure_common(
                CoreDeviceEnvelopeMode::Legacy,
                Some(&[BUTTON_FEATURE.to_string()]),
                BUTTON_FEATURE
            ),
            Err(HidError::LegacyUnsupported)
        ));
        assert!(matches!(
            ensure_common(
                CoreDeviceEnvelopeMode::Modern,
                Some(&["other.feature".to_string()]),
                BUTTON_FEATURE
            ),
            Err(HidError::FeatureMissing(BUTTON_FEATURE))
        ));
        let metadata = ResolvedServiceMetadata {
            resolved_service_name: format!("{UNIVERSAL_SERVICE_NAME}.shim.remote"),
            features: vec![],
        };
        assert!(matches!(
            ensure_canonical(&metadata),
            Err(HidError::ShimUnsupported(_))
        ));
    }

    #[test]
    fn direct_response_errors_are_bounded_and_unknown_shapes_rejected() {
        let response = XpcMessage {
            flags: 0,
            msg_id: 0,
            body: Some(dict([(
                "receivedError",
                XpcValue::String("bad".repeat(1000)),
            )])),
        };
        let Err(HidError::Protocol(message)) = parse_direct_response(response) else {
            panic!("expected HID service error")
        };
        assert!(message.len() <= 540);
        assert!(parse_direct_response(XpcMessage {
            flags: 0,
            msg_id: 0,
            body: None
        })
        .is_err());
        assert!(parse_direct_response(XpcMessage {
            flags: 0,
            msg_id: 0,
            body: Some(XpcValue::Null)
        })
        .is_err());
    }

    #[test]
    fn request_dictionaries_match_pmd3_wire_contract() {
        let button = build_button_request(0x0c, 0x40, ButtonState::Down);
        let button = button.as_dict().unwrap();
        assert_eq!(button["messageType"].as_str(), Some(BUTTON_MESSAGE_TYPE));
        assert_eq!(button["featureIdentifier"].as_str(), Some(BUTTON_FEATURE));
        let payload = button["payload"].as_dict().unwrap();
        assert_eq!(payload["state"], XpcValue::Uint64(1));
        assert_eq!(payload["usagePage"], XpcValue::Uint64(0x0c));
        assert_eq!(payload["usageCode"], XpcValue::Uint64(0x40));

        let send = build_send_report_request(257, Bytes::from_static(&[9, 1, 2]));
        let send = send.as_dict().unwrap();
        assert_eq!(send["messageType"].as_str(), Some(UNIVERSAL_MESSAGE_TYPE));
        assert_eq!(send["featureIdentifier"].as_str(), Some(UNIVERSAL_FEATURE));
        let send = send["payload"].as_dict().unwrap()["send"]
            .as_dict()
            .unwrap();
        assert_eq!(send["_1"], XpcValue::Uint64(257));
        assert_eq!(send["_0"], XpcValue::Data(Bytes::from_static(&[9, 1, 2])));

        let list = universal_request(dict([(
            "connectedServices",
            XpcValue::Dictionary(IndexMap::new()),
        )]));
        assert_eq!(
            list.as_dict().unwrap()["payload"].as_dict().unwrap()["connectedServices"],
            XpcValue::Dictionary(IndexMap::new())
        );
    }
}

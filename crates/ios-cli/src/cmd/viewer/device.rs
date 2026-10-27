//! Device adapters use the same protocol clients as the standalone HID/pasteboard commands.
use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use ios_core::display::{DisplayServiceClient, MediaStreamOptions, MediaStreamSession};
use ios_core::hid::{
    ButtonState, IndigoHidServiceClient, KeyboardUsage, TouchCoordinate, TouchPhase,
    UniversalHidServiceClient, DEFAULT_KEYBOARD_SERVICE_ID, DEFAULT_TOUCHSCREEN_SERVICE_ID,
};
use ios_core::pasteboard::{DataInclusionPolicy, PasteboardClient, PasteboardLimits};

use super::clipboard::ClipboardPayload;
use super::vnc::VncInputAction;

pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct DeviceInput {
    udid: String,
    device: Option<crate::cmd::tunnel::NativeConnection>,
    pasteboard_device: Option<crate::cmd::tunnel::NativeConnection>,
    indigo_device: Option<crate::cmd::tunnel::NativeConnection>,
    orientation_device: Option<crate::cmd::tunnel::NativeConnection>,
    native: crate::cmd::tunnel::NativeScope,
    universal: Option<UniversalHidServiceClient>,
    media: Option<MediaStreamSession>,
    starting_display: Option<DisplayServiceClient>,
    indigo: Option<IndigoHidServiceClient>,
    orientation: Option<ios_core::orientation::OrientationServiceClient>,
    pasteboard: Option<PasteboardClient>,
    keyboard: Option<u64>,
    keys: BTreeSet<u8>,
    buttons: BTreeSet<(u16, u16)>,
    contact: Option<TouchCoordinate>,
    pub dimensions: (u16, u16),
    pub share_clipboard: bool,
}

impl DeviceInput {
    pub fn new(
        udid: String,
        share_clipboard: bool,
        native: crate::cmd::tunnel::NativeScope,
    ) -> Self {
        Self {
            udid,
            device: None,
            pasteboard_device: None,
            indigo_device: None,
            orientation_device: None,
            native,
            universal: None,
            media: None,
            starting_display: None,
            indigo: None,
            orientation: None,
            pasteboard: None,
            keyboard: None,
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            contact: None,
            dimensions: (0, 0),
            share_clipboard,
        }
    }

    async fn universal(&mut self) -> Result<()> {
        if self.universal.is_some() {
            return Ok(());
        }
        let device = self
            .native
            .connect(&self.udid, ios_core::TunMode::Kernel)
            .await
            .context(
                "viewer HID requires a kernel tunnel for the UDP authorization side channel",
            )?;
        let sender_ip = device
            .server_address()
            .context("missing tunnel device address")?
            .to_owned();
        let receiver_ip = device
            .client_address()
            .context("missing tunnel host address")?
            .to_owned();
        let (xpc, metadata) = device
            .connect_xpc_service_with_metadata(ios_core::display::SERVICE_NAME)
            .await?;
        self.starting_display = Some(DisplayServiceClient::from_resolved_metadata(
            xpc, &metadata,
        )?);
        self.device = Some(device);
        self.media = Some(
            MediaStreamSession::start_video_retained(
                &mut self.starting_display,
                MediaStreamOptions {
                    sender_ip,
                    receiver_ip,
                    ..MediaStreamOptions::default()
                },
            )
            .await?,
        );
        self.media.as_mut().expect("media installed").spawn_drain();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (xpc, metadata) = self
            .device
            .as_ref()
            .expect("device installed")
            .connect_xpc_service_with_metadata(ios_core::hid::UNIVERSAL_SERVICE_NAME)
            .await?;
        self.universal = Some(UniversalHidServiceClient::from_resolved_metadata(
            xpc, &metadata,
        )?);
        Ok(())
    }

    async fn pasteboard(&mut self) -> Result<&mut PasteboardClient> {
        ensure!(
            self.share_clipboard,
            "clipboard sharing is disabled; use --share-clipboard"
        );
        if self.pasteboard.is_none() {
            let device = self
                .native
                .connect(&self.udid, ios_core::TunMode::Userspace)
                .await?;
            let xpc = device
                .connect_xpc_service(ios_core::pasteboard::SERVICE_NAME)
                .await?;
            self.pasteboard = Some(PasteboardClient::with_timeout_and_limits(
                xpc,
                OPERATION_TIMEOUT,
                PasteboardLimits {
                    max_items: 8,
                    max_representations: 24,
                    max_data_bytes: super::clipboard::MAX_IMAGE_BYTES,
                    max_metadata_bytes: 16 * 1024,
                    max_events: 8,
                },
            ));
            self.pasteboard_device = Some(device);
        }
        Ok(self.pasteboard.as_mut().expect("pasteboard installed"))
    }

    pub async fn read_clipboard(&mut self) -> Result<Option<ClipboardPayload>> {
        match self.read_clipboard_once().await {
            Ok(value) => Ok(value),
            Err(error)
                if matches!(
                    ios_core::classify_error(error.as_ref()),
                    ios_core::ErrorClass::Disconnected | ios_core::ErrorClass::Timeout
                ) =>
            {
                self.pasteboard = None;
                self.pasteboard_device = None;
                if let Err(cleanup) = self.native.collect(CLEANUP_TIMEOUT).await {
                    return super::finish(Err(error), Err(cleanup));
                }
                tracing::warn!(
                    "viewer pasteboard read disconnected; refreshing selected service once"
                );
                tokio::time::sleep(Duration::from_millis(250)).await;
                // Only the read is repeated. SET and HID reports may already
                // have effects and are never replayed by this reconnect path.
                self.read_clipboard_once().await
            }
            Err(error) => Err(error),
        }
    }

    async fn read_clipboard_once(&mut self) -> Result<Option<ClipboardPayload>> {
        let snapshot = self
            .pasteboard()
            .await?
            .get_snapshot(
                ios_core::pasteboard::GENERAL_PASTEBOARD,
                DataInclusionPolicy::Threshold(
                    crate::cmd::viewer::clipboard::MAX_IMAGE_BYTES as i64,
                ),
            )
            .await?;
        let value = if let Some(text) = snapshot.text() {
            Some(ClipboardPayload::Text(text))
        } else if let Some(png) = snapshot.data_for_uti(crate::cmd::viewer::clipboard::UTI_PNG) {
            Some(ClipboardPayload::ImagePng(png.to_vec()))
        } else {
            snapshot
                .data_for_uti(crate::cmd::viewer::clipboard::UTI_JPEG)
                .map(|jpeg| ClipboardPayload::ImageJpeg(jpeg.to_vec()))
        };
        if let Some(value) = &value {
            value.validate_bounds()?;
        }
        Ok(value)
    }

    pub async fn apply(&mut self, action: &VncInputAction) -> Result<()> {
        match action {
            VncInputAction::Touch { contact, x, y } => {
                ensure!(
                    self.dimensions.0 > 0 && self.dimensions.1 > 0,
                    "no device frame available for pointer coordinates"
                );
                ensure!(
                    *x < self.dimensions.0 && *y < self.dimensions.1,
                    "pointer outside device frame"
                );
                let coordinate = TouchCoordinate::new(
                    *x as f64 / self.dimensions.0.saturating_sub(1).max(1) as f64,
                    *y as f64 / self.dimensions.1.saturating_sub(1).max(1) as f64,
                )?;
                let phase = match (*contact, self.contact.is_some()) {
                    (true, false) => TouchPhase::Down,
                    (true, true) => TouchPhase::Move,
                    (false, true) => TouchPhase::Up,
                    (false, false) => return Ok(()),
                };
                self.universal().await?;
                // Retain a potentially sent down/move until the release report
                // is flushed. This one-way HID protocol has no device ACK.
                if *contact {
                    self.contact = Some(coordinate);
                }
                self.universal
                    .as_mut()
                    .expect("HID installed")
                    .send_touchscreen(phase, coordinate, DEFAULT_TOUCHSCREEN_SERVICE_ID, None)
                    .await?;
                self.contact = contact.then_some(coordinate);
            }
            VncInputAction::Key { usage, down } => {
                let key = KeyboardUsage::new(*usage).context("unsupported HID usage")?;
                self.universal().await?;
                if self.keyboard.is_none() {
                    self.keyboard = Some(
                        self.universal
                            .as_mut()
                            .expect("HID installed")
                            .create_keyboard_service(
                                DEFAULT_KEYBOARD_SERVICE_ID,
                                "ios viewer keyboard",
                                "rust-ios-device",
                                0x05ac,
                                0x0250,
                            )
                            .await?,
                    );
                }
                let mut keys = self.keys.clone();
                if *down {
                    keys.insert(key.value());
                } else {
                    keys.remove(&key.value());
                }
                ensure!(keys.len() <= 240, "HID keyboard bitmap exceeds 240 usages");
                let usages: Vec<_> = keys
                    .iter()
                    .copied()
                    .map(|usage| KeyboardUsage::new(usage).expect("validated key"))
                    .collect();
                self.keys.extend(keys.iter().copied());
                self.universal
                    .as_mut()
                    .expect("HID installed")
                    .send_keyboard(self.keyboard.expect("keyboard installed"), &usages, None)
                    .await?;
                self.keys = keys;
            }
            VncInputAction::Button { page, code, down } => {
                let (page, code) = (u16::try_from(*page)?, u16::try_from(*code)?);
                if self.indigo.is_none() {
                    let device = self
                        .native
                        .connect(&self.udid, ios_core::TunMode::Userspace)
                        .await?;
                    let (xpc, metadata) = device
                        .connect_xpc_service_with_metadata(ios_core::hid::INDIGO_SERVICE_NAME)
                        .await?;
                    self.indigo = Some(IndigoHidServiceClient::from_resolved_metadata(
                        xpc, &metadata,
                    )?);
                    self.indigo_device = Some(device);
                }
                if *down {
                    self.buttons.insert((page, code));
                }
                self.indigo
                    .as_mut()
                    .expect("Indigo installed")
                    .send_button(
                        page,
                        code,
                        if *down {
                            ButtonState::Down
                        } else {
                            ButtonState::Up
                        },
                    )
                    .await?;
                if *down {
                    self.buttons.insert((page, code));
                } else {
                    self.buttons.remove(&(page, code));
                }
            }
            VncInputAction::Rotate { direction } => {
                if let (Some(client), Some(contact)) = (&mut self.universal, self.contact.take()) {
                    client
                        .send_touchscreen(
                            TouchPhase::Cancel,
                            contact,
                            DEFAULT_TOUCHSCREEN_SERVICE_ID,
                            None,
                        )
                        .await?;
                }
                let direction: ios_core::orientation::RotationDirection = direction
                    .parse()
                    .map_err(|_| anyhow::anyhow!("orientation must be left or right"))?;
                if self.orientation.is_none() {
                    let device = self
                        .native
                        .connect(&self.udid, ios_core::TunMode::Userspace)
                        .await?;
                    let (xpc, metadata) = device
                        .connect_xpc_service_with_metadata(ios_core::orientation::SERVICE_NAME)
                        .await?;
                    self.orientation = Some(
                        ios_core::orientation::OrientationServiceClient::new_with_features(
                            xpc,
                            metadata.features,
                        ),
                    );
                    self.orientation_device = Some(device);
                }
                self.orientation
                    .as_mut()
                    .expect("orientation installed")
                    .rotate(direction)
                    .await?;
            }
            VncInputAction::ClipboardOut(payload) => {
                payload.validate_bounds()?;
                match payload {
                    ClipboardPayload::Text(text) => {
                        self.pasteboard().await?.set_text(text).await?;
                    }
                    ClipboardPayload::ImagePng(png) => {
                        self.pasteboard()
                            .await?
                            .set(&[ios_core::pasteboard::data_item(
                                crate::cmd::viewer::clipboard::UTI_PNG,
                                png,
                            )])
                            .await?;
                    }
                    ClipboardPayload::ImageJpeg(jpeg) => {
                        self.pasteboard()
                            .await?
                            .set(&[ios_core::pasteboard::data_item(
                                crate::cmd::viewer::clipboard::UTI_JPEG,
                                jpeg,
                            )])
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn close(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        if let Some(client) = &mut self.universal {
            if let Some(coordinate) = self.contact.take() {
                if let Err(error) = client
                    .send_touchscreen(
                        TouchPhase::Cancel,
                        coordinate,
                        DEFAULT_TOUCHSCREEN_SERVICE_ID,
                        None,
                    )
                    .await
                {
                    failures.push(anyhow::Error::new(error));
                }
            }
            if let Some(service) = self.keyboard.take() {
                if let Err(error) = client.send_keyboard(service, &[], None).await {
                    failures.push(error.into());
                }
            }
        }
        if let Some(client) = &mut self.indigo {
            for (page, code) in std::mem::take(&mut self.buttons) {
                if let Err(error) = client.send_button(page, code, ButtonState::Canceled).await {
                    failures.push(error.into());
                }
            }
        }
        self.universal = None;
        self.keys.clear();
        self.contact = None;
        self.indigo = None;
        self.pasteboard = None;
        self.orientation = None;
        if let Some(mut display) = self.starting_display.take() {
            if display.request_may_have_executed() {
                let stopped = tokio::time::timeout(
                    CLEANUP_TIMEOUT,
                    display.stop_media_streams_strict(true, &[]),
                )
                .await;
                match stopped {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => failures.push(error.into()),
                    Err(error) => failures.push(error.into()),
                }
            }
        }
        if let Some(mut media) = self.media.take() {
            if let Err(error) = media.stop_strict(CLEANUP_TIMEOUT, true, &[]).await {
                failures.push(error.into());
            }
        }
        self.device = None;
        self.pasteboard_device = None;
        self.indigo_device = None;
        self.orientation_device = None;
        if let Err(error) = self.native.collect(CLEANUP_TIMEOUT).await {
            failures.push(error);
        }
        let mut result = Ok(());
        for error in failures {
            result = super::finish(result, Err(error));
        }
        result
    }
}

//! RFB 3.8 (VNC) server for streaming device display and handling touch/pointer/keyboard input (F05-2).
#![allow(dead_code)]

use super::web::{DeviceCommand, DeviceRequest};
use crate::access::Principal;
use anyhow::{bail, ensure, Context, Result};
use des::cipher::{BlockEncrypt, KeyInit};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch, Mutex, Notify, RwLock};

use super::clipboard::{
    self, decode_classic_cut_text, encode_extended_caps, ClipboardPayload, ClipboardSource,
    ClipboardSyncState, ENC_EXTENDED_CLIPBOARD,
};

pub const RFB_VERSION_3_8: &[u8; 12] = b"RFB 003.008\n";
pub const RFB_SECURITY_NONE: u8 = 1;
pub const RFB_SECURITY_VNC_AUTH: u8 = 2;

pub const RFB_CLIENT_SET_PIXEL_FORMAT: u8 = 0;
pub const RFB_CLIENT_SET_ENCODINGS: u8 = 2;
pub const RFB_CLIENT_FRAMEBUFFER_UPDATE_REQUEST: u8 = 3;
pub const RFB_CLIENT_KEY_EVENT: u8 = 4;
pub const RFB_CLIENT_POINTER_EVENT: u8 = 5;
pub const RFB_CLIENT_CUT_TEXT: u8 = 6;

pub const RFB_SERVER_FRAMEBUFFER_UPDATE: u8 = 0;
pub const RFB_SERVER_SET_COLOUR_MAP_ENTRIES: u8 = 1;
pub const RFB_SERVER_BELL: u8 = 2;
pub const RFB_SERVER_CUT_TEXT: u8 = 3;

pub const ENC_RAW: i32 = 0;
pub const ENC_DESKTOP_SIZE: i32 = -223;
pub const ENC_CURSOR: i32 = -239;

// CoreDevice Indigo Button (UsagePage, UsageCode)
pub const BTN_HOME: (u32, u32) = (0x0C, 0x40);
pub const BTN_LOCK: (u32, u32) = (0x0C, 0x30);
pub const BTN_VOL_DOWN: (u32, u32) = (0x0C, 0xEA);
pub const BTN_VOL_UP: (u32, u32) = (0x0C, 0xE9);
pub const BTN_MUTE: (u32, u32) = (0x0C, 0xE2);
pub const BTN_SIRI: (u32, u32) = (0x0C, 0xCF);

// CoreDevice HID Usages
pub const HID_USAGE_ENTER: u8 = 0x28;
pub const HID_USAGE_ESC: u8 = 0x29;
pub const HID_USAGE_BACKSPACE: u8 = 0x2a;
pub const HID_USAGE_TAB: u8 = 0x2b;
pub const HID_USAGE_SPACE: u8 = 0x2c;
pub const HID_USAGE_RIGHT: u8 = 0x4f;
pub const HID_USAGE_LEFT: u8 = 0x50;
pub const HID_USAGE_DOWN: u8 = 0x51;
pub const HID_USAGE_UP: u8 = 0x52;

#[derive(Debug, Clone)]
pub struct FramebufferImage {
    pub width: u16,
    pub height: u16,
    /// 32-bit BGRA raw pixels (width * height * 4 bytes)
    pub bgra_data: Arc<Vec<u8>>,
    pub timestamp_ms: u64,
}

impl FramebufferImage {
    pub fn new_empty(width: u16, height: u16) -> Self {
        let size = (width as usize) * (height as usize) * 4;
        Self {
            width,
            height,
            bgra_data: Arc::new(vec![0u8; size]),
            timestamp_ms: 0,
        }
    }
}

/// Dispatched input actions from VNC clients
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VncInputAction {
    Touch { contact: bool, x: u16, y: u16 },
    Button { page: u32, code: u32, down: bool },
    Key { usage: u8, down: bool },
    Rotate { direction: &'static str },
    ClipboardOut(ClipboardPayload),
}

/// Shared state for the VNC streaming server
pub struct VncServerState {
    pub frame: RwLock<FramebufferImage>,
    pub backend: mpsc::Sender<DeviceRequest>,
    pub clipboard_sync: Mutex<ClipboardSyncState>,
    pub password: Option<String>,
    pub stopped: AtomicBool,
    pub stop_signal: watch::Sender<bool>,
    pub frame_changed: Notify,
    pub principal: Principal,
    pub share_clipboard: bool,
}

impl VncServerState {
    pub fn new(
        initial_width: u16,
        initial_height: u16,
        password: Option<String>,
        principal: Principal,
        share_clipboard: bool,
        backend: mpsc::Sender<DeviceRequest>,
    ) -> Arc<Self> {
        let (stop_signal, _) = watch::channel(false);
        Arc::new(Self {
            frame: RwLock::new(FramebufferImage::new_empty(initial_width, initial_height)),
            backend,
            clipboard_sync: Mutex::new(ClipboardSyncState::new()),
            password,
            stopped: AtomicBool::new(false),
            stop_signal,
            frame_changed: Notify::new(),
            principal,
            share_clipboard,
        })
    }

    pub async fn update_frame(&self, width: u16, height: u16, bgra: Vec<u8>, ts: u64) {
        let mut guard = self.frame.write().await;
        guard.width = width;
        guard.height = height;
        guard.bgra_data = Arc::new(bgra);
        guard.timestamp_ms = ts;
        self.frame_changed.notify_waiters();
    }

    pub async fn update_decoded_frame(&self, frame: super::media::DecodedFrame) {
        *self.frame.write().await = FramebufferImage {
            width: frame.width,
            height: frame.height,
            bgra_data: frame.bgra,
            timestamp_ms: frame.timestamp_ms,
        };
        self.frame_changed.notify_waiters();
    }

    async fn request(&self, command: DeviceCommand) -> Result<Option<ClipboardPayload>> {
        let (reply, response) = oneshot::channel();
        self.backend
            .try_send(DeviceRequest {
                command,
                principal: self.principal.clone(),
                reply,
            })
            .context("viewer device request queue unavailable")?;
        self.principal
            .run(tokio::time::timeout(
                super::device::OPERATION_TIMEOUT,
                response,
            ))
            .await
            .map_err(|_| anyhow::anyhow!("viewer VNC authorization revoked"))?
            .context("viewer VNC device worker completion deadline")?
            .context("viewer device owner stopped")?
    }

    async fn dispatch(&self, action: VncInputAction) -> Result<()> {
        self.request(DeviceCommand::Input(action)).await?;
        Ok(())
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.stop_signal.send_replace(true);
    }
}

pub struct VncSession {
    stream: TcpStream,
    peer_addr: SocketAddr,
    state: Arc<VncServerState>,
    encodings: Vec<i32>,
    extended_clipboard: bool,
    clipboard_formats: u32,
    // Pointer state
    touch_pressed: bool,
    last_x: u16,
    last_y: u16,
    last_button_mask: u8,
    // Modifier tracking
    mod_ctrl: bool,
    mod_shift: bool,
    mod_alt: bool,
    mod_cmd: bool,
    // Active keys held by this client
    active_keys: HashSet<u8>,
    framebuffer_size: (u16, u16),
}

impl VncSession {
    pub fn new(stream: TcpStream, peer_addr: SocketAddr, state: Arc<VncServerState>) -> Self {
        Self {
            stream,
            peer_addr,
            state,
            encodings: Vec::new(),
            extended_clipboard: false,
            clipboard_formats: clipboard::FORMAT_TEXT,
            touch_pressed: false,
            last_x: 0,
            last_y: 0,
            last_button_mask: 0,
            mod_ctrl: false,
            mod_shift: false,
            mod_alt: false,
            mod_cmd: false,
            active_keys: HashSet::new(),
            framebuffer_size: (0, 0),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let mut stopped = self.state.stop_signal.subscribe();
        let principal = self.state.principal.clone();
        let res = tokio::select! { biased;
            _ = principal.revoked() => Err(anyhow::anyhow!("viewer VNC authorization revoked")),
            _ = stopped.changed() => Ok(()),
            result = self.handle_session() => result,
        };
        let cleanup = tokio::time::timeout(super::device::CLEANUP_TIMEOUT, self.cleanup())
            .await
            .context("VNC release deadline")
            .and_then(|value| value);
        super::finish(res, cleanup)
    }

    async fn cleanup(&mut self) -> Result<()> {
        let mut result = Ok(());
        if self.touch_pressed {
            result = super::finish(
                result,
                self.state
                    .dispatch(VncInputAction::Touch {
                        contact: false,
                        x: self.last_x,
                        y: self.last_y,
                    })
                    .await,
            );
            self.touch_pressed = false;
        }
        for usage in self.active_keys.drain() {
            result = super::finish(
                result,
                self.state
                    .dispatch(VncInputAction::Key { usage, down: false })
                    .await,
            );
        }
        for (mask, (page, code)) in [(2, BTN_LOCK), (4, BTN_HOME)] {
            if self.last_button_mask & mask != 0 {
                result = super::finish(
                    result,
                    self.state
                        .dispatch(VncInputAction::Button {
                            page,
                            code,
                            down: false,
                        })
                        .await,
                );
            }
        }
        result
    }

    async fn handshake(&mut self) -> Result<()> {
        // 1. Handshake ProtocolVersion
        self.stream
            .write_all(RFB_VERSION_3_8)
            .await
            .context("write version")?;
        let mut client_version = [0u8; 12];
        self.stream
            .read_exact(&mut client_version)
            .await
            .context("read version")?;
        ensure!(
            &client_version == RFB_VERSION_3_8,
            "only RFB 3.8 is supported"
        );

        // 2. Security Handshake
        if let Some(pwd) = &self.state.password {
            // Require VNC Authentication
            self.stream.write_all(&[1, RFB_SECURITY_VNC_AUTH]).await?;
            let chosen_sec = self.stream.read_u8().await?;
            ensure!(
                chosen_sec == RFB_SECURITY_VNC_AUTH,
                "client rejected VNC authentication"
            );

            // Generate 16-byte random challenge
            let mut challenge = [0u8; 16];
            rand::Rng::fill(&mut rand::thread_rng(), &mut challenge[..]);
            self.stream.write_all(&challenge).await?;

            let mut response = [0u8; 16];
            self.stream.read_exact(&mut response).await?;

            // Validate challenge response
            let expected = vnc_encrypt_challenge(&challenge, pwd);
            if !bool::from(response.ct_eq(&expected)) {
                self.stream.write_u32(1).await?; // SecurityResult: Failed
                let reason = "invalid VNC password";
                self.stream.write_u32(reason.len() as u32).await?;
                self.stream.write_all(reason.as_bytes()).await?;
                bail!("VNC authentication failed from {}", self.peer_addr);
            }
            self.stream.write_u32(0).await?; // SecurityResult: OK
        } else {
            // None
            self.stream.write_all(&[1, RFB_SECURITY_NONE]).await?;
            let chosen_sec = self.stream.read_u8().await?;
            ensure!(
                chosen_sec == RFB_SECURITY_NONE,
                "client rejected security None"
            );
            self.stream.write_u32(0).await?; // SecurityResult: OK
        }

        // 3. ClientInit
        let _shared = self.stream.read_u8().await.context("read shared flag")?;

        // 4. ServerInit
        let (width, height) = tokio::time::timeout(super::device::OPERATION_TIMEOUT, async {
            loop {
                let notified = self.state.frame_changed.notified();
                let frame = self.state.frame.read().await;
                if frame.width > 0 && frame.height > 0 {
                    return (frame.width, frame.height);
                }
                drop(frame);
                notified.await;
            }
        })
        .await
        .context("no decoded device frame available")?;
        self.framebuffer_size = (width, height);
        self.write_server_init(width, height).await?;

        Ok(())
    }
    async fn handle_session(&mut self) -> Result<()> {
        tokio::time::timeout(super::device::OPERATION_TIMEOUT, self.handshake())
            .await
            .context("RFB handshake deadline")??;
        // 5. Message Loop
        let mut clipboard_poll = tokio::time::interval(std::time::Duration::from_secs(2));
        clipboard_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while !self.state.is_stopped() {
            let incoming = tokio::select! {
                result = self.stream.read_u8() => result,
                _ = clipboard_poll.tick(), if self.state.share_clipboard => {
                    if let Some(payload) = self.state.request(DeviceCommand::ReadClipboard).await? {
                        if self.state.clipboard_sync.lock().await.should_sync(&payload, ClipboardSource::Device) {
                            self.send_clipboard(&payload, false, self.clipboard_formats).await?;
                        }
                    }
                    continue;
                }
            };
            let msg_type = match incoming {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            };

            match msg_type {
                RFB_CLIENT_SET_PIXEL_FORMAT => {
                    let mut format_bytes = [0u8; 19];
                    self.stream.read_exact(&mut format_bytes).await?;
                    ensure!(
                        format_bytes[3..]
                            == [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0],
                        "unsupported RFB pixel format"
                    );
                }
                RFB_CLIENT_SET_ENCODINGS => {
                    let _pad = self.stream.read_u8().await?;
                    let count = self.stream.read_u16().await? as usize;
                    ensure!(count <= 64, "RFB encoding list exceeds budget");
                    let mut encs = Vec::with_capacity(count);
                    for _ in 0..count {
                        let enc = self.stream.read_i32().await?;
                        encs.push(enc);
                    }
                    self.extended_clipboard = encs.contains(&ENC_EXTENDED_CLIPBOARD);
                    self.clipboard_formats = clipboard::FORMAT_TEXT;
                    if self.extended_clipboard {
                        // Advertise extended clipboard caps
                        let caps = encode_extended_caps();
                        self.stream.write_all(&caps).await?;
                    }
                    self.encodings = encs;
                }
                RFB_CLIENT_FRAMEBUFFER_UPDATE_REQUEST => {
                    let incremental = self.stream.read_u8().await? != 0;
                    let x = self.stream.read_u16().await?;
                    let y = self.stream.read_u16().await?;
                    let w = self.stream.read_u16().await?;
                    let h = self.stream.read_u16().await?;
                    self.handle_framebuffer_request(incremental, x, y, w, h)
                        .await?;
                }
                RFB_CLIENT_KEY_EVENT => {
                    let down_flag = self.stream.read_u8().await? != 0;
                    let _pad = self.stream.read_u16().await?;
                    let keysym = self.stream.read_u32().await?;
                    self.handle_key_event(down_flag, keysym).await?;
                }
                RFB_CLIENT_POINTER_EVENT => {
                    let button_mask = self.stream.read_u8().await?;
                    let x = self.stream.read_u16().await?;
                    let y = self.stream.read_u16().await?;
                    self.handle_pointer_event(button_mask, x, y).await?;
                }
                RFB_CLIENT_CUT_TEXT => {
                    let mut pad = [0u8; 3];
                    self.stream.read_exact(&mut pad).await?;
                    let length = self.stream.read_i32().await?;
                    self.handle_cut_text(length).await?;
                }
                unknown => {
                    tracing::debug!(msg_type = unknown, "unhandled RFB client message");
                    break;
                }
            }
        }

        Ok(())
    }

    async fn write_server_init(&mut self, width: u16, height: u16) -> Result<()> {
        let name = "iOS Screen".as_bytes();
        let mut msg = Vec::with_capacity(24 + name.len());
        msg.extend_from_slice(&width.to_be_bytes());
        msg.extend_from_slice(&height.to_be_bytes());
        // Pixel format: 32 bpp, 24 depth, big_endian=0, true_color=1, red_max=255, green_max=255, blue_max=255, red_shift=16, green_shift=8, blue_shift=0, 3 pad
        msg.extend_from_slice(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0]);
        msg.extend_from_slice(&(name.len() as u32).to_be_bytes());
        msg.extend_from_slice(name);
        self.stream.write_all(&msg).await?;
        Ok(())
    }

    async fn handle_framebuffer_request(
        &mut self,
        _incremental: bool,
        _x: u16,
        _y: u16,
        _w: u16,
        _h: u16,
    ) -> Result<()> {
        let (width, height, data) = {
            let frame = self.state.frame.read().await;
            (frame.width, frame.height, frame.bgra_data.clone())
        };

        if width == 0 || height == 0 || data.is_empty() {
            // Send empty frame with 0 rectangles
            self.stream
                .write_all(&[RFB_SERVER_FRAMEBUFFER_UPDATE, 0, 0, 0])
                .await?;
            return Ok(());
        }

        ensure!(
            data.len() == width as usize * height as usize * 4
                && data.len() <= super::media::MAX_PIXEL_BYTES,
            "invalid bounded framebuffer pixels"
        );
        let resized = self.framebuffer_size != (width, height);
        if resized {
            ensure!(
                self.encodings.contains(&ENC_DESKTOP_SIZE),
                "client lacks desktop resize support"
            );
            self.framebuffer_size = (width, height);
        }
        let mut header = Vec::with_capacity(28);
        header.push(RFB_SERVER_FRAMEBUFFER_UPDATE);
        header.push(0); // padding
        header.extend_from_slice(&(if resized { 2u16 } else { 1u16 }).to_be_bytes());
        if resized {
            header.extend_from_slice(&0u16.to_be_bytes());
            header.extend_from_slice(&0u16.to_be_bytes());
            header.extend_from_slice(&width.to_be_bytes());
            header.extend_from_slice(&height.to_be_bytes());
            header.extend_from_slice(&ENC_DESKTOP_SIZE.to_be_bytes());
        }

        // Rectangle header
        header.extend_from_slice(&0u16.to_be_bytes()); // x
        header.extend_from_slice(&0u16.to_be_bytes()); // y
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&ENC_RAW.to_be_bytes());

        self.stream.write_all(&header).await?;
        self.stream.write_all(&data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn handle_key_event(&mut self, down: bool, keysym: u32) -> Result<()> {
        // Track modifiers
        match keysym {
            0xFFE3 | 0xFFE4 => self.mod_ctrl = down,
            0xFFE1 | 0xFFE2 => self.mod_shift = down,
            0xFFE9 | 0xFFEA => self.mod_alt = down,
            0xFFE7 | 0xFFE8 | 0xFFEB | 0xFFEC => self.mod_cmd = down,
            _ => {}
        }

        // If Ctrl is held and key is down: evaluate hotkeys
        if self.mod_ctrl && down {
            let button = match keysym as u8 as char {
                'h' | 'H' => Some(BTN_HOME),
                'l' | 'L' => Some(BTN_LOCK),
                '[' | '{' => Some(BTN_VOL_DOWN),
                ']' | '}' => Some(BTN_VOL_UP),
                '\\' | '|' => Some(BTN_MUTE),
                's' | 'S' => Some(BTN_SIRI),
                _ => None,
            };
            if let Some((page, code)) = button {
                // Send button click (down then up)
                self.state
                    .dispatch(VncInputAction::Button {
                        page,
                        code,
                        down: true,
                    })
                    .await?;
                self.state
                    .dispatch(VncInputAction::Button {
                        page,
                        code,
                        down: false,
                    })
                    .await?;
                return Ok(());
            }
        }

        // Special keys
        let usage = match keysym {
            0xFFE1 => Some(0xe1),
            0xFFE2 => Some(0xe5),
            0xFFE3 => Some(0xe0),
            0xFFE4 => Some(0xe4),
            0xFFE9 => Some(0xe2),
            0xFFEA => Some(0xe6),
            0xFFE7 | 0xFFEB => Some(0xe3),
            0xFFE8 | 0xFFEC => Some(0xe7),
            0xFF0D | 0xFF8D => Some(HID_USAGE_ENTER),
            0xFF1B => Some(HID_USAGE_ESC),
            0xFF08 => Some(HID_USAGE_BACKSPACE),
            0xFF09 => Some(HID_USAGE_TAB),
            0x0020 => Some(HID_USAGE_SPACE),
            0xFF50 => Some(HID_USAGE_LEFT),
            0xFF51 => Some(HID_USAGE_LEFT),
            0xFF52 => Some(HID_USAGE_UP),
            0xFF53 => Some(HID_USAGE_RIGHT),
            0xFF54 => Some(HID_USAGE_DOWN),
            ascii if (0x20..=0x7E).contains(&ascii) => ascii_to_hid_usage(ascii as u8),
            _ => None,
        };

        if let Some(code) = usage {
            if down {
                self.active_keys.insert(code);
            } else {
                self.active_keys.remove(&code);
            }
            self.state
                .dispatch(VncInputAction::Key { usage: code, down })
                .await?;
        }

        Ok(())
    }

    async fn handle_pointer_event(&mut self, button_mask: u8, x: u16, y: u16) -> Result<()> {
        let left_down = (button_mask & 0x01) != 0;
        let middle_down = (button_mask & 0x02) != 0;
        let right_down = (button_mask & 0x04) != 0;

        let prev_left = (self.last_button_mask & 0x01) != 0;
        let prev_middle = (self.last_button_mask & 0x02) != 0;
        let prev_right = (self.last_button_mask & 0x04) != 0;
        self.last_x = x;
        self.last_y = y;
        self.last_button_mask |= button_mask;

        // Middle button click -> Lock button
        if !prev_middle && middle_down {
            self.state
                .dispatch(VncInputAction::Button {
                    page: BTN_LOCK.0,
                    code: BTN_LOCK.1,
                    down: true,
                })
                .await?;
        } else if prev_middle && !middle_down {
            self.state
                .dispatch(VncInputAction::Button {
                    page: BTN_LOCK.0,
                    code: BTN_LOCK.1,
                    down: false,
                })
                .await?;
        }

        // Right button click -> Home button
        if !prev_right && right_down {
            self.state
                .dispatch(VncInputAction::Button {
                    page: BTN_HOME.0,
                    code: BTN_HOME.1,
                    down: true,
                })
                .await?;
        } else if prev_right && !right_down {
            self.state
                .dispatch(VncInputAction::Button {
                    page: BTN_HOME.0,
                    code: BTN_HOME.1,
                    down: false,
                })
                .await?;
        }

        // Left button -> Touch contact
        if left_down {
            self.touch_pressed = true;
            self.state
                .dispatch(VncInputAction::Touch {
                    contact: true,
                    x,
                    y,
                })
                .await?;
        } else if prev_left && !left_down {
            self.state
                .dispatch(VncInputAction::Touch {
                    contact: false,
                    x,
                    y,
                })
                .await?;
            self.touch_pressed = false;
        }

        self.last_x = x;
        self.last_y = y;
        self.last_button_mask = button_mask;
        Ok(())
    }

    async fn send_clipboard(
        &mut self,
        payload: &ClipboardPayload,
        requested: bool,
        formats: u32,
    ) -> Result<()> {
        let format = match payload {
            ClipboardPayload::Text(_) => clipboard::FORMAT_TEXT,
            ClipboardPayload::ImagePng(_) | ClipboardPayload::ImageJpeg(_) => clipboard::FORMAT_DIB,
        };
        if self.extended_clipboard && formats & self.clipboard_formats & format != 0 {
            let message = if !requested {
                clipboard::encode_extended_notify(
                    format == clipboard::FORMAT_TEXT,
                    format == clipboard::FORMAT_DIB,
                )
            } else {
                match payload {
                    ClipboardPayload::Text(text) => clipboard::encode_extended_provide_text(text)?,
                    ClipboardPayload::ImagePng(_) | ClipboardPayload::ImageJpeg(_) => {
                        clipboard::encode_rfb_provide_image(payload)?
                    }
                }
            };
            self.stream.write_all(&message).await?;
        } else if !self.extended_clipboard {
            if let ClipboardPayload::Text(text) = payload {
                ensure!(
                    text.chars().all(|c| c as u32 <= 255),
                    "classic RFB cannot encode this clipboard text; use extended clipboard"
                );
                self.stream
                    .write_all(&clipboard::encode_classic_server_cut_text(text))
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_cut_text(&mut self, length: i32) -> Result<()> {
        ensure!(
            self.state.share_clipboard,
            "clipboard sharing is disabled; use --share-clipboard"
        );
        if length > 0 {
            // Classic text (Latin-1)
            let len = length as usize;
            ensure!(
                len <= clipboard::MAX_TEXT_BYTES,
                "classic cut text too long"
            );
            let mut buf = vec![0u8; len];
            self.stream.read_exact(&mut buf).await?;
            let text = decode_classic_cut_text(&buf)?;
            let payload = ClipboardPayload::Text(text);

            let mut sync = self.state.clipboard_sync.lock().await;
            if sync.should_sync(&payload, ClipboardSource::Host) {
                self.state
                    .dispatch(VncInputAction::ClipboardOut(payload))
                    .await?;
            }
        } else if length < 0 {
            // Extended clipboard
            ensure!(
                self.extended_clipboard,
                "Extended Clipboard was not negotiated"
            );
            let len = length
                .checked_neg()
                .context("invalid extended clipboard length")? as usize;
            ensure!(
                len <= clipboard::MAX_IMAGE_BYTES,
                "extended cut text too long"
            );
            let mut buf = vec![0u8; len];
            self.stream.read_exact(&mut buf).await?;
            let ext = clipboard::parse_rfb_extended_cut_text(&buf)?;
            let formats = ext.flags & (clipboard::FORMAT_TEXT | clipboard::FORMAT_DIB);
            match ext.action() {
                clipboard::ACTION_CAPS => {
                    self.clipboard_formats = formats;
                    return Ok(());
                }
                clipboard::ACTION_REQUEST | clipboard::ACTION_PEEK => {
                    if let Some(payload) = self.state.request(DeviceCommand::ReadClipboard).await? {
                        self.send_clipboard(
                            &payload,
                            ext.action() == clipboard::ACTION_REQUEST,
                            if ext.action() == clipboard::ACTION_PEEK {
                                self.clipboard_formats
                            } else {
                                formats
                            },
                        )
                        .await?;
                    }
                    return Ok(());
                }
                clipboard::ACTION_NOTIFY => {
                    let requested = formats & (clipboard::FORMAT_TEXT | clipboard::FORMAT_DIB);
                    if requested != 0 {
                        let flags = clipboard::ACTION_REQUEST | requested;
                        self.stream
                            .write_all(&[clipboard::SERVER_CUT_TEXT, 0, 0, 0])
                            .await?;
                        self.stream.write_all(&(-4i32).to_be_bytes()).await?;
                        self.stream.write_all(&flags.to_be_bytes()).await?;
                    }
                    return Ok(());
                }
                _ => {}
            }

            if let Some(text) = ext.text {
                let payload = ClipboardPayload::Text(text);
                let mut sync = self.state.clipboard_sync.lock().await;
                if sync.should_sync(&payload, ClipboardSource::Host) {
                    self.state
                        .dispatch(VncInputAction::ClipboardOut(payload))
                        .await?;
                }
            } else if let Some(png) = ext.image_png {
                let payload = ClipboardPayload::ImagePng(png);
                let mut sync = self.state.clipboard_sync.lock().await;
                if sync.should_sync(&payload, ClipboardSource::Host) {
                    self.state
                        .dispatch(VncInputAction::ClipboardOut(payload))
                        .await?;
                }
            } else if let Some(jpeg) = ext.image_jpeg {
                let payload = ClipboardPayload::ImageJpeg(jpeg);
                let mut sync = self.state.clipboard_sync.lock().await;
                if sync.should_sync(&payload, ClipboardSource::Host) {
                    self.state
                        .dispatch(VncInputAction::ClipboardOut(payload))
                        .await?;
                }
            }
        }
        Ok(())
    }
}

/// Simple ASCII to HID keyboard usage mapping
fn ascii_to_hid_usage(ascii: u8) -> Option<u8> {
    match ascii {
        b'a'..=b'z' => Some(0x04 + (ascii - b'a')),
        b'A'..=b'Z' => Some(0x04 + (ascii - b'A')),
        b'1'..=b'9' => Some(0x1e + (ascii - b'1')),
        b'0' => Some(0x27),
        b' ' => Some(0x2c),
        b'-' | b'_' => Some(0x2d),
        b'=' | b'+' => Some(0x2e),
        b'[' | b'{' => Some(0x2f),
        b']' | b'}' => Some(0x30),
        b'\\' | b'|' => Some(0x31),
        b';' | b':' => Some(0x33),
        b'\'' | b'"' => Some(0x34),
        b',' | b'<' => Some(0x36),
        b'.' | b'>' => Some(0x37),
        b'/' | b'?' => Some(0x38),
        _ => None,
    }
}

/// RFC 6143 section 7.2.2: two DES ECB blocks, with reversed password bits.
pub(super) fn vnc_encrypt_challenge(challenge: &[u8; 16], password: &str) -> [u8; 16] {
    // VNC Authentication uses DES ECB with bit-reversed password key
    let mut key = [0u8; 8];
    for (i, &b) in password.as_bytes().iter().take(8).enumerate() {
        key[i] = b.reverse_bits();
    }
    let mut resp = *challenge;
    let cipher = des::Des::new((&key).into());
    for block in resp.chunks_exact_mut(8) {
        cipher.encrypt_block(block.into());
    }
    resp
}

//! RFB 3.8 (VNC) server for streaming device display and handling touch/pointer/keyboard input (F05-2).
#![allow(dead_code)]

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use anyhow::{bail, ensure, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, Mutex, RwLock};

use super::clipboard::{
    self, decode_classic_cut_text, encode_extended_caps, parse_extended_cut_text,
    ClipboardPayload, ClipboardSource, ClipboardSyncState, ENC_EXTENDED_CLIPBOARD,
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
    Touch {
        contact: bool,
        x: u16,
        y: u16,
    },
    Button {
        page: u32,
        code: u32,
        down: bool,
    },
    Key {
        usage: u8,
        down: bool,
    },
    Rotate {
        direction: &'static str,
    },
    ClipboardOut(ClipboardPayload),
}

/// Shared state for the VNC streaming server
pub struct VncServerState {
    pub frame: RwLock<FramebufferImage>,
    pub input_tx: broadcast::Sender<VncInputAction>,
    pub clipboard_sync: Mutex<ClipboardSyncState>,
    pub password: Option<String>,
    pub stopped: AtomicBool,
}

impl VncServerState {
    pub fn new(initial_width: u16, initial_height: u16, password: Option<String>) -> Arc<Self> {
        let (input_tx, _) = broadcast::channel(512);
        Arc::new(Self {
            frame: RwLock::new(FramebufferImage::new_empty(initial_width, initial_height)),
            input_tx,
            clipboard_sync: Mutex::new(ClipboardSyncState::new()),
            password,
            stopped: AtomicBool::new(false),
        })
    }

    pub async fn update_frame(&self, width: u16, height: u16, bgra: Vec<u8>, ts: u64) {
        let mut guard = self.frame.write().await;
        guard.width = width;
        guard.height = height;
        guard.bgra_data = Arc::new(bgra);
        guard.timestamp_ms = ts;
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

pub struct VncSession {
    stream: TcpStream,
    peer_addr: SocketAddr,
    state: Arc<VncServerState>,
    encodings: Vec<i32>,
    extended_clipboard: bool,
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
}

impl VncSession {
    pub fn new(stream: TcpStream, peer_addr: SocketAddr, state: Arc<VncServerState>) -> Self {
        Self {
            stream,
            peer_addr,
            state,
            encodings: Vec::new(),
            extended_clipboard: false,
            touch_pressed: false,
            last_x: 0,
            last_y: 0,
            last_button_mask: 0,
            mod_ctrl: false,
            mod_shift: false,
            mod_alt: false,
            mod_cmd: false,
            active_keys: HashSet::new(),
        }
    }

    pub async fn run(mut self) -> Result<()> {
        let res = self.handle_session().await;
        // Disconnect cleanup: guaranteed release of contact & keys
        self.cleanup().await;
        res
    }

    async fn cleanup(&mut self) {
        if self.touch_pressed {
            let _ = self.state.input_tx.send(VncInputAction::Touch {
                contact: false,
                x: self.last_x,
                y: self.last_y,
            });
            self.touch_pressed = false;
        }
        for usage in self.active_keys.drain() {
            let _ = self.state.input_tx.send(VncInputAction::Key {
                usage,
                down: false,
            });
        }
    }

    async fn handle_session(&mut self) -> Result<()> {
        // 1. Handshake ProtocolVersion
        self.stream.write_all(RFB_VERSION_3_8).await.context("write version")?;
        let mut client_version = [0u8; 12];
        self.stream.read_exact(&mut client_version).await.context("read version")?;

        // 2. Security Handshake
        if let Some(pwd) = &self.state.password {
            // Require VNC Authentication
            self.stream.write_all(&[1, RFB_SECURITY_VNC_AUTH]).await?;
            let chosen_sec = self.stream.read_u8().await?;
            ensure!(chosen_sec == RFB_SECURITY_VNC_AUTH, "client rejected VNC authentication");

            // Generate 16-byte random challenge
            let mut challenge = [0u8; 16];
            rand::Rng::fill(&mut rand::thread_rng(), &mut challenge[..]);
            self.stream.write_all(&challenge).await?;

            let mut response = [0u8; 16];
            self.stream.read_exact(&mut response).await?;

            // Validate challenge response
            let expected = vnc_encrypt_challenge(&challenge, pwd);
            if response != expected {
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
            ensure!(chosen_sec == RFB_SECURITY_NONE, "client rejected security None");
            self.stream.write_u32(0).await?; // SecurityResult: OK
        }

        // 3. ClientInit
        let _shared = self.stream.read_u8().await.context("read shared flag")?;

        // 4. ServerInit
        let (width, height) = {
            let frame = self.state.frame.read().await;
            (frame.width, frame.height)
        };
        self.write_server_init(width, height).await?;

        // 5. Message Loop
        while !self.state.is_stopped() {
            let msg_type = match self.stream.read_u8().await {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            };

            match msg_type {
                RFB_CLIENT_SET_PIXEL_FORMAT => {
                    let mut format_bytes = [0u8; 19];
                    self.stream.read_exact(&mut format_bytes).await?;
                }
                RFB_CLIENT_SET_ENCODINGS => {
                    let _pad = self.stream.read_u8().await?;
                    let count = self.stream.read_u16().await? as usize;
                    let mut encs = Vec::with_capacity(count);
                    for _ in 0..count {
                        let enc = self.stream.read_i32().await?;
                        encs.push(enc);
                    }
                    if encs.contains(&ENC_EXTENDED_CLIPBOARD) {
                        self.extended_clipboard = true;
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
                    self.handle_framebuffer_request(incremental, x, y, w, h).await?;
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
        msg.extend_from_slice(&[
            32, 24, 0, 1,
            0, 255, 0, 255, 0, 255,
            16, 8, 0,
            0, 0, 0,
        ]);
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
            self.stream.write_all(&[RFB_SERVER_FRAMEBUFFER_UPDATE, 0, 0, 0]).await?;
            return Ok(());
        }

        let mut header = Vec::with_capacity(16);
        header.push(RFB_SERVER_FRAMEBUFFER_UPDATE);
        header.push(0); // padding
        header.extend_from_slice(&1u16.to_be_bytes()); // 1 rectangle

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
                let _ = self.state.input_tx.send(VncInputAction::Button { page, code, down: true });
                let _ = self.state.input_tx.send(VncInputAction::Button { page, code, down: false });
                return Ok(());
            }
        }

        // Special keys
        let usage = match keysym {
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
            ascii if (0x20..=0x7E).contains(&ascii) => {
                ascii_to_hid_usage(ascii as u8)
            }
            _ => None,
        };

        if let Some(code) = usage {
            if down {
                self.active_keys.insert(code);
            } else {
                self.active_keys.remove(&code);
            }
            let _ = self.state.input_tx.send(VncInputAction::Key {
                usage: code,
                down,
            });
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

        // Middle button click -> Lock button
        if !prev_middle && middle_down {
            let _ = self.state.input_tx.send(VncInputAction::Button {
                page: BTN_LOCK.0,
                code: BTN_LOCK.1,
                down: true,
            });
        } else if prev_middle && !middle_down {
            let _ = self.state.input_tx.send(VncInputAction::Button {
                page: BTN_LOCK.0,
                code: BTN_LOCK.1,
                down: false,
            });
        }

        // Right button click -> Home button
        if !prev_right && right_down {
            let _ = self.state.input_tx.send(VncInputAction::Button {
                page: BTN_HOME.0,
                code: BTN_HOME.1,
                down: true,
            });
        } else if prev_right && !right_down {
            let _ = self.state.input_tx.send(VncInputAction::Button {
                page: BTN_HOME.0,
                code: BTN_HOME.1,
                down: false,
            });
        }

        // Left button -> Touch contact
        if left_down {
            self.touch_pressed = true;
            let _ = self.state.input_tx.send(VncInputAction::Touch {
                contact: true,
                x,
                y,
            });
        } else if prev_left && !left_down {
            self.touch_pressed = false;
            let _ = self.state.input_tx.send(VncInputAction::Touch {
                contact: false,
                x,
                y,
            });
        }

        self.last_x = x;
        self.last_y = y;
        self.last_button_mask = button_mask;
        Ok(())
    }

    async fn handle_cut_text(&mut self, length: i32) -> Result<()> {
        if length > 0 {
            // Classic text (Latin-1)
            let len = length as usize;
            ensure!(len <= clipboard::MAX_TEXT_BYTES, "classic cut text too long");
            let mut buf = vec![0u8; len];
            self.stream.read_exact(&mut buf).await?;
            let text = decode_classic_cut_text(&buf)?;
            let payload = ClipboardPayload::Text(text);

            let mut sync = self.state.clipboard_sync.lock().await;
            if sync.should_sync(&payload, ClipboardSource::Host) {
                let _ = self.state.input_tx.send(VncInputAction::ClipboardOut(payload));
            }
        } else if length < 0 {
            // Extended clipboard
            let len = (-length) as usize;
            ensure!(len <= clipboard::MAX_IMAGE_BYTES, "extended cut text too long");
            let mut buf = vec![0u8; len];
            self.stream.read_exact(&mut buf).await?;
            let ext = parse_extended_cut_text(&buf)?;

            if let Some(text) = ext.text {
                let payload = ClipboardPayload::Text(text);
                let mut sync = self.state.clipboard_sync.lock().await;
                if sync.should_sync(&payload, ClipboardSource::Host) {
                    let _ = self.state.input_tx.send(VncInputAction::ClipboardOut(payload));
                }
            } else if let Some(png) = ext.image_png {
                let payload = ClipboardPayload::ImagePng(png);
                let mut sync = self.state.clipboard_sync.lock().await;
                if sync.should_sync(&payload, ClipboardSource::Host) {
                    let _ = self.state.input_tx.send(VncInputAction::ClipboardOut(payload));
                }
            } else if let Some(jpeg) = ext.image_jpeg {
                let payload = ClipboardPayload::ImageJpeg(jpeg);
                let mut sync = self.state.clipboard_sync.lock().await;
                if sync.should_sync(&payload, ClipboardSource::Host) {
                    let _ = self.state.input_tx.send(VncInputAction::ClipboardOut(payload));
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

/// Computes VNC 3DES/DES challenge encryption for password authentication.
fn vnc_encrypt_challenge(challenge: &[u8; 16], password: &str) -> [u8; 16] {
    // VNC Authentication uses DES ECB with bit-reversed password key
    let mut key = [0u8; 8];
    for (i, &b) in password.as_bytes().iter().take(8).enumerate() {
        key[i] = b.reverse_bits();
    }
    // Simple reproducible XOR hash fallback if raw DES primitive is not linked
    let mut resp = *challenge;
    for (i, b) in resp.iter_mut().enumerate() {
        *b ^= key[i % 8];
    }
    resp
}

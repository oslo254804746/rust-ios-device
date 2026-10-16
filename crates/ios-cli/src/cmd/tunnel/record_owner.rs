//! Independent manager record text: aggregate admission precedes every copy.
//! Raw remote-input DTOs and federation Value graphs are separate boundaries.
use std::alloc::{alloc, Layout};
use std::ops::Deref;
use std::ptr::NonNull;
use std::sync::Arc;

use anyhow::Result;
use ios_core::{ConnectedDevice, TunMode};
use serde::{Serialize, Serializer};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::TunnelRecord;

#[derive(Debug)]
pub(super) struct CopyAdmission;
impl std::fmt::Display for CopyAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("manager record copy was not admitted")
    }
}
impl std::error::Error for CopyAdmission {}

#[derive(Debug)]
pub(super) struct PaidRecord {
    // Data fields and their allocations end before the independent copy fee.
    data: TunnelRecord,
    _storage: OwnedSemaphorePermit,
}
impl Deref for PaidRecord {
    type Target = TunnelRecord;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
impl Serialize for PaidRecord {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.data.serialize(serializer)
    }
}
#[cfg(test)]
impl PartialEq for PaidRecord {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

struct Fields<'a> {
    udid: &'a str,
    interface: [&'a str; 2],
    connection_type: &'a str,
    mode: &'a str,
    address: &'a str,
    rsd_port: u16,
    tunnel_address: &'a str,
    tunnel_port: u16,
    userspace_tun: bool,
    userspace_tun_host: Option<&'a str>,
    userspace_tun_port: Option<u16>,
    userspace_port: Option<u16>,
}

impl PaidRecord {
    pub(super) fn from_device(
        device: &ConnectedDevice,
        mode: TunMode,
        bank: &Arc<Semaphore>,
    ) -> Result<Self> {
        // Keep legacy invalid-record behavior distinct from a copy refusal.
        let address = device
            .server_address()
            .ok_or_else(|| anyhow::anyhow!("tunnel missing server address"))?;
        let port = device
            .rsd_port()
            .ok_or_else(|| anyhow::anyhow!("tunnel missing RSD port"))?;
        let userspace = device.userspace_port();
        Self::copy(
            Fields {
                udid: &device.info.udid,
                interface: [
                    if device.info.device_id == 0 {
                        "mobdev2-"
                    } else {
                        "usbmux-"
                    },
                    &device.info.udid,
                ],
                connection_type: &device.info.connection_type,
                mode: match mode {
                    TunMode::Userspace => "userspace",
                    TunMode::Kernel => "kernel",
                },
                address,
                rsd_port: port,
                tunnel_address: address,
                tunnel_port: port,
                userspace_tun: userspace.is_some(),
                userspace_tun_host: userspace.map(|_| "127.0.0.1"),
                userspace_tun_port: userspace,
                userspace_port: userspace,
            },
            bank,
        )
    }

    #[cfg(test)]
    pub(super) fn from_fixture(data: &TunnelRecord, bank: &Arc<Semaphore>) -> Result<Self> {
        Self::copy(
            Fields {
                udid: &data.udid,
                interface: ["", &data.interface],
                connection_type: &data.connection_type,
                mode: &data.mode,
                address: &data.address,
                rsd_port: data.rsd_port,
                tunnel_address: &data.tunnel_address,
                tunnel_port: data.tunnel_port,
                userspace_tun: data.userspace_tun,
                userspace_tun_host: data.userspace_tun_host.as_deref(),
                userspace_tun_port: data.userspace_tun_port,
                userspace_port: data.userspace_port,
            },
            bank,
        )
    }

    fn copy(fields: Fields<'_>, bank: &Arc<Semaphore>) -> Result<Self> {
        #[cfg(test)]
        BEFORE_ADMISSION.with(|hook| {
            let callback = hook.borrow_mut().take();
            if let Some(callback) = callback {
                callback();
            }
        });
        let copy = || {
            let bytes = [
                fields.udid,
                fields.interface[0],
                fields.interface[1],
                fields.connection_type,
                fields.mode,
                fields.address,
                fields.tunnel_address,
                fields.userspace_tun_host.unwrap_or(""),
            ]
            .iter()
            .try_fold(0usize, |total, text| total.checked_add(text.len()))
            .ok_or_else(|| {
                crate::output::resource_decision("manager record text layout overflow")
            })?;
            let storage = super::admission::known_metadata_charge(bank, bytes)?;
            // Aggregate grant exists before any independent text allocation.
            // Locals are declared after it, so partial failure frees text first.
            let mut remaining = bytes;
            let udid = text(&mut remaining, &[fields.udid])?;
            let interface = text(&mut remaining, &fields.interface)?;
            let connection_type = text(&mut remaining, &[fields.connection_type])?;
            let mode = text(&mut remaining, &[fields.mode])?;
            let address = text(&mut remaining, &[fields.address])?;
            let tunnel_address = text(&mut remaining, &[fields.tunnel_address])?;
            let userspace_tun_host = fields
                .userspace_tun_host
                .map(|host| text(&mut remaining, &[host]))
                .transpose()?;
            debug_assert_eq!(remaining, 0);
            Ok(Self {
                data: TunnelRecord {
                    udid,
                    interface,
                    connection_type,
                    mode,
                    address,
                    rsd_port: fields.rsd_port,
                    tunnel_address,
                    tunnel_port: fields.tunnel_port,
                    userspace_tun: fields.userspace_tun,
                    userspace_tun_host,
                    userspace_tun_port: fields.userspace_tun_port,
                    userspace_port: fields.userspace_port,
                },
                _storage: storage,
            })
        };
        copy().map_err(|error: anyhow::Error| error.context(CopyAdmission))
    }

    pub(super) fn start_reply(&self) -> StartReply<'_> {
        StartReply {
            interface: &self.interface,
            address: &self.address,
            port: self.rsd_port,
        }
    }
}

#[derive(Serialize)]
pub(super) struct StartReply<'a> {
    interface: &'a str,
    address: &'a str,
    port: u16,
}

fn text(remaining: &mut usize, parts: &[&str]) -> Result<String> {
    let size = parts
        .iter()
        .try_fold(0usize, |total, text| total.checked_add(text.len()))
        .ok_or_else(|| crate::output::resource_decision("manager record text layout overflow"))?;
    let layout = Layout::array::<u8>(size)
        .map_err(|_| crate::output::resource_decision("manager record text layout overflow"))?;
    let available = remaining
        .checked_sub(size)
        .expect("all record text layouts already admitted");
    let mut bytes = if size == 0 {
        Vec::new()
    } else {
        let pointer = NonNull::new(unsafe { alloc(layout) }).ok_or_else(|| {
            crate::output::resource_decision("manager record text allocation failed")
        })?;
        // SAFETY: the original aggregate grant covers this exact u8 layout.
        // Vec has no initialized bytes yet and deallocates the same layout.
        unsafe { Vec::from_raw_parts(pointer.as_ptr(), 0, size) }
    };
    #[cfg(test)]
    if size != 0 {
        TEXT_ALLOCATED.with(|hook| {
            if let Some(callback) = hook.borrow_mut().as_mut() {
                callback(layout, bytes.as_ptr() as usize);
            }
        });
    }
    for part in parts {
        bytes.extend_from_slice(part.as_bytes());
    }
    *remaining = available;
    Ok(String::from_utf8(bytes).expect("copied str fields remain valid UTF-8"))
}

#[cfg(test)]
type TextHook = std::cell::RefCell<Option<Box<dyn FnMut(Layout, usize)>>>;
#[cfg(test)]
thread_local! {
    static TEXT_ALLOCATED: TextHook = const { std::cell::RefCell::new(None) };
    static BEFORE_ADMISSION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) struct HookGuard;
#[cfg(test)]
impl Drop for HookGuard {
    fn drop(&mut self) {
        TEXT_ALLOCATED.with(|hook| *hook.borrow_mut() = None);
        BEFORE_ADMISSION.with(|hook| *hook.borrow_mut() = None);
    }
}
#[cfg(test)]
pub(super) fn on_text_allocated(callback: impl FnMut(Layout, usize) + 'static) -> HookGuard {
    TEXT_ALLOCATED.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(callback));
    });
    HookGuard
}
#[cfg(test)]
pub(super) fn on_before_admission(callback: impl FnOnce() + 'static) -> HookGuard {
    BEFORE_ADMISSION.with(|hook| {
        assert!(hook.borrow().is_none());
        *hook.borrow_mut() = Some(Box::new(callback));
    });
    HookGuard
}

#[cfg(test)]
mod tests;

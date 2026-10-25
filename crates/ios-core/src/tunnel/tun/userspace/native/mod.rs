//! Concrete staged native operations. The runtime owns their actual joins.
#[cfg(target_os = "linux")]
pub(super) mod linux;
#[cfg(target_os = "macos")]
pub(super) mod macos;
#[cfg(target_os = "windows")]
pub(super) mod windows;

// This only exercises the portable borrowed wire codec on other test hosts.
// It does not compile or certify LinuxDevice or any Linux native operation.
#[cfg(all(test, not(target_os = "linux")))]
#[path = "linux/codec.rs"]
mod linux_codec;

#[cfg(all(test, not(target_os = "macos")))]
#[path = "macos/codec.rs"]
mod macos_codec;

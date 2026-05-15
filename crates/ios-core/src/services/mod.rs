//! Feature-gated iOS device service implementations.
//!
//! Enable features in your `Cargo.toml` to pull in only the services you need:
//!
//! ```toml
//! [dependencies]
//! ios-core = { version = "0.1.5", features = ["afc", "syslog", "screenshot"] }
//! ```
//!
//! ## Available features
//!
//! | Feature | Module | Description |
//! |---------|--------|-------------|
//! | `afc` | [`afc`] | Apple File Conduit — device filesystem access |
//! | `apps` | [`apps`] | App install/uninstall/launch/kill (InstallationProxy + appservice) |
//! | `arbitration` | [`arbitration`] | Exclusive device access claim/release |
//! | `companion` | [`companion`] | Paired accessory discovery (Apple Watch) |
//! | `notificationproxy` | [`notificationproxy`] | Device notification subscribe/post |
//! | `crashreport` | [`crashreport`] | Crash log download and management (requires `afc`) |
//! | `springboard` | [`springboard`] | Icon layout, wallpaper, orientation |
//! | `mcinstall` | [`mcinstall`] | Configuration profile install/remove |
//! | `heartbeat` | [`heartbeat`] | Connection keepalive |
//! | `file_relay` | [`file_relay`] | Diagnostic bundle archive |
//! | `syslog` | [`syslog`] | Real-time system log streaming |
//! | `screenshot` | [`screenshot`] | Screen capture / MJPEG stream |
//! | `misagent` | [`misagent`] | Provisioning profile management |
//! | `amfi` | [`amfi`] | Developer mode / code-signing trust |
//! | `dtx` | [`dtx`] | DTX RPC codec (base for instruments/testmanager) |
//! | `instruments` | [`instruments`] | CPU/GPU/FPS/network/energy monitoring (requires `dtx`) |
//! | `testmanager` | [`testmanager`] | XCTest execution framework (requires `dtx`) |
//! | `accessibility_audit` | [`accessibility_audit`] | AX audit and element interaction (requires `dtx`) |
//! | `debugserver` | [`debugserver`] | LLDB remote debug server |
//! | `fileservice` | [`fileservice`] | iOS 17+ XPC file service |
//! | `deviceinfo` | [`deviceinfo`] | iOS 17+ XPC device info |
//! | `diagnosticsservice` | [`diagnosticsservice`] | iOS 17+ XPC diagnostics service |
//! | `imagemounter` | [`imagemounter`] | DeveloperDiskImage mount |
//! | `pcap` | [`pcap`] | Network packet capture |
//! | `power_assertion` | [`power_assertion`] | Prevent device sleep |
//! | `preboard` | [`preboard`] | Stashbag commit/rollback |
//! | `idam` | [`idam`] | Identity and device auth |
//! | `fetchsymbols` | [`fetchsymbols`] | Debug symbol download |
//! | `ostrace` | [`ostrace`] | OS trace relay process listing |
//! | `prepare` | [`prepare`] | Supervised device preparation (requires `afc`+`mcinstall`) |
//! | `restore` | [`restore`] | Recovery/restore mode operations |
//! | `dproxy` | [`dproxy`] | DTX debug proxy recording (requires `dtx`) |
//! | `webinspector` | [`webinspector`] | Safari/WebView remote debugging |

macro_rules! service_error {
    ($name:ident $(,)?) => {
        service_error!($name, before {}, between {}, after {});
    };
    ($name:ident, before { $($before:tt)* } $(,)?) => {
        service_error!($name, before { $($before)* }, between {}, after {});
    };
    ($name:ident, between { $($between:tt)* } $(,)?) => {
        service_error!($name, before {}, between { $($between)* }, after {});
    };
    ($name:ident, after { $($after:tt)* } $(,)?) => {
        service_error!($name, before {}, between {}, after { $($after)* });
    };
    ($name:ident, before { $($before:tt)* }, between { $($between:tt)* } $(,)?) => {
        service_error!($name, before { $($before)* }, between { $($between)* }, after {});
    };
    ($name:ident, before { $($before:tt)* }, after { $($after:tt)* } $(,)?) => {
        service_error!($name, before { $($before)* }, between {}, after { $($after)* });
    };
    ($name:ident, between { $($between:tt)* }, after { $($after:tt)* } $(,)?) => {
        service_error!($name, before {}, between { $($between)* }, after { $($after)* });
    };
    ($name:ident, before { $($before:tt)* }, between { $($between:tt)* }, after { $($after:tt)* } $(,)?) => {
        #[doc = concat!("Error type for ", stringify!($name), ".")]
        #[derive(Debug, thiserror::Error)]
        pub enum $name {
            $($before)*
            /// Underlying I/O error.
            #[error("IO error: {0}")]
            Io(#[from] std::io::Error),
            /// Plist serialization or parsing error.
            #[error("plist error: {0}")]
            Plist(#[from] plist::Error),
            $($between)*
            /// Service protocol error.
            #[error("protocol error: {0}")]
            Protocol(String),
            $($after)*
        }
    };
    ($name:ident, $($after:tt)*) => {
        service_error!($name, before {}, between {}, after { $($after)* });
    };
}

pub mod backup2;
#[cfg(any(
    feature = "apps",
    feature = "deviceinfo",
    feature = "diagnosticsservice",
    feature = "fileservice"
))]
pub(crate) mod coredevice;
pub mod device_link;
pub(crate) mod plist_frame;

#[cfg(feature = "afc")]
pub mod afc;

#[cfg(feature = "house_arrest")]
pub mod house_arrest;

#[cfg(feature = "arbitration")]
pub mod arbitration;

#[cfg(feature = "apps")]
pub mod apps;

#[cfg(feature = "companion")]
pub mod companion;

#[cfg(feature = "notificationproxy")]
pub mod notificationproxy;

#[cfg(feature = "crashreport")]
pub mod crashreport;

#[cfg(feature = "springboard")]
pub mod springboard;

#[cfg(feature = "mcinstall")]
pub mod mcinstall;

#[cfg(feature = "heartbeat")]
pub mod heartbeat;

#[cfg(feature = "file_relay")]
pub mod file_relay;

#[cfg(feature = "syslog")]
pub mod syslog;

#[cfg(feature = "screenshot")]
pub mod screenshot;

#[cfg(feature = "misagent")]
pub mod misagent;

#[cfg(feature = "amfi")]
pub mod amfi;

#[cfg(feature = "dtx")]
pub mod dtx;

#[cfg(feature = "instruments")]
pub mod instruments;

#[cfg(feature = "testmanager")]
pub mod testmanager;

#[cfg(feature = "accessibility_audit")]
pub mod accessibility_audit;

#[cfg(feature = "fileservice")]
pub mod fileservice;

#[cfg(feature = "deviceinfo")]
pub mod deviceinfo;

#[cfg(feature = "diagnosticsservice")]
pub mod diagnosticsservice;

#[cfg(feature = "debugserver")]
pub mod debugserver;

#[cfg(feature = "pcap")]
pub mod pcap;

#[cfg(feature = "power_assertion")]
pub mod power_assertion;

#[cfg(feature = "preboard")]
pub mod preboard;

#[cfg(feature = "idam")]
pub mod idam;

#[cfg(feature = "fetchsymbols")]
pub mod fetchsymbols;

#[cfg(feature = "ostrace")]
pub mod ostrace;

#[cfg(feature = "prepare")]
pub mod prepare;

#[cfg(feature = "restore")]
pub mod restore;

#[cfg(feature = "dproxy")]
pub mod dproxy;

#[cfg(feature = "webinspector")]
pub mod webinspector;

#[cfg(feature = "mobileactivation")]
pub mod mobileactivation;
pub mod simlocation;

#[cfg(feature = "imagemounter")]
pub mod imagemounter;

// Always-available modules
#[cfg(feature = "diagnostics")]
pub mod diagnostics;

#[cfg(test)]
mod tests {
    service_error!(
        MacroSmokeError,
        after {
        #[error("extra error: {0}")]
        Extra(String),
        },
    );

    #[test]
    fn service_error_macro_preserves_common_variants_and_display() {
        let io_error: MacroSmokeError =
            std::io::Error::from(std::io::ErrorKind::Interrupted).into();
        assert!(matches!(io_error, MacroSmokeError::Io(_)));

        // Plist variant now wraps plist::Error via #[from]
        let plist_err: MacroSmokeError =
            plist::from_bytes::<plist::Value>(br#"<?xml version="1.0"?><plist><dict>"#)
                .unwrap_err()
                .into();
        assert!(matches!(plist_err, MacroSmokeError::Plist(_)));
        assert!(plist_err.to_string().starts_with("plist error: "));

        assert_eq!(
            MacroSmokeError::Protocol("bad frame".into()).to_string(),
            "protocol error: bad frame"
        );
        assert_eq!(
            MacroSmokeError::Extra("specific".into()).to_string(),
            "extra error: specific"
        );
    }
}

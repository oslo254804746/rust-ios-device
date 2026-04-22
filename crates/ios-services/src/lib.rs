//! ios-services: Feature-gated iOS device service implementations.
//!
//! Enable features in your `Cargo.toml` to pull in only the services you need:
//!
//! ```toml
//! [dependencies]
//! ios-services = { version = "0.1", features = ["afc", "syslog", "screenshot"] }
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

pub mod backup2;
pub mod device_link;

#[cfg(feature = "afc")]
pub mod afc;

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

pub mod mobileactivation;
pub mod simlocation;

#[cfg(feature = "imagemounter")]
pub mod imagemounter;

// Always-available modules
pub mod diagnostics;

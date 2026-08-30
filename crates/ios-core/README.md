# ios-core

High-level Rust API for discovering, pairing, connecting to, and controlling
iOS devices over usbmuxd, lockdown, CoreDevice/RemoteXPC, and the common
Apple device services.

This is the library crate at the center of the
[`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device)
workspace. The `ios` CLI, the PyO3 Python bindings, and the C FFI all build on
top of it.

## Highlights

- Device discovery and event watching across usbmuxd and Bonjour/mDNS.
- Lockdown client, TLS sessions, pair records, SRP pairing, and supervised
  pairing helpers.
- iOS 17+ CoreDevice support: CDTunnel (userspace and kernel TUN modes),
  Remote Service Discovery (RSD), HTTP/2 RemoteXPC transport, appservice,
  iconservice, screencapture, fileservice, diagnosticsservice, deviceinfo,
  InstallCoordinationProxy, Instruments, and TestManager.
- 30+ feature-gated service modules covering AFC, syslog, screenshots,
  configuration/provisioning profiles, crash reports, Instruments, debugserver,
  WebInspector, ImageMounter, pcap, and more.
- High-level connection API used by the CLI, Python, and FFI consumers.

## Install

```toml
[dependencies]
ios-core = "0.1.13"
```

The crate ships **no default service features**. Pick the services you need,
or use a grouped flag:

```toml
ios-core = { version = "0.1.13", features = ["afc", "syslog"] }
```

| Group        | Includes                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| `classic`    | afc, apps, crashreport, diagnostics, file_relay, heartbeat, house_arrest, installation, mcinstall, mobileactivation, notificationproxy, profiles, screenshot, springboard, syslog |
| `developer`  | accessibility_audit, amfi, debugserver, dproxy, dtx, fetchsymbols, imagemounter, instruments, pcap, testmanager, webinspector |
| `management` | arbitration, companion, idam, misagent, power_assertion, preboard, prepare, restore                   |
| `ios17`      | apps, deviceinfo, diagnosticsservice, dproxy, fileservice, iconservice, installcoordination, instruments, orientation, pasteboard, screencapture, testmanager, mdns, tunnel-userspace |
| `full`       | classic + developer + ios17 + management + ostrace + supervised-pair + tunnel-kernel                  |

CoreDevice service availability is **service-surface dependent**: a device can
expose USB, lockdown, tunnel, and RSD while still omitting a specific
CoreDevice feature service. Inspect the RSD service list at runtime instead
of relying on `ProductVersion` alone.

The `companion` service implements the classic and RSD companion-proxy plist
commands for registry listing/lookup, event listening, and port forwarding.
`start_forwarding_service_port` returns the device-assigned
`CompanionProxyServicePort`; use `start_forwarding_service_port_handle` for a
bounded RAII lifetime and explicit idempotent stop. The protocol stops a
forward by companion (remote) port rather than a reusable connection ID.

`ios_core::RsdHandshake::services` maps names to `ServiceDescriptor` values
with a device port and advertised capability identifiers. Missing RSD feature
metadata is represented by an empty list and remains permissive through
`supports_feature`. Prefer `ServiceDescriptor::new(port)` over struct literals
so downstream code is resilient when this public descriptor gains fields.
When upgrading from 0.1.8, existing struct literals must add the new
`features` field or switch to that constructor.

## Example

```rust,no_run
use ios_core::{ConnectOptions, list_devices};

# async fn run() -> anyhow::Result<()> {
let devices = list_devices().await?;
println!("found {} device(s)", devices.len());

if let Some(device) = devices.first() {
    let connected = ios_core::connect(
        &device.udid,
        ConnectOptions { skip_tunnel: true, ..Default::default() },
    )
    .await?;
    let version = connected.product_version().await?;
    println!("{} runs iOS {}", connected.info.udid, version);
}
# Ok(())
# }
```

For lower-level access, use the modules re-exported at the crate root:
`ios_core::mux`, `ios_core::lockdown`, `ios_core::xpc`, plus the gated service
modules (`ios_core::afc`, `ios_core::apps`, `ios_core::syslog`, etc.) that
become available with the matching feature.

## Requirements

- Rust **1.80+**.
- usbmux on the host (usbmuxd on Linux, Apple Mobile Device Support on
  Windows, Apple device support on macOS).
- A trusted iOS device for most real-device operations.
- iOS 17+ device with the CoreDevice tunnel surface for RSD/RemoteXPC features.

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-core>
- Feature flags: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/features.md>
- Architecture: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/architecture.md>
- CoreDevice tunnel: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/tunnel.md>

## License

Licensed under either of Apache-2.0 or MIT at your option.

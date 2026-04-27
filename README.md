# rust-ios-device

English | [简体中文](README.zh-CN.md)

Rust libraries and a command-line tool for communicating with iOS devices through usbmuxd, lockdown, CoreDevice/RemoteXPC, and common device services.

The project is currently **experimental**. It is useful for development, testing, and protocol work, but the API and CLI may change before a stable release. Some services require a real, trusted device and may vary by iOS version, host operating system, pairing state, and installed Apple components.

## Features

- USB device discovery and event watching through usbmuxd.
- Lockdown client support, TLS sessions, pair records, and pairing helpers.
- Lockdown/usbmux service support for devices across multiple iOS generations.
- CoreDeviceProxy/CDTunnel support for iOS versions that expose the CoreDevice tunnel path, with userspace and kernel TUN modes.
- Remote Service Discovery (RSD), HTTP/2 XPC transport, OPACK, NSKeyedArchiver, AFC, DTX, lockdown, usbmuxd, and XPC protocol codecs.
- CLI commands for device info, pairing, file operations, app management, syslog, screenshots, diagnostics, provisioning/configuration profiles, crash reports, Instruments, WebInspector, debugserver, backup/restore helpers, and tunnel management.
- Feature-gated service clients for AFC, apps, syslog, screenshot, DTX/Instruments, TestManager, accessibility audit, developer disk image mounting, pcap, WebInspector, and related services.
- Python bindings (`rust-ios-device-tunnel`, imported as `ios_rs`) for device listing and userspace tunnel workflows.
- C FFI bindings for device listing, lockdown queries, and tunnel metadata.

## Non-goals and limitations

- This is not an Apple-supported SDK and does not replace Xcode, Finder, Apple Configurator, or official MDM tooling.
- Not every command is validated on every iOS version. Some advanced commands are best treated as protocol experiments.
- CoreDevice and tunnel paths require a trusted device, compatible iOS version, and the correct pairing material.
- Kernel TUN mode may require administrator/root privileges. Userspace mode is usually easier to run.
- Some services require Developer Mode, a mounted Developer Disk Image, installed test bundles, supervision, or app-specific entitlements.
- Commands that modify device state can be disruptive. Read command help before using profile, erase, restore, backup restore, location, preboard, and supervision-related commands.

## Repository layout

| Crate | Purpose |
| --- | --- |
| `ios-core` | Public Rust library. Contains protocol codecs, usbmuxd, lockdown, tunneling, XPC/RSD, feature-gated service clients, discovery, pairing, and high-level device APIs. |
| `ios-cli` | `ios` command-line tool. |
| `ios-py` | PyO3 Python extension module. Not currently published to crates.io. |
| `ios-ffi` | C ABI wrapper. Not currently published to crates.io. |

## Requirements

- Rust 1.75 or newer.
- A trusted iOS device for most real-device operations.
- Host support for usbmux:
  - macOS: Apple device support is normally available with Xcode/Finder components.
  - Linux: install and run `usbmuxd`; udev permissions may be required.
  - Windows: install Apple Mobile Device Support, typically through iTunes or Apple Devices.
- OpenSSL development headers may be needed for some builds on Linux. The CI installs `libssl-dev` and `pkg-config`.
- Python 3.9+ and `maturin` are required only for `ios-py`.

## Build

```sh
cargo build --workspace --exclude ios-py
cargo build --release --package ios-cli
```

Run the CLI from source:

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- --help
```

The release binary is named `ios`.

## Feature flags

`ios-core` has no default service features. Enable only the service clients you use:

```toml
[dependencies]
ios-core = { version = "0.1.2", features = ["afc", "syslog"] }
```

For broader tools, use grouped features such as `classic`, `developer`, `management`, `ios17`, or `full`. The CLI enables `full`; libraries should usually choose a smaller set. See [docs/features.md](docs/features.md).

## Quick start

List visible devices:

```sh
ios list
```

Read basic device information:

```sh
ios -u <UDID> info
ios -u <UDID> lockdown get --key ProductVersion
```

Stream syslog:

```sh
ios -u <UDID> syslog
```

Capture a screenshot:

```sh
ios -u <UDID> screenshot --output screenshot.png
```

Explore command-specific options:

```sh
ios tunnel --help
ios file --help
ios apps --help
ios instruments --help
```

## CoreDevice tunnel

Start a tunnel for a trusted device:

```sh
ios -u <UDID> tunnel start --userspace
```

Run the tunnel manager HTTP service:

```sh
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

Userspace tunnels expose a local TCP proxy. Clients send a 16-byte IPv6 address followed by a 4-byte little-endian port before proxying traffic.

## Library example

```rust
use ios_core::{ConnectOptions, list_devices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = list_devices().await?;
    let Some(device) = devices.first() else {
        println!("no device found");
        return Ok(());
    };

    let connected = ios_core::connect(&device.udid, ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    }).await?;

    let version = connected.product_version().await?;
    println!("{} runs iOS {}", connected.info.udid, version);
    Ok(())
}
```

For lower-level access, use the modules exposed by `ios-core`, such as `ios_core::mux`,
`ios_core::lockdown`, `ios_core::xpc`, and service modules re-exported at the crate root
like `ios_core::afc`, `ios_core::apps`, and `ios_core::syslog` when their features are enabled.

## Python binding

Install the published package:

```sh
uv pip install rust-ios-device-tunnel
```

Build and install the local Python module from a checkout:

```sh
cd crates/ios-py
uvx maturin develop
```

Example:

```python
import ios_rs

devices = ios_rs.list_devices()
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
print(tunnel.connect_info())

with tunnel.asyncio_proxy():
    # asyncio.open_connection() calls to the device tunnel address are routed
    # through the userspace proxy while this context is active.
    pass

tunnel.close()
```

## Examples

The CLI crate contains Rust examples:

```sh
cargo run -p ios-cli --example device_info -- <UDID>
cargo run -p ios-cli --example app_list -- <UDID>
cargo run -p ios-cli --example file_transfer -- <UDID>
cargo run -p ios-cli --example screenshot -- <UDID>
cargo run -p ios-cli --example syslog_stream -- <UDID>
cargo run -p ios-cli --example instruments_cpu -- <UDID>
```

Exact arguments may vary by example; use `--help` or read the example source if a command needs additional paths.

## Troubleshooting

- `No such file or directory` or connection refused for usbmuxd: ensure usbmuxd or Apple Mobile Device Support is installed and running.
- Device does not appear: unlock the device, trust the host, reconnect USB, and check host permissions.
- Pairing fails: remove stale pair records only if you understand the impact, then pair again from an unlocked device.
- Tunnel fails on some devices: confirm the device/iOS version exposes the CoreDevice tunnel service; use lockdown/usbmux commands for older service paths.
- Kernel tunnel fails: retry userspace mode or run with the privileges required to create a TUN interface.
- Developer services fail: enable Developer Mode where required and mount an appropriate Developer Disk Image if the service depends on it.

See [docs/troubleshooting.md](docs/troubleshooting.md) for more detail.

## Roadmap

- Improve real-device validation across macOS, Linux, and Windows.
- Stabilize high-level Rust APIs and document service-level contracts.
- Expand examples for common workflows.
- Harden tunnel, RemoteXPC, and developer service compatibility across iOS versions.
- Improve Python and C binding packaging.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, testing expectations, and PR guidance.

## Security

Please report vulnerabilities privately. See [SECURITY.md](SECURITY.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Acknowledgements

This project is informed by the broader iOS device tooling ecosystem. Special thanks to:

- [go-ios](https://github.com/danielpaulus/go-ios.git)
- [pymobiledevice3](https://github.com/doronz88/pymobiledevice3.git)

Compatibility is implemented only where this repository's code and tests support it.

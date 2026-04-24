# rust-ios-device

Rust libraries and a command-line tool for communicating with iOS devices through usbmuxd, lockdown, CoreDevice/RemoteXPC, and common device services.

The project is currently **experimental**. It is useful for development, testing, and protocol work, but the API and CLI may change before a stable release. Some services require a real, trusted device and may vary by iOS version, host operating system, pairing state, and installed Apple components.

## Features

- USB device discovery and event watching through usbmuxd.
- Lockdown client support, TLS sessions, pair records, and pairing helpers.
- iOS 17+ tunnel support through CoreDeviceProxy/CDTunnel, with userspace and kernel TUN modes.
- Remote Service Discovery (RSD), HTTP/2 XPC transport, OPACK, NSKeyedArchiver, AFC, DTX, lockdown, usbmuxd, and XPC protocol codecs.
- CLI commands for device info, pairing, file operations, app management, syslog, screenshots, diagnostics, provisioning/configuration profiles, crash reports, Instruments, WebInspector, debugserver, backup/restore helpers, and tunnel management.
- Feature-gated service crates for AFC, apps, syslog, screenshot, DTX/Instruments, TestManager, accessibility audit, developer disk image mounting, pcap, WebInspector, and related services.
- Python bindings (`rust-ios-device-tunnel`, imported as `ios_rs`) for device listing and iOS 17+ userspace tunnels.
- C FFI bindings for device listing, lockdown queries, and tunnel metadata.

## Non-goals and limitations

- This is not an Apple-supported SDK and does not replace Xcode, Finder, Apple Configurator, or official MDM tooling.
- Not every command is validated on every iOS version. Some advanced commands are best treated as protocol experiments.
- iOS 17+ CoreDevice and tunnel paths require a trusted device and the correct pairing material.
- Kernel TUN mode may require administrator/root privileges. Userspace mode is usually easier to run.
- Some services require Developer Mode, a mounted Developer Disk Image, installed test bundles, supervision, or app-specific entitlements.
- Commands that modify device state can be disruptive. Read command help before using profile, erase, restore, backup restore, location, preboard, and supervision-related commands.

## Repository layout

| Crate | Purpose |
| --- | --- |
| `ios-proto` | Protocol types and codecs for AFC, DTX, lockdown, usbmuxd, XPC, OPACK, TLV, and related formats. |
| `ios-mux` | usbmuxd client for discovery, attach/detach events, and port connections. |
| `ios-lockdown` | Lockdown protocol, TLS sessions, pair records, pairing, and supervised pairing helpers. |
| `ios-tunnel` | CDTunnel handshake and userspace/kernel TUN forwarding. |
| `ios-xpc` | HTTP/2 + RemoteXPC client and RSD handshake. |
| `ios-services` | Feature-gated clients for device services such as AFC, syslog, apps, DTX/Instruments, and WebInspector. |
| `ios-core` | Higher-level device discovery, connection, pairing transport, and service access API. |
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

## iOS 17+ tunnel

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

For lower-level access, use `ios-mux`, `ios-lockdown`, `ios-services`, and `ios-xpc` directly.

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
- Tunnel fails on older iOS versions: CoreDevice tunnel support is primarily for iOS 17+ paths.
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

This project is informed by the broader iOS device tooling ecosystem, including libimobiledevice, go-ios, and pymobiledevice3. Compatibility is implemented only where this repository's code and tests support it.

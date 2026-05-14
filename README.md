# rust-ios-device

English | [简体中文](README.zh-CN.md)

Rust libraries, bindings, and an `ios` command-line tool for working with real
iOS devices through usbmuxd, lockdown, CoreDevice tunnels, Remote Service
Discovery (RSD), RemoteXPC, and common Apple device services.

This project is **experimental but already broad**: it is useful for device
automation, protocol research, developer tooling, diagnostics, and compatibility
checks against workflows from `go-ios` and `pymobiledevice3`. The API and CLI can
still change before a stable release, and many service surfaces depend on the
device, iOS version, trust state, Developer Mode, supervision, and installed
Apple components.

## What is in this workspace

| Entry point | Purpose |
| --- | --- |
| `ios-core` | Rust library with discovery, pairing, lockdown, usbmux, tunnel, XPC/RSD, protocol codecs, and feature-gated service clients. |
| `ios-cli` | End-user CLI binary named `ios`; enables the full `ios-core` service surface. |
| `ios-py` | PyO3 module published as `rust-ios-device-tunnel` and imported as `ios_rs`; focused on device listing and CoreDevice tunnel workflows. |
| `ios-ffi` | C ABI wrapper that builds static/shared libraries and the `ios_rs.h` header. |
| `docs/` | Task guides for build, usage, architecture, feature flags, CLI mapping, tunneling, protocols, Python bindings, and troubleshooting. |

## Capability overview

`rust-ios-device` currently covers these major areas:

- Device discovery over usbmuxd and Bonjour/mDNS, plus attach/detach event
  listening.
- Lockdown access, TLS sessions, pair records, SRP pairing, service startup, and
  selected device settings.
- Classic lockdown/usbmux services: AFC, House Arrest, crash reports,
  diagnostics relay, file relay, heartbeat, installation/app management,
  notification proxy, profiles, provisioning profiles, screenshots, SpringBoard,
  syslog, backup helpers, and related management services.
- iOS 17+ CoreDevice workflows: CDTunnel, userspace and kernel tunnel modes, RSD
  service inspection, RemoteXPC/HTTP2 transport, appservice, fileservice,
  diagnosticsservice, deviceinfo, Instruments, TestManager, and forwarding where
  the device exposes the required services.
- Developer workflows: Developer Disk Image mounting, DTX/Instruments,
  debugserver helpers, WebInspector, XCTest launching, WebDriverAgent helpers,
  accessibility audit, packet capture, symbols, os_trace, process control, and
  induced device-state conditions.
- Device management and supervised-device helpers: activation state, AMFI
  developer-mode helpers, arbitration, companion devices, global HTTP proxy,
  IDAM, power assertions, preboard, prepare/supervision certificate helpers,
  restore-mode event helpers, erase, and restore entry points.
- Protocol building blocks: usbmuxd, lockdown, AFC, DTX, OPACK,
  NSKeyedArchiver, XPC, HTTP/2 XPC, TLV, TLS/PSK, and tunnel packet forwarding.
- Python and C integration surfaces for tooling that needs to reuse discovery or
  tunnel support outside Rust.

The short version: use the CLI for day-to-day inspection and automation, use
`ios-core` when building Rust tooling, use `ios_rs` when you need a Python
userspace tunnel bridge, and use `ios-ffi` for C-compatible consumers.

## Requirements

- Rust 1.80 or newer.
- A trusted physical iOS device for most real-device operations.
- Host usbmux support:
  - macOS: Apple device support from Finder/Xcode is usually enough.
  - Linux: install and run `usbmuxd`; udev permissions may be required.
  - Windows: install Apple Mobile Device Support, usually via iTunes or Apple
    Devices.
- Linux builds may need OpenSSL development files such as `libssl-dev` and
  `pkg-config`.
- Windows builds that use OpenSSL are expected to link through vcpkg with
  `x64-windows-static-md`.
- Python 3.9+ and `maturin` are only needed for `ios-py`.

## Build from source

```sh
cargo build --workspace --exclude ios-py
cargo build --release --package ios-cli
```

Run the CLI from the checkout:

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- --help
```

The release binary is named `ios`.

Most CLI commands print JSON by default for scripting. Pass `--no-json` when a
human-readable table/text mode is available. Commands that target a device use
the first device from `ios list` when `-u/--udid` is omitted; set `IOS_UDID` or
pass `-u <UDID>` to choose explicitly.

## Quick start

```sh
ios list
ios info
ios lockdown get --key ProductVersion
ios syslog
ios screenshot --output screenshot.png
```

Explore command groups:

```sh
ios file --help
ios apps --help
ios diagnostics --help
ios tunnel --help
ios instruments --help
ios prepare --help
```

## Common CLI workflows

| Workflow | Representative commands |
| --- | --- |
| Discovery and pairing | `list`, `listen`, `discover`, `pair`, `lockdown` |
| Device facts and settings | `info`, `diskspace`, `mobilegestalt`, `batterycheck`, `batteryregistry`, `activation`, `amfi` |
| Files and containers | `file`, `file --app`, `file --coredevice`, `crash`, `file-relay` |
| Apps and tests | `apps list/install/uninstall/launch/kill`, `runtest`, `runwda`, `wda` |
| Logs and diagnostics | `syslog`, `diagnostics`, `os-trace`, `notify`, `pcap` |
| Developer services | `ddi`, `instruments`, `debugserver`, `debug`, `symbols`, `accessibility-audit`, `webinspector`, `devicestate`, `memlimitoff` |
| iOS 17+ transport | `tunnel start`, `tunnel serve`, `tunnel list`, `rsd services`, `rsd check`, `forward` |
| Management and supervision | `profiles`, `provisioning`, `prepare`, `httpproxy`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` |

Task-focused examples live in [docs/usage.md](docs/usage.md). A go-ios /
pymobiledevice3 comparison lives in [docs/cli-map.md](docs/cli-map.md).

## CoreDevice, RSD, and fileservice notes

iOS 17+ support is service-surface dependent. A device can have working USB,
lockdown, tunnel, RSD, AFC, and InstallationProxy while still not exposing a
specific CoreDevice service.

For example, CoreDevice fileservice uses:

- `com.apple.coredevice.fileservice.control`
- `com.apple.coredevice.fileservice.data`

Check availability before assuming an implementation bug:

```sh
ios rsd services --all
ios rsd check com.apple.coredevice.fileservice.control
ios file --coredevice --domain temporary ls /
```

If RSD does not expose the fileservice control/data services, the CLI should
report a clear missing-service error. That behavior matches current reference
tooling rather than falling back to a different service name.

## Tunnels

Start one CoreDevice tunnel:

```sh
ios tunnel start --userspace
```

Run the local tunnel manager used by integration tools:

```sh
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

Userspace mode is the recommended first choice. It exposes a local TCP proxy
where clients send a 16-byte IPv6 address followed by a 4-byte little-endian
port before proxying traffic. Kernel TUN mode is also available, but may require
administrator/root privileges.

See [docs/tunnel.md](docs/tunnel.md) for details.

## Rust API

`ios-core` has no default service features. Enable only the services your tool
uses:

```toml
[dependencies]
ios-core = { version = "0.1.5", features = ["afc", "syslog"] }
```

Use grouped features for wider tools: `classic`, `developer`, `management`,
`ios17`, or `full`. The CLI uses `full`; library users usually should not.

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

Feature details are in [docs/features.md](docs/features.md). Architecture notes
are in [docs/architecture.md](docs/architecture.md).

## Python binding

Install the Python package:

```sh
pip install rust-ios-device-tunnel
```

Build the local module from this checkout:

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

The example bridge in
`crates/ios-py/examples/pymobiledevice3_coredevice_bridge.py` shows how to run
pymobiledevice3 RemoteXPC code over the Rust userspace tunnel.

## C FFI

Build the C-compatible library and header:

```sh
cargo build --release -p ios-ffi
```

The FFI crate exposes device listing, pairing/service access, and tunnel
lifecycle functions for consumers that cannot call the Rust API directly.

## Examples

The CLI crate includes Rust examples:

```sh
cargo run -p ios-cli --example device_info -- <UDID>
cargo run -p ios-cli --example app_list -- <UDID>
cargo run -p ios-cli --example file_transfer -- <UDID>
cargo run -p ios-cli --example screenshot -- <UDID>
cargo run -p ios-cli --example syslog_stream -- <UDID>
cargo run -p ios-cli --example instruments_cpu -- <UDID>
```

Exact arguments may vary by example; inspect the source or command output if an
example expects paths or app identifiers.

## Safety and limitations

- This is not an Apple-supported SDK and does not replace Xcode, Finder, Apple
  Configurator, or official MDM tooling.
- Not every command has been validated on every iOS release or host OS.
- Some services require Developer Mode, a mounted Developer Disk Image,
  supervision, installed test bundles, or app-specific entitlements.
- Commands such as `erase`, `restore`, `prepare`, `httpproxy`, `location`,
  `preboard`, profile management, and backup restore paths can change device
  state. Prefer test devices and read `--help` first.
- Pair records and supervision credentials are sensitive. Do not commit them or
  include them in logs.

## Troubleshooting

- Device not visible: unlock it, trust the host, reconnect USB, and verify
  usbmuxd or Apple Mobile Device Support.
- Pairing fails: remove stale pair records only if you understand the impact,
  then pair again from an unlocked device.
- Tunnel fails: verify the device exposes the required CoreDevice tunnel/RSD
  services; fall back to classic lockdown/usbmux services where appropriate.
- CoreDevice fileservice fails: inspect RSD for the control/data service names
  before assuming the implementation is wrong.
- Kernel tunnel fails: retry userspace mode or run with privileges needed to
  create a TUN interface.
- Developer services fail: enable Developer Mode and mount a compatible
  Developer Disk Image where the service requires it.

More detail is in [docs/troubleshooting.md](docs/troubleshooting.md).

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for
development setup, tests, formatting, linting, and PR expectations.

Useful checks:

```sh
cargo build --workspace --exclude ios-py
cargo test --workspace --exclude ios-core --exclude ios-py
cargo test -p ios-core --all-features
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Acknowledgements

This project is informed by the broader iOS device tooling ecosystem, especially:

- [go-ios](https://github.com/danielpaulus/go-ios.git)
- [pymobiledevice3](https://github.com/doronz88/pymobiledevice3.git)

Compatibility is implemented only where this repository's code and tests support
it.

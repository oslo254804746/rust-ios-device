# rust-ios-device

English | [简体中文](README.zh-CN.md)

Rust libraries, language bindings, and the `ios` command-line tool for talking
to real iOS devices over usbmuxd, lockdown, CoreDevice/RemoteXPC, and the
common Apple device services.

[![Crates.io — ios-core](https://img.shields.io/crates/v/ios-core.svg?label=ios-core)](https://crates.io/crates/ios-core)
[![Crates.io — ios-cli](https://img.shields.io/crates/v/ios-cli.svg?label=ios-cli)](https://crates.io/crates/ios-cli)
[![PyPI — rust-ios-device-tunnel](https://img.shields.io/pypi/v/rust-ios-device-tunnel.svg?label=rust-ios-device-tunnel)](https://pypi.org/project/rust-ios-device-tunnel/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.80-orange.svg)](#requirements)

> **Status: experimental.** The project covers a wide capability surface and is
> useful for automation, protocol work, and developer tooling, but the public
> API and CLI may still change before a 1.0 release. Service availability
> depends on iOS version, pairing/trust state, Developer Mode, supervision,
> and the host's Apple Mobile Device components.

## Highlights

- **Cross-platform CLI (`ios`)** with 54+ subcommands covering devices, files,
  apps, instruments, debugging, profiles, restore, supervision, and tunnels.
- **iOS 17+ first-class support** — CoreDevice tunnel (userspace and kernel
  TUN), RSD service discovery, RemoteXPC over HTTP/2, appservice, fileservice,
  diagnosticsservice, deviceinfo, pasteboard, CoreDevice configuration/orientation,
  Instruments, and TestManager.
- **Lockdown-era services** — AFC, House Arrest, syslog, screenshots,
  configuration/provisioning profiles, crash reports, diagnostics relay,
  notification proxy, springboard, backup, and more.
- **Developer workflows** — Developer Disk Image mounting, DTX/Instruments,
  debugserver, WebInspector, XCTest runner, WebDriverAgent helpers,
  accessibility audit, pcap, and symbol fetching.
- **Multi-language consumers** — pure Rust library (`ios-core`),
  PyO3 Python module (`ios_rs`), and C FFI (`ios-ffi`) sharing one
  implementation.

## Workspace

| Crate    | Artifact                                            | Purpose                                                                 |
| -------- | --------------------------------------------------- | ----------------------------------------------------------------------- |
| `ios-core` | crates.io                                          | Library: discovery, pairing, lockdown, tunnel, RSD/XPC, service clients |
| `ios-cli`  | crates.io · prebuilt `ios` binary                  | End-user command-line tool                                              |
| `ios-py`   | PyPI as `rust-ios-device-tunnel` (import `ios_rs`) | Python bindings (PyO3) for device listing and tunnel workflows          |
| `ios-ffi`  | prebuilt `cdylib` + `staticlib` + `ios_rs.h`       | C ABI for non-Rust consumers                                            |

Detailed docs live in [`docs/`](docs/): [architecture](docs/architecture.md),
[build](docs/build.md), [features](docs/features.md),
[usage](docs/usage.md), [CLI map](docs/cli-map.md),
[tunnel](docs/tunnel.md), [protocol](docs/protocol.md),
[python binding](docs/python-binding.md), and
[troubleshooting](docs/troubleshooting.md).

## Install

### Pre-built CLI binary

Download the latest `ios-<version>-<target>.{tar.gz,zip}` from the
[Releases page](https://github.com/oslo254804746/rust-ios-device/releases).
Targets published per release:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

A matching `ios-ffi-*` archive ships the FFI library and the `ios_rs.h` header
for the same targets. Each asset has a sibling `.sha256` file.

### From crates.io

```sh
cargo install ios-cli            # installs the `ios` binary
```

```toml
# Cargo.toml — pull in the library
[dependencies]
ios-core = { version = "0.1.9", features = ["classic"] }
```

### Python

```sh
pip install rust-ios-device-tunnel    # imported as `ios_rs`
```

## Quick start

```sh
ios list                                       # connected devices (USB + network)
ios info                                       # default device summary
ios -u <UDID> lockdown get --key ProductVersion
ios syslog                                     # stream device logs
ios screenshot --output screenshot.png
ios tunnel start --userspace                   # iOS 17+ CoreDevice tunnel
```

When a command targets a device and `-u/--udid` is omitted, the CLI uses the
first device returned by `ios list`. Override with `-u <UDID>` or set
`IOS_UDID` to pin a specific device. Most commands emit JSON by default; pass
`--no-json` for human-readable tables.

Explore each command group:

```sh
ios --help
ios apps --help
ios file --help
ios instruments --help
ios tunnel --help
ios prepare --help
```

## Capability matrix

The CLI groups closely follow the service modules in `ios-core`. The mapping
below also lists the comparable surface in [go-ios] and [pymobiledevice3] for
orientation.

| Area                        | `ios` commands                                                                                  | Comparable go-ios / pymobiledevice3                                       |
| --------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Discovery & pairing         | `list`, `listen`, `discover`, `pair`, `lockdown`                                                | go-ios `list`/`listen`/`pair`; pmd3 `usbmux`/`lockdown`/`bonjour`         |
| Device info & settings      | `info`, `mobilegestalt`, `diskspace`, `batterycheck`, `batteryregistry`, `activation`, `amfi`   | go-ios `info`/`mobilegestalt`; pmd3 `lockdown`/`amfi`/`activation`        |
| Files & containers          | `file` (AFC, app, CoreDevice), `crash`, `file-relay`                                            | go-ios `fsync`/`crash`; pmd3 `afc`/`crash`                                |
| Apps & UI tests             | `apps`, `runtest`, `runxctest`, `runwda`, `wda`, `springboard`                                  | go-ios `apps`/`install`/`launch`/`runtest`/`runwda`; pmd3 `apps`/dvt      |
| Diagnostics & logs          | `syslog`, `diagnostics` (reboot/shutdown), `os-trace` (including raw archive/collect), `notify`, `pcap`, `ip`, `btlogger` | go-ios `syslog`/`diagnostics`/`pcap`; pmd3 `syslog`/`diagnostics`/`pcap`/`btlogger` |
| Developer services          | `instruments`, `debugserver`, `debug`, `ddi`, `symbols`, `accessibility-audit`, `webinspector`, `devicestate`, `memlimitoff` | go-ios `instruments`/`debug`/`image`/`ax`; pmd3 `developer dvt`/`mounter`/`webinspector` |
| iOS 17+ transport           | `tunnel`, `rsd`, `forward`, `dproxy`                                                            | go-ios `tunnel`/`rsd`/`forward`; pmd3 RemoteXPC/tunnel                    |
| Device pasteboard           | `pasteboard get`, `pasteboard set TEXT`, `pasteboard set --url URL`                            | go-ios `pasteboard`; pmd3 CoreDevice `paste`/`copy`                     |
| CoreDevice configuration    | `device-control configuration get|set ...`                                                      | pmd3 CoreDevice configuration actions                                     |
| CoreDevice orientation      | `device-control orientation [left|right]`                                                        | pmd3 CoreDevice `rotate [left|right]`                                     |
| CoreDevice HID input        | `hid --confirm button ...` (Universal touch/keyboard requires Display/RTP authorization)        | pmd3 CoreDevice HID button/touch/keyboard helpers                          |
| Management & supervision    | `profiles`, `wifi`, `provisioning`, `prepare`, `httpproxy`, `mdm`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` | go-ios `profile`/`wifi`/`prepare`/`httpproxy`/`mdm`/`erase`; pmd3 `profile`/`provision`/`restore` |
| Backup, location, screen    | `backup`, `location`, `screenshot`, `notify`                                                    | go-ios/pmd3 `backup`/`location`/`screenshot`                              |

Task-focused walkthroughs: [`docs/usage.md`](docs/usage.md). Side-by-side
command map: [`docs/cli-map.md`](docs/cli-map.md).

## CoreDevice / iOS 17+ tunnel

iOS 17+ workflows route through a CoreDevice tunnel and a per-device RSD
service directory. Whether a specific feature is reachable depends on the
service surface that the device exposes — not on the iOS version alone.

```sh
# Start a single tunnel (default = userspace mode)
ios tunnel start --userspace

# Run the local tunnel manager HTTP service (go-ios compatible JSON)
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

Userspace tunnels publish a local TCP proxy. Clients send a 16-byte IPv6
address followed by a 4-byte little-endian port, then proxy traffic. Kernel
TUN mode is also available but typically requires admin/root.

Inspect what the device actually exposes before assuming an implementation
issue:

```sh
ios rsd services --all
ios rsd services --all --features
ios rsd check com.apple.coredevice.fileservice.control
ios file --coredevice --domain temporary ls /
```

RSD service listings are JSON by default and retain the legacy `name`/`port`
entry shape. Pass `--features` to add advertised `features` to JSON or to the
human-readable output used with `--no-json`; entries remain sorted by service
name. A missing feature list is emitted as `[]` when requested and means that
the device did not advertise capability metadata, not that every operation is
unsupported. `rsd check` follows the same opt-in rule.

TCP dials and initial tunnel/RSD/XPC setup are bounded by a 15-second timeout;
TCP-backed remote pairing and lockdown setup use the same bound. Stale tunnel
routes therefore fail promptly instead of waiting through the host's long SYN
retry window. Later service requests keep their own timeout behavior.

If a device does not expose the requested CoreDevice service (e.g. the
fileservice control/data pair), the CLI surfaces a clear missing-service
error rather than silently falling back. See
[`docs/tunnel.md`](docs/tunnel.md) for the full lifecycle.

## Library usage (Rust)

`ios-core` ships **no default service features**. Pick what you need, or use
a grouped flag:

```toml
[dependencies]
ios-core = { version = "0.1.9", features = ["afc", "syslog"] }
```

| Group        | Includes                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| `classic`    | afc, apps, crashreport, diagnostics, file_relay, heartbeat, house_arrest, installation, mcinstall, mobileactivation, notificationproxy, profiles, screenshot, springboard, syslog |
| `developer`  | accessibility_audit, amfi, btlogger, debugserver, dproxy, dtx, fetchsymbols, imagemounter, instruments, pcap, testmanager, webinspector |
| `management` | arbitration, companion, idam, misagent, power_assertion, preboard, prepare, restore                   |
| `ios17`      | apps, configuration, deviceinfo, diagnosticsservice, dproxy, fileservice, instruments, orientation, hid, pasteboard, testmanager, mdns, tunnel-userspace |
| `full`       | classic + developer + ios17 + management + ostrace + supervised-pair + tunnel-kernel + backup2-manifest |

The CLI builds with `full`; libraries should usually pick a smaller subset.

```rust
use ios_core::{ConnectOptions, list_devices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = list_devices().await?;
    let Some(device) = devices.first() else {
        println!("no device found");
        return Ok(());
    };

    let connected = ios_core::connect(
        &device.udid,
        ConnectOptions { skip_tunnel: true, ..Default::default() },
    )
    .await?;

    let version = connected.product_version().await?;
    println!("{} runs iOS {}", connected.info.udid, version);
    Ok(())
}
```

For lower-level access, use the modules re-exported at the crate root, such as
`ios_core::mux`, `ios_core::lockdown`, `ios_core::xpc`, and the gated service
modules `ios_core::afc`, `ios_core::apps`, `ios_core::syslog`, etc.

## Python bindings

```sh
pip install rust-ios-device-tunnel
```

Or build the local checkout into a venv:

```sh
cd crates/ios-py
uvx maturin develop
```

```python
import ios_rs

devices = ios_rs.list_devices()
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
print(tunnel.services)
print(tunnel.service_ports)    # service name -> device port
print(tunnel.service_features) # service name -> advertised identifiers (possibly [])
print(tunnel.connect_info())

with tunnel.asyncio_proxy():
    # asyncio.open_connection() to the device tunnel address is routed
    # through the userspace proxy while this context is active.
    ...

tunnel.close()
```

`crates/ios-py/examples/pymobiledevice3_coredevice_bridge.py` shows how to run
pymobiledevice3 RemoteXPC code on top of the Rust userspace tunnel.
`services`, `service_ports`, and `service_features` use stable service-name
ordering. Both mappings include every discovered service; `service_features`
uses `[]` when the RSD entry does not advertise capability metadata.

## C FFI

Build the C-compatible library and its header:

```sh
cargo build --release -p ios-ffi
```

Outputs include `libios_ffi.{so,dylib,a}` (or `ios_ffi.dll` + `.lib` on
Windows) and `crates/ios-ffi/include/ios_rs.h`. The FFI surface covers device
listing, lockdown queries, pairing/service access, and tunnel lifecycle.
`ios_tunnel_rsd_services_json` returns the discovered RSD service map as
deterministic compact JSON; each service value contains `port` and `features`.
Pre-built archives for the supported targets are attached to each release.

## Build from source

### Requirements

- Rust **1.80+** (workspace MSRV).
- usbmux on the host:
  - **macOS** — Apple device support from Xcode/Finder, normally pre-installed.
  - **Linux** — `usbmuxd` running with appropriate udev rules.
  - **Windows** — Apple Mobile Device Support, via iTunes or Apple Devices.
- OpenSSL development headers on Linux (`libssl-dev`, `pkg-config`).
- On Windows, OpenSSL is linked statically through vcpkg
  (`x64-windows-static-md`); set `VCPKG_ROOT`, `VCPKGRS_TRIPLET`,
  `OPENSSL_STATIC=1`.
- Python 3.9+ development headers for host-testing `ios-py`; `maturin` is also
  needed for wheel/development builds.

### Common commands

```sh
# Workspace build for the native crates
cargo build --workspace --exclude ios-py

# Release CLI binary
cargo build --release -p ios-cli

# Tests
cargo test --workspace --exclude ios-core --exclude ios-py
cargo test -p ios-core --all-features
# Host-side Python binding tests (requires Python development headers)
PYO3_PYTHON=/path/to/python cargo test -p ios-py

# Lint / format
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Run the CLI from a checkout:

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- --help
```

## Examples

The CLI crate ships runnable Rust examples:

```sh
cargo run -p ios-cli --example device_info     -- <UDID>
cargo run -p ios-cli --example app_list        -- <UDID>
cargo run -p ios-cli --example file_transfer   -- <UDID>
cargo run -p ios-cli --example screenshot      -- <UDID>
cargo run -p ios-cli --example syslog_stream   -- <UDID>
cargo run -p ios-cli --example instruments_cpu -- <UDID>
cargo run -p ios-cli --example afc_debug       -- <UDID>
```

Some examples take additional arguments (paths, bundle IDs); check the source
or run with `--help` first.

## Troubleshooting

- **Device not visible** — unlock the device, trust the host, reconnect USB,
  and verify usbmuxd / Apple Mobile Device Support is running.
- **Pairing failures** — only delete stale pair records when you understand
  the impact, then re-pair from an unlocked device.
- **Tunnel failures on older devices** — the device may not expose CoreDevice
  tunnel/RSD; fall back to lockdown/usbmux service paths.
- **Kernel tunnel fails** — retry with userspace mode or run with the
  privileges required to create a TUN interface.
- **Developer services fail** — enable Developer Mode and mount a compatible
  Developer Disk Image where the service requires it (`ios ddi`).
- **CoreDevice fileservice unavailable** — verify `com.apple.coredevice.fileservice.control`
  and `.data` are listed in `ios rsd services --all`. Absence is a device-side
  service-surface issue, not a client bug.

More detail: [`docs/troubleshooting.md`](docs/troubleshooting.md).

## Safety and limitations

- This is **not an Apple-supported SDK**. It does not replace Xcode, Finder,
  Apple Configurator, or official MDM tooling.
- Not every command is validated on every iOS version, host OS, or pairing
  state; some advanced commands are best treated as protocol experiments.
- Commands that mutate device state — `erase`, `restore`, `prepare`,
  `httpproxy`, `location`, `preboard`, profile install/remove, backup
  restore — can be disruptive. Read `--help` and prefer test devices.
- Pair records and supervision certificates are sensitive credentials. Do not
  commit them or write them into shared logs.

## Contributing

Contributions are welcome. Development setup, testing expectations, and PR
guidance live in [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports and feature
requests have templates in [`.github/ISSUE_TEMPLATE`](.github/ISSUE_TEMPLATE).

## Security

Please report vulnerabilities privately. See [SECURITY.md](SECURITY.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

## Acknowledgements

This project is informed by the broader iOS device tooling ecosystem.
Special thanks to:

- [go-ios](https://github.com/danielpaulus/go-ios)
- [pymobiledevice3](https://github.com/doronz88/pymobiledevice3)

Compatibility is implemented only where this repository's code and tests
support it.

[go-ios]: https://github.com/danielpaulus/go-ios
[pymobiledevice3]: https://github.com/doronz88/pymobiledevice3

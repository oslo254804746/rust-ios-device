# rust-ios-device

English | [简体中文](README.zh-CN.md)

Rust libraries, language bindings, and the `ios` command-line tool for talking
to real iOS devices over usbmuxd, lockdown, CoreDevice/RemoteXPC, and Apple
device services across **iOS 15.x – iOS 27.2**.

[![Crates.io — ios-core](https://img.shields.io/crates/v/ios-core.svg?label=ios-core)](https://crates.io/crates/ios-core)
[![Crates.io — ios-cli](https://img.shields.io/crates/v/ios-cli.svg?label=ios-cli)](https://crates.io/crates/ios-cli)
[![PyPI — rust-ios-device-tunnel](https://img.shields.io/pypi/v/rust-ios-device-tunnel.svg?label=rust-ios-device-tunnel)](https://pypi.org/project/rust-ios-device-tunnel/)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.80-orange.svg)](#requirements)

> **Status: experimental (`v0.1.17`).** The project covers a wide capability
> surface and is verified across **iOS 15.8.6 (Legacy)**, **iOS 18.6 / 18.7
> (Modern)**, and **iOS 27.2 (Modern)** physical devices, but the public API
> and CLI may still change before a 1.0 release. Service availability depends on
> iOS version, pairing/trust state, Developer Mode, supervision, and the host's
> Apple Mobile Device components.

The maintained [capability inventory](docs/capabilities.md) separates each
command/service's implementation from host, simulated, and real-device evidence.

## Highlights

- **Supported iOS versions: iOS 15.x – iOS 27.2** — tested across iOS 15.8.6
  Legacy (usbmuxd / Lockdown / DDI `.dmg`), iOS 18.6/18.7 Modern, and iOS 27.2
  Modern (CoreDevice / RemoteXPC / Personalized Cryptex DDI).
- **Cross-platform CLI (`ios`)** with 60+ subcommands covering devices, files,
  apps, instruments, debugging, media streaming, HID automation, WebInspector,
  profiles, restore, supervision, and tunnels.
- **CoreDevice Tunnel (iOS 17–27+)** — Kernel TUN and unprivileged **Userspace
  Tunnel** (`--userspace`), `tunnel start`, `tunnel serve`, a **20ms ICMPv6 Echo
  Request Nudge** for low-latency burst stability over `CDTunnel`, and IPv6 UDP
  datagram forwarding alongside TCP stream proxying.
- **Screen & Media Streaming (`device-control display` & `viewer`)** —
  - `ios device-control display status / video / audio`: iOS 27+
    `dtremotedisplayd` real-time **H.265 (HEVC)** and **AAC** RTP access-unit
    capture over Userspace Tunnel without Administrator privileges.
  - `ios viewer web` & `ios viewer vnc`: Interactive Web viewer (MJPEG stream,
    PNG frames, touch/key/button/orientation controls) and RFB 3.8 VNC server
    with bidirectional clipboard sync (`--share-clipboard`), supporting Pro Max
    16-bit Display P3 screenshots.
- **Universal HID (`ios hid`)** — `tap`, `swipe`, `button`, phased `touch`,
  5-finger `multitouch` (`--points`), `text`, and `key` with automatic 500ms
  touch authorization settle, 300ms virtual keyboard settle, and Shift modifier
  lead-in/lead-out for uppercase and shifted symbols.
- **WebInspector, CDP & WebDriver (`ios webinspector`)** — `opened-tabs`,
  `eval`, `launch`, `js-shell`, `cdp` (compatible with `puppeteer-core` and
  `playwright-core` `connectOverCDP`, supporting `Runtime`, `Page`, `Network`,
  `Fetch`, `Input`, `Performance`, and `HeapProfiler`), and W3C `webdriver`
  (compatible with `selenium-webdriver`).
- **File Management (`ios file` & CoreDevice `fileservice`)** — Full AFC client
  including extended opcodes (`connection-info`, `hash` with range, `pread`,
  `pwrite`, `truncate`, `set-time`, `dir-size`) and iOS 17+ CoreDevice
  `fileservice` (`--coredevice` `ls`, `mkdir`, `push`, `pull`, `mv`,
  `rm --recursive`).
- **Developer & Diagnostic Suite** — Cryptex DDI auto-install (`ddi auto`,
  `ddi cryptex`), Instruments (`ps`, `apps`, `cpu`, `sysmon-process`, `gpu`,
  `fps`, `kdebug`), CoreProfileSession Stackshot (`sharedCache` UUID
  extraction), `accessibility-audit` (`axAudit` DTX), `debug` &
  `debugserver send` (with automatic iOS 17+ RSD `debugproxy` fallback),
  `symbols`, `devicestate`, `memlimitoff`, `pcap --interface`, `btlogger`
  (`.pklg`/`.pcapng`), `pasteboard`, and `device-control configuration`
  (including iOS 26/27 `liquid-glass-opacity` and `increase-contrast`).
- **Multi-language consumers** — pure Rust library (`ios-core`), PyO3 Python
  module (`ios_rs`), and C FFI (`ios-ffi`) sharing one implementation.

## Workspace

| Crate    | Artifact                                            | Purpose                                                                 |
| -------- | --------------------------------------------------- | ----------------------------------------------------------------------- |
| `ios-core` | crates.io                                          | Library: discovery, pairing, lockdown, tunnel, RSD/XPC, service clients |
| `ios-cli`  | crates.io · prebuilt `ios` binary                  | End-user command-line tool                                              |
| `ios-py`   | PyPI as `rust-ios-device-tunnel` (import `ios_rs`) | Python bindings (PyO3) for device listing, sessions, and tunnels        |
| `ios-ffi`  | prebuilt `cdylib` + `staticlib` + `ios_rs.h`       | C ABI for non-Rust consumers                                            |

Detailed docs live in [`docs/`](docs/): [architecture](docs/architecture.md),
[build](docs/build.md), [features](docs/features.md),
[usage](docs/usage.md), [CLI map](docs/cli-map.md),
[tunnel](docs/tunnel.md), [viewer](docs/viewer.md), [protocol](docs/protocol.md),
[python binding](docs/python-binding.md),
[troubleshooting](docs/troubleshooting.md), and
[host output/cancellation contracts](docs/output-contract.md).
The [public compatibility rules](docs/public-contract.md) define maintained
candidate Rust/CLI/C/Python interfaces and migrations; host compatibility
fixtures do not certify a stable device workflow.
The [plist/JSON result contract](docs/plist-json-contract.md) records preserved
encodings and failed-stdout exits: result/help/version write failures return 1,
including a closed pipe, while successful output formats remain compatible.

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
ios-core = { version = "0.1.17", features = ["classic"] }
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
ios tunnel start --userspace                   # iOS 17–27+ CoreDevice tunnel
```

When a command targets a device and `-u/--udid` is omitted, the CLI uses the
first device returned by `ios list`. Override with `-u <UDID>` or set
`IOS_UDID` to pin a specific device. Most commands emit JSON by default; pass
`--no-json` for human-readable tables.

### Feature workflows at a glance

```sh
# Screen & Media Streaming (iOS 27+ H.265/AAC RTP & Interactive Web/VNC Viewer)
ios -u <UDID> device-control display video --output capture.h265 --duration-ms 5000
ios -u <UDID> device-control display audio --output capture.aac --duration-ms 5000
ios -u <UDID> viewer web --listen 127.0.0.1:8090 --share-clipboard
ios -u <UDID> viewer vnc --listen 127.0.0.1:5901 --share-clipboard

# Universal HID (tap, swipe, 5-finger multitouch, text with Shift lead-in/out)
ios -u <UDID> hid --confirm tap 0.5 0.5
ios -u <UDID> hid --confirm swipe 0.5 0.8 0.5 0.2 --duration-ms 300
ios -u <UDID> hid --confirm multitouch --points "1:0.20:0.40,2:0.35:0.35,3:0.50:0.30,4:0.65:0.35,5:0.80:0.40"
ios -u <UDID> hid --confirm text "Hello, iOS 27!"

# WebInspector, CDP (Puppeteer/Playwright) & W3C WebDriver (Selenium)
ios -u <UDID> webinspector opened-tabs
ios -u <UDID> webinspector cdp --host 127.0.0.1 --port 9222 --allow-loopback-without-auth
ios -u <UDID> webinspector webdriver --host 127.0.0.1 --port 4444 --allow-loopback-without-auth

# File Management (AFC extended opcodes & iOS 17+ CoreDevice fileservice)
ios -u <UDID> file hash /Books/Purchases/sample.plist --offset 0 --length 64
ios -u <UDID> file --coredevice --domain temporary ls /

# Developer & Diagnostic Suite
ios -u <UDID> ddi auto
ios -u <UDID> instruments fps --duration 5
ios -u <UDID> debugserver send --command qSupported
ios -u <UDID> device-control configuration set liquid-glass-opacity --mode tinted
```

## Capability matrix

The CLI groups closely follow the service modules in `ios-core`. The mapping
below also lists the comparable surface in [go-ios] and [pymobiledevice3] for
orientation.

| Area                        | `ios` commands                                                                                  | Comparable go-ios / pymobiledevice3                                       |
| --------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Discovery & pairing         | `list`, `listen`, `discover`, `pair`, `lockdown`, `doctor`                                      | go-ios `list`/`listen`/`pair`; pmd3 `usbmux`/`lockdown`/`bonjour`         |
| Device info & settings      | `info`, `mobilegestalt`, `diskspace`, `batterycheck`, `batteryregistry`, `activation` (state, session-info, info, activate, deactivate, itunes-activate), `amfi` | go-ios `info`/`mobilegestalt`; pmd3 `lockdown`/`amfi`/`activation`        |
| Files & containers          | `file` (AFC + extended opcodes `connection-info`/`hash`/`pread`/`pwrite`/`truncate`/`set-time`/`dir-size`, `--app` House Arrest, `--coredevice` `fileservice`), `crash`, `file-relay` | go-ios `fsync`/`crash`; pmd3 `afc`/`crash`/CoreDevice `fileservice`       |
| Apps & UI tests             | `apps` (including iOS 17+ `install-record`), `runtest`, `runxctest`, `runwda`, `wda`, `springboard` | go-ios `apps`/`install`/`launch`/`runtest`/`runwda`; pmd3 `apps`/dvt      |
| Diagnostics & logs          | `syslog`, `diagnostics` (reboot/shutdown), `os-trace` (including raw archive/collect), `notify`, `pcap` (`--interface`), `ip`, `btlogger` (`.pklg`/`.pcapng`) | go-ios `syslog`/`diagnostics`/`pcap`; pmd3 `syslog`/`diagnostics`/`pcap`/`btlogger` |
| Developer services          | `instruments` (`ps`, `apps`, `cpu`, `sysmon-process`, `gpu`, `fps`, `kdebug`), `debugserver` (Lockdown + RSD `debugproxy` fallback), `debug`, `ddi` (`auto`, `cryptex`), `symbols` (CoreProfileSession Stackshot + RSD), `accessibility-audit`, `webinspector` (`opened-tabs`, `eval`, `launch`, `js-shell`, `cdp`, `webdriver`), `devicestate`, `memlimitoff` | go-ios `instruments`/`debug`/`image`/`ax`; pmd3 `developer dvt`/`mounter`/`webinspector` |
| iOS 17–27+ transport        | `tunnel`, `rsd`, `forward`, `dproxy`                                                            | go-ios `tunnel`/`rsd`/`forward`; pmd3 RemoteXPC/tunnel                    |
| Device pasteboard           | `pasteboard get\|set\|watch` (Darwin notifications, fresh PULLs, and binary UTI data); experimental `resolve\|export --experimental` | go-ios `pasteboard`; pmd3 CoreDevice `paste`/`copy` and pasteboard monitoring |
| CoreDevice configuration    | `device-control configuration get\|set ...` (including iOS 26/27 `liquid-glass-opacity` and `increase-contrast`) | pmd3 CoreDevice configuration actions                                     |
| CoreDevice orientation      | `device-control orientation [left\|right]`                                                      | pmd3 CoreDevice `rotate [left\|right]`                                    |
| Universal HID input         | `hid --confirm tap\|swipe\|button\|touch\|multitouch\|text\|key` (authenticated Display/RTP touch authorization, 5-finger multitouch, Shift modifier sequences) | pmd3 CoreDevice HID button/touch/keyboard helpers                         |
| CoreDevice display media    | `device-control display status\|video\|audio` (iOS 27+ `dtremotedisplayd` real-time H.265/HEVC and AAC RTP access-unit capture over Userspace or Kernel tunnel) | pmd3 CoreDevice display/screen-stream helpers                             |
| Interactive viewer & VNC    | `viewer web\|vnc\|clipboard` (MJPEG/PNG Web UI, RFB 3.8 VNC server, 16-bit Display P3 decoding, native HID, `--share-clipboard`; see [viewer docs](docs/viewer.md)) | CoreDevice screencapture/HID/pasteboard workflows                         |
| Management & supervision    | `profiles`, `wifi`, `provisioning`, `prepare`, `httpproxy`, `mdm`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` | go-ios `profile`/`wifi`/`prepare`/`httpproxy`/`mdm`/`erase`; pmd3 `profile`/`provision`/`restore` |
| Backup, location, screen    | `backup`, `location`, `screenshot` (PNG / MJPEG stream), `notify`                               | go-ios/pmd3 `backup`/`location`/`screenshot`                              |

Task-focused walkthroughs: [`docs/usage.md`](docs/usage.md). Side-by-side
command map: [`docs/cli-map.md`](docs/cli-map.md).

Activation has separate device and network stages. `activation activate` uses
Apple's official HTTPS endpoints unless an explicit unsafe custom HTTPS endpoint
opt-in is supplied; TLS verification remains enabled. Offline records use
owner-only files, and activation records/identifiers are redacted from JSON and
human diagnostic output. Online activation waits for a fresh Tunnel1
nonce/session; `activation activate --now` skips that wait and performs one
session probe, so it should only be used when the daemon session is known to be
fresh. Destructive `activation deactivate` requires `--force`.

## CoreDevice / iOS 17–27+ tunnel

iOS 17–27+ workflows route through a CoreDevice tunnel and a per-device RSD
service directory. Whether a specific feature is reachable depends on the
service surface that the device exposes — not on the iOS version alone.

```sh
# Start a single tunnel (default = userspace mode)
ios tunnel start --userspace

# Run the local tunnel manager HTTP service (go-ios compatible JSON)
ios tunnel serve --userspace --host 127.0.0.1 --port 49151 --auth-config PRIVATE_POLICY_FILE
```

- **Userspace & Kernel modes** — Userspace tunnels work without Administrator /
  root privileges and publish a local proxy. Clients send a 16-byte IPv6 address
  followed by a 4-byte little-endian port, then proxy TCP streams or IPv6 UDP
  datagrams (used by `dtremotedisplayd` RTP video/audio streaming and HID touch
  authorization). Kernel TUN mode (`--kernel`) is also available when a system
  network interface is needed.
- **20ms ICMPv6 Echo Request Nudge** — The userspace `smoltcp` stack runs a 20ms
  ICMPv6 Echo Request Nudge when active TCP streams are awaiting device replies,
  ensuring timely `CDTunnel` packet flushing during high-frequency HTTP/2
  RemoteXPC and DTX bursts on iOS 18–27+.
- **Scoped manager policy** — `tunnel serve` requires a scoped policy or
  explicit loopback opt-in; create/check private credentials and use
  `--token-file` for clients as described in
  [management access](docs/management-access.md).

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
ios-core = { version = "0.1.17", features = ["afc", "syslog"] }
```

| Group        | Includes                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------- |
| `classic`    | afc, apps, crashreport, diagnostics, file_relay, heartbeat, house_arrest, installation, mcinstall, mobileactivation, notificationproxy, profiles, screenshot, springboard, syslog |
| `developer`  | accessibility_audit, amfi, btlogger, cryptex, debugserver, dproxy, dtx, fetchsymbols, imagemounter, instruments, pcap, testmanager, webinspector |
| `management` | arbitration, companion, idam, misagent, power_assertion, preboard, prepare, restore                   |
| `ios17`      | apps, configuration, cryptex, deviceinfo, diagnosticsservice, dproxy, fileservice, iconservice, installcoordination, instruments, orientation, screencapture, hid, display, pasteboard, testmanager, mdns, tunnel-userspace |
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

For lower-level access, use public types re-exported at the crate root:
`ios_core::MuxClient`, `ios_core::LockdownClient`, `ios_core::RsdHandshake`,
and `ios_core::XpcClient` (requires `tunnel`). The private transport modules
are implementation details. Feature-gated public service modules include
`ios_core::afc`, `ios_core::apps`, `ios_core::syslog`, etc.

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
if not devices:
    raise SystemExit("no device found")
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
try:
    print(tunnel.services)
    print(tunnel.service_ports)    # service name -> device port
    print(tunnel.service_features) # service name -> advertised identifiers (possibly [])
    print(tunnel.connect_info())

    with tunnel.asyncio_proxy():
        # asyncio.open_connection() to the device tunnel address is routed
        # through the userspace proxy while this context is active.
        print("userspace proxy active")

finally:
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
listing, existing-trust sessions (`IosDeviceSession`), lockdown queries, tunnel
lifecycle, RSD metadata, installed apps, AFC file operations, and syslog reads.
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

# Complete local/CI checks (Python 3.9+, Rust 1.80 toolchain, uv, C compiler)
python scripts/validate.py full --python /path/to/python
# Focused checks while developing
python scripts/validate.py quick --package ios-core
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
- **Kernel tunnel fails** — retry with `--userspace` mode or run with the
  privileges required to create a TUN interface.
- **Developer services fail** — enable Developer Mode and mount a compatible
  Developer Disk Image where the service requires it (`ios ddi auto`).
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

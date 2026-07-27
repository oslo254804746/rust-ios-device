# Getting started

This guide walks through installing the CLI, connecting a device, and running
the first commands.

## Install

The fastest path is the pre-built CLI:

```sh
# 1. Pre-built binary (no Rust toolchain required)
#    Download from https://github.com/oslo254804746/rust-ios-device/releases
#    Targets: x86_64-linux, aarch64-linux, aarch64-apple-darwin, x86_64-windows-msvc

# 2. From crates.io (requires Rust 1.80+)
cargo install ios-cli

# 3. From source (this checkout)
cargo build --release -p ios-cli
# Binary at target/release/ios
```

To use the library in a Rust project, add `ios-core` and pick the features
you need:

```toml
[dependencies]
ios-core = { version = "0.1.8", features = ["classic"] }
```

The Python binding is published as `rust-ios-device-tunnel` (imported as
`ios_rs`):

```sh
pip install rust-ios-device-tunnel
```

## Prerequisites

- A trusted iOS device connected over USB for the most reliable first test.
- usbmux support on the host:
  - **macOS** — Apple device support from Xcode/Finder is usually enough.
  - **Linux** — install and start `usbmuxd`; configure udev permissions if needed.
  - **Windows** — install Apple Mobile Device Support (via iTunes or Apple Devices).
- For source builds: Rust **1.80+** and OpenSSL development files where
  applicable. See [build.md](build.md) for full details, including Windows
  vcpkg setup.

## First commands

```sh
ios list
ios info
ios lockdown get --key ProductVersion
```

When a command targets a device and no UDID is specified, the CLI uses the
first device returned by `ios list`. For multiple devices, pass `-u <UDID>`
or set the environment variable:

```sh
export IOS_UDID=<UDID>     # macOS / Linux
$env:IOS_UDID = "<UDID>"   # Windows PowerShell
```

`ios list`, `ios listen`, and `ios discover` ignore the default UDID because
they enumerate devices.

Most commands emit JSON by default for scripting. Pass `--no-json` for a
human-readable table where the command supports it. Increase tracing detail
with `-v`, `-vv`, `-vvv`.

## Pairing

Most services require the device to trust the host. Keep the device unlocked
and accept the trust prompt if one appears.

Useful commands:

```sh
ios pair --help
ios pair show-record
ios lockdown info
ios lockdown save-pair-record pair-record.plist
```

Pair records are sensitive credentials that authorize device access. Do not
commit them to source control or include them in shared logs.

## A first iOS 17+ tunnel

For CoreDevice/RemoteXPC features, start a userspace tunnel:

```sh
ios tunnel start --userspace
ios -u <UDID> rsd services --all
```

If a specific CoreDevice feature looks unavailable, verify with
`ios rsd check <service-name>` first — the device may simply not expose that
service. See [tunnel.md](tunnel.md) for the full lifecycle.

## Next steps

- [Usage](usage.md) — CLI walkthroughs and Rust API examples per task family.
- [CLI map](cli-map.md) — go-ios and pymobiledevice3 command-family mapping.
- [Build](build.md) — workspace builds, packaging, Python/FFI notes.
- [Features](features.md) — feature flags for `ios-core`.
- [Tunnel](tunnel.md) — CoreDevice tunnel modes and userspace proxy protocol.
- [Troubleshooting](troubleshooting.md) — connection, pairing, and build issues.


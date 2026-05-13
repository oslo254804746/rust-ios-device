# Getting started

This guide covers building from source and running the CLI locally. Pre-built binaries are available from [GitHub Releases](https://github.com/oslo254804746/rust-ios-device/releases), and the Python package can be installed via `pip install rust-ios-device-tunnel`.

## Prerequisites

- Rust 1.80 or newer.
- A trusted iOS device connected over USB for the most reliable first test.
- usbmux support on the host:
  - macOS: Apple device support from Xcode/Finder is usually enough.
  - Linux: install and start `usbmuxd`; configure udev permissions if needed.
  - Windows: install Apple Mobile Device Support.
- OpenSSL development files on Linux when building crates that use OpenSSL.

## Build the CLI

```sh
cargo build --workspace --exclude ios-py
cargo build --release --package ios-cli
```

The debug binary is at `target/debug/ios`; the release binary is at `target/release/ios`.

## First commands

```sh
cargo run -p ios-cli -- list
cargo run -p ios-cli -- info
cargo run -p ios-cli -- lockdown get --key ProductVersion
```

When a command targets a device and no UDID is specified, the CLI uses the
first device returned by `ios list`. If multiple devices are connected, pass
`-u <UDID>` or set:

```sh
export IOS_UDID=<UDID>
```

## Pairing

Most services require the device to trust the host. Keep the device unlocked and accept the trust prompt if one appears.

Useful commands:

```sh
ios pair --help
ios lockdown info
ios lockdown save-pair-record pair-record.plist
```

Pair records are sensitive because they can authorize access to a paired device. Do not commit them.

## Next steps

- [Usage](usage.md) for CLI and Rust API examples.
- [CLI map](cli-map.md) for go-ios and pymobiledevice3 command-family mapping.
- [Build](build.md) for CI-style checks and Python/FFI notes.
- [Tunnel](tunnel.md) for CoreDevice tunnel usage.
- [Troubleshooting](troubleshooting.md) if the device is not visible.

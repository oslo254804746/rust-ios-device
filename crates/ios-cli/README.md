# ios-cli

Cross-platform command-line tool for iOS device management, tunneling, and
service interaction. The published binary name is `ios`.

This is the binary crate in the
[`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device)
workspace.

## Highlights

- 54+ subcommands covering device discovery, pairing, files, apps,
  diagnostics, instruments, debugging, profiles, restore, supervision, and
  CoreDevice tunneling.
- Default JSON output for scripting; pass `--no-json` for human-readable
  tables where supported.
- CoreDevice pasteboard `get` and `set` support verified bounded data policies
  and binary UTI representations. `watch`, `resolve`, and `export` are
  experimental and require `--experimental`; output is redacted to size/hash
  by default, with `--show-data` for intentional byte output.
- iOS 17+ CoreDevice tunnel manager (`ios tunnel serve`) with go-ios-compatible
  fields (`tunnel-address`, `tunnel-port`, `userspace-port`).
- `apps install-record` queries the iOS 17+ RSD InstallCoordinationProxy;
  install/uninstall/stash mutations are intentionally not exposed because the
  pinned upstream client does not implement their file-transfer protocol.
- Built on `ios-core` with the `full` feature set.

## Install

From crates.io:

```sh
cargo install ios-cli            # installs the `ios` binary
```

Pre-built binaries (`x86_64-linux`, `aarch64-linux`, `aarch64-apple-darwin`,
`x86_64-windows-msvc`) are attached to each
[GitHub Release](https://github.com/oslo254804746/rust-ios-device/releases)
together with `.sha256` files.

## Quick start

```sh
ios --help
ios list                                         # connected devices (USB + network)
ios info                                         # default device summary
ios -u <UDID> lockdown get --key ProductVersion
ios syslog                                       # stream device logs
ios screenshot --output screenshot.png
ios tunnel start --userspace                     # iOS 17+ CoreDevice tunnel
```

## Device selection

Commands that target a device default to the first device returned by
`ios list`. Override with one of:

- `-u <UDID>` on the command line.
- `IOS_UDID=<UDID>` environment variable.

`ios list`, `ios listen`, and `ios discover` do not need a UDID.

## Command groups

| Area                     | Examples                                                                                  |
| ------------------------ | ----------------------------------------------------------------------------------------- |
| Discovery & pairing      | `list`, `listen`, `discover`, `pair`, `lockdown`                                          |
| Device info & settings   | `info`, `mobilegestalt`, `diskspace`, `batterycheck`, `activation` (state, session-info, info, activate, deactivate, itunes-activate), `amfi` |
| Files & containers       | `file` (AFC, app, CoreDevice), `crash`, `file-relay`                                      |
| Apps & UI tests          | `apps` (including `install-record`), `runtest`, `runxctest`, `runwda`, `wda`, `springboard` |
| Diagnostics & logs       | `syslog`, `diagnostics`, `os-trace` (including archive/collect), `notify`, `pcap`          |
| Developer services       | `instruments`, `debugserver`, `debug`, `ddi`, `symbols`, `accessibility-audit`, `webinspector`, `devicestate`, `memlimitoff` |
| iOS 17+ transport        | `tunnel`, `rsd`, `forward`, `dproxy`                                                      |
| Management & supervision | `profiles`, `provisioning`, `prepare`, `httpproxy`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` |
| Backup, location, screen | `backup`, `location`, `screenshot`                                                        |

Companion proxy commands select the classic lockdown service on older devices
and the RSD `com.apple.companion_proxy.shim.remote` service on iOS 17+.
`companion listen` prints one event per JSON line; `companion forward
REMOTE_PORT` reports the device-side `CompanionProxyServicePort` and keeps the
forward alive until Ctrl+C; `companion stop REMOTE_PORT` uses that same
companion-side port because the protocol has no persistent host-side forwarding
ID. These commands do not create a host TCP listener or print pairing
credentials.

For each command, `ios <command> --help` lists the exact subcommands and
flags. A side-by-side mapping with `go-ios` and `pymobiledevice3` lives in
[docs/cli-map.md](https://github.com/oslo254804746/rust-ios-device/blob/master/docs/cli-map.md).

## Documentation

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- API docs: <https://docs.rs/ios-cli>
- Usage guide: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/usage.md>
- CLI map: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/cli-map.md>
- CoreDevice tunnel: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/tunnel.md>
- Troubleshooting: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/troubleshooting.md>

## License

Licensed under either of Apache-2.0 or MIT at your option.

# ios-cli

Cross-platform command-line tool for iOS device management, tunneling, and
service interaction across **iOS 15.x – iOS 27.2**. The published binary name
is `ios`.

This is the binary crate in the
[`rust-ios-device`](https://github.com/oslo254804746/rust-ios-device)
workspace.

## Highlights

- **60+ subcommands** covering device discovery, pairing, files, apps,
  diagnostics, instruments, debugging, screen/media streaming, HID automation,
  WebInspector (CDP & WebDriver), profiles, restore, supervision, and CoreDevice
  tunneling.
- **Default JSON output** for scripting; pass `--no-json` for human-readable
  tables where supported.
- **CoreDevice Tunnel (iOS 17–27+)** — Kernel TUN and unprivileged Userspace
  Tunnel (`--userspace`), `ios tunnel serve` with go-ios-compatible fields
  (`tunnel-address`, `tunnel-port`, `userspace-port`), 20ms ICMPv6 Echo Request
  Nudge for low-latency burst stability, and IPv6 UDP datagram forwarding.
- **Screen & Media Streaming (`device-control display` & `viewer`)** —
  `ios device-control display status|video|audio` captures real-time H.265
  (HEVC) and AAC RTP access units over Userspace Tunnel without Administrator
  privileges; `ios viewer web` and `ios viewer vnc` provide an interactive
  browser UI and RFB 3.8 VNC server with `--share-clipboard` and 16-bit Display
  P3 PNG screenshot decoding.
- **Universal HID (`ios hid`)** — `tap`, `swipe`, `button`, phased `touch`,
  5-finger `multitouch` (`--points`), `text`, and `key` with automatic 500ms
  touch authorization settle, 300ms virtual keyboard settle, and Shift modifier
  lead-in/lead-out.
- **WebInspector, CDP & WebDriver (`ios webinspector`)** — `opened-tabs`,
  `eval`, `launch`, `js-shell`, `cdp` (compatible with `puppeteer-core` and
  `playwright-core` `connectOverCDP`), and W3C `webdriver` (compatible with
  `selenium-webdriver`).
- **File Management (`ios file`)** — Full AFC client including extended opcodes
  (`connection-info`, `hash`, `pread`, `pwrite`, `truncate`, `set-time`,
  `dir-size`) and iOS 17+ CoreDevice `fileservice` (`--coredevice`).
- **CoreDevice Pasteboard (`ios pasteboard`)** — `get`, `set`, and `watch`
  (Darwin notification monitoring with fresh PULL connections and
  `(nonce, changeCount)` deduplication); `resolve` and `export` require
  `--experimental`.
- **Apps & Debug** — `apps install-record` queries the iOS 17+ RSD
  `InstallCoordinationProxy`; `debugserver` (`send`, `proxy`) and `debug`
  automatically fall back to iOS 17+ RSD `debugproxy` when Lockdown
  `com.apple.debugserver` is absent.
- Built on `ios-core` with the `full` feature set.

SpringBoard requests use a 30-second total send/receive/parse budget. Set
`ios springboard orientation --request-timeout-ms 5000` to change that request
budget. A cancelled or failed request after wire progress retires the original
connection; reconnect before another command. CLI service setup independently
uses a shared 30-second budget covering discovery/lockdown/StartService/port/TLS.
Long listeners keep cancellable idle waits rather than inheriting this setup clock.

From a checkout, `python scripts/validate.py quick --package ios-cli` runs
focused tests. `python scripts/validate.py full --python <interpreter>` runs
the same complete check set used by CI; see [build notes](../../docs/build.md).

## Reusable test workflow

The crate also exports `TestWorkflowEngine`, `TestWorkflowArgs`, and
`TestWorkflowRunOptions`. `TestWorkflowEngine::run` executes the same
orchestrator used by `runtest` and `runxctest`: explicit target/readiness,
optional package installation, startup, bounded result/log collection,
JUnit/sidecar output, and cleanup. It accepts a launch-plan resolver rather
than maintaining a separate orchestration engine. See the
[external Rust API test](tests/workflow_api.rs). Package installation is
explicit, is never automatically replayed after unknown effects, and retains
original cleanup failures.

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
ios --version                                    # matches the workspace/artifact version
ios list                                         # connected devices (USB + network)
ios doctor                                       # bounded, read-only host diagnostic report
ios info                                         # default device summary
ios -u <UDID> lockdown get --key ProductVersion
ios syslog                                       # stream device logs
ios screenshot --output screenshot.png
ios tunnel start --userspace                     # iOS 17–27+ CoreDevice tunnel
```

Use `ios --error-format=json <command>` for a versioned final failure on stderr;
default human diagnostics and normal stdout stay unchanged. Select the format
before invalid arguments. Typed categories and original exits support scripts;
operation effects remain unknown. See the
[error contract](../../docs/error-contract.md).

## Device selection

Commands that target a device default to the first device returned by
`ios list`. Override with one of:

- `-u <UDID>` on the command line.
- `IOS_UDID=<UDID>` environment variable.

`ios list`, `ios listen`, `ios discover`, and `ios doctor` do not need a UDID.

## Command groups

| Area                     | Examples                                                                                  |
| ------------------------ | ----------------------------------------------------------------------------------------- |
| Discovery & pairing      | `list`, `listen`, `discover`, `pair`, `lockdown`, `doctor`, `heartbeat`                   |
| Device info & settings   | `info`, `mobilegestalt`, `diskspace`, `batterycheck`, `batteryregistry`, `activation` (state, session-info, info, activate, deactivate, itunes-activate), `amfi` |
| Files & containers       | `file` (AFC + extended opcodes, `--app` House Arrest, `--coredevice` `fileservice`), `crash`, `file-relay` |
| Apps & UI tests          | `apps` (including `install-record`), `runtest`, `runxctest`, `runwda`, `wda`, `springboard` |
| Diagnostics & logs       | `syslog`, `diagnostics`, `os-trace` (including archive/collect), `notify`, `pcap` (`--interface`), `ip`, `btlogger` |
| Developer services       | `instruments`, `debugserver` (Lockdown + RSD `debugproxy` fallback), `debug`, `ddi` (`auto`, `cryptex`), `symbols` (CoreProfileSession Stackshot + RSD), `accessibility-audit`, `webinspector` (`opened-tabs`, `eval`, `launch`, `js-shell`, `cdp`, `webdriver`), `devicestate`, `memlimitoff` |
| iOS 17–27+ transport     | `tunnel`, `rsd`, `forward`, `dproxy`                                                      |
| CoreDevice & media       | `device-control` (`configuration`, `orientation`, `display status\|video\|audio`), `hid` (`tap`, `swipe`, `button`, `touch`, `multitouch`, `text`, `key`), `viewer` (`web`, `vnc`, `clipboard`), `pasteboard` (`get`, `set`, `watch`) |
| Management & supervision | `profiles`, `wifi`, `provisioning`, `prepare`, `httpproxy`, `mdm`, `power-assert`, `preboard`, `restore`, `erase`, `arbitration`, `companion`, `idam` |
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

File pulls and screenshots stage output before replacing the destination.
`--force` refuses symlink/directory targets; without it, publication requires
filesystem hard-link support and cannot overwrite a concurrently created file.
Cancellation before commit preserves the old file; an already-started commit
can finish. See the [host output contract](../../docs/output-contract.md) for
per-file backup semantics, cleanup failures, and platform acceptance conditions.

- Repository: <https://github.com/oslo254804746/rust-ios-device>
- [Public compatibility rules](../../docs/public-contract.md) govern arguments,
  result shapes, exits, and explicit migrations.
- [Encoding and stdout failure contracts](../../docs/plist-json-contract.md)
  preserve historical result bytes and document the exit 101/0 to 1 migration
  for failed result/help/version output.
- API docs: <https://docs.rs/ios-cli>
- Usage guide: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/usage.md>
- CLI map: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/cli-map.md>
- CoreDevice tunnel: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/tunnel.md>
- Interactive viewer: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/viewer.md>
- Scoped manager access (`tunnel auth create|check`, `serve --auth-config`, clients `--token-file`): <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/management-access.md>
- Troubleshooting: <https://github.com/oslo254804746/rust-ios-device/blob/master/docs/troubleshooting.md>

## License

Licensed under either of Apache-2.0 or MIT at your option.

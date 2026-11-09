# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.17] — 2026-11-09

### Fixed

- **Legacy iOS < 17 Lockdown `debugserver` & `ios debug` support** (`ios-core`, `ios-cli`):
  - Lowered the `com.apple.debugserver.DVTSecureSocketProxy` service selection threshold from iOS 15 to iOS 14 (`version.major >= 14`).
  - Exposed `ConnectedDevice::pair_record` and `ConnectedDevice::connect_lockdown_port`, and added `connect_debugserver_with_device` to reuse a single connected device handle.
  - Wired legacy Lockdown `debugserver` proxying into `ios debug` for iOS < 17 devices with 30-second timeouts and Ctrl-C cancellation across version probing, installed-app lookup, and handshake.
  - Added `LockdownTlsDebugStream` to treat missing TLS `close_notify` (`UnexpectedEof`) from `DVTSecureSocketProxy` after local client write shutdown (`poll_shutdown`) as clean EOF while still surfacing mid-session disconnects.
- **iOS 27.2 RemoteXPC streaming (`streamapplist`) timeout** (`ios-core`):
  - Updated `XpcOperation::recv` to drain pending messages on both `STREAM_SERVER_CLIENT` (stream 3) and `STREAM_CLIENT_SERVER` (stream 1) before awaiting `recv_any_stream_inner()`, fixing 30-second timeouts in `ios apps list --coredevice` on iOS 27.2 where `dtappserviced` delivers `pushing.elements` and `finishStreaming` frames on `stream=1`.
- **CLI capability gating, argument validation ordering, and `unsupported` error classification** (`ios-cli`):
  - Added structured `CoreDeviceRequiresIos17` and `UnsupportedCliDecision` error types in `output.rs` so single-pass error chain inspection preserves the `unsupported` error class and primary diagnostic across `.context(...)` and `std::io::Error` wrappers.
  - Validated `--coredevice` file arguments (`validate_coredevice_file_args`) before probing device `ProductVersion` in `ios file`.
  - Added an iOS 17+ version gate (`ensure_cryptex_supported`) to `ios ddi cryptex`, marked `ios apps list --coredevice` on iOS < 17 with `unsupported_cli_decision`, and allowed `LockdownError::Protocol` containing `"InvalidService"` to trigger DTX screenshot fallback in `ios screenshot`.

## [0.1.16] — 2026-11-08

### Added

#### iOS 26 / 27 / 27.2 Protocol Adaptations & CoreDevice Tunnel
- Added a **20ms ICMPv6 Echo Request Nudge** in the userspace `smoltcp` tunnel
  stack (`CDTunnel`) when active TCP sockets await device responses, eliminating
  multi-second packet stalls during high-frequency HTTP/2 RemoteXPC and DTX
  bursts on iOS 18–27.2.
- Added **IPv6 UDP datagram forwarding** in the Userspace Tunnel proxy alongside
  TCP stream proxying, enabling unprivileged `dtremotedisplayd` RTP media
  streaming and HID touch authorization on Windows, macOS, and Linux without
  Administrator/root privileges.
- Added iOS 26/27 `com.apple.coredevice.feature.configuredevice` configuration
  actions (`liquid-glass-opacity` and `increase-contrast`) under
  `ios device-control configuration`, and included `LiquidGlass` in the default
  supervised setup skip list.
- Added CoreProfileSession Stackshot (`com.apple.instruments.server.services.coreprofilesessiontap`)
  `sharedCache` UUID extraction in `ios symbols list` for iOS 17–27+ devices that
  omit `com.apple.dt.fetchsymbols` from `DTServiceHub`.
- Added automatic 100ms TCP FIN drain before tearing down ephemeral Userspace
  Tunnels in `ios file --coredevice` so `dtfileserviced` cleanly releases
  sessions across consecutive invocations.

#### Screen & Media Streaming (`DisplayService` & `Viewer` — F04)
- Added real-time **H.265 (HEVC)** video and **AAC** audio RTP access-unit
  streaming (`ios device-control display status | video | audio`) via
  `com.apple.coredevice.diagnosticsservice` (`dtremotedisplayd`) over both
  Kernel TUN and unprivileged Userspace Tunnel (`UAFH` UDP framing, `CUID`,
  `VRAT` dictionary negotiation, device-issued stream-token stopping, and
  `DisplayError::MediaInUse` for camera/microphone conflicts).
- Added interactive Web viewer (`ios viewer web`) and RFB 3.8 VNC server
  (`ios viewer vnc`) with MJPEG/PNG frame streaming, persistent timestamped
  HEVC decoding via external FFmpeg, **16-bit Display P3 PNG screenshot**
  expansion (`png::Transformations::EXPAND | STRIP_16`) for Pro Max displays,
  interactive touch/key/button/orientation controls, and bidirectional clipboard
  sync (`--share-clipboard`).

#### Universal HID (`ios hid`)
- Added `ios hid` subcommands for `tap`, `swipe`, `button`, phased `touch`,
  **5-finger `multitouch`** (`--points` with independent `TouchIdentifier`
  finger slots), `text`, and `key`.
- Added automatic **500ms touch authorization settle** after `dtremotedisplayd`
  RTP session startup, **300ms virtual keyboard settle** before typing, and
  **Shift modifier lead-in/lead-out** (25ms lead-in/hold/release) for uppercase
  letters and shifted ASCII symbols on iOS 18–27.2.

#### WebInspector, CDP & W3C WebDriver (F02)
- Added `ios webinspector` subcommands (`opened-tabs`, `eval`, `launch`,
  `js-shell`, `cdp`, `webdriver`) with separated wire/model/client/session
  ownership and bounded application/page caches.
- Added Chrome DevTools Protocol (`cdp`) bridge compatible with `puppeteer-core`
  and `playwright-core` (`connectOverCDP`), supporting `Runtime`, `Page`
  (navigation, lifecycle events, frame tree, screenshots), `Network`, `Fetch`
  (interception/fulfill/continue), `Input` (`dispatchMouseEvent`,
  `dispatchTouchEvent`, `dispatchKeyEvent`, `insertText`), `Performance`, and
  `HeapProfiler` (`takeHeapSnapshot` via `Heap.gc` + `Heap.snapshot`). See
  [CDP client workflow](docs/cdp-client-workflow.md).
- Added W3C WebDriver (`webdriver`) HTTP server compatible with
  `selenium-webdriver` (session lifecycle, navigation, script execution,
  screenshots, element location, click, and text entry).

#### File Management (`AFC` Extended Opcodes & CoreDevice `FileService`)
- Added complete Apple File Conduit (AFC) extended opcodes to `ios-core::afc`
  and `ios file`: `GetConnectionInfo` (`connection-info`), `GetFileHash` and
  `GetFileHashWithRange` (`hash --offset --length`), `FileRefReadWithOffset` and
  `FileRefTell` (`pread`), `FileRefWriteWithOffset` (`pwrite`), `TruncateFile`
  and `FileRefSetFileSize` (`truncate [--by-fd]`), `SetFileTime` (`set-time`),
  and `GetSizeOfPathContents` (`dir-size`).
- Added full CoreDevice `fileservice` (`ios file --coredevice` `ls`, `mkdir`,
  `push`, `pull`, `mv`, `rm --recursive`) across `temporary`,
  `app-data-container`, `app-group-data-container`, `root-staging`, and
  `system-crash-logs` domains.

#### Developer Diagnostics, Pasteboard, XCTest & Restore/SDK (F03, F05, F06)
- Added Darwin-notification pasteboard monitoring (`ios pasteboard watch`) with
  fresh PULL connections, baseline suppression, and `(nonce, changeCount)`
  deduplication alongside `pasteboard get` and `pasteboard set` (text, URL, and
  binary UTI payloads).
- Added automatic iOS 17+ RSD `com.apple.internal.dt.remote.debugproxy` fallback
  in `ios debugserver` (`send`, `proxy`) and `ios debug` when Lockdown
  `com.apple.debugserver` is unavailable on iOS 17–27+.
- Added reusable `TestWorkflowEngine` (`runtest`, `runxctest`) with `.xctestrun`
  v1/v2 parsing, bounded package installation, JUnit/JSON result sidecars, and
  `runwda --once`.
- Added restored plist transport, IPSW OS image inspection/extraction, and
  bounded ASR data-port serving (`ios restore`).
- Added finite existing-trust `DeviceSession` (`IosDeviceSession` in C FFI,
  `ios_rs.DeviceSession` in Python) for bounded device queries (`ProductType`,
  `ProductVersion`, `DeviceName`), application listing, AFC file reads, and
  syslog streaming with explicit cancellation and resource budgets. See
  [owned device query contract](docs/owned-device-query.md).
- Added `ios tunnel auth create|check` and scoped access policies for `tunnel serve`,
  `webinspector cdp`, and `webinspector webdriver`, plus `--error-format=json`
  for structured machine-readable CLI failure envelopes. See
  [error contract](docs/error-contract.md) and
  [manager route intent](docs/manager-route-intent.md).
- Streamed bounded ZIP32 IPA installs from admitted directory metadata and 64 KiB
  decode buffers, preserving CRC checks and deadlines.
- Extended explicit-target `ios doctor` reports with read-only personalization
  manifest, DDI component, pairing-directory, and RSD route observations.

### Fixed

- Hardened lifecycle, cancellation, and deadline boundaries across Lockdown,
  RSD, HTTP/2 RemoteXPC, DTX, AFC, MobileBackup2, DeviceLink, NotificationProxy,
  Companion, SpringBoard (`--request-timeout-ms`), FPS capture, and WebInspector
  sessions. See [lifecycle contract](docs/lifecycle-contract.md).
- Propagated CLI stdout write/newline/flush failures through a shared fallible
  boundary so closed result pipes return exit 1 instead of panicking. See
  [plist/JSON contract](docs/plist-json-contract.md) and
  [backup contract](docs/backup-contract.md).
- Preserved existing file-pull and backup output until staged transfers succeed,
  refused symlink destinations with `--force`, and cleaned temporary files on
  cancellation.
- Restricted CLI logging to reviewed project targets and redacted credentials,
  pair records, and raw device payloads from `Debug` and automatic error
  diagnostics.
- Opened a fresh RemoteXPC connection for each subsequent Display request,
  omitted `VRAE:0` from video negotiation so the device can adapt its encoder,
  and stopped display capture or accessibility visuals cleanly on Ctrl-C.

## [0.1.15] — 2026-09-07

### Added

- Added kernel tunnel support to the Python and C FFI bindings alongside the
  existing userspace tunnel backend.
- Added deterministic coverage for XPC/RSD initialization deadlines,
  cancellation recovery, fallback handshakes, and release artifact gates.

### Changed

- Bounded HTTP/2, RemoteXPC, and RSD proxy initialization with shared
  deadlines, dropping partial sessions before fallback attempts.
- Made interrupted RemoteXPC writes, reads, stream opens, and window updates
  preserve framing state safely and reject poisoned connections.
- Added `ios --version` output and expanded release CI artifact smoke checks.

## [0.1.14] — 2026-09-04

### Added

- Added bounded RemoteXPC file-transfer side-channel handling and remote
  symbol listing/download support for current iOS releases.

### Fixed

- Preserved unsigned sysmontap and DTX integer values instead of narrowing
  device-provided 64-bit attributes.
- Aligned filtered Backup2 status codes and modification-date encoding with
  the iOS 15 DeviceLink wire format.
- Hardened iOS 26 symbol side-channel and OS trace archive handling, including
  bounded stream state and complete archive validation.
- Kept the workspace checks compatible with the documented Rust 1.80 MSRV.

## [0.1.13] — 2026-08-30

### Fixed

- Made backup integration expectations compare canonical layout paths, so
  macOS's `/var` alias does not turn a successful backup into a false failure.

## [0.1.12] — 2026-08-30

### Fixed

- Validated the directory target after accepting macOS's exact root-owned
  temporary-directory aliases, so OS-trace output works through `/var` and
  `/tmp` without weakening user-symlink rejection.

## [0.1.11] — 2026-08-30

### Fixed

- Accepted macOS's root-owned `/var`, `/tmp`, and `/etc` aliases into
  `/private` while continuing to reject user-controlled symlink components.
- Made backup and OS-trace path fixtures use the same canonical spelling as
  production so macOS system aliases and Windows short/long path forms do not
  cause false failures or premature mock-stream EOFs.

## [0.1.10] — 2026-08-30

### Added

- Added legacy MobileBackup2 backup helpers, crash-report workflows, and
  read-only InstallCoordination install-record queries.
- Added CoreDevice app, file, diagnostics, media, pasteboard, companion, HID,
  configuration, orientation, XCTest, WebInspector, and trace workflows.
- Added matching Rust, C FFI, Python, CLI, protocol, and feature documentation
  for the expanded service surface.

### Changed

- Extended tunnel and RemoteXPC service handling for CoreDevice paths while
  keeping service-dependent behavior explicit when a device omits an endpoint.
- Added bounded deadlines, transfers, streams, and device-input parsing across
  the new service clients, with atomic host-file output where applicable.
- Documented Git Bash/MSYS device-path conversion and the RSD default filter,
  full-directory, prefix, and feature-metadata contracts.

### Fixed

- Hardened AFC framing/status validation, file replacement and Windows long-path
  handling, XPC/HTTP2 response routing, cancellation, and cleanup paths.
- Corrected activation diagnostics to redact daemon-supplied payloads and fixed
  backup manifest persistence and InstallCoordination request/response handling.

## [0.1.9] — 2026-08-29

### Added

- Exposed RSD-advertised service feature identifiers through `ios-core`, the
  `ios rsd services --features` command, the C FFI, and Python tunnel objects.
- Added detailed MobileBackup2 free-space diagnostics after device purge
  requests.
- Added `ServiceDescriptor::features`, `ServiceDescriptor::new`, and RSD
  capability helpers for Rust consumers.
- Added `ios_tunnel_rsd_services_json` to the C ABI and `service_ports` plus
  `service_features` to the Python `Tunnel` object.

### Changed

- Reduced MobileBackup2 host-to-device transfer frames to 32 KiB and streamed
  device-to-host payloads through bounded buffers.
- Decoded BSD `vis(3)` escapes in syslog relay messages and made process filters
  recognize Apple's `Process(Library)` annotation.
- Preserved the existing JSON schema for `ios rsd services` and `rsd check` by
  keeping feature metadata opt-in. Passing `--features` adds `features` to
  JSON and human-readable output; without it, services JSON remains
  `name`/`port` and check JSON remains unchanged.
- TCP dials and initial tunnel/RSD/XPC setup paths now fail after 15 seconds
  instead of inheriting the host operating system's long retry window.

### Fixed

- Bounded tunnel TCP connection attempts, InstallationProxy Browse responses,
  and pending DTX state so stale or malformed device traffic cannot stall or
  grow memory indefinitely.
- Rejected invalid RSD ports and malformed DTX length/fragment metadata instead
  of truncating values or accepting incomplete frames.
- Accepted additional NSKeyedArchiver UID indirections and nullable XCTest
  fields, and encoded object-valued XCTest configuration fields as keyed
  references.
- InstallationProxy Browse now enforces separate response-chunk and app-entry
  limits before retaining more device data.
- MobileBackup2 file transfers use bounded buffers and 32 KiB host-to-device
  frames; insufficient-space failures include the available-space diagnostics
  collected around purge requests.

## [0.1.8] — 2026-07-27

### Added

- Added Bluetooth HCI capture through `ios btlogger`, CoreDevice feature groups, and remote symbol download support.

### Changed

- Refactored tunnel, RemoteXPC, and device connection orchestration for direct and userspace-proxy transports.
- Added explicit overwrite and destructive-operation guards across the CLI.
- Bumped workspace crates and internal `ios-core` dependency versions to `0.1.8`.

### Security

- Restricted persisted private-key material to the owner on Unix and zeroized pairing secrets where supported.
- Bounded device-controlled allocations and parser recursion across XPC, DTX, OPACK, NSKeyedArchiver, AFC, fileservice, SpringBoard, and syslog paths.

### Fixed

- Fixed XPC HTTP/2 receive-window replenishment and cancellation-safe tunnel packet reads.
- Corrected personalized DDI selection and download validation.
- Fixed protocol decoding, device discovery, file/path handling, and CLI/service correctness issues.

## [0.1.4] — 2026-04-27

### Changed

- Bumped workspace version to `0.1.4`.
- Code review P0/P1/P2 fixes: allocation guards, FFI safety, async FS, module visibility, dead features, test coverage, shared test utilities, service error macro, house_arrest module structure.

## [0.1.2] — 2026-04-27

### Added

- PyPI package now includes a user-facing README with install instructions, API reference, and usage examples.
- CI validates that the git tag version matches the workspace `Cargo.toml` version before publishing.

### Changed

- Bumped workspace version to `0.1.2`.

### Fixed

- Fixed empty PyPI documentation by adding the `readme` field to `pyproject.toml`.
- Corrected stale crate count and name references in CHANGELOG v0.1.0 infrastructure section.

## [0.1.1] — 2026-04-24

### Added

- Added crate-level README files for every workspace crate so crates.io package pages have documentation.
- Added a reusable crates.io publish script with retry/backoff handling for crates.io rate limits.
- Public README, contribution guide, security policy, and user/developer documentation.
- GitHub issue templates and pull request template.

### Changed

- Bumped workspace crates and internal crate dependency versions to `0.1.1` for a documentation refresh release.
- Updated the tag release workflow to use the shared publish script instead of fixed 30-second sleeps.
- Removed local machine-specific Cargo/PyO3 configuration from the repository.
- Normalized example/test placeholder names and temporary paths for public release.

## [0.1.0] — 2026-04-21

### Added

#### Device Management
- USB and network device discovery via usbmuxd
- Lockdown protocol with TLS session, pair record, and supervised P12 pairing
- WiFi pairing CLI for network-only setups
- iOS 17+ CDTunnel handshake with kernel/userspace TUN forwarding
- XPC/RemoteXPC service discovery (RSD) over HTTP/2
- mDNS/Bonjour device discovery CLI

#### App Management
- App install/uninstall/launch/kill via InstallationProxy
- Streaming Zip Conduit fast install (Xcode-style)
- iOS 17+ CoreDevice appservice support
- Process signal sending (arbitrary signals) and pkill by name

#### File System
- Apple File Conduit (AFC) — ls, pull, push, mkdir, rm
- iOS 17+ XPC file service

#### Instruments & Performance
- CPU/GPU/FPS/network/energy monitoring via sysmontap
- Per-process monitoring with CPU threshold alerts
- Core Profile Session (FPS frame timing)
- KDebug trace event CLI
- HAR (HTTP Archive) logging

#### Screen & UI
- Screenshot capture (single and MJPEG stream)
- SpringBoard icon layout get/set, wallpaper export, orientation
- Accessibility audit and interactive element navigation

#### Diagnostics
- Real-time syslog streaming
- Crash report download and management
- OS trace relay process listing
- Network packet capture (pcapd)
- Developer disk image auto-download and mount

#### Device Configuration
- Configuration profile install/remove (MCInstall)
- Provisioning profile management (misagent)
- Location simulation (coordinate set/reset/GPX playback)
- Device state induction (enable/disable thermal, network conditions)
- Notification subscribe/post
- Backup create/restore

#### Security & Debug
- AMFI developer mode management
- LLDB debugserver connection
- XCTest execution framework (testmanager)
- WebInspector protocol for Safari/WebView debugging

#### Infrastructure
- Workspace with 4 crates, unified dependency management
- Feature-gated service modules (30+ features in ios-core)
- Python bindings (PyO3) — `ios-py`
- C FFI bindings — `ios-ffi`
- Cross-platform CLI binary (`ios`)
- Protocol documentation for AFC, DTX, lockdown, OPACK, XPC

[Unreleased]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.17...HEAD
[0.1.17]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.16...v0.1.17
[0.1.16]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.15...v0.1.16
[0.1.15]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.14...v0.1.15
[0.1.14]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.13...v0.1.14
[0.1.13]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.12...v0.1.13
[0.1.12]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.11...v0.1.12
[0.1.11]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.10...v0.1.11
[0.1.10]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.7...v0.1.8
[0.1.4]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.2...v0.1.4
[0.1.2]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/oslo254804746/rust-ios-device/releases/tag/v0.1.0

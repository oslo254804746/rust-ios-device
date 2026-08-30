# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

No unreleased changes yet.

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

[Unreleased]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.13...HEAD
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

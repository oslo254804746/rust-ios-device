# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Exposed RSD-advertised service feature identifiers through `ios-core`, the
  `ios rsd services --features` command, the C FFI, and Python tunnel objects.
- Added detailed MobileBackup2 free-space diagnostics after device purge
  requests.

### Changed

- Reduced MobileBackup2 host-to-device transfer frames to 32 KiB and streamed
  device-to-host payloads through bounded buffers.
- Decoded BSD `vis(3)` escapes in syslog relay messages and made process filters
  recognize Apple's `Process(Library)` annotation.

### Fixed

- Bounded tunnel TCP connection attempts, InstallationProxy Browse responses,
  and pending DTX state so stale or malformed device traffic cannot stall or
  grow memory indefinitely.
- Rejected invalid RSD ports and malformed DTX length/fragment metadata instead
  of truncating values or accepting incomplete frames.

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

[Unreleased]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.8...HEAD
[0.1.8]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.7...v0.1.8
[0.1.4]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.2...v0.1.4
[0.1.2]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/oslo254804746/rust-ios-device/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/oslo254804746/rust-ios-device/releases/tag/v0.1.0

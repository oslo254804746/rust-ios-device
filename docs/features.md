# Feature flags

`ios-core` is published with no default service features. A minimal dependency can list devices, talk to usbmuxd/lockdown, and use the high-level connection types without pulling every service client into downstream builds.

Enable only the services your application needs:

```toml
[dependencies]
ios-core = { version = "0.1.13", features = ["afc", "syslog"] }
```

For tools that intentionally expose a broad surface, use grouped features:

```toml
ios-core = { version = "0.1.13", features = ["classic", "developer"] }
```

## Groups

| Feature | Purpose |
| --- | --- |
| `classic` | Common lockdown/usbmux services used across many iOS versions. |
| `developer` | DTX, Instruments, debugserver, WebInspector, image mounting, Bluetooth packet logging, and related developer workflows. |
| `management` | Device management, supervision/preparation, supervised MCInstall passcode/security operations, restore, power assertion, and companion registry/event/forwarding helpers. |
| `ios17` | CoreDevice/RSD-oriented services and tunnel workflows used primarily by iOS 17+ devices. |
| `coredevice-base` | Minimal CoreDevice transport support: userspace tunnel plus mDNS discovery. |
| `coredevice-files` | CoreDevice fileservice plus the base CoreDevice transport support. |
| `coredevice-info` | CoreDevice deviceinfo plus the base CoreDevice transport support. |
| `full` | Everything exposed by `ios-core`; intended for the CLI and integration testing. |

## Service features

Most service modules are available as one feature per module, including `afc`, `crashreport`, `apps`, `syslog`, `screenshot`, `iconservice`, `screencapture`, `installcoordination` (the iOS 17+ InstallCoordinationProxy query), `mcinstall` (profiles, deterministic Wi-Fi profiles, and supervised MDM passcode/security), `mobileactivation` (state, session handshake, online/offline activation, deactivation, and the iTunes lockdown marker), `dtx`, `instruments`, `testmanager`, `accessibility_audit`, `btlogger`, `debugserver`, `imagemounter`, `pcap` (capture and packet-derived IP discovery), `webinspector`, `fileservice`, `deviceinfo`, `diagnosticsservice` (including reboot/shutdown), `configuration`, `orientation`, `ostrace` (process listing, structured live log stream, and raw PAX archive/collect), `pasteboard`, `restore`, `dproxy`, and `fetchsymbols`.

The `crashreport` feature selects the classic go-ios mover `ping` handshake or
the iOS 17+ RSD crash-report shims with their `ping\0` handshake. It provides
bounded AFC list/read/delete, `.ips` and legacy `.crash` structured parsing,
local latest-by-event-time selection, and a bounded directory-poll stream for
new files. Apple-specific sysdiagnose notification/archive collection is not
claimed here. The separate diagnostics service currently exposes only a
dry-run metadata probe; it does not download the archive.

`iconservice` and `screencapture` are modern CoreDevice/RSD services. The
`apps icons` and `screenshot` CLI commands select them automatically on iOS
17+ when the resolved RSD service is present, and use the legacy SpringBoard /
lockdown paths on older devices. `icon_service` and `screen_capture` are
feature aliases for downstream naming conventions.

`installcoordination` is an RSD-only service. It currently implements the
upstream `Query` request as `apps install-record`; Install, Uninstall, and
RevertStash are not exposed because the upstream client leaves their
out-of-band file-transfer protocol unimplemented.

Query deadline semantics: the configured timeout is one shared budget that
covers sending the request (including HTTP/2 flow-control waits) and
receiving the reply, matching the pymobiledevice3 client. The daemon answers
with an uncorrelated fresh message and the response stream is not part of the
request contract, so the reply is accepted from whichever stream it arrives
on while message reassembly stays per-stream; empty wrapper frames and empty
dictionaries are skipped. Once those input and deadline prechecks pass, a
query marks the connection unusable before its first I/O await, so a timeout,
transport failure, or caller cancellation at any later await requires
reconnecting; a later query returns an error immediately instead of consuming
a late uncorrelated response. A complete non-empty response restores
reusability before business parsing, so a consumed protocol-error response
keeps the existing reusable-connection behavior. Invalid input and a zero
timeout return before I/O and leave the connection reusable.

Features not included in any group except `full`: `ostrace`, `supervised-pair`, `tunnel-kernel`.

`configuration`, `orientation`, and `hid` are iOS 17+ CoreDevice services. They use
the modern RemoteXPC/RSD tunnel and return an explicit unsupported error when
the resolved endpoint is a legacy or `.shim.remote` service. Configuration
setters change device-wide appearance/accessibility state; orientation rotates
the active UI, so callers should treat both as mutating operations.

Pasteboard PULL/SET, multi-item UTI data, and the documented data-policy
encodings are verified against go-ios `ced7e53d` and pymobiledevice3
`38fbd227`. The listed RESOLVE/DATA/AUTONOTIFY/PUSH verbs are experimental:
those pinned reference clients only scaffold or enumerate them. The CLI gates
`pasteboard watch`, `resolve`, and `export` behind `--experimental`; library
APIs for those verbs carry the same warning and require real-device validation.

`hid` exposes pmd3-compatible Indigo button and Universal HID report services.
The `ios hid` command requires `--confirm`, bounds input, and never prints
keyboard text in JSON or human output. Touch reports use normalized coordinates
and the single-contact report format supported by CoreDevice. Indigo button
events are available directly; Universal touch/keyboard commands first open an
authenticated Display/RTP media stream and keep it alive until HID release.
They use the kernel tunnel because the current userspace tunnel has no UDP
bridge; a missing kernel-TUN capability or tunnel endpoint is reported as an
error rather than claiming that input was delivered.

`display` provides `device-control display status` and bounded `video`/`audio`
capture of encoded RTP access units. Capture binds the CDTunnel client address,
uses the negotiated `streamConfig` RTCP port/SSRC values, and writes optional
output atomically with owner-only permissions. It does not decode HEVC/AAC or
provide a VNC/pixel viewer; consumers must decode the raw Annex-B HEVC or
AAC-ELD payloads separately.

MobileBackup2's DeviceLink client is part of the core service surface and
includes the device-side `Unback` and `Extract` operations. The optional
`backup2-manifest` feature is only for host-side Manifest.db/Manifest.mbdb
filtering and local expansion (including modern and legacy encrypted payloads);
it does not gate or provide the device protocol. Legacy `Manifest.mbdb` is
plaintext, uses the flat file-ID payload layout, and does not require an MBDX
sidecar.

Some features add heavier optional dependencies only when enabled:

| Feature | Extra dependency surface |
| --- | --- |
| `apps` | IPA/Zip parsing and CRC support. |
| `imagemounter` | HTTP downloads plus Zip handling for Developer Disk Images. |
| `dtx`, `instruments`, `testmanager`, `accessibility_audit`, `dproxy` | DTX codec support. |
| `mdns` | Bonjour/mDNS discovery via `mdns-sd`. Required for iOS 17+ network discovery and remote pairing target discovery. |
| `tunnel` | CoreDevice tunnel infrastructure and TLS-PSK support via `openssl` and `tokio-openssl`. |
| `tunnel-userspace` | Userspace tunnel backend via `smoltcp`; implies `tunnel`. |
| `tunnel-kernel` | Kernel TUN backend via `tun-rs`; implies `tunnel`. |
| `supervised-pair` | Supervised pairing/P12 signing helpers via `openssl`; implied by `prepare`. |
| `backup2-manifest` | Host-side Backup2 Manifest.db/Manifest.mbdb filtering and local extraction, including modern and legacy BackupKeyBag/AES encrypted payloads; modern manifests use bundled SQLite. |

The `ios-cli` crate enables `ios-core/full` because the binary exposes many commands. Library users should prefer a narrower feature list.

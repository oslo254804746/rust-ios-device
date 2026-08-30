# Usage

The binary is named `ios`. Most commands print JSON by default so they can be
used from scripts. Pass `--no-json` for a more human-readable table or text
format where a command supports it.

## CLI conventions

```sh
ios --help
ios <command>
ios -u <UDID> <command>
IOS_UDID=<UDID> ios <command>
ios --no-json <command>
ios -v <command>
```

For commands that target a device, omitting `-u/--udid` selects the first
device returned by `ios list`. Use `-u <UDID>` or `IOS_UDID=<UDID>` when you
need to choose a specific device.

Use command help for exact arguments:

```sh
ios file --help
ios apps --help
ios backup --help
ios tunnel --help
ios instruments --help
```

## Device discovery and pairing

```sh
ios list
ios listen
ios discover mobdev2
ios -u <UDID> pair
ios -u <UDID> pair show-record
ios -u <UDID> lockdown info
ios -u <UDID> lockdown get --key ProductVersion
ios -u <UDID> lockdown save-pair-record pair-record.plist
```

Comparable upstream workflows:

- go-ios: `ios list`, `ios listen`, `ios pair`, `ios lockdown get`.
- pymobiledevice3: `pymobiledevice3 usbmux list`,
  `pymobiledevice3 lockdown ...`, `pymobiledevice3 bonjour rsd`.

Pair records are credentials for device access. Do not commit them or include
them in logs.

## Companion proxy

The companion commands use `com.apple.companion_proxy` through lockdown on
older devices and the RSD `com.apple.companion_proxy.shim.remote` service on
iOS 17+ to inspect paired accessories and request a companion port forward:

```sh
ios -u <UDID> companion list
ios -u <UDID> companion get <COMPANION_UDID> <KEY>
ios -u <UDID> companion listen --timeout 60
ios -u <UDID> companion forward 8100 --service-name com.example.watch
ios -u <UDID> companion stop 8100
```

`listen` emits one JSON event per line by default and preserves unknown event
fields for forward compatibility. `forward` reports the device-side
`CompanionProxyServicePort`; it does not create a host TCP listener. The
protocol identifies a forwarding by its companion (remote) port, so `stop`
takes that same port. Ctrl+C performs a bounded best-effort stop. Pairing
credentials are never printed by these commands.

## Device information and lockdown values

```sh
ios -u <UDID> info
ios -u <UDID> diskspace
ios -u <UDID> mobilegestalt ProductType ProductVersion
ios -u <UDID> batterycheck
ios -u <UDID> batteryregistry
ios -u <UDID> activation state
ios -u <UDID> activation session-info
ios -u <UDID> activation activate
ios -u <UDID> activation activate --now
ios -u <UDID> activation activate --record-input activation-record.plist
ios -u <UDID> activation deactivate --force
ios -u <UDID> activation itunes-activate
```

Online `activation activate` waits for mobileactivationd to publish a fresh
Tunnel1 nonce/session before contacting Apple's endpoints. `--now` performs a
single session probe and skips that wait; use it only when the caller knows the
daemon session is fresh, because reusing a consumed nonce can make activation
fail. The timeout covers device connection, session polling, HTTPS requests,
and applying the record. `itunes-activate` is a separate legacy marker command
and does not accept `--now`.

Use `lockdown get` when you know the lockdown domain or key, and use the
higher-level commands when you want a narrower, typed view of common data.

## Files, app containers, and crash reports

The `file` command uses AFC by default:

```sh
ios -u <UDID> file ls /
ios -u <UDID> file tree /
ios -u <UDID> file pull /DCIM ./dcim
ios -u <UDID> file push local.txt /Downloads/local.txt
ios -u <UDID> file stat /Downloads/local.txt
ios -u <UDID> file rm /Downloads/local.txt
```

Use House Arrest for an app container:

```sh
ios -u <UDID> file --app com.example.app ls /
ios -u <UDID> file --app com.example.app --documents pull / ./Documents
```

Crash and file relay helpers:

```sh
ios -u <UDID> crash ls
ios crash parse ./report.ips --json
ios crash parse-latest ./reports --pattern '*.ips' --count 3 --json
ios -u <UDID> crash flush
ios -u <UDID> crash watch --pattern '*.ips' --timeout 60 --json
ios -u <UDID> crash rm '*.ips' --force
ios -u <UDID> crash clear --force
ios -u <UDID> file --crash ls /
ios -u <UDID> file-relay Network --output network-relay.zip
```

`crash parse` accepts Apple's two-line JSON `.ips` format and legacy `.crash`
text without a device. Parsing is bounded and retains unknown JSON fields under
`raw`; malformed reports are reported rather than silently returned as complete
records. `parse-latest` orders by the report event timestamp, not the filename.
`crash watch` uses a bounded AFC directory poll because neither the classic
nor RSD crash-report mover exposes a report event stream. The command selects
classic lockdown on older devices and the RSD `.shim.remote` services on iOS
17+. `crash rm` and `crash clear` require `--force`. The crash mover does not
implement sysdiagnose collection. `ios diagnostics sysdiagnose` is a separate
iOS 17+ CoreDevice metadata probe in this release (dry-run only); it does not
download the archive.

CoreDevice fileservice (iOS 17+ devices that expose the service):

```sh
ios -u <UDID> file --coredevice --domain temporary ls /
ios -u <UDID> file --coredevice --domain app-data-container --identifier com.example.app ls /
ios -u <UDID> file --coredevice --domain temporary push local.txt remote.txt
ios -u <UDID> file --coredevice --domain temporary pull remote.txt local.txt
ios -u <UDID> file --coredevice --domain temporary mkdir new-dir
ios -u <UDID> file --coredevice --domain temporary mv old.txt new.txt
ios -u <UDID> file --coredevice --domain temporary rm old.txt
```

Not all devices expose `com.apple.coredevice.fileservice.control` / `.data` in
their RSD directory. Use `ios rsd check com.apple.coredevice.fileservice.control`
to verify availability before using `--coredevice`.

Comparable upstream workflows:

- go-ios: `ios fsync ...`, `ios crash ls`, `ios crash cp`.
- pymobiledevice3: `pymobiledevice3 afc ...`,
  `pymobiledevice3 crash pull ...`.

## Applications and test automation

```sh
ios -u <UDID> apps list
ios -u <UDID> apps list --coredevice
ios -u <UDID> apps show com.example.app
ios -u <UDID> apps install-record com.example.app
ios -u <UDID> apps install ./Example.ipa
ios -u <UDID> apps uninstall com.example.app
ios -u <UDID> apps launch com.example.app
ios -u <UDID> apps processes
ios -u <UDID> apps kill <PID>
ios -u <UDID> apps roots
ios -u <UDID> apps spawn /usr/bin/log -- stream --style json
ios -u <UDID> apps icons com.example.app --output-dir ./icons
ios -u <UDID> apps icons com.example.app --output-dir ./icons --force
ios -u <UDID> apps monitor <PID> --timeout-secs 30
ios -u <UDID> runtest ./Build/Products/Example.xctestrun
ios -u <UDID> runtest ./Build/Products/Example.xctestrun --configuration UITests --test-target com.example.Runner --wait
ios -u <UDID> runtest ./Build/Products/Example.xctestrun --wait --junit-output ./test-results.xml
ios -u <UDID> runxctest --test-runner-bundle-id com.example.Runner --xctest-config ExampleTests.xctest --bundle-id com.example.App --test LoginTests/testHappyPath --wait --junit-output ./test-results.xml
ios -u <UDID> runwda --help
ios wda status --base-url http://127.0.0.1:8100
ios -u <UDID> wda --device-port 8100 status
ios -u <UDID> wda --device-port 8100 session --bundle-id com.example.Aut
```

`apps list --coredevice`, `apps processes`, `apps launch`, `apps roots`,
`apps spawn`, and `apps monitor` use newer app service paths. `apps icons` uses
the independent CoreDevice `iconservice` on iOS 17+ and the legacy SpringBoard
icon service on older devices. Icon output is written through a private
temporary file and atomic replacement; JSON reports metadata and paths rather
than embedding image bytes. CoreDevice icon requests default to 60x60 points,
scale 2, and allow a placeholder; use `--no-placeholder` to reject one. The
legacy SpringBoard path cannot apply those rendering options.

The `screenshot` command similarly selects CoreDevice
`screencaptureservice` on iOS 17+, then uses the read-only DTX/lockdown paths as
an explicit fallback when that service is unavailable. Use `--force` to replace
an existing output file.

`runtest` chooses the XCTest transport by iOS generation: iOS 17+ uses Remote
Service Discovery, iOS 14-16 uses the secure lockdown testmanager service, and
older versions use the legacy lockdown service. `wda --device-port` talks to a
WDA listener directly through usbmux, so it does not require a local `forward`
process when the runner is already listening on the device.

`--junit-output` requires `--wait` and writes the complete result event summary
as standard JUnit XML using an atomic same-directory temporary file and rename.
On Unix an existing destination is atomically replaced; platforms that reject
replacement return an error. Startup or result-stream failures still write a
diagnostic `testsuite` and return a non-zero exit status. Passed, failed,
skipped, stalled, and unknown cases map to standard JUnit elements; expected
failures and newer unknown statuses are retained in testcase properties,
without falsely increasing the skipped/failure totals. Existing XCTest events
currently provide log text only, so normal/debug logs are mapped to
`system-out` / `system-err`; actual stdout/stderr provenance and attachment
metadata are not available in the current model.
The direct runner command (`runxctest`) builds the XCTest configuration in
memory and accepts repeated `--test`, `--test-to-skip`, `--env KEY=VALUE`,
and `--arg` options, plus the `--class`/`--method` convenience selector.
The test runner application and its `*.xctest` bundle must already be
installed and signed for the device. This command does not build, install,
sign, or provision a runner; Apple signing services and App Store Connect
workflows remain outside this CLI. iOS 17+ uses the DDI/testmanagerd route and
older classic devices use the corresponding lockdown testmanager service.
Result attachments and runner stdout/stderr provenance are not yet exposed.

Comparable upstream workflows:

- go-ios: `ios apps`, `ios install`, `ios launch`, `ios kill`, `ios runtest`
  (bundle-ID runner), `ios runxctest` (.xctestrun), and `ios runwda`.
- pymobiledevice3: `pymobiledevice3 apps ...` and developer DVT launch/kill
  commands.

## iOS 17+ pasteboard

Pasteboard access uses the CoreDevice/RSD tunnel and the
`com.apple.coredevice.pasteboardservice` service:

```sh
ios -u <UDID> pasteboard get
ios -u <UDID> pasteboard set "こんにちは 👋"
printf 'text from stdin' | ios -u <UDID> pasteboard set
ios -u <UDID> pasteboard set --url https://example.test/path
ios -u <UDID> pasteboard get --raw
ios -u <UDID> pasteboard get --policy promised
ios -u <UDID> pasteboard set --data public.data=AP8= --data public.url=aHR0cHM6Ly9leGFtcGxlLnRlc3Q=
ios -u <UDID> pasteboard resolve 0 public.data --out ./payload.bin --experimental
ios -u <UDID> pasteboard watch --policy promisesecondary --experimental
```

`set` with no text argument reads UTF-8 from stdin; an explicit empty argument
(`set ""`) writes an empty text item. Repeat `--uti` to add text/URL
representations, or repeat `--data UTI=BASE64` for binary representations.
The default output is structured JSON; use `--no-json` for text output.

Pasteboard PULL/SET and the CoreDevice data policies `resolved`, `promised`,
`matchsource`, `promisesecondary`, and `threshold:N` match the pinned go-ios
and pymobiledevice3 wire evidence. Output redacts content by default and
reports only UTI, byte count, and SHA-256. Add `--show-data` to emit
inline/resolved bytes as base64. `get --raw --show-data` retains the complete
direct XPC dictionary; `--raw` without `--show-data` is still redacted.

`resolve`, `export`, and `watch` use the additional RESOLVE/DATA and
AUTONOTIFY/PUSH verbs. Those verbs are only enumerated or scaffolded by the
pinned upstream clients and are therefore experimental; each command must be
passed `--experimental` and requires real-device validation. `watch` stops
with Ctrl-C; the library's explicit `close`/`unsubscribe` (and a dropped
session) performs a bounded best-effort unsubscribe. The client enforces
bounded item, representation, metadata, event, and data budgets. It skips empty
control frames, closes a timed-out connection rather than reusing a partial
frame, and rejects unknown events. Pasteboard service availability is device
and OS dependent: this command requires an iOS 17+ CoreDevice tunnel and an
RSD-advertised `com.apple.coredevice.pasteboardservice`; it does not provide a
lockdown-era fallback.

## Supervised MDM passcode operations

The MCInstall MDM helpers require a supervised device and the supervisor
PKCS#12 identity generated by `prepare create-cert` (or an equivalent
identity). The P12 password may be supplied with `--password` or
`P12_PASSWORD`; token data is always kept in a file and is never printed:

```sh
# The device must not have a lock passcode when minting the token.
ios -u <UDID> mdm fetch-unlock-token --p12 identity.p12 --output unlock-token.bin

# Optional base64 file form (still protected with 0600 permissions).
ios -u <UDID> mdm fetch-unlock-token --p12 identity.p12 --output unlock-token.txt --base64

ios -u <UDID> mdm security-info --p12 identity.p12
ios -u <UDID> mdm passcode-present --p12 identity.p12

# Destructive operations require --force and never echo the token.
ios -u <UDID> mdm clear-passcode --p12 identity.p12 --token unlock-token.bin --force
ios -u <UDID> mdm clear-passcode --p12 identity.p12 --token unlock-token.txt --token-base64 --force
ios -u <UDID> mdm clear-screen-time-password --p12 identity.p12 --force
```

Token output refuses to overwrite an existing file unless `--force` is given;
the resulting file is owner-only (`0600` on Unix). `security-info` is read-only
and redacts any secret-bearing fields before JSON or human output. MCInstall
failures expose only status/ErrorChain domain, code, and safe descriptions,
never the full response dictionary.

## CoreDevice appearance and orientation (iOS 17+)

The CoreDevice configuration and orientation services require a userspace
tunnel and a canonical RSD RemoteXPC endpoint. Configuration setters change
device-wide UI/accessibility state; rotation changes the active UI, so use
them with a device whose current user can tolerate the change:

```sh
ios -u <UDID> device-control configuration get style
ios -u <UDID> device-control configuration set style dark
ios -u <UDID> device-control configuration get color-filter
ios -u <UDID> device-control configuration set color-filter true --filter-type Protanopia --intensity 0.5
ios -u <UDID> device-control configuration set reduce-motion false
ios -u <UDID> device-control orientation left
```

JSON is the default output (`--no-json` selects stable human-readable lines).
The color-filter JSON value uses a flat `filterType` string even though the
CoreDevice wire dictionary is `filterType: {"name": "..."}`. Unknown style,
filter, text-size, and orientation strings from a newer device are preserved
in JSON; setters still reject unknown values until their semantics are known.
The daemon does not expose getters for `increase-contrast` or
`liquid-glass-opacity`; attempting `configuration get` for either returns a
clear local error. Devices that only expose a legacy or `.shim.remote` route
are rejected before the XPC request because these operations require the
modern CoreDevice envelope.

Display media and Universal HID authorization
---------------------------------------------

```sh
ios -u <UDID> device-control display status
ios -u <UDID> device-control display video --max-units 10 --timeout 10 --output screen.hevc
ios -u <UDID> device-control display audio --max-units 10 --timeout 10 --output audio.aac
ios -u <UDID> hid --confirm tap --x 0.5 --y 0.5
ios -u <UDID> hid --confirm text 'hello'
```

`video` and `audio` return bounded metadata JSON lines (or a human summary)
and optionally save concatenated encoded access units through an atomic 0600
staging file. The output is raw encoded Annex-B HEVC or AAC-ELD payload data,
not decoded pixels/PCM; a VNC/viewer pipeline is not included. Capture and
Universal HID use the kernel tunnel so device-initiated RTP can reach the host
UDP socket. If kernel TUN support or a concrete CDTunnel client address is not
available, the command fails with a diagnostic instead of advertising `::1` or
claiming that input/capture succeeded.

## Logs, diagnostics, and packet capture

```sh
ios -u <UDID> syslog
ios -u <UDID> diagnostics list
ios -u <UDID> diagnostics sysdiagnose
ios -u <UDID> diagnostics reboot
ios -u <UDID> diagnostics shutdown --force
ios -u <UDID> os-trace ps
ios -u <UDID> os-trace stream --process SpringBoard --level error,info
ios -u <UDID> os-trace live --pid 42 --subsystem com.example --match timeout
ios -u <UDID> os-trace archive ./diagnostics.tar --size-limit 1073741824
ios -u <UDID> os-trace collect ./diagnostics.logarchive --age-limit 7
ios -u <UDID> pcap --output device.pcap
ios -u <UDID> ip --json
ios -u <UDID> notify wait com.apple.mobile.lockdown.host_attached
```

`ios ip` reads `WiFiAddress` only as the packet-source selector, then
discovers IPv4 and IPv6 addresses from matching `pcapd` Ethernet frames. It
does not guess addresses from lockdown alone and has finite time, packet, and
byte budgets. The device must expose a stable Wi-Fi MAC: iOS's
automatic/private Wi-Fi address rotation can make the source-MAC match fail,
in which case the command times out rather than returning an address from an
unrelated interface.

Managed Wi-Fi profiles can be installed or removed through MCInstall:

```sh
ios -u <UDID> wifi install "Office Wi-Fi" --password "$WIFI_PASSWORD" \
  --profile-output office.mobileconfig
ios -u <UDID> wifi remove "Office Wi-Fi" --force
```

Passwords are never included in command output or JSON. An explicitly saved
profile is created with owner-only (0600) permissions; profile removal and
shutdown require `--force`.

`diagnostics sysdiagnose` uses the iOS 17+ CoreDevice diagnostics service when
that service is exposed in RSD. It runs in dry-run mode and prints the preferred
archive name plus expected byte count; it does not download or collect the full
sysdiagnose bundle.

Use a test device for commands that restart the device, change state, or collect
large streams.

The syslog client decodes the BSD `vis(3)` escapes used by Apple's relay, while
retaining unknown/control escapes as text. `--process Name` also matches the
common `Name(Library)` sender annotation; an exact annotated name remains
accepted. `--filter`, regex, PID, count, timeout, parsed text, and JSON output
continue to apply after decoding and parsing.

`os-trace stream` (also available as `os-trace live`) performs the binary
`StartActivity` handshake and emits one structured record per line. Filters
for PID, process, level, subsystem, category, message inclusion, message
exclusion, and regular expressions are applied without changing the core stream
API. Repeat `--match` to require every substring, repeat `--exclude` to reject
if any substring matches, and repeat `--regex` to accept if any expression
matches the message. `--ignore-case` applies to all text filters. Invalid or
oversized expressions are rejected before connecting. JSON is the default
output; use `--no-json` for a human-readable line. Ctrl+C and `--timeout`
cancel the complete operation, including connection, RSDCheckin, handshake,
and reads. On iOS 17 and later it establishes the userspace CoreDevice tunnel
and uses the `com.apple.os_trace_relay.shim.remote` RSD service; older devices
use the classic `com.apple.os_trace_relay` lockdown service. Stream JSON uses
`schema_version: 2`, standard UUID strings, an RFC3339 `timestamp`, and enum
`level`; the `*_hex`, `timestamp_parts`, and `level_value` fields retain the
prior raw representations for consumers migrating from the initial schema.
`os-trace archive` requests the relay's raw PAX-format tar stream and installs
it with a 0600 temporary file followed by an atomic rename. It does not create
or claim to create a zip file. `os-trace collect` performs the same transfer,
validates tar metadata and path/type/size limits, extracts into a 0700 staging
directory, and atomically installs a new `.logarchive` directory; an existing
collect destination is refused. Both commands use the classic
`com.apple.os_trace_relay` service on older devices and the
`com.apple.os_trace_relay.shim.remote` RSD service on iOS 17+, and `--timeout`
covers connection, transfer, validation, and extraction. Device-side
`SizeLimit`, `AgeLimit`, and Unix-timestamp `StartTime` are sent unchanged;
the core API also enforces bounded host-side archive, entry, and extracted-byte
budgets. Ctrl+C removes incomplete temporary output.

## Backups

```sh
ios -u <UDID> backup version
ios -u <UDID> backup create ./backup
ios -u <UDID> backup create ./backup --full
ios -u <UDID> backup info ./backup
ios -u <UDID> backup list ./backup
# Keep selected domains/files while creating a backup (repeatable filters).
ios -u <UDID> backup create ./backup --only sms --only-regex 'Library/Notes/.*'
ios -u <UDID> backup create ./backup --only sms --patch-manifest
# Host-only operations; --source is the backup directory identifier, not a device connection.
ios backup unback ./backup --source <UDID> --output ./expanded
ios backup extract ./backup HomeDomain Library/SMS/sms.db --source <UDID> --output ./sms.db
ios backup list-local ./backup --source <UDID>
# Device-side MobileBackup2 operations (the connected UDID is required).
ios -u <UDID> backup unback-device ./backup
ios -u <UDID> backup extract-device ./backup HomeDomain Library/SMS/sms.db
ios -u <UDID> backup encryption
ios -u <UDID> backup encryption on --password '<new-password>'
ios -u <UDID> backup encryption off --password '<current-password>'
```

`--only` accepts the `bookmarks`, `call_history`, `contacts`, `messages`, `sms`,
and `whatsapp` presets; `--only-regex` is matched against the device and
Manifest.db/Manifest.mbdb domain/path forms. Selection is host-side and uses the normal
`Backup` DeviceLink request. `--patch-manifest` prunes rejected Manifest.db
rows (or legacy Manifest.mbdb records) and stored payloads after the transfer and requires a selection. The
optional `backup2-manifest` feature (included by the CLI's `full` feature)
provides local SQLite/MBDB manifest filtering and extraction. Modern encrypted
backups (ProductVersion greater than 10.2) are supported when `--password` is
provided: the host verifies the BackupKeyBag, decrypts/re-encrypts Manifest.db,
and decrypts file payloads. Passwords and key material are never included in
operation output. Legacy backups using Manifest.mbdb (10.2 and older) use the
single PBKDF2-HMAC-SHA1 keybag path, flat file-ID payload names, and are rewritten
without inventing an encrypted manifest; encrypted legacy operations require
`--password`. Non-empty
Manifest.db WAL/SHM/journal sidecars are also rejected because they cannot be
decrypted safely as an isolated completed backup.

`unback` and `extract` are retained compatibility names for local operations
over a completed backup. They preserve regular-file bytes, basic Unix
permission bits, and safe relative symlinks. The `list-local` command lists
redacted manifest entries without contacting a device; `list` remains the
device-side MobileBackup2 operation. The explicit
`unback-device` and `extract-device` commands send the real MobileBackup2
`Unback` and `Extract` messages over the connected device's DeviceLink session;
the device performs any encrypted-backup handling. Their backup directory is
the local DeviceLink transfer workspace, and is not an output directory for a
host expansion. Each device-side Unback/Extract exchange has one five-minute
deadline covering version negotiation, the transfer loop, and best-effort
disconnect cleanup; a stalled peer is abandoned when that deadline expires.
Device-side `backup encryption on|off` uses the real
MobileBackup2 `ChangePassword` request; it does not decrypt or rewrite a local
backup. No backup erase-device command is exposed because that destructive
DeviceLink operation needs an explicit two-step confirmation contract.

The backup directory is a security boundary: use a path owned exclusively by
the current user and do not share it with untrusted local processes. Existing
path and symlink checks protect against ordinary traversal and pre-existing
symlink escapes, but concurrent replacement of an intermediate directory is
not atomically prevented; keep the root and its parent trusted for the whole
operation. See [backup root safety](troubleshooting.md#backup-root-safety).

MobileBackup2 transfers use bounded host buffers and smaller transfer frames.
When the device reports insufficient space after a purge request, the error
includes the last host free-space report and the device's estimated requirement
when available. This is diagnostic information only; the backup command still
returns a failure and does not delete arbitrary host files.

## Developer services

```sh
ios -u <UDID> ddi status
ios -u <UDID> ddi mount --path /path/to/DeveloperDiskImage.dmg
ios -u <UDID> instruments ps
ios -u <UDID> instruments cpu
ios -u <UDID> instruments sysmon-process <PID>
ios -u <UDID> instruments launch com.example.app
ios -u <UDID> instruments kill <PID>
ios -u <UDID> debugserver --help
ios -u <UDID> debug --help
ios -u <UDID> symbols list
ios -u <UDID> accessibility-audit capabilities
ios -u <UDID> webinspector opened-tabs
ios -u <UDID> webinspector launch https://example.com
ios -u <UDID> webinspector launch https://example.com --bundle-id com.example.app
ios -u <UDID> webinspector js-shell --bundle-id com.apple.mobilesafari
printf 'document.title\n1 + 1\n.exit\n' | ios -u <UDID> webinspector js-shell --page-id 1
```

Many developer services require Developer Mode, a mounted Developer Disk Image,
or the CoreDevice tunnel path on newer iOS versions.

`webinspector launch` uses Remote Automation to launch the selected bundle,
create one browsing context, optionally navigate to the positional URL, and
report page/title/connection metadata. The connection, launch, page wait,
navigation, and title lookup share one deadline; Ctrl+C cancels the operation
without retrying the launch. The default bundle is Safari.

`webinspector js-shell` selects an existing Web/WebPage/JavaScript page by
`--page-id` or `--bundle-id` (otherwise the first matching page), optionally
opens Safari and navigates first, then evaluates stdin one line at a time.
It works with a TTY or a piped script, accepts `.exit`, `exit`, and `quit`, and
continues after an evaluation error by default. Use
`--continue-on-error=false` to stop at the first error. JSON mode emits one
session metadata/result/error object per line; human mode keeps results on
stdout and evaluation errors on stderr. `--timeout` bounds connection and
initial page setup, and each individual evaluation; it does not expire an
otherwise idle interactive shell. Ctrl+C or EOF exits the shell.

Comparable upstream workflows:

- go-ios: `ios image ...`, `ios instruments ...`, `ios debug ...`, `ios ax ...`.
- pymobiledevice3: `pymobiledevice3 mounter ...`,
  `pymobiledevice3 developer dvt ...`, `pymobiledevice3 webinspector ...`.

## iOS 17+ tunnel, RSD, and forwarding

```sh
ios -u <UDID> tunnel start --userspace
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
ios tunnel list
ios -u <UDID> rsd services
ios -u <UDID> rsd services --all --features
ios -u <UDID> forward 1234 62078 --once
```

Userspace tunnels expose a local TCP proxy. Kernel TUN mode may require
administrator or root privileges. See [tunnel.md](tunnel.md) for details.

TCP dials and initial protocol setup on the tunnel/RSD/XPC paths are bounded by
15 seconds; TCP-backed remote pairing and lockdown setup use the same bound. A
stale tunnel route therefore returns a timeout error instead of waiting for the
operating system's much longer TCP retry window. The timeout is not a guarantee
that every later service operation completes within 15 seconds; device-side
request and stream timeouts remain service-specific.

For machine-readable output, the default `rsd services` mode preserves the
legacy sorted JSON array of objects with `name` and `port`. Pass the
subcommand's explicit `--features` flag to add `features` to that JSON; use the
global `--no-json` flag for human-readable output, where the same flag adds
feature lines. The default JSON mode of `rsd check` likewise preserves its
existing fields, while `rsd check --features` adds the selected service's
feature list. A requested but missing RSD feature list is represented by `[]`
and is treated as unknown capability metadata rather than an explicit deny-all
list.

Comparable upstream workflows:

- go-ios: `ios tunnel start`, `ios tunnel ls`, `ios rsd ls`, `ios forward`.
- pymobiledevice3: RemoteXPC/tunnel workflows and
  `pymobiledevice3 usbmux forward`.

## Management, profiles, and supervision

```sh
ios -u <UDID> profiles list
ios -u <UDID> provisioning list
ios -u <UDID> httpproxy set proxy.example.com 8080 --p12 identity.p12
ios -u <UDID> httpproxy remove
ios prepare create-cert ./supervision
ios -u <UDID> prepare --cert-der ./supervision.der
ios -u <UDID> power-assert --timeout 10
ios -u <UDID> preboard create
ios -u <UDID> restore enter-recovery
ios -u <UDID> restore events --count 5 --timeout-secs 30
ios -u <UDID> erase --force
```

These commands can change persistent device state. Prefer a test device, inspect
`--help`, and confirm the expected iOS version and supervision state before
running them.

`restore events` is a read-only RestoreRemoteServices event consumer. It waits
for lifecycle messages such as progress, status, checkpoint, data request,
previous log, or restored-crash notifications; it does not start a restore by
itself. Data request events include an `async` flag so later restore-loop work
can distinguish `DataRequestMsg` from `AsyncDataRequestMsg`.

## Rust API

Use `ios-core` for a high-level entry point:

```rust
use ios_core::{ConnectOptions, list_devices};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = list_devices().await?;
    let device = devices.first().ok_or("no device found")?;
    let connected = ios_core::connect(&device.udid, ConnectOptions {
        skip_tunnel: true,
        ..Default::default()
    }).await?;

    println!("{:?}", connected.lockdown_get_value(Some("DeviceName")).await?);
    Ok(())
}
```

Use lower-level modules when you need direct control over usbmux, lockdown
sessions, service startup, DTX, XPC, or tunnel setup.

For RSD consumers, `ios_core::RsdHandshake::services` maps each service name to
an `ios_core::ServiceDescriptor` containing its `port` and advertised
`features`. Construct descriptors with `ServiceDescriptor::new(port)` and use
`supports_feature` when capability metadata is present. Since this public
descriptor may gain fields in future 0.1.x releases, downstream code should
prefer the constructor and accessors over struct literals.

## Related documents

- [cli-map.md](cli-map.md) maps `ios` commands to comparable go-ios and
  pymobiledevice3 command families.
- [features.md](features.md) explains feature flags for library users.
- [tunnel.md](tunnel.md) covers CoreDevice tunnel setup in more detail.
- [troubleshooting.md](troubleshooting.md) covers common host and device issues.

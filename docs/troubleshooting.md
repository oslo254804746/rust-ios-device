# Troubleshooting

## Host diagnostic report

Run `ios doctor` without `--udid` or `IOS_UDID` for a read-only host JSON report,
or `ios --no-json doctor` for human-readable output. It does not require a
device or create a pairing:

```bash
ios doctor
ios --no-json doctor --mdns --timeout 5
ios doctor --tunnel-port 49151
ios doctor --tunnel-port 49151 --token-file PRIVATE_CLIENT_TOKEN_FILE
```

The report checks usbmuxd enumeration, IPv6 loopback sockets, Linux TUN presence,
compiled userspace tunnel support, and the optional loopback tunnel manager.
`--mdns` also observes mobdev2 and RemotePairing advertisements. Each probe is
bounded by `--timeout` (1–60 seconds); Bonjour has an additional one-second
setup/cleanup allowance. The optional Bonjour phase runs after the other probes.
The manager probe bypasses HTTP proxies and redirects and caps responses at 1 MiB.
Use a private `--token-file` with scoped list permission for a policy-mode
manager. Without a token, a protected optional manager produces a warning and
hint. An explicitly supplied unreadable credential or HTTP 401/403 is a failed
check, causing exit 1; the remote error body and credential are omitted from
reports. Private-file read errors retain a safe error category/correlation.

Exit status is 1 when a required check fails (including unavailable usbmuxd).
Warnings, an absent optional tunnel manager, and unknown checks do not cause a
failure exit. Enumeration success does not prove device service connectivity;
no Bonjour responses does not prove that multicast is broken. TUN presence does
not prove permission to create an interface. DDI usability still needs separate
device checks. The host report contains counts, not device identifiers or pairing
records. CLI logs go to stderr to keep JSON clean.

## Explicit device diagnostic report

```bash
ios --udid <UDID> doctor --timeout 5
ios --udid <UDID> --no-json doctor
```

An explicit `--udid` or `IOS_UDID` selects schema 2 (`host_and_device`), adding
`device.checks` to the host report. With neither selector the report stays schema
1 (`host`). Scripts that previously set an ignored doctor target must accept
schema 2 or clear their device selector to request the original host report.

Each device probe gets a fresh connection and its own total `--timeout`: existing
trust must complete a read-only lockdown query; Developer Mode records explicit
true/false or omitted metadata; DDI records signature presence. Signature presence
does not prove that an image is mounted or usable. Doctor does not pair, activate
a tunnel, mount an image or change settings. Ctrl-C cancels remaining probes.

Device checks distinguish `not_probed`, `unreachable`, `permission_denied`,
`service_unavailable`, `metadata_missing` and other `failed` states. An explicit
false is a warning with `value: false`; unknown metadata has no value. Failed
checks cause exit 1 after the report; the JSON machine error retains the first
failed device probe's category. Target IDs are hashed and private response data
is omitted. Without an explicit existing RSD route, its service/route checks
stay `not_probed`.

Pairing files must be regular files, or existing links to them. Unix FIFOs are
rejected without waiting for a writer. Filesystem metadata and regular-file reads
can still block in the OS: a probe deadline cannot forcibly cancel that worker,
and runtime exit can wait for it. This is not evidence of bounded physical exit
on stalled filesystems. See the [capability inventory](capabilities.md) for
host and device validation records.

## Observe an existing RSD route

```bash
ios --udid <UDID> doctor --rsd-address '[fd00::1]:58783'
ios --udid <UDID> doctor --rsd-address '[fd00::1]:58783' --rsd-proxy-port 49152 --rsd-service com.apple.coredevice.deviceinfo
```

Use an already established IPv6 RSD endpoint; the optional existing userspace
proxy listens on IPv4 loopback. Both ports must be nonzero. Direct routes preserve
numeric scope/flow; proxy routes reject scope/flow because their 20-byte prelude
cannot carry them. An explicit target is required and checked against RSD identity
before attributing an advertisement. Doctor owns its fresh observation sockets and
does not start, stop or adopt the tunnel/proxy, pair, or execute the selected service.

Each observation uses one total `--timeout` across identity, dial, bootstrap and
fresh compatibility attempts. Missing metadata remains unknown; malformed metadata
fails. A complete checked directory can establish advertisement/absence, and only
complete port metadata supports positive route observations. This proves neither
selected-service execution nor DDI usability. Reports omit raw endpoints, selectors
and Features; unrelated earlier existing-trust failures can remain the machine-error
primary.

## Device is not listed

- Unlock the device and accept the trust prompt.
- Try a different USB cable or port.
- Restart usbmuxd or Apple Mobile Device Support.
- On Linux, check udev permissions and whether the current user can access the USB device.
- On Windows, confirm Apple Mobile Device Support is installed.

## Pairing or lockdown fails

- Keep the device unlocked during pairing.
- Check that the host clock is correct.
- If stale pair records are suspected, remove them only after confirming you do not need the existing trust relationship.
- Pair records are sensitive. Do not share or commit them.

## Tunnel fails

- CoreDevice tunnel paths require a compatible device/iOS version and trusted pairing material.
- Prefer `--userspace` mode before kernel mode. The userspace tunnel includes a
  20ms ICMPv6 Echo Request Nudge (flushing USB NCM packet aggregation buffers on
  iOS 17–27.2) and IPv6 UDP datagram forwarding (supporting Display/RTP video/audio
  capture and Universal HID media authorization without administrator/root TUN
  privileges).
- Kernel TUN mode may require root or administrator privileges.
- Network discovery depends on mDNS/Bonjour visibility and local firewall rules.
- Some Wi-Fi paths require Wi-Fi connections to be enabled through lockdown and may still vary by iOS version.

## Device paths rewritten under Git Bash (Windows)

On Windows, running the CLI from Git Bash (MSYS) silently rewrites arguments
that look like host paths. A device path such as `/` passed to an AFC command
is converted to the Git Bash installation root before the CLI process starts:

```bash
$ ios file ls /
# the CLI actually receives: C:/Program Files/Git/
# AFC answers: object not found (8), because the rewritten host path does
#              not exist on the device
```

This explains `object-not-found(8)` failures on `file ls /` and `file stat /`
when invoked from MSYS/Git Bash: the shell rewrites `/` to `C:/Program Files/Git/`
before the CLI starts. Feeding the same rewritten argument to Rust,
pymobiledevice3, and go-ios makes all three fail identically with status 8.

Verified workaround — disable MSYS path conversion for the command:

```bash
MSYS_NO_PATHCONV=1 ios file ls /
MSYS_NO_PATHCONV=1 ios file stat /
```

With conversion disabled, the root listing and stat succeed. Escaping styles
such as a doubled `//` have not been verified; do not assume they are
equivalent to disabling conversion.

Notes for test harnesses:

- Record the arguments the child process actually received, not the command
  line as typed. Judge `/`, `.`, and deliberately nonexistent paths as
  separate cases: `.` is not rewritten, and a nonexistent device path must
  still produce a clean `object not found (8)` business error.
- The shell may apply this rewriting to POSIX-looking arguments. Treat every
  device-path argument as exposed and verify per command; do not assume a
  fixed list of affected commands.

## RSD services list is empty or a service is unavailable

`ios rsd services` deliberately shows only `com.apple.coredevice.*` services
by default. An empty default list is the documented filter behavior, not proof
that the device's RSD directory is empty or that the tunnel is broken.

- `rsd services --all` lists the complete RSD directory.
- `rsd services --prefix <prefix>` narrows the listing by name prefix (for
  example `com.apple.remote`).
- `rsd check <name>` resolves one service and reports whether it is
  available. Use it to check the service a command depends on; for example,
  `rsd check com.apple.remote.installcoordination_proxy` checks the canonical
  `com.apple.remote.installcoordination_proxy` entry. Do not substitute the
  `.shim.remote` form for this RemoteXPC service.

The catalog itself differs per device, iOS version, Developer Mode state, and
mounted Developer Disk Image:

- On an `iPhone11,8 (iOS 18.7)` without Developer Mode enabled, the `--all`
  catalog exposed 59 services with no `com.apple.coredevice.*` entries.
- On an `iPhone16,2 (iOS 27.2)` with Developer Mode enabled and Cryptex DDI
  `com.apple.MobileAsset.DDI` `v27.1.5228.8` mounted via `ios -u <UDID> ddi auto`,
  the full catalog exposes `com.apple.coredevice.*` services (such as
  `appservice`, `deviceinfo`, `fileservice.control`/`.data`, `pasteboardservice`,
  `screencaptureservice`, `displayservice`, `hid.indigo`, `hid.universalhidservice`,
  `configuration`, `devicecontrol`, and `diagnosticsservice`) as well as
  `com.apple.internal.dt.remote.debugproxy` and Instruments DTX shims.

If a CoreDevice-dependent command fails, first run `rsd services --all` and
`rsd check <service>` on the same device, and check `ios -u <UDID> ddi status`.

## Backup root safety

Treat the directory passed to `ios backup create`, `info`, `list`, and `restore`
as a private trust boundary. It must be owned by the current user and not be
writable by, or shared with, an untrusted local process. Use a private directory
(on Unix, mode `0700` is recommended) and keep its parent path trusted; do not
use a shared `/tmp` directory, network share, or a path managed by another
user/service.

MobileBackup2 validates path components and rejects symlinks that already exist
when an operation starts. Those checks prevent ordinary path traversal and
pre-existing symlink escapes, but they do not provide an atomic dirfd/openat2
guarantee. A process that can concurrently replace an intermediate directory
after validation can still race a later filesystem operation. Keep the backup
root exclusive until the operation finishes; this limitation is especially
important when restoring or overwriting a backup.

## Developer services fail

- Enable Developer Mode where the device requires it (`ios -u <UDID> amfi reveal-developer-mode` / `enable-developer-mode`).
- Mount a compatible Developer Disk Image when a service depends on developer tooling (`ios -u <UDID> ddi auto`; on iOS 27+, this automatically installs Cryptex DDI `v27.1.5228.8`).
- Confirm that a test bundle, WebDriverAgent runner, app, or provisioning profile exists before running commands that refer to it.

## Build fails on Linux

Install OpenSSL headers and `pkg-config`:

```sh
sudo apt-get install -y libssl-dev pkg-config
```

Distribution package names may differ.

## Build fails on Windows

OpenSSL must be installed via vcpkg with static linking:

```powershell
vcpkg install openssl:x64-windows-static-md
```

Set the following environment variables before building:

```powershell
$env:VCPKG_ROOT = $env:VCPKG_INSTALLATION_ROOT   # or your vcpkg root
$env:VCPKGRS_TRIPLET = "x64-windows-static-md"
$env:OPENSSL_STATIC = "1"
```

If using GitHub Actions runners, `VCPKG_INSTALLATION_ROOT` is pre-set. For local development, point `VCPKG_ROOT` at your vcpkg checkout.

## Python build fails

Use Python 3.9+ and set `PYO3_PYTHON` in your shell if PyO3 picks the wrong interpreter:

```sh
cd crates/ios-py
PYO3_PYTHON="/path/to/python" uvx maturin develop
```

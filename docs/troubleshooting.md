# Troubleshooting

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
- Try userspace mode before kernel mode.
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

This explains the historical `object-not-found(8)` failures on `file ls /`
and `file stat /`: at the time of the failure, the shell had already rewritten
`/` to `C:/Program Files/Git/`. Feeding the same rewritten argument to Rust,
pymobiledevice3, and go-ios makes all three fail identically with status 8,
which by itself proves nothing about device policy or about the Rust AFC
encoding; the shell conversion happens before client-specific AFC handling,
so compare the actual received argument and protocol encoding separately.

Verified workaround (2026-08-30, three-way comparison on a real device) —
disable MSYS path conversion for the command:

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

The catalog itself differs per device and iOS version. On one device
(iPhone11,8, iOS 18.7.9, checked 2026-08-30), the full `--all` catalog held 59
services with an identical name set in Rust, pymobiledevice3, and go-ios — but
that count and name set describe that device and round, not a fixed catalog.
Notably, that device exposed no `com.apple.coredevice.*` entries at all, so
`info lock-state`, which needs `com.apple.coredevice.deviceinfo`, fails on it.
Why a given device lacks those entries is unknown; do not attribute the
absence to Developer Mode, a Developer Disk Image, supervision state, or a
routing defect without fresh evidence.

If a CoreDevice-dependent command fails, first run `rsd services --all` and
`rsd check <service>` on the same device. A service that is absent from the
full catalog is not exposed by the device. A friendlier error message for this
case is a possible, separate improvement; any change there must first
re-confirm the existing error contract and its compatibility.

Coverage still requiring real-device evidence: RSD connection establishment
has only been observed through the queued bootstrap path; the direct, legacy,
and passive establishment paths remain untriggered, and behavior on devices
that do expose `com.apple.coredevice.*` is unverified.

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

- Enable Developer Mode where the device requires it.
- Mount a compatible Developer Disk Image when a service depends on developer tooling.
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

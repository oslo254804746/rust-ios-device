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

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

## Developer services fail

- Enable Developer Mode where the device requires it.
- Mount a compatible Developer Disk Image when a service depends on developer tooling.
- Confirm that a test bundle, WebDriverAgent runner, app, or provisioning profile exists before running commands that refer to it.

## Valeria recording fails

- `ios valeria record` is experimental and records video only as raw Annex-B H.264.
- Stop other tools that may hold the device USB interfaces, then unplug/replug the device and retry.
- On Linux, ensure udev permissions allow access to the Apple USB device and that another process has not claimed the interface. This is the preferred host path for validating the raw USB backend.
- On Windows, the stock Apple Mobile Device / Apple USBMux driver stack can expose the device to usbmux tools while still refusing raw `nusb` access. `failed to claim Apple USB interface ... Windows error 50` means the current Windows raw USB backend cannot open that interface on this host; use a Linux/libusb-compatible host or a driver stack that permits raw USB access.
- On macOS, Apple services may already own the interface. Close screen mirroring, QuickTime, Xcode device windows, and other device tools before retrying.
- If recording succeeds but the file does not play, verify the file starts with `00 00 00 01` and decode it with `ffplay -f h264 capture.h264`.

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

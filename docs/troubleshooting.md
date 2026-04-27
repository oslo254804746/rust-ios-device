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

## Build fails on Linux

Install OpenSSL headers and `pkg-config`:

```sh
sudo apt-get install -y libssl-dev pkg-config
```

Distribution package names may differ.

## Python build fails

Use Python 3.9+ and set `PYO3_PYTHON` in your shell if PyO3 picks the wrong interpreter:

```sh
cd crates/ios-py
PYO3_PYTHON="/path/to/python" uvx maturin develop
```

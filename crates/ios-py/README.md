# rust-ios-device-tunnel

Python bindings for communicating with iOS devices — device discovery, CoreDevice tunnel management, and asyncio integration.

Built on top of [rust-ios-device](https://github.com/oslo254804746/rust-ios-device), a Rust library for iOS device interaction through usbmuxd, lockdown, and CoreDevice/RemoteXPC protocols.

## Install

```sh
pip install rust-ios-device-tunnel
```

Requires Python 3.9+. Pre-built wheels are available for Linux (x86_64, aarch64), macOS (Apple Silicon), and Windows (x86_64).

## Quick start

```python
import ios_rs

# List connected devices
devices = ios_rs.list_devices()
for d in devices:
    print(f"{d['udid']}  {d['connection_type']}")
```

## Tunnel usage

Start a CoreDevice tunnel to a trusted iOS device:

```python
import ios_rs

devices = ios_rs.list_devices()
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")

print(tunnel.server_address)   # device tunnel IPv6 address
print(tunnel.rsd_port)         # Remote Service Discovery port
print(tunnel.userspace_port)   # local TCP proxy port
print(tunnel.services)         # discovered RSD service names
print(tunnel.connect_info())   # connection summary dict

tunnel.close()
```

Kernel TUN mode (`mode="kernel"`) requires root/administrator privileges. Userspace mode works without elevated permissions and is the default.

## asyncio integration

The userspace tunnel includes a context manager that patches `asyncio.open_connection` so asyncio-based libraries can connect to the device tunnel transparently:

```python
import asyncio
import ios_rs

devices = ios_rs.list_devices()
tunnel = ios_rs.start_tunnel(devices[0]["udid"])

with tunnel.asyncio_proxy():
    # Connections to the tunnel IPv6 address are routed
    # through the local userspace proxy automatically.
    reader, writer = asyncio.get_event_loop().run_until_complete(
        asyncio.open_connection(tunnel.server_address, tunnel.rsd_port)
    )

tunnel.close()
```

## API reference

| Function / Class | Description |
|---|---|
| `ios_rs.list_devices()` | Returns a list of dicts with `udid`, `device_id`, and `connection_type` for each connected device. |
| `ios_rs.start_tunnel(udid, mode="userspace")` | Opens a CoreDevice tunnel. Returns a `Tunnel` object. |
| `Tunnel.server_address` | Device tunnel IPv6 address. |
| `Tunnel.rsd_port` | Remote Service Discovery port. |
| `Tunnel.userspace_port` | Local TCP proxy port (userspace mode only). |
| `Tunnel.services` | List of discovered RSD service names. |
| `Tunnel.connect_info()` | Dict summarizing connection parameters. |
| `Tunnel.asyncio_proxy()` | Context manager that patches `asyncio.open_connection`. |
| `Tunnel.close()` | Tears down the tunnel. |

## Requirements

- A trusted iOS device connected via USB.
- usbmuxd (Linux), Apple Mobile Device Support (Windows), or macOS device support components.
- For CoreDevice tunnels: a compatible iOS version (17+) with pairing material on the host.

## Links

- Source: <https://github.com/oslo254804746/rust-ios-device>
- Rust crate: <https://crates.io/crates/ios-core>
- Issues: <https://github.com/oslo254804746/rust-ios-device/issues>

## License

Licensed under either of Apache-2.0 or MIT at your option.

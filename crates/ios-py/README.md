# rust-ios-device-tunnel

Python bindings for device discovery, CoreDevice tunnel management, and asyncio
transport bridging.

Built on top of [rust-ios-device](https://github.com/oslo254804746/rust-ios-device).
This package exposes device discovery plus CoreDevice tunnel metadata and a local
userspace bridge; it does not expose the Rust lockdown or service clients as
direct Python APIs.

## Install

```sh
pip install rust-ios-device-tunnel
```

Requires Python 3.9+. Pre-built abi3 wheels are published for:

- Linux x86_64 (`x86_64-unknown-linux-gnu`)
- Linux aarch64 (`aarch64-unknown-linux-gnu`)
- macOS Apple Silicon (`aarch64-apple-darwin`)
- Windows x86_64 (`x86_64-pc-windows-msvc`)

A source distribution is also available for other targets via `pip install
--no-binary rust-ios-device-tunnel rust-ios-device-tunnel`.

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
print(tunnel.service_ports)    # service name -> device TCP port
print(tunnel.service_features) # service name -> advertised identifiers (possibly [])
print(tunnel.connect_info())   # connection summary dict

tunnel.close()
```

The `services` list and both mapping attributes use stable service-name order.
`service_ports` and `service_features` contain every discovered service;
`service_features` uses an empty list when the device did not advertise
capability metadata. Missing metadata is not an explicit deny-all result.
Tunnel setup and initial RSD/XPC connection setup are limited to 15 seconds.

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

## pymobiledevice3 interoperability

`Tunnel.asyncio_proxy()` can also be used as a transport bridge for
asyncio-based RemoteXPC clients. For example, pymobiledevice3's
`RemoteServiceDiscoveryService` calls `asyncio.open_connection()` internally, so
it can run over an `ios_rs` userspace tunnel without requiring pymobiledevice3's
own privileged tunnel startup path:

```sh
cd crates/ios-py
uvx maturin develop
uv pip install pymobiledevice3
uv run python examples/pymobiledevice3_coredevice_bridge.py --udid <UDID>
```

The example is read-only by default and reports RSD service presence. Add
`--probe-coredevice` to try opening selected pymobiledevice3 CoreDevice service
classes through the same tunnel; the diagnostics probe is connect-only and does
not capture a full sysdiagnose.

## API reference

| Function / Class | Description |
|---|---|
| `ios_rs.list_devices()` | Returns a list of dicts with `udid`, `device_id`, and `connection_type` for each connected device. |
| `ios_rs.start_tunnel(udid, mode="userspace")` | Opens a CoreDevice tunnel. Returns a `Tunnel` object. |
| `Tunnel.server_address` | Device tunnel IPv6 address. |
| `Tunnel.rsd_port` | Remote Service Discovery port. |
| `Tunnel.userspace_port` | Local TCP proxy port (userspace mode only). |
| `Tunnel.services` | List of discovered RSD service names. |
| `Tunnel.service_ports` | Dict mapping RSD service names to device TCP ports. |
| `Tunnel.service_features` | Dict mapping every RSD service to advertised capability identifiers; an empty list means metadata was not advertised. |
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

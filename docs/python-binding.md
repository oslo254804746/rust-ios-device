# Python binding

The `ios-py` crate publishes the `rust-ios-device-tunnel` Python distribution and builds a PyO3 extension module imported as `ios_rs`. It currently exposes device listing and CoreDevice tunnel helpers for compatible devices.

## Build locally

```sh
uv pip install rust-ios-device-tunnel
```

From a source checkout:

```sh
cd crates/ios-py
uvx maturin develop
```

If needed, set `PYO3_PYTHON` in the shell:

```sh
export PYO3_PYTHON=/path/to/python
```

Do not commit machine-specific Python paths.

## API

```python
import ios_rs

devices = ios_rs.list_devices()
print(devices)

tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
print(tunnel.server_address)
print(tunnel.rsd_port)
print(tunnel.userspace_port)
print(tunnel.services)
print(tunnel.connect_info())
tunnel.close()
```

`start_tunnel(..., mode="kernel")` requests kernel TUN mode and may require elevated privileges.

## asyncio proxy helper

Userspace tunnel mode includes a context manager that temporarily patches `asyncio.open_connection` for clients that connect to the tunnel IPv6 address:

```python
with tunnel.asyncio_proxy():
    # asyncio.open_connection(tunnel.server_address, some_port)
    # is routed through the local userspace proxy.
    pass
```

The patch is process-local and should be kept scoped with the context manager.

## pymobiledevice3 bridge example

Because pymobiledevice3's RemoteXPC stack uses `asyncio.open_connection()`,
`Tunnel.asyncio_proxy()` can act as a userspace transport bridge for it. This is
useful on hosts where pymobiledevice3's own tunnel command needs elevated
privileges, but `ios_rs.start_tunnel(..., mode="userspace")` can already create
the local proxy.

```sh
cd crates/ios-py
uvx maturin develop
uv pip install pymobiledevice3
uv run python examples/pymobiledevice3_coredevice_bridge.py --udid <UDID>
```

The example reports RSD peer metadata and service presence. With
`--probe-coredevice`, it opens selected pymobiledevice3 CoreDevice service
classes through the `ios_rs` tunnel. It does not invoke WDA/XCTest, restore,
reset, or full sysdiagnose capture.

# Python binding

The `ios-py` crate publishes the `rust-ios-device-tunnel` Python distribution and builds a PyO3 extension module imported as `ios_rs`. It currently exposes device listing and iOS 17+ tunnel helpers.

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

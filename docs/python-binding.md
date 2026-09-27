# Python binding

The `ios-py` crate publishes the `rust-ios-device-tunnel` Python distribution and
builds a PyO3 extension module imported as `ios_rs`. It exposes device listing,
CoreDevice tunnel metadata, and a local userspace transport bridge for compatible
devices. It does not provide direct Python bindings for the Rust lockdown or
individual service clients.

## Build locally

```sh
uv pip install rust-ios-device-tunnel
```

From a source checkout:

```sh
# Run from the repository root; reuse .venv if it already exists.
uv venv --python 3.12 .venv
source .venv/bin/activate
cd crates/ios-py
uvx maturin develop
python -c "import ios_rs; print(ios_rs.__file__)"
```

`maturin` enables the package's `extension-module` feature automatically. A
normal host-side `cargo test` intentionally leaves that feature disabled and
links the tests to the selected Python runtime instead.

With the environment activated, run the binding tests from the repository root:

```sh
PYO3_PYTHON="$VIRTUAL_ENV/bin/python" cargo test -p ios-py --no-default-features
```

On Linux, a uv-managed Python may keep `libpython` outside the dynamic linker's
search path. The embedded test runtime may also need its standard-library prefix.
Set both from the selected interpreter for that command:

```sh
ios_python_libdir=$(python -c 'import sysconfig; print(sysconfig.get_config_var("LIBDIR"))')
ios_python_prefix=$(python -c 'import sys; print(sys.base_prefix)')
PYO3_PYTHON="$VIRTUAL_ENV/bin/python" PYTHONHOME="$ios_python_prefix" \
  LD_LIBRARY_PATH="$ios_python_libdir${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
  cargo test -p ios-py --no-default-features
```

Creating a venv does not make `cargo test --workspace --all-features` a valid
host-test configuration: it still enables the packaging-only feature. Keep the
Rust workspace and Python host tests separate as described in [build.md](build.md).

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
print(tunnel.service_ports)
print(tunnel.service_features)
print(tunnel.connect_info())
tunnel.close()
```

The `services` list and both mapping attributes use stable service-name order.
`service_ports` and `service_features` contain every discovered service;
`service_features` uses `[]` when the RSD entry did not provide capability
metadata. The FFI JSON API uses the same explicit empty-list representation. In
either API, missing metadata is not an explicit deny-all result.

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

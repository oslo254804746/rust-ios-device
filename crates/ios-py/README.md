# rust-ios-device-tunnel

Python bindings for device discovery, finite existing-trust device queries and
checked services, CoreDevice tunnel management (iOS 17–27+), and asyncio
transport bridging across **iOS 15.x – iOS 27.2**.

Built on top of [rust-ios-device](https://github.com/oslo254804746/rust-ios-device).
This package exposes device discovery, `Device` and `DeviceSession` handles for
Lockdown queries, application listing, AFC file reads, and syslog streaming,
plus CoreDevice tunnel metadata and a local userspace proxy bridge (including
the 20ms ICMPv6 Echo Request Nudge and IPv6 UDP datagram forwarding).

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

Start a CoreDevice tunnel to a trusted iOS 17–27+ device:

```python
import ios_rs

devices = ios_rs.list_devices()
if not devices:
    raise SystemExit("no device found")
tunnel = ios_rs.start_tunnel(devices[0]["udid"], mode="userspace")
try:
    print(tunnel.server_address)   # device tunnel IPv6 address
    print(tunnel.rsd_port)         # Remote Service Discovery port
    print(tunnel.userspace_port)   # local TCP/UDP proxy port
    print(tunnel.services)         # discovered RSD service names
    print(tunnel.service_ports)    # service name -> device TCP port
    print(tunnel.service_features) # service name -> advertised identifiers (possibly [])
    print(tunnel.connect_info())   # connection summary dict

finally:
    tunnel.close()
```

The `services` list and both mapping attributes use stable service-name order.
`service_ports` and `service_features` contain every discovered service;
`service_features` uses an empty list when the device did not advertise
capability metadata. Missing metadata is not an explicit deny-all result.
Tunnel setup and initial RSD/XPC connection setup are limited to 15 seconds.

Kernel TUN mode (`mode="kernel"`) requires root/administrator privileges.
Userspace mode works without elevated permissions and is the default.

## asyncio integration

The userspace tunnel includes a context manager that patches
`asyncio.open_connection` so asyncio-based libraries can connect to the device
tunnel transparently:

```python
import asyncio
import ios_rs

devices = ios_rs.list_devices()
if not devices:
    raise SystemExit("no device found")
tunnel = ios_rs.start_tunnel(devices[0]["udid"])
try:
    with tunnel.asyncio_proxy():
        # Connections to the tunnel IPv6 address are routed
        # through the local userspace proxy automatically.
        async def connect_and_close():
            reader, writer = await asyncio.open_connection(tunnel.server_address, tunnel.rsd_port)
            writer.close()
            await writer.wait_closed()
        asyncio.run(connect_and_close())

finally:
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
| `ios_rs.open_device(udid)` | Opens a device session for application, AFC file, and syslog services. Returns a `Device` object. Supports sync and async context managers. |
| `ios_rs.start_tunnel(udid, mode="userspace")` | Opens a CoreDevice tunnel. Returns a `Tunnel` object. |
| `Device.product_version()` | Query iOS product version string (sync and `async_product_version()`). |
| `Device.list_apps()` | List user-installed apps as dicts with bundle ID, display name, version, path (sync and `async_list_apps()`). |
| `Device.list_files(path="/")` | List AFC directory contents (sync and `async_list_files()`). |
| `Device.read_file(path, max_bytes=1048576)` | Read a complete AFC file, rejecting excess bytes (sync and `async_read_file()`). |
| `Device.read_syslog(max_messages=100, timeout_ms=5000)` | Read the requested message count under a total, returning partial evidence on errors (sync and `async_read_syslog()`). |
| `Device.close()` / `await Device.aclose()` | Close device handle and release underlying session. |
| `Tunnel.server_address` | Device tunnel IPv6 address. |
| `Tunnel.rsd_port` | Remote Service Discovery port. |
| `Tunnel.userspace_port` | Local proxy port (userspace mode only). |
| `Tunnel.services` | List of discovered RSD service names. |
| `Tunnel.service_ports` | Dict mapping RSD service names to device TCP ports. |
| `Tunnel.service_features` | Dict mapping every RSD service to advertised capability identifiers; an empty list means metadata was not advertised. |
| `Tunnel.connect_info()` | Dict summarizing connection parameters. |
| `Tunnel.asyncio_proxy()` | Context manager that patches `asyncio.open_connection`. |
| `Tunnel.get_device()` | Return a `Device` handle sharing this tunnel's connection. |
| `Tunnel.list_apps()` / `list_files()` / `read_file()` / `read_syslog()` | Tunnel service forwarders (sync and async equivalents). |
| `Tunnel.close()` / `await Tunnel.aclose()` | Requests closure; installed asyncio patches retain the resource until restored. |

## Error handling

Device-list/tunnel operation failures keep `RuntimeError` and their safe message.
Invalid tunnel mode keeps `ValueError`. Requesting `asyncio_proxy()` without a
userspace proxy keeps `RuntimeError`. These exceptions also expose
`ios_error_schema_version` (integer `1`), `ios_error_class` (stable category),
`ios_error_phase` (fixed operation name), and `ios_error_effects`.

```python
import ios_rs

try:
    devices = ios_rs.list_devices()
except RuntimeError as error:
    if getattr(error, "ios_error_schema_version", None) == 1:
        category = error.ios_error_class
        # Handle a known category; keep a fallback for future/unknown codes.
        print(category, error.ios_error_effects)
    raise
```

Operation effects are `unknown`; input/proxy rejection before the corresponding
operation is `not_started`. A category alone does not make retry safe or confirm
device cleanup. Existing exception `args` and signatures remain unchanged.
Interpreter/argument/callback and runtime-initialization errors are outside this
mapped profile. Metadata allocation failure remains an exception with the
original safe exception as its cause. See the full
[error contract](../../docs/error-contract.md) for fields and remaining limits.

## Requirements

- A trusted iOS device connected via USB.
- `usbmuxd` (Linux), Apple Mobile Device Support (Windows), or macOS device support components.
- For CoreDevice tunnels: a compatible iOS version (17–27+) with pairing material on the host.

## Links

- [Public compatibility rules](../../docs/public-contract.md) govern factory
  signatures, attributes, exceptions, and patch ownership.
- Source: <https://github.com/oslo254804746/rust-ios-device>
- Rust crate: <https://crates.io/crates/ios-core>
- Issues: <https://github.com/oslo254804746/rust-ios-device/issues>

## License

Licensed under either of Apache-2.0 or MIT at your option.

## Finite device information

Supply an explicit UDID, immutable already loaded pairing-record `bytes`, and an
explicit numeric TCP or Unix usbmux endpoint. Construction does not contact the
device. The binding does not load credentials from a path, resolve hostnames,
pair, save credentials, start services, or create a tunnel for these queries.

```python
import ios_rs

def query_version(udid: str, pair_record_bytes: bytes) -> None:
    # The caller supplies these inputs; no implicit device/credential selection.
    with ios_rs.DeviceSession(udid, pair_record_bytes, "unix:///var/run/usbmuxd") as device:
        result = device.query("ProductVersion", timeout=15.0)
        print(result["value"])
```

`DeviceSession(udid, pair_record_bytes, endpoint, *, concurrency=4,
admission=None)` accepts 1..1048576 credential bytes. `endpoint` accepts a
numeric address such as `tcp://127.0.0.1:27015` or `tcp://[::1]:27015`, with a
nonzero port, and `unix://...` on Unix. The decoded private key uses native
zeroizing storage. The caller's input bytes are not erased by the binding.

`query(field, *, timeout=15.0, max_reply_wire_bytes=1048576,
max_output_utf8_bytes=4096, cancel=None)` accepts `ProductType`, `ProductVersion`,
`DeviceName`, or the explicit selector `root`. `root` retains exactly those three
fields, with a `None` for each omitted field. It does not return the raw root
plist or any unselected key. Every attempt opens a fresh socket, rediscovers the
exact UDID, and obtains the current DeviceID before connecting to Lockdown.

A result has this shape:

```python
{
    "metadata": {"device_id": 7, "connection_type": "USB", "product_id": None},
    "field": "DeviceName",
    "value": "",                  # Empty string is retained.
    "value_present": True,        # Missing Value instead yields False and None.
    "enable_session_ssl": True,
}
```

A present empty root dictionary yields `value_present=True` and a dictionary of
three `None` values. An omitted ProductID yields `None`; advertised zero stays
zero. Selected remote fields must be strings. Explicit remote `MissingValue`
remains a typed `not_found` exception.

Both query budgets are nonzero. Reply wire bytes count the two usbmux replies and
three Lockdown replies, including headers/prefixes and declared body lengths;
requests and TLS records do not count. The maximum configurable total is
46137356 bytes, with native per-frame caps of 16 MiB for usbmux and 4 MiB for
Lockdown. Retained output counts selected string UTF-8 plus ConnectionType UTF-8,
with a maximum of 16 MiB. Metadata permits at most 1024 entries and 16 MiB of
aggregate SerialNumber/ConnectionType UTF-8. Ordinary plist decoding has no CPU,
temporary allocation, or process RSS guarantee. Input parsing, runtime
initialization, queries, and blocking cleanup release the GIL.

## Async cancellation and cleanup

Call `query_async` from a running event loop. It accepts the same query arguments
plus `cleanup_timeout=5.0`, starts admitted native work immediately, and returns
an `asyncio.Future` subclass. Await it directly or use `asyncio.ensure_future`.
`asyncio.create_task` requires a coroutine, so wrap the await in a coroutine if a
Task is needed.

```python
import ios_rs

async def query_product_type(udid: str, pair_record_bytes: bytes, endpoint: str):
    async with ios_rs.DeviceSession(udid, pair_record_bytes, endpoint) as device:
        future = device.query_async("ProductType")
        return await future
```

`future.cancel(msg=None)` immediately signals the real native operation token.
It keeps the Future pending until the supervisor consumes the original native
JoinHandle. Direct await, same-tick `ensure_future(...).cancel()`, and cancellation
of a Task awaiting the Future preserve this cleanup requirement. A completed
non-cancellation native error takes priority over a later cancellation request.
Dropping an unawaited Future also requests real cancellation. Neither the Future
nor its observer keeps a hidden DeviceSession/client owner alive.

After successful cancellation cleanup, `CancelledError.ios_cleanup_complete`
is `True`. If cancellation cleanup exceeds its budget, the Future instead raises
`TimeoutError` with `ios_cleanup_complete=False` and an owned `query_receipt`
(also available as `ios_query_receipt`). The native original task remains under
bounded supervision. `future.receipt` is always its owned `DeviceQueryReceipt`:

```python
async def cancel_and_wait(future):
    receipt = future.receipt
    receipt.cancel()                          # Real cancellation, idempotent.
    _ = receipt.done                          # Original native task actually joined.
    await receipt.wait_async(cleanup_timeout=5.0)
```

Both receipt wait methods (`receipt.result(cleanup_timeout=5.0)` and
`await receipt.wait_async(cleanup_timeout=5.0)`) have their own bounded cleanup
deadline; cancellation of an existing receipt wait does not restart that
deadline. An async receipt wait timeout requests real cancellation and returns
`TimeoutError` with its owned receipt while supervision continues.

A receipt preserves the original owned result/error after session close,
registry reaping, or cancellation cleanup timeout. A receipt wait can raise the
original typed native cancellation/error. Receipt waiting does not restart the
operation. `DeviceCancel()` is also a shareable explicit native token; pass it
as `cancel=token`, call `token.cancel()`, and read `token.cancelled`.

`close(cleanup_timeout=5.0)` and `await aclose(cleanup_timeout=5.0)` seal new
admission, broadcast cancellation, and wait for actual leases and original
native joins within one cleanup budget. Close acknowledges local cleanup only.
A timeout leaves the session closed, preserves outstanding counts and receipts,
and permits repeated close to finish later. Close itself does not consume or
replace a query's original failure. Context managers use the same cleanup path;
if context cleanup fails, the original body exception is retained as the cleanup
exception's context. Drop signals real cancellation and does not report that
unfinished cleanup completed. `closed`, `active`, and `pending_cleanup` expose
local state.

## Admission and native fixture validation

`DeviceAdmission(limit=64)` accepts 1..64 and shares native lease admission plus a
separate hard limit for tasks whose original JoinHandle has not been consumed.
Without an explicit pool, DeviceSessions share the extension's default pool of
64. Per-session concurrency is 1..4, the receipt registry has at most four
unfinished original jobs, and there is no waiter queue. Saturation fails before
source IO with typed `resource_limit` or local `conflict` metadata. Python Future
and observer allocation completes before native source IO is launched; refusal
drops the real unpolled lease and restores admission.

Async observers have at most twice the shared pool limit and an additional
extension-wide hard cap of 128 tasks. This permits a close observer alongside
active query observers. Additional close/receipt observers may fail with typed
`resource_limit`. These are task/lease limits, not CPU/RSS bounds or a limit on
caller-retained receipt objects; explicit pools and separately linked libraries
have their own native admission scopes. `available`, `job_available`, and
`observer_available` show pool availability; observer availability remains
subject to the additional global cap.

Native failures use safe exception text and the existing metadata fields
`ios_error_schema_version`, `ios_error_class`, `ios_error_phase`, and
`ios_error_effects`. Native InvalidInput uses ValueError; other native classes use
RuntimeError. This includes malformed caller-supplied local pairing bytes:
their original plist parsing source is retained as invalid_input before any
device IO. A malformed on-wire reply is protocol and remains RuntimeError.
Python argument type conversions retain ordinary TypeError or
OverflowError. Async cancellation-cleanup timeout uses TimeoutError as above.

[finite_device_query.py](examples/finite_device_query.py) is an executable caller
example; it explicitly loads a bounded file before passing immutable bytes.
With a wheel already installed, run the owned protocol validator from the
repository:

```sh
python -I scripts/python-device-query.py
```

The validator requires the actual installed native extension and the existing
synthetic TLS assets. It exercises real usbmux -> Lockdown -> plaintext/TLS wire,
strict native TCP EOF, and original peer/thread joins before behavioral checks.

## Checked application, AFC and log services

`DeviceSession.service(operation, ...)` and `service_async(operation, ...)`
share the finite-query owner, admission pool, cancellation token, original-task
receipts, and close path. Operations are `apps`, `files`, `read_file`, and `logs`.
The SDK opens fresh owned usbmux/Lockdown/service sockets for each admitted job;
discovery, StartSession, optional TLS, StartService, service Connect, requests,
and replies all use the same native absolute total. No pairing, installation,
file write, tunnel creation, mutation replay, or automatic reconnect occurs.

```python
import ios_rs

async def inspect_services(udid: str, pair_bytes: bytes, endpoint: str):
    async with ios_rs.DeviceSession(udid, pair_bytes, endpoint, concurrency=1) as dev:
        apps = await dev.service_async("apps", timeout=5.0)
        names = await dev.service_async("files", path="/", timeout=5.0)
        content = await dev.service_async(
            "read_file", path="/owned-file", max_bytes=65536, output_bytes=65536
        )
        return apps, names, content
```

Defaults: 15s total, 16 MiB cumulative decrypted service reply bytes, 1 MiB
retained text/data, 10,000 entries, and 1 MiB AFC frame bodies. The finite
control-plane reply budget is separately 16 MiB, with 1 MiB mux/Lockdown frames
and bounded discovery metadata. Native validation caps service bytes at 64 MiB,
retained output at 16 MiB, entries at 100,000, and AFC bodies at 4 MiB. Each AFC
body is admitted before allocation. Browse retains its existing 4 MiB
plist and 1,000-frame hard bounds. Syslog retains its 1 MiB raw-message
ceiling. These are logical byte/task limits, not parser temporary-allocation,
TLS wire, CPU, or RSS bounds.

File reads require complete EOF evidence and never return truncated success.
`max_bytes=0` permits only an empty file. Logs require the requested message
count; EOF, native IO, deadline, cancellation, and output refusal are errors.
The exception's `ios_partial_result` owns already completed/admitted messages,
including when an async Future is cancelled. The primary typed native error
and original source remain in the job receipt. A raw `UnexpectedEof` retains
the `io` category; it is not guessed to be a device disconnect.

`Future.cancel()` cancels the actual Core token and waits for the original
native JoinHandle through the same receipt adapter as queries. Close seals
query/service admission, cancels active jobs, and waits for their original
joins. A cleanup timeout retains the closed owner and outstanding receipts for
a subsequent close. Destroying local sockets does not acknowledge remote
FileClose or Apple teardown.

The convenience `open_device` handle uses the same checked implementation;
apps/files have a 15s total and fixed default limits, file reads use their
caller ceiling, and `read_syslog(timeout_ms=...)` includes connection setup.
Local pairing-file loading precedes the checked operation, is bounded to a
regular file of at most 1 MiB, and cannot physically cancel stalled filesystem
IO. Use explicit `DeviceSession` bytes/endpoints to keep file loading outside
the operation. `Device.aclose()` uses the real close receipt and supports
repeated close. Tunnel forwarding shares one checked service owner;
RSD-only routes are explicitly unsupported by this usbmux profile. The
existing userspace proxy bridge remains available for RSD clients.

[finite_device_services.py](examples/finite_device_services.py) demonstrates
the configured interface. With a newly installed wheel, run:

```sh
python -I scripts/sdk-device-services.py
```

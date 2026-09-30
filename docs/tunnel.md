# CoreDevice tunnel

The tunnel layer establishes a CDTunnel to devices that expose the required CoreDevice services. It is primarily useful for RemoteXPC/RSD workflows on iOS versions that support the CoreDevice tunnel path.

## Modes

- `userspace`: runs packet forwarding in userspace and exposes a local TCP proxy. This is the recommended first mode.
- `kernel`: creates a TUN interface through the host OS. This may require administrator/root privileges.

## CLI

Start a tunnel:

```sh
ios -u <UDID> tunnel start --userspace
```

Run the tunnel manager:

```sh
ios tunnel serve --userspace --host 127.0.0.1 --port 49151
```

The manager exposes health, tunnel, and federation endpoints for local tooling.
Check its CLI options with:

```sh
ios tunnel serve --help
```

## Manager federation and WebSocket bridge

`GET /tunnels` keeps the manager's local tunnel record list. `GET /` returns a
UDID-keyed tunnel listing including registered upstream managers. Register one
with `POST /upstream` and `{"url":"http://host:port/"}`; `GET /upstream` lists
registrations and `DELETE /upstream` with the same JSON body removes one. The
manager fetches upstream listings with a two-second deadline and a 1 MiB body
limit. Listings stop following upstreams after three manager hops, including
cycles. An explicit request beyond that budget receives HTTP 508.

`GET /connect?udid=<UDID>&port=<DEVICE_PORT>` upgrades to a WebSocket and
relays binary frames to that device-side TCP port. Use `interface=<NAME>` to
select one tunnel when a UDID has multiple records, or `upstream=<REGISTERED_URL>`
to select a registered remote manager. The optional `hop` query counts relays;
the manager increments it on each remote bridge and rejects values above three.
Frames are limited to 64 KiB in either direction and each write awaits the
receiver, so a slow endpoint applies backpressure. The existing userspace TCP
proxy remains available.

This `/connect` and hop contract is specific to this project's managers. The
reference tunneld currently implements `/upstream` listing aggregation but has
no matching WebSocket bridge or hop wire protocol. Upstream URLs currently use
`http`/`ws` only. The manager API has no authentication; bind it to a trusted
interface or use a protected host transport such as an SSH forward for remote
federation.

## Connection timeout

TCP dials and the initial tunnel/RSD/XPC setup are bounded by a 15-second
timeout. The same bound applies to TCP-backed remote-pairing and lockdown
setup. This prevents a stale tunnel route from inheriting the host operating
system's long SYN retry window. A timeout is reported as an I/O timeout; later
service requests and long-running streams retain their own service-specific
behavior.

RSD now announces a stable host UUID with the modern active handshake. On
macOS it first queries the local remoted identity (a maximum of ten seconds,
once per process); otherwise it derives UUIDv3 from the OS hostname. Direct
and userspace routes use the same identity. Active, queued, synchronous and
passive bootstrap modes fall back on fresh connections within a shared
30-second discovery budget. A hostname change changes the fallback identity.
Resolve the identity before externally suspending remoted if that workflow
is used; this library does not manage remoted's lifecycle.

Wi-Fi discovery recognizes saved host/device authentication tags, including
private MAC advertisements. See [Wi-Fi discovery](wifi-discovery.md) for
candidate validation, time limits, existing-record compatibility and ambiguity
handling. `ios doctor` provides bounded host prerequisites checks;
`ios --no-json doctor --mdns` also checks for local advertisements.

## Userspace proxy protocol

The local proxy expects each new connection to begin with:

1. 16 bytes: target IPv6 address.
2. 4 bytes: target port as little-endian `u32`.

The Python binding's `asyncio_proxy()` helper installs this preamble automatically for asyncio clients while the context manager is active.

## Python interoperability

The same helper can bridge third-party asyncio RemoteXPC clients. For example,
`crates/ios-py/examples/pymobiledevice3_coredevice_bridge.py` starts an
`ios_rs` userspace tunnel, scopes `tunnel.asyncio_proxy()`, and then runs
pymobiledevice3's `RemoteServiceDiscoveryService` over the patched
`asyncio.open_connection()` transport.

This is intentionally an optional example rather than a hard dependency:
pymobiledevice3 is only needed when running that script.

## Common failure causes

- Device is not trusted or required pairing material is missing.
- Device/iOS version does not expose the expected CoreDevice service.
- Firewall or mDNS settings prevent network discovery.
- Kernel mode is run without sufficient privileges.

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
ios tunnel serve --userspace --host 127.0.0.1 --port 49151 --auth-config PRIVATE_POLICY_FILE
```

The manager requires a private scoped policy, including for health/readiness.
`--local-unauthenticated` is an explicit alternative that trusts all local
processes. Plain HTTP binding/client access is restricted to loopback.
See [management access configuration](management-access.md) for credential
creation, `--token-file` clients, scopes and reload. The manager exposes health,
tunnel, and federation endpoints for local tooling.
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
When a selected listing entry belongs to a second upstream, `/connect` carries
that next registered upstream URL to the intermediate manager. This preserves
the chosen interface and target across federation hops. Closing either side of
a remote WebSocket bridge attempts to forward a close frame to the other side
within one second.

This `/connect` and hop contract is specific to this project's managers. The
reference tunneld currently implements `/upstream` listing aggregation but has
no matching WebSocket bridge or hop wire protocol. Upstream URLs currently use
`http`/`ws` only and have no credential configuration. Incoming manager tokens
are never reused upstream. Policy-mode managers cannot yet federate with each
other; protected transport and per-upstream credentials are S03 work.

Access-control implementation is tracked as S01–S03 in the
[engineering roadmap](engineering-roadmap-2026-09-30.md). The first S01
foundation adds `ios_core::secret_file::read_secret` and `write_secret_new` for
manager/client credentials.
`ios tunnel auth create|check` now provides host-only policy preparation and
precise scope checks; see [management access configuration](management-access.md).
Server/client authentication, device/port scopes, expiry and POST policy reload
are now integrated. Browser admission and legacy GET management methods remain
S02 work. The existing loopback userspace TCP proxy does not authenticate its
IPv6/port preamble; manager credentials do not restrict local processes' direct
proxy access or kernel TUN routes. The [entry audit](management-access.md)
records loopback-only forward/WDA/dproxy/screenshot listeners and the remaining
debug/browser admission work.
The reader bounds its buffer to a caller-selected maximum of 1 MiB and wipes it
on drop. On Unix, it requires the effective user's regular file without
group/other permissions and rejects leaf symlinks/FIFOs. New-secret publication
uses owner-only staging and exclusive hard-link creation, preserves an existing
destination and cleans only its own staging file. It rejects an existing shared
parent on Unix instead of changing that directory's permissions.

These APIs require trusted parent paths; they do not provide an atomic
directory-traversal guarantee or a multi-file credential transaction. An
unsupported hard-link filesystem fails, and a cleanup failure may be reported
after publication. Windows inherits the configured parent ACL and has a leaf
reparse-point rejection path; Windows/macOS execution remains pending. Flag
semantics were checked against [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and [file attributes](https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants),
not treated as platform execution evidence.

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

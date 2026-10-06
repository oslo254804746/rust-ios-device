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

## Manager connection lifecycle

Local `GET /tunnels`, `GET /tunnel/<UDID>` and `DELETE /tunnel/<UDID>` use the
tunnel table without waiting for another device's network connection. Starts
for one device serialize; manual starts and scans share four connection permits.
Manual creation has one15-second total after device coordinator acquisition,
including queue/permit waits, connection, existing-trust discovery/fallback and
publication. Expiry reports a timeout. Existing source routes/options remain.

Stop cancels active and queued starts for that device and rejects their late
publication. An absent record still returns404 even if a pending start was
cancelled; that start receives the existing502 with a cancelled error header.
A later explicit start is allowed. Subsequent scans may recreate a present
device, so stop is not a persistent exclusion policy.

Automatic scans deduplicate USB and existing-trust mobdev2 candidates together
and own up to four connection futures. Healthy B can publish while A stalls;
stopping A leaves B's active generation running. Paired discovery now precedes
connection attempts, so its wait can delay scan startup. Scan-lock queue, USB
snapshot and existing-trust paired discovery now share one15-second deadline.
Stop/clear cancels an old waiting generation; real expiry remains a timeout.
This covers setup, while each candidate retains its own connection budget.
Whole-scan and metadata CPU/handoff budgets remain pending. See
[scan discovery evidence](validation/tunnel-discovery-2026-10-06.md).
Failure backoff stays30/60/120/240/300 seconds for
the same observed native ID and route/paired host. A newly observed ID or known
host/route change can retry immediately; a failure on that identity starts at
count1. Failure records hide retained hosts in Debug output. See
[backoff identity evidence](validation/tunnel-backoff-2026-10-06.md).

Effective scan snapshots remove unauthorized, observed absent or known changed
failure identities. Present expired identities keep their counter. A paired
discovery error preserves unknown mobdev2 rows; successful empty discovery can
release them. This is observed cache reconciliation, not physical removal proof.
See [retention evidence](validation/tunnel-retention-2026-10-06.md).

Failure records and pending reservations share 4096 slots. Existing retries reuse
a slot; new scanned attempts at capacity defer with resource_limit before native
connection work. Success/cancel releases reservations. Clear/stop fences queued
failure recording. The count does not bound bytes, candidate metadata, native
workers, sockets or RSS.

Linux fake-device tests cover local query/stop isolation, duplicate starts,
stop/discovery races, sixteen candidates with four active connections, cross-
transport isolation and owned release. Scans retain observed native attachment
IDs, retire old records after an observed replacement, and reject candidates
returning another ID without connection-failure backoff. Selection keeps the
existing first-USB/last-other preference and retains another attached transport.
New publications survive older snapshots. See
[snapshot identity evidence](validation/tunnel-identity-2026-10-06.md).

These checks use point-in-time snapshots; manual starts carry no expected ID.
Usbmux/direct-USB creation now queries the daemon once more before publication,
checking that the returned(UDID,ID) remains attached. This extra query shares the
existing connection total/cancellation and can increase startup time. An absent
result is released locally; query IO and actual deadline expiry remain typed.
Wi-Fi/mobdev2 skip it. Every route also checks known local liveness before the
query and again after the publication lock. Known inactive results release with
local cancellation. See
[publication query evidence](validation/tunnel-publication-2026-10-06.md).

This checks the query reply's snapshot. No event listener is implemented.
Local liveness uses kernel receiver existence or unfinished userspace tasks,
which does not certify physical connectivity or native task joins. See
[local liveness evidence](validation/tunnel-liveness-2026-10-06.md).
Changes after the last check, reused IDs/daemon restart, receiver false positives/joins,
other metadata/byte bounds and physical/resource/performance evidence remain pending.
See [local operation evidence](validation/tunnel-isolation-2026-10-06.md) and
[parallel scan evidence](validation/tunnel-parallel-2026-10-06.md).

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
no matching WebSocket bridge or hop wire protocol. Upstream HTTPS/WSS verifies
certificates and uses independently configured private credentials through
`--upstream-config`. Direct plaintext upstreams require literal loopback;
remote/DNS HTTP is rejected. Incoming manager tokens are never reused upstream.
See [management access configuration](management-access.md) for the current
transport, credential and deployment boundaries.

Access-control implementation is tracked as S01–S03 in the
[engineering roadmap](engineering-roadmap-2026-09-30.md). The first S01
foundation adds `ios_core::secret_file::read_secret` and `write_secret_new` for
manager/client credentials.
`ios tunnel auth create|check` now provides host-only policy preparation and
precise scope checks; see [management access configuration](management-access.md).
Server/client authentication, device/port scopes, expiry and POST policy reload
are now integrated. Browser admission and POST-only management methods are
integrated for the existing host-tested manager routes; platform/device and
future entry conditions remain in the roadmap. The existing loopback userspace
TCP proxy does not authenticate its
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

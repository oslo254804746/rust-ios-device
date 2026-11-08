# CoreDevice tunnel

The tunnel layer establishes a CDTunnel to devices that expose the required CoreDevice services. It is primarily useful for RemoteXPC/RSD workflows on iOS 17–27.2 devices that support the CoreDevice tunnel path.

## Modes

- `userspace`: runs packet forwarding in userspace via `smoltcp` and exposes a local TCP proxy plus userspace IPv6 UDP datagram forwarding. This is the recommended mode and requires no administrator/root TUN privileges.
- `kernel`: creates a TUN interface through the host OS. This may require administrator/root privileges.

Userspace construction binds its loopback listener and obtains its port before
starting packet workers. A native initialization error releases the input
stream with its original IO source and leaves no newly spawned worker. The
successful owner aborts its task handles on final Drop. Explicit shutdown
also collects those worker joins and the original proxy-client task set.

## Userspace 20ms ICMPv6 Echo Request Nudge & IPv6 UDP Forwarding

On iOS 17–27.2 over USB (`CoreDeviceProxy`), the device's USB NCM / `remoted`
transport can hold small outgoing TCP/HTTP2 or UDP frames in its transmit
coalescing buffer until another inbound IPv6 packet arrives from the host. To
prevent multi-second stalls during RemoteXPC/DTX handshakes, Cryptex DDI
uploads, or rapid connection bursts, the userspace tunnel automatically emits a
periodic **ICMPv6 Echo Request (`type=128, code=0`) nudge every 20ms** from the
host tunnel IPv6 address to the device tunnel IPv6 address. On physical iOS 27.2
(`iPhone16,2`) burst benchmarks (30 consecutive RSD service queries), this
achieves a 100% pass rate with an 18.0ms median latency (28.23ms p95).

In addition to TCP proxying, the userspace tunnel implements **IPv6 UDP
datagram forwarding** between host UDP sockets and the `smoltcp` userspace
stack. Device-initiated RTP/RTCP UDP streams from `com.apple.coredevice.displayservice`
(`ios device-control display video|audio`, `ios hid` Universal HID media
authorization, and `ios viewer`) work directly over `--userspace` tunnels on
iOS 17–27.2 without requiring a kernel TUN interface.

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
Manual creation captures one 15-second absolute deadline before device
coordinator acquisition. Coordinator, device queue, cache/permit waits,
connection, existing-trust discovery/fallback, attachment query and publication
share that deadline. Expiry retains the native timeout source and the existing
HTTP 502/timeout response. Zero budgets never poll source work; an unrepresentable
deadline returns `invalid_input`. A due deadline prevents the next work poll,
including a late ready connection or publication lock. Single-poll CPU work,
startup and whole-scan totals remain separate conditions.

Stop cancels active and queued starts for that device and rejects their late
publication. An absent record still returns 404 even if a pending start was
cancelled; that start receives the existing 502 with a cancelled error header.
Stop persists across scans and replug, including an absent target. A successful
authorized explicit start or reusable global clear can reactivate it; failed,
denied or cancelled starts cannot. This state shares the bounded device
coordinator namespace and survives only within this manager process. Successful
manual starts recover the actual established transport/filter using fresh Source
metadata and their original grant revision/expiry. See
[route recovery and durable stop](manager-route-intent.md).

Automatic scans deduplicate USB and existing-trust mobdev2 candidates together
and own up to four connection futures. Healthy B can publish while A stalls;
stopping A leaves B's active generation running. Paired discovery precedes
connection attempts, so its wait can delay scan startup. Scan-lock queue, USB
snapshot and existing-trust paired discovery share one 15-second deadline.
Stop/clear cancels an old waiting generation; real expiry remains a timeout.
Initial/post-USB tunnel table locks, USB coordinator admission and the
no-authorized-device failure/ready locks share that same clock and cancel
while held. A lock released after expiry cannot advance the scan. An already
returned native USB error survives cancellation; `ListError` cleanup expiry
retains the primary and cleanup errors separately.

Each candidate retains its own connection budget. A subsequent checked
30-second whole-scan clock also covers post-paired handoff, reconciliation,
failure writes and final readiness; synchronous metadata CPU preemption remains
outside that guarantee. Failure backoff stays 30/60/120/240/300 seconds for
the same observed native ID and route/paired host. A newly observed ID or known
host/route change can retry immediately; a failure on that identity starts at
count 1. Failure records hide retained hosts in Debug output.

Effective scan snapshots remove unauthorized, observed absent or known changed
failure identities. Present expired identities keep their counter. A paired
discovery error preserves unknown mobdev2 rows; successful empty discovery can
release them. This is observed cache reconciliation, not physical removal proof.

Failure records and pending reservations share 4096 slots. Existing retries reuse
a slot; new scanned attempts at capacity defer with `resource_limit` before native
connection work. Success/cancel releases reservations. Clear/stop fences queued
failure recording. The count does not bound bytes, candidate metadata, native
workers, sockets or RSS.

Host fake-device tests cover local query/stop isolation, duplicate starts,
stop/discovery races, sixteen candidates with four active connections, cross-
transport isolation and owned release. Scans retain observed native attachment
IDs, retire old records after an observed replacement, and reject candidates
returning another ID without connection-failure backoff. Selection keeps the
existing first-USB/last-other preference and retains another attached transport.
New publications survive older snapshots.

These checks use point-in-time snapshots; manual starts carry no expected ID.
Usbmux/direct-USB creation queries the daemon once more before publication,
checking that the returned `(UDID, ID)` remains attached. This extra query shares
the existing connection total/cancellation and can increase startup time. An absent
result is released locally; query IO and actual deadline expiry remain typed.
Wi-Fi/mobdev2 skip it. Every route also checks known local liveness before the
query and again after the publication lock. Known inactive results release with
local cancellation.

This checks the query reply's snapshot. No event listener is implemented.
Local liveness first rejects a stopped shared runtime, then checks kernel
receiver existence or unfinished userspace tasks.

## Manager federation and WebSocket bridge

The server retains shutdown state for late waiters and owns its scanner future.
After startup, one pinned Ctrl-C future or authenticated POST shutdown stops the
scanner before cleanup. Table clear and graceful HTTP drain share an independent
2-second budget; an expired drain reports a timeout. Native primary and cleanup
errors stay inspectable.

Caller Drop signals shutdown/cancellation and releases unlocked tables only.
It cannot await a locked table or delegated Axum/Core workers. A capped drain
does not forcibly join pending handlers/upgrades.

Shutdown closes connection admission before waiting for table cleanup.
Existing connection tickets are cancelled; fresh manual starts and scans cannot
reopen that state, including after `clear`. A delegated `/start-tunnel` request
whose JSON body finishes after shutdown returns the existing 502 with
`x-ios-error-class: cancelled` without connecting. Ordinary internal `clear`
still allows a fresh generation; restarting a server needs a new manager state.

Publication and failure-row writes retain their ticket epoch guards
through the actual synchronous insertion/update. If close, clear or stop has
already won, no stale write occurs. If insertion wins first, close waits for
that short action and later table cleanup removes the record; the response
may finish afterward. Scan admission also rechecks terminal state after watch
capture, preventing discovery from missing a completed close.
The server's 2-second cleanup clock starts after signalling closure, so it
does not bound a synchronous writer's wait for a guarded action.

The Core packet forwarder also retains its reader in a local task set.
Dropping or aborting the forwarding future requests reader cancellation even
while a TUN write, stream write or flush is pending. This releases the stream
once cancellation is scheduled. Normal return aborts and joins that reader,
returning an unexpected native join failure alongside any forwarding primary.
The library-created kernel runtime additionally owns its parent and persistent
reader set, so explicit shutdown can collect children after parent panic.

The forwarder's watch cancellation also covers selected TUN/stream writes and
flush waits. One receiver is checked before every new iteration, including
consecutive Ready steps. A closed sender or changed event ends a pending step;
an IO error already acquired in that closing poll remains the result. Partial
writes can be abandoned when the stream closes. Watch cancellation does not
acknowledge device teardown. Core owner shutdown provides explicit native joins
under a separate cooperative budget.

`TunnelHandle::request_stop()` synchronously stops the shared runtime without
waiting for completion. With either tunnel backend, `shutdown(budget)` starts
its absolute clock and stop request when called, before its future is polled.
Join-mutex waits share that clock. Zero still requests stop and returns native
expiry; overflow returns `invalid_input` without stopping. Keep the Arc across
timeout/cancellation and retry shutdown with a new budget. Native failures stay
inspectable after recovery; final Drop only requests cancellation.

Core `TunnelManager::stop` removes the row and signals retained clones before
returning. Replacement `register` signals a displaced different Arc; the same Arc
remains live. Both calls remain non-joining. Use `retire` to obtain a stopped Arc
and retain it for shutdown. Legacy `TunnelHandle::new` owns no arbitrary tasks the
caller spawns with its receiver.

Coordinator admissions capture their clear epoch before locking, so an old
waiter cannot acquire the replacement epoch. Coordinator lock wait also
observes clear/shutdown directly and returns cancellation while the lock is
still held. Manual connection-total coverage includes that acquisition.

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

`ios_core::secret_file::read_secret` and `write_secret_new` handle
manager/client credentials. `ios tunnel auth create|check` provides host-only
policy preparation and precise scope checks; see
[management access configuration](management-access.md).
Server/client authentication, device/port scopes, expiry and POST policy reload
are integrated. Browser admission and POST-only management methods are
integrated for the manager routes. The loopback userspace TCP proxy does not
authenticate its IPv6/port preamble; manager credentials do not restrict local
processes' direct proxy access or kernel TUN routes. The
[entry audit](management-access.md) records loopback-only
forward/WDA/dproxy/screenshot listeners and debug/browser admission rules.

The secret reader bounds its buffer to a caller-selected maximum of 1 MiB and
wipes it on drop. On Unix, it requires the effective user's regular file without
group/other permissions and rejects leaf symlinks/FIFOs. New-secret publication
uses owner-only staging and exclusive hard-link creation, preserves an existing
destination and cleans only its own staging file. It rejects an existing shared
parent on Unix instead of changing that directory's permissions.

These APIs require trusted parent paths; they do not provide an atomic
directory-traversal guarantee or a multi-file credential transaction. An
unsupported hard-link filesystem fails, and a cleanup failure may be reported
after publication. Windows inherits the configured parent ACL and has a leaf
reparse-point rejection path. Flag semantics follow
[CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)
and [file attributes](https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants).

## Connection timeout

TCP dials and the initial tunnel/RSD/XPC setup are bounded by a 15-second
timeout. The same bound applies to TCP-backed remote-pairing and lockdown
setup. This prevents a stale tunnel route from inheriting the host operating
system's long SYN retry window. A timeout is reported as an I/O timeout; later
service requests and long-running streams retain their own service-specific
behavior.

RSD announces a stable host UUID with the modern active handshake. On
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
2. 4 bytes: target port as little-endian `u32` (optionally ORed with `USERSPACE_PROXY_CONNECT_ACK` when requesting a 1-byte remote-connect status acknowledgement).

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
- Device/iOS version does not expose the expected CoreDevice service (verify Developer Mode and `ios -u <UDID> ddi status`).
- Firewall or mDNS settings prevent network discovery.
- Kernel mode is run without sufficient privileges (prefer `--userspace`).

## CLI native runtime retirement and deadline guards

The manager retains original native runtime Arcs before RSD probing and
through cancelled publication, removal and shutdown. All owner phases share
4096 slots; a full registry rejects another Source before contact. Stop/clear,
replacement and discarded records request stop while the registry retains native
join progress. Native drain runs outside tables and uses a fresh cleanup clock
on retry. Fully joined faults stay available even after the slot is released.

Returned server shutdown gives native joins and metadata/HTTP drain the same
independent 2s total, including a held-table case. Standalone start also uses a
2s native drain after returned operation/output/signal results. Caller Drop
requests stop but cannot await.

The manual 15s total fences Source/native handoff, metadata/query
and publication even when synchronous work crosses the clock within one poll.
A late real insertion is removed before another table reader can observe it;
a displaced original row/runtime stays live. Each admitted automatic scan
candidate shares one checked absolute 15s clock across
serial/cache/backoff/permit/connection and the same native-handoff,
metadata/query and synchronous publication fences as manual starts.

Each manager scan shares one 30-second total from its scan-lock wait through
readiness. Clear/shutdown cancels the whole scan; stopping one device after
discovery leaves other candidates running. Whole expiry creates no new
connection backoff or readiness. An acquired paired or USB-list error survives
scanner cancellation and caller Drop; the manager retains up to eight abandoned
errors across clear, and server shutdown reports them alongside independent
native cleanup.

Successful candidates clear an observed failure row with a non-queuing
`try_write` when available. Contested cleanup releases the connection slot and
joins the bounded bookkeeping batch after the connection window.

Manual manager starts apply a 4096-byte UTF-8 limit to UDID and optional
route host before retaining a lifecycle key or contacting a device.
`tunnel serve` supports `--retry-initial-delay-ms`, `--retry-max-delay-ms`,
`--retry-max-attempts` and `--retry-total-ms`. The default retains unlimited
30/60/120/240/300-second automatic retries. See the
[capability inventory](capabilities.md) for host and physical-device validation
records.

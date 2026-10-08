# CoreDevice tunnel

The tunnel layer establishes a CDTunnel to devices that expose the required CoreDevice services. It is primarily useful for RemoteXPC/RSD workflows on iOS versions that support the CoreDevice tunnel path.

## Modes

- `userspace`: runs packet forwarding in userspace and exposes a local TCP proxy. This is the recommended first mode.
- `kernel`: creates a TUN interface through the host OS. This may require administrator/root privileges.

Userspace construction binds its loopback listener and obtains its port before
starting packet workers. A native initialization error releases the input
stream with its original IO source and leaves no newly spawned worker. The
successful owner aborts its four task handles on final Drop. Explicit shutdown
also collects those worker joins and the original proxy-client task set. See
[construction evidence](validation/userspace-construction-2026-10-06.md) and
[native owner evidence](validation/native-tunnel-owner-2026-10-07.md).

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
deadline returns invalid_input. A due deadline prevents the next work poll,
including a late ready connection or publication lock. Single-poll CPU work,
startup and whole-scan totals remain separate conditions. See
[manual connection deadline evidence](validation/tunnel-manual-deadline-2026-10-06.md).

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
Initial/post-USB tunnel table locks, USB coordinator admission and the
no-authorized-device failure/ready locks now share that same clock and cancel
while held. A lock released after expiry cannot advance the scan. An already
returned native USB error survives cancellation; ListError cleanup expiry
retains the primary and cleanup errors separately. See
[scan setup evidence](validation/tunnel-scan-setup-2026-10-06.md).
The setup clock ends at the paired reply; each candidate retains its own
connection budget. Post-paired handoff, reconciliation/final ready, whole-scan
and metadata CPU budgets remain pending. See
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
Local liveness first rejects a stopped shared runtime, then checks kernel
receiver existence or unfinished userspace tasks. It does not certify physical
connectivity or native task joins. See
[local liveness evidence](validation/tunnel-liveness-2026-10-06.md).
Changes after the last check, reused IDs/daemon restart, receiver false positives/joins,
other metadata/byte bounds and physical/resource/performance evidence remain pending.
See [local operation evidence](validation/tunnel-isolation-2026-10-06.md) and
[parallel scan evidence](validation/tunnel-parallel-2026-10-06.md).

## Manager federation and WebSocket bridge

The server retains shutdown state for late waiters and owns its scanner future.
After startup, one pinned Ctrl-C future or authenticated POST shutdown stops the
scanner before cleanup. Table clear and graceful HTTP drain share an independent
2-second budget; an expired drain reports a timeout. Native primary and cleanup
errors stay inspectable. See [server evidence](validation/tunnel-server-2026-10-06.md).

Caller Drop signals shutdown/cancellation and releases unlocked tables only.
It cannot await a locked table or delegated Axum/Core workers. A capped drain
does not forcibly join pending handlers/upgrades; startup totals, later admission
races and those delegated resources remain separate work.

Shutdown now closes connection admission before waiting for table cleanup.
Existing connection tickets are cancelled; fresh manual starts and scans cannot
reopen that state, including after `clear`. A delegated `/start-tunnel` request
whose JSON body finishes after shutdown returns the existing502 with
`x-ios-error-class: cancelled` without connecting. Ordinary internal `clear`
still allows a fresh generation; restarting a server needs a new manager state.

Publication and failure-row writes now retain their ticket epoch guards
through the actual synchronous insertion/update. If close, clear or stop has
already won, no stale write occurs. If insertion wins first, close waits for
that short action and later table cleanup removes the record; the response
may finish afterward. Scan admission also rechecks terminal state after watch
capture, preventing discovery from missing a completed close. See
[shutdown state evidence](validation/tunnel-shutdown-fences-2026-10-07.md).
Policy/liveness snapshots and other scan mutations remain separate checks.
The server's 2-second cleanup clock starts after signalling closure, so it
does not bound a synchronous writer's wait for a guarded action.

The Core packet forwarder also retains its reader in a local task set.
Dropping or aborting the forwarding future requests reader cancellation even
while a TUN write, stream write or flush is pending. This releases the stream
once cancellation is scheduled. Normal return now aborts and joins that reader,
returning an unexpected native join failure alongside any forwarding primary.
The library-created kernel runtime additionally owns its parent and persistent
reader set, so explicit shutdown can collect children after parent panic. See
[reader ownership evidence](validation/forward-reader-owner-2026-10-07.md) and
[native owner evidence](validation/native-tunnel-owner-2026-10-07.md).

The forwarder's watch cancellation also covers selected TUN/stream writes and
flush waits. One receiver is checked before every new iteration, including
consecutive Ready steps. A closed sender or changed event ends a pending step;
an IO error already acquired in that closing poll remains the result. Partial
writes can be abandoned when the stream closes. See
[forwarding cancellation evidence](validation/forward-watch-shutdown-2026-10-07.md).
Watch cancellation does not acknowledge device teardown. Core owner shutdown
provides explicit native joins under a separate cooperative budget.

`TunnelHandle::request_stop()` synchronously stops the shared runtime without
waiting for completion. With either tunnel backend, `shutdown(budget)` starts
its absolute clock and stop request when called, before its future is polled.
Join-mutex waits share that clock. Zero still requests stop and returns native
expiry; overflow returns invalid_input without stopping. Keep the Arc across
timeout/cancellation and retry shutdown with a new budget. Native failures stay
inspectable after recovery; final Drop only requests cancellation.

Core `TunnelManager::stop` removes the row and signals retained clones before
returning.
Replacement `register` signals a displaced different Arc; the same Arc remains
live. Both calls remain non-joining. Use `retire` to obtain a stopped Arc and
retain it for shutdown. Legacy `TunnelHandle::new` owns no arbitrary tasks the
caller spawns with its receiver. CLI eviction/server cleanup and the activation
RSD wait before Source returns still need independent retirement integration.
These shared-state/timing/error changes require pre-1.0 compatibility review.

Coordinator admissions capture their clear epoch before locking, so an old
waiter cannot acquire the replacement epoch. Coordinator lock wait now also
observes clear/shutdown directly and returns cancellation while the lock is
still held. Manual connection-total coverage now includes that acquisition;
checks racing with shutdown, existing forwarding and delegated worker joins
remain separate work. See
[shutdown admission evidence](validation/tunnel-shutdown-admission-2026-10-06.md).
The later [coordinator cancellation evidence](validation/tunnel-coordinator-cancel-2026-10-06.md)
records the cancellation improvement; the later
[manual connection deadline profile](validation/tunnel-manual-deadline-2026-10-06.md)
adds acquisition to the total.

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

## CLI native runtime retirement

The manager now retains original native runtime Arcs before RSD probing and
through cancelled publication, removal and shutdown. All owner phases share
4096 slots; a full registry rejects another Source before contact. Stop/clear,
replacement and discarded records request stop while the registry retains native
join progress. Native drain runs outside tables and uses a fresh cleanup clock
on retry. Fully joined faults stay available even after the slot is released.

Returned server shutdown gives native joins and metadata/HTTP drain the same
independent2s total, including a held-table case. Standalone start also uses a
2s native drain after returned operation/output/signal results. Caller Drop
requests stop but cannot await; startup-signal and generic HTTP/delegated-task
conditions remain. Existing options/stdout/HTTP payloads remain; return timing
and cleanup errors need pre-1.0 review. See
[native CLI evidence](validation/cli-tunnel-native-retirement-2026-10-07.md).

## Manual connection crossing a deadline during one poll

The existing manual15s total now fences Source/native handoff, metadata/query
and publication even when synchronous work crosses the clock within one poll.
A late real insertion is removed before another table reader can observe it;
a displaced original row/runtime stays live. The new exact native Arc retires
after table/epoch guards release and retains its joins. Actual Tokio Elapsed
and acquired original IO remain; default fallback stops before another contact
after expiry. Synchronous work remains cooperative. Automatic candidate and
whole-scan policy still need their own acceptance. See
[same-poll deadline evidence](validation/tunnel-manual-poll-crossing-2026-10-07.md).

## Automatic candidate deadline boundaries

Each admitted scan candidate now shares one checked absolute15s clock across
serial/cache/backoff/permit/connection and the same native-handoff,metadata/query
and synchronous publication fences as manual starts. Late insertion rolls back
the unobserved row; the original new native runtime remains retained for joins.
Zero/overflow internal budgets reject Source. Actual scanned Elapsed context and
acquired original IO remain. The four-candidate window/admission policy stays.
Failure bookkeeping can still wait outside this clock; whole-scan handoff,
reconciliation and readiness totals remain. See
[automatic deadline evidence](validation/tunnel-automatic-deadline-2026-10-07.md).

## Paired discovery error during clear or close

The scanner still discards successful paired snapshots from an old epoch.
An original error already returned in the same Source poll as clear/close now
survives instead of becoming success. Candidate handoff/readiness remain fenced;
existing discovery/candidate clocks and stop-A/active-B isolation stay. See
[native paired error evidence](validation/tunnel-paired-error-retention-2026-10-07.md).

This evidence covers clear/close completed before the Source reply; an epoch
change at the later paired check still needs a separate error-retention fix.

## Paired discovery final handoff check

Implementation `aa3c3a46c19dd19175c879bcb44f5856d24259ec`; [final paired fence evidence](validation/tunnel-paired-final-fence-2026-10-07.md).
Both stale paired checks retain already acquired errors. A clear-success fixture supplies a unique nonempty paired target,which never connects or marks ready. Close-success here is an empty snapshot;coordinator zero-mutation is not claimed.
Whole-scan total/cancellation/paired primary across caller drop,out-of-clock failure bookkeeping,metadata admission and all other original L01-L04/classes5-9 requirements remain.

## Discovery snapshot limits

Implementation `327b0ef38e43576f2313c51c597b5c16cb7abc01`; [snapshot admission evidence](validation/tunnel-snapshot-admission-2026-10-08.md).
All four manager Source consumers validate complete USB/paired snapshots,including tails after an early matching row/target. Combined planning completes before paired clones/tickets;permission reload can only remove planned rows. See the profile for exact private ceilings and byte accounting;device authorization does not exclude raw USB rows from input limits.
Source allocation/RSS,global gate/failure/cache/native/list/federation bytes,whole-scan total/cancel/paired primary across caller drop/out-of-clock failure writes,L03-L04/classes5-9 and all other original acceptance remain.

## Failure record writes

Implementation `4415be9a65921a346208f912dbe59e399e471591`; [failure bookkeeping evidence](validation/tunnel-bookkeeping-2026-10-08.md).
Failed connection results no longer occupy a connection slot while waiting to write backoff. The scanner collects at most4096 intents,then shares one private1s bookkeeping total. Timeout refuses new readiness and leaves unwritten outcomes without new backoff. See the profile for same-poll rollback,original paired primary/secondary and exact scope.
Whole-scan total/global cancellation/durable paired or List primary across callerDrop,reconciliation/readiness/global metadata,successful-candidate writer fairness,L03-L04/classes5-9 and all other original acceptance remain.

## Whole-scan budget and interrupted discovery

Each manager scan shares one 30-second total from its scan-lock wait through readiness. The existing discovery and per-device 15-second budgets remain. Clear/shutdown cancels the whole scan; stopping one device after discovery leaves other candidates running. Whole expiry creates no new connection backoff or readiness.

An acquired paired or USB-list error survives scanner cancellation and caller Drop. The manager retains up to eight abandoned errors across clear, and server shutdown reports them alongside independent native cleanup. Full history rejects another scan before discovery. These are cooperative host-side limits; they do not establish physical teardown timing or global memory bounds. See [scan control evidence](validation/tunnel-scan-control-2026-10-08.md).

## Successful scan cleanup

Successful candidates clear an observed failure row with a non-queuing
`try_write` when available. Contested cleanup releases the connection slot and
joins the bounded bookkeeping batch after the connection window. A cleanup
timeout cannot record an already published connection as a new failure. Private
row versions and lifecycle tickets protect later failures from stale success
cleanup; deadline crossings restore an unobserved removed row. See
[successful bookkeeping evidence](validation/tunnel-success-bookkeeping-2026-10-08.md).

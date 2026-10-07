# CLI native tunnel retirement and ownership before RSD

Implementation `6debaf9b6d78c10b4be1cfd55e2fbcbb69da8756`, parent `3bf026b29af8798e7600fc51aee5e9f290b7ffe9`.
Linux x86_64, 2026-10-07, Rust1.97.1/MSRV1.80.1, project Python3.12.13.
All30 original main tasks and all children excluding Q04 remain required:
9complete/21unfinished (9notstarted/12inprogress-or-conditions). This profile
advances the native ownership portion of L01/L02; it completes no main task.

## Connection ownership and compatibility

Four additive owner-aware Core connect functions preserve the original functions,
signatures, fields and error enums: connect_with_owner,
connect_direct_usb_tunnel_with_owner, connect_remote_pairing_tunnel_with_owner
and connect_tcp_lockdown_tunnel_with_owner. Public TunnelOwner::reserve returns
a borrowed TunnelReservation. Each strategy target reserves before its transport
work. Earlier discovery and credential loading can precede this callback; the
CLI separately reserves initial capacity before polling Source at all.

After constructing a runtime, Core attaches its exact original handle Arc before
the initial RSD probe can suspend. Reservation Drop releases empty capacity;
after attachment the caller must retain and stop that runtime for explicit
shutdown. complete transfers that responsibility as ConnectedDevice returns,
without certifying publication or native joins. Attachment rejection stops the
runtime and returns the original error. The optional RSD probe retains its
existing nonfatal outcome policy; a completed failed probe can still return a
device. Ordinary owner-less Core connection functions keep their prior behavior.

RemotePairing's existing per-target timeout can abandon an activated runtime and
try another target in the same Source call. Each candidate therefore has its own
guard and reservation. The successful return selects the exact attached Arc;
failed or abandoned earlier candidates remain separately retired. No synthetic
ConnectedDevice constructor or protocol/pairing-policy change was introduced.

With either backend, shutdown_status observes immutable acquired outcomes and
join accounting without stopping or polling workers or starting a new attempt.
TunnelShutdownFailure::has_retained_outcomes distinguishes acquired work/join
faults from pure current expiry or incomplete accounting. Original native causes
survive cancellation of a shutdown future. Status itself never releases a CLI
owner, and an incomplete status error is not necessarily a worker fault.
The root UserspaceTunDevice export requires tunnel-userspace; owner traits and
connection entry points keep the ordinary device API feature surface.

## Bounded CLI retention and cleanup

One fixed4096-slot registry covers reserved, inflight, unpublished, active and
retired owners together. No phase can evict or grow past that capacity. Slots
have opaque incarnations and lifecycle identities; a stale guard cannot retire
a later occupant. Each slot keeps the original Arc and its Core native joins.
Admission denial occurs before another Source is polled. Empty retired inflight
slots wait for the owning guard's release or late attachment; Notify is armed
before checking either condition. Late attachment remains retained and stopped.

Source's successful runtime handoff precedes metadata, attachment-query and table
awaits. A ManagedTunnel lease retires on every removal or destruction path:
stop/clear, dead get/list, replacement, policy/snapshot selection and errors.
Replacement, explicit stop and clear release the table before dropping removed
records or signaling selected epochs. Other retain paths can retire under the
table but never await native shutdown there. Actual worker joins run outside
the table, lifecycle watch borrows and registry's synchronous mutex.

Explicit drain selects a bounded set of retired incarnations, synchronously
requests stop and uses one checked absolute cooperative clock for drain-mutex
and native waits. Timeout or caller-future Drop leaves original joins available
for a new attempt. Release requires a directly returned native shutdown Ok or
the first outer TunnelShutdownFailure's joins_complete. A nested foreign report,
local liveness, pending count alone or finished abort handle cannot certify it.
joins_complete requires valid accounting and says nothing about an error-free
attempt, its deadline, delegated HTTP tasks or a device acknowledgement.

Two fixed historical causes retain the first pure Core expiry and first original
non-expiry native report. A later native fault takes priority over an earlier
expiry, including a retry of the same runtime. A saturating count records other
distinct failed owners. Per-attempt evidence and the current native Elapsed are
separate from history. Before an outer clock cancels inner shutdown, readonly
Core status captures acquired IO so the newer outer timer cannot bury it.

Server shutdown closes admission and signals every registered native owner before
dropping its owned scanner, independently of table-lock availability. Native
drain and metadata/HTTP drain run concurrently under one independent2s absolute
cleanup clock captured before shutdown side effects. Phase failures are stored
before Ready, preserving them if the clock wins. Success requires both phases,
no current/historical faults and a sealed empty registry. A held metadata table
can cause expiry after all native joins succeeded; that result retains both facts.
Metadata-only timeout preserves its previous direct Elapsed contract. Original
signal/server primary and cleanup errors remain separately inspectable.

Standalone tunnel start wires the same owner-aware Core call and an independent
2s native terminal drain after returned output/signal/operation results. Final
caller Drop requests stop and retains joins while the registry remains held;
it cannot await or certify cleanup. This is source wiring plus shared owner
fixtures, not a physical standalone command or startup-signal certification.
No detached cleanup task is added. CLI options, stdout and HTTP payloads remain;
native teardown can now delay return or produce an error after joins finish.
These timing/error changes and additive APIs require pre-1.0 compatibility review.

## Host fixtures and deliberate wrong controls

24 source files3671+/77-;39 additive tests:14 sender-only registry identity/capacity/fencing,
5 actual userspace native results,13 CLI ownership paths with real native workers,
3 actual Core activation/CDTunnel peer,2 status/public consumers and2 raw-forward
work/reader-panic cases. The14 sender-only cases own no native task and do not
prove cleanup. CLI discovery/device metadata use a private test seam; their
runtime, original Arcs, parents/proxy children, publication and server owners are
real. Core activation uses a local CDTunnel peer without Apple/device services.
The capacity case holds one real runtime plus4095 empty reservations.

Native fixtures cover held other Arcs, actual duplex EOF, caller drop while
query/table waits, locked-table server drop with a retained state, independent
native joins despite metadata expiry, stop/clear/dead rows/replacement/list IO/
detach/fencing, abandoned candidates and fully joined failed-owner history.
Condvar-gated destructors control shutdown stalls/panic on two worker threads;
supplied BrokenPipe is a controlled error, not an OS permission/network failure.
TCP proxy closure may be EOF or reset; an actual child is observed before stop.
Raw forwarding exposes original work payload identity and native reader panic
flags after real stream/task recovery. Gates/locks are released and recovery
is driven before regression assertions. Original inline manager/userspace/
forward tests, reader_owner_tests.rs and Cargo.lock remain byte identical;
original CLI assertions remain. The private skip-tunnel call gains only None.

| Wrong control | Terminal result | Regression |
| --- | --- | --- |
|85843: omit NativeLease retirement on Drop|101;0PASS/1FAIL|dropped publication query left original owner live/unpublished|
|12785: omit attachment before RSD|101;0PASS/1FAIL|cancelled activation exposed no retained original Arc for retry|
|5419: omit acquired cause capture on outer expiry|101;0PASS/1FAIL|actual outer timer replaced the retained native IO primary|

All control original/mutated files, logs and hashes remain. The missing-attachment
control deliberately has no original retained owner: only final-Drop stream/task
recovery is claimed, not an original native join. After restoration all450
product hashes matched; no control change remained in the final freeze.

## Validation and recorded failures

Development70385 exited101 on three new Debug closure type errors, before tests.
97136 had187PASS/1FAIL/1existing ignored: the new TCP proxy fixture expected EOF
only despite valid reset. It now verifies an actual child and closure by EOF or
reset. Development-v4 core75313 exited101 before tests on the new raw fixture's
missing borrow lifetime and nonexistent Internal category; the existing Unknown
join-panic classification is retained. Clippy62685 exited101 on io_other_error
and collapsible_if; equivalent Error::other/collapsed-if fixes preserve behavior.
Exact failed source/log snapshots were archived before fixes after terminal exit.

Final frozen73569 exited0; all16 gates ran serially, with450 product hashes checked
before and after each. Root alone applied source and ran Rust; no source edits
occurred while validation lived. Existing subagents exhausted quota after their
v1 proposals/review; no later delivery was assumed. CARGO_INCREMENTAL=0 throughout;
no cache deletion or cargo clean in this slice. Earlier bare-backend strict Clippy
baseline findings remain documented in the previous native owner profile;
minimum behavior/checks stay, strict backend checks use integration feature sets.

| Check | Result |
| --- | --- |
|CLI tunnel/Core native-filter regression|194PASS/1existing ignored;227PASS|
|Native userspace/kernel minimum|83/83PASS|
|Bare forward/Core tunnel regression|26/95PASS|
|Core bare/tunnel and CLI minimum all-targets;original stack|PASS;5PASS|
|Strict backend integrations/workspace Clippy;fmt|PASS|
|Complete section11 validation|2718Rust/13Python;0FAIL/2existing ignored|
|Project .venv tooling|73PASS|

CLI1057/Core1496/public55 PASS; workspace checks/MSRV1.80.1,66features/fourgroups,
7+7doctests/three snippets,C ABI,Python3.12 rebuilt wheel/sdist/install/import,
tools73/help118/raw38/host5/CLI95/backup3 PASS.22device NOT_RUN;
Chromium/installedPython3.9/3.13 NOT_RERUN. No capability maturity upgrade.

Patch SHA256 `11582477b507c70a75dced8f9ccfbdb81dbbbc800f0d55b1099403e377b287a0`.
Actual debug ios SHA256 `376add85e6cfec4f2b1b11d213021c03ce3bee6d1a454c87a66682fde51d4193`.
Committed source matches the frozen24 files;450 product hashes and read-only
reference HEAD/status verified. Evidence, failed runs, controls and source audit:
`target/validation/cli-native-retirement-2026-10-07/`.

## Remaining conditions

Native ownership evidence does not complete generic HTTP/upgrades/delegated
or blocking workers, startup signal/total, whole-scan or other metadata budgets,
physical attachment freshness/ID reuse/daemon restart/manual epochs, other L01
protocols, L02-L04 or any original fifth-to-ninth category requirement. Standalone
setup retains its original timeout string and signal timing; other SDK/frontends
must opt into or implement ownership. No OS TUN, physical-device, other-platform,
Apple pairing/ACK, rollback, safe retry, RSS/peak allocation, throughput or long-run
certification. All old history/subtasks/118capability bodies and maturities remain.
Q04 and every child are excluded; UID risk is untouched; other acceptance is
unchanged. Goal stays active with9complete/21unfinished. No push/publication/
history rewrite, credentials, device or elevated-TUN operation was performed.

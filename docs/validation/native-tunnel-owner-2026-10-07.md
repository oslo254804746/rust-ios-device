# Native tunnel owners and retryable shutdown

Implementation `f00ff25395a8432087daf578c9dd23975b09be5a`, parent `a90d70f9436e39f7b4be2f350610315b68376ddd`.
Linux x86_64, 2026-10-07, Rust1.97.1/MSRV1.80.1, project Python3.12.13.
Active scope: all30 main tasks and subtasks excluding Q04,9complete/21unfinished
(9notstarted/12inprogress-or-conditions). This host profile advances Core L01/L02.

## Persistent ownership and native outcomes

Previously the kernel activation path spawned a detached forwarding parent,
userspace Drop aborted its four workers without collecting their outcomes,
and normal packet-forwarder return aborted its reader without awaiting it.
The library-created kernel runtime now retains its original typed parent
JoinHandle and a shared original reader JoinSet. Userspace retains four typed
parents plus its existing proxy-client JoinSet. Listener binding and local-port
lookup still precede worker construction. No global manager/table lock crosses
a native join.

An OwnedTask borrows the original handle inside a dedicated asynchronous mutex
until Ready. It stores the native result before clearing that handle or marking
the join complete. Timeout and caller-future Drop release the borrow but leave
that exact handle available while the runtime owner remains held. Completed
handles are not repolled. Parent panic releases its child-set guard and requests
abort while the runtime retains the original set for later explicit collection.
Final runtime Drop requests cancellation; it cannot await completion.

Each runtime has ten fixed outcome slots: four original work causes, four
parent JoinErrors, one forwarding-reader JoinError and one first proxy-client
JoinError. Additional omitted observations increment a saturating counter;
this is not a distinct-fault count. Checked pending-join accounting prevents
overflow/underflow from silently certifying success. Immutable snapshots retain
original Arc causes and bounded cleanup entries, including an attempt's native
Tokio Elapsed. The first work cause drives classification and Error::source;
otherwise the first acquired join cause or current expiry does. Debug and
Display render fixed context/categories, with native objects still inspectable.

Expected locally requested cancellation stays quiet but retains its native
JoinError. Abort requests made after a task already finished cannot relabel an
unexpected cancellation; child abort first reaps already-completed results.
Tokio cannot distinguish competing abort requests racing on an unfinished task.
The flag records this owner's request while unfinished, not exclusive provenance.

An owned forwarding parent records its acquired original work cause before
child cleanup. Its direct work report carries a private fresh Arc identity.
The parent join recognizes only that exact ledger's directly wrapped work
report, avoiding duplicate recording or promotion of a child cause to work.
Foreign, full-shutdown, additionally wrapped reports and another ordinary native
fault remain work errors. The identity token has no strong Ledger cycle.

## Public contract and compatibility

`TunnelHandle::request_stop` synchronously marks the shared runtime terminal
and requests cancellation. `is_alive` then returns false even if other Arcs
remain; this local admission observation does not certify native completion.
With either backend enabled, `shutdown(budget)` captures one checked absolute
cooperative clock and requests stop at the method call, before polling its
future. All parent/child joins and join-mutex waits share that clock. Zero still
requests stop and yields a native Elapsed; unrepresentable durations return
InvalidInput without requesting stop. Pre-poll and post-Ready clock checks
prevent a late Ready result from becoming a successful timed attempt. CPU or
blocking destruction inside one poll is not preempted.

Retain the owner across timeout/cancellation, then call shutdown with a new
budget. Acquired failures remain errors after resources recover. Opaque
`TunnelShutdownFailure` exposes primary(), cleanup(), pending_joins(),
joins_complete() and additional_unretained_outcomes(). joins_complete requires
zero pending slots and valid accounting. It certifies collection of this
runtime's owned native joins, not an error-free attempt, its deadline, peer ACK,
external caller tasks or CLI-wide cleanup. An older snapshot is unchanged by retry.

`TunnelManager::stop` removes its row and signals the shared owner before
returning; its bool reports presence only. `retire` transfers a stopped Arc for explicit
shutdown/retry. Replacement register signals a displaced different Arc, while
registering the same Arc keeps it live. The manager has no retirement queue and
does not join removed or displaced owners. Legacy `TunnelHandle::new` keeps its
constructor/receiver contract; arbitrary caller-spawned receiver tasks remain
the caller's join responsibility.

The raw packet forwarder now aborts and explicitly joins its local reader on
normal return. Unexpected native JoinError is returned; simultaneous work and
cleanup failure expose both originals through opaque `ForwardReaderCleanup`
and Error::source follows the work primary. Its raw cleanup has no independent
deadline. Caller-future Drop remains cancellation fallback. The owned kernel
path supplies a persistent runtime and explicit shutdown budget.

Public fields, original signatures/error enums, Cargo.lock and production
dependencies remain. Additive root exports are gated: ForwardReaderCleanup by
tunnel; TunnelShutdownFailure/shutdown by either backend. Only a test Tokio
rt-multi-thread feature was added for controlled blocking teardown. Stop and
replacement now affect retained clones; raw normal forwarding can wait longer
and return a previously discarded native join failure. These timing and error
changes require pre-1.0 compatibility review. CLI/FFI/Python close APIs did not change.

## Host fixtures and negative controls

Fourteen source files3261+/62-,35 new tests:21 owner/accounting/provenance,
six userspace runtime, six kernel runtime and two external public consumers.
All prior assertions remain, including byte-identical inline manager/userspace/
forward test bodies, the prior reader ownership file and existing public tests.
Actual Tokio parents/children, capacity-limited DuplexStream IO and loopback
proxy clients are exercised. Kernel fixtures use a private host TunDevice with
the actual owned factory/forwarder, without an OS TUN. One two-thread fixture
controls a blocking destructor using a Condvar gate; another private prefix-EOF
fixture holds reader cleanup after an actual framing failure. A supplied raw
errno13 tests native retention, not a real host permission failure.

Fixtures cover held shared Arcs, four workers, stalled preamble clients, pending
packet writes, manager replacement/retire, partial-header EOF, actual reader and
parent panic, child persistence after parent panic, mutex/native wait cancellation,
zero/overflow, late Ready/no work poll, Ready crossing the clock, fixed-slot
retention, invalid count and same/foreign/full/wrapped report identity.

| Deliberately wrong control | Terminal result | Observed regression |
| --- | --- | --- |
|38711: mark an already-finished foreign abort as locally requested|101;0PASS/1FAIL|unexpected native cancellation became quiet|
|21708: take the parent handle before its await|101;0PASS/1FAIL|cancelled join future lost the original retained handle ID|
|36220: omit runtime reader drain after parent join|101;0PASS/1FAIL|parent panic returned before the blocked original reader joined|
|3228: omit child prereap before local abort marking|101;0PASS/1FAIL|already-completed foreign child cancellation became quiet|
|53494: re-record this ledger's direct work report|101;0PASS/1FAIL|reader panic's original native primary became a nested work report|

All five control logs/patches/exit codes are retained. Gates are released and
stream/task recovery occurs before regression assertions. The take-handle
control deliberately destroys the original join handle, so its recovered task
is not claimed to have an original native join. The omitted-reader control
deliberately omits that join. First four controls preceded the identity and
later fixture additions; their target logic/assertions remained unchanged.
The fifth preceded two equivalent test-lint fixes; production logic/assertions
remained. Controls are not claimed to be runs against every byte of the final freeze.

## Serial validation and recorded failures

Initial kernel fixture compilation exposed the missing test-only multi-thread
runtime feature. An initial public test compile found Result equality against
watch RecvError; it was corrected to matches without weakening assertions.
That attempt also filled the host filesystem, causing linker/LLVM failures.
After terminal exit, only166 rebuildable target/debug/incremental/ios_core-*
directories were removed with exact path checks. Sources, reference checkouts,
validation artifacts, binaries and other caches were retained. No cargo clean
was run. Later gates used CARGO_INCREMENTAL=0, recorded in evidence.

Frozen78491 exited101: optional strict bare userspace Clippy exposed existing
unused code without mdns. Exact frozen sources/logs remain in failed-run-78491.
A detached clean baseline audit1446 also exited101 with identical40 library
and19 test diagnostic messages. Required minimum feature behavior/checks stayed;
extra strict backend checks use real integration combinations coredevice-base
and tunnel-kernel,mdns, retaining -D warnings. Workspace strict all-features
Clippy remained required. Frozen45196 then exited101 on two new test-only lints;
exact source/logs remain failed-run-45196. After terminal, equivalent if-let and
std::io::Error::other rewrites fixed them. No production or assertion change.

Final frozen76373 exited0, all14 gates rerun serially. No source edits occurred
during any validation process. Root alone applied product changes and ran Rust;
subagents delivered ignored frozen proposals or read-only reviews.

| Check | Result |
| --- | --- |
|Full/minimum userspace/minimum kernel native-filter regressions|220/77/80 PASS, including35/24/29 new tests|
|Bare tunnel forward regression/Core all-features tunnel regression|24/92 PASS|
|Minimum Core tunnel/CLI all-targets, original CLI stack|PASS/PASS;5 PASS|
|Strict userspace integration/kernel integration/Core all-features Clippy;fmt|all PASS|
|Section11 complete validation|2679 Rust/13 Python;0FAIL/2existing ignored|
|Project .venv tooling|73 PASS|

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

CLI1025/Core1490/public54 PASS; workspace fmt/check/strict Clippy/MSRV1.80.1,
66features/fourgroups/7+7doctests/three snippets/C ABI/Python3.12wheel-sdist
rebuild-install-import/tools73/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installedPython3.9/3.13 NOT_RERUN;no maturity upgrade.

Patch SHA256 `031ba4f40065514b6579beded599824344ec43b54a15ef2d15d589d3c726cd26`.
Actual debug ios SHA256 `8d17a4f2d664af6bfba7db61d180eb2f2fceed4faa2e2db043a37a3729ddcaa3`.
Committed patch matches frozen source;442 product hashes and reference HEAD/
status checked. Ignored controls/source/gate logs/hashes/reviews are under
`target/validation/native-owner-design-2026-10-07/`.

## Remaining conditions

Core activation now attaches a typed kernel parent before its RSD probe await,
but cancellation before Source returns still drops the complete runtime owner;
it has abort fallback without a retained CLI retry owner. CLI creation,
table eviction/replacement/clear/policy/snapshot/error paths and server Drop
with held tables or retained state clones still require independent bounded
retirement ownership and explicit native drain. Standalone tunnel start,
generic HTTP/upgrades/blocking workers, startup/whole-scan/metadata/physical
freshness/reuse/restart/manual epochs and other L01-L04/classes5-9 continue.
Partial-write cancellation, unconsumed worker errors and single-poll CPU/Drop
remain limited as documented. No device ACK/rollback, OS TUN, RSS/long-run,
performance or other-platform certification. Only Q04/all its children are
excluded; UID finding remains unfixed and all other acceptance conditions remain.
This profile does not complete a main roadmap task; the goal stays active.

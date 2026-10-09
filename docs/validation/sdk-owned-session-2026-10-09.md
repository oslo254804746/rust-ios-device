# Owned finite existing-trust session operations

Accepted on Linux x86_64, 2026-10-09. Implementation: `2b906d7840590f1019ff4fa1b52e148433bdc7d7`. This is a bounded native
A03/L01 prerequisite work item, not the Python/C SDK or a completed main task.

## Delivered boundary

The additive `ios_core::session` module has `SharedAdmission` (1..64),
`SessionOwner` (1..4 concurrent fresh operations per owner), `OperationOptions`,
`OperationCancel` and consumed `OperationLease`. Admission has no waiter queue;
shared saturation is typed ResourceLimit, local saturation Conflict, closed
owners Precondition. The checked absolute total begins at lease admission and
includes time before the prepared future is polled. Zero total is local Timeout;
unrepresentable total InvalidInput. It does not create a native Elapsed.

Owner close seals admission, broadcasts local cancellation and awaits actual
lease retirement with a separate cleanup budget. A timeout retains the closed
state, actual outstanding count and permits until callers really release their
work. Repeated close can complete later. Last client owner-handle Drop broadcasts
cancellation; leases alone do not masquerade as client handles. An unpolled
prepared operation can still keep a closed owner outstanding until dropped.

The finite public `prepared_service_observation(stream, Arc<PairRecord>, lease,
service)` returns owned port/session-SSL/service-SSL metadata only. Callers
already own a stream and existing local trust; selectors are explicit, nonempty,
NUL-free and <=4096 UTF-8 bytes. No enumeration, implicit Pair, credential save,
retry, tunnel activation or advertised-port connection occurs. StartService can
have remote service effects; local socket close does not acknowledge rollback
or remote resource retirement.

Private HRTB factories, checked-session/IO context borrows and a private whitelist
of owned result types prevent raw transport/checked session escape through the
public entry. The prepared factory is not invoked after cancellation, close or
expiry. Stream/future destruction precedes returning admission and publishing
zero active count, including unpolled Drop, selector rejection and caller Drop.
Registration releases permits and updates active count in one synchronous book
critical section; actual close waits register Notify before inspecting the count.
Shared limits apply only to owners given the same pool, not every binding/runtime
in the process or arbitrary externally created IO.

## Wire, failures and timing

The additive checked lockdown path shares one absolute operation context across
QueryType, StartSession, real TLS and StartService. Error-only replies are checked
before success metadata. Optional Request must identify the operation; QueryType
must identify lockdown. SessionID must be a nonempty string, SSL metadata explicit
boolean, and StartService Port a nonzero u16. Unknown response codes keep typed
ServiceFailure; only owning StartSession permission codes are added. Existing
GetValue/StartService classes and legacy helper wire/defaults stay compatible.
The legacy TLS config builder changes visibility only, and exact unbuffered frame
reads preserve coalesced TLS bytes. The existing 4MiB frame cap does not establish
parser/RSS/CPU bounds.

A failed/cancelled/partially consumed StartService takes and retires the private
stream, rejecting reuse. Dropping an in-flight method closes its transport; an
unpolled method which performed no IO does not claim remote mutation. Completed
native/protocol errors win in the crossing poll; late successful payloads are
suppressed and no further native IO occurs after cancellation/expiry. Actual
Elapsed and native error source objects remain inspectable, with safe diagnostics.

The IO fence can retain an actual native Ready(Err) cause across an upper Pending
until cancellation/expiry. A handled error must be explicitly acknowledged before
non-IO waiting; successful or terminal completion and a subsequent allowed native
poll discard that candidate. Arbitrary Future causality cannot be inferred.
Zero-byte native read success is not invented as an acquired error. Synchronous
crypto/parse/OS work cannot be physically preempted by cooperative timers.

## Validation

Twelve owner groups plus eighteen checked-session groups exercise owned actual
TCP/duplex/TLS, original task join and strict socket EOF; real Unix TCP reset
checks exact original io::Error object identity across Pending. Other cases cover
blocked writes/TLS, partial reads, Error-first/malformed metadata, coalesced TLS
alert original rustls source, selector UTF-8 exact/plus-one limits, native error
crossing polls, actual Elapsed, zero/pre-cancel, unpolled Drop, closing isolation,
last owner handle, close timeout and repeated close, count/permit recovery before
behavior assertions. The reset fixture deliberately obtains real Ready(Err);
this does not weaken ordinary owned stream EOF assertions.

The first compile found a missing named borrowing lifetime in the source-chain
test helper; root fixed that fixture signature without altering assertions.
Affected Core all-feature unit tests: 1587 passed, zero failed, one existing
ignored; public/integration/doc contracts passed. Minimal default-feature tests: 349 passed, zero failed/ignored.
The initial full command passed format and workspace all-target/all-feature
check, then strict Clippy rejected one explicit auto-dereference in a test
`contains_key(*key)`. Root changed it to `contains_key(key)`, preserving the
assertion and product implementation. The two full manifests differ only in
that fixture. The initial full command exited one; it is not recorded as a
single successful full run.

Root then resumed format, strict all-target/all-feature Clippy and every remaining
required stage using the repository validator, serially and fail-fast. The resumed
run exited zero in 813.858 seconds; the preceding all-target check remains separate
evidence. Primary Rust tests: 2953 passed, zero failed, two existing ignored;
Python host: 13 passed. MSRV 1.80.1, 66 feature checks/four groups, default/full
doctests (seven each), three downstream examples, linked C ABI, freshly rebuilt
wheel/sdist and installed contracts, 73 tooling tests, 118-record inventory,
48 raw diagnostic captures, five host workflows, 95 CLI contracts and three
backup contracts passed. Additional doctest executions are not double-counted.

Root alone ran Rust tools. Each run froze its dynamic product path set and hashes;
final 493 paths and every committed product blob match the resumed manifest.
Evidence: target/validation/sdk-owned-session-2026-10-09/, including original
failed logs, both full manifests/results, `resume-full.py`,
`frozen-implementation.patch` (all new files included) and `accepted-evidence.json`.

Resumed log SHA256: `870844aa332ade9408854df98d62a2fb847ecec20774337f9e9f45763d5e3fc8`.
Initial fmt/check/lint-failure log SHA256:
`c06e83a385f21e006ff75373de415b6845f747af39692b66a19e2fe457270cad`.
Resumption script SHA256: `652b7df81b2c3249dfe10ea6d87231bebe2215dff3e97f8ca45e1f3ef7632d9d`.
Frozen implementation patch SHA256: `32265f6bcb68fa301c3be13315b245a6ba240a0057c44d4ff69bf47725797a5f`.
CLI SHA256: `6608dcb41900cb95dd11c2852416ec3c264dbc5e1f2e8674e87cf72180d7c8b2`.

## Remaining acceptance

UDID transport factory, finite device queries, apps/file/log integration,
Python context/async/GIL/cancellation and C ownership/thread/runtime/ABI entry
points and runnable examples remain next work. Existing pair-record blocking
filesystem workers still have a physical cancellation/exit gap. No bindings,
remote service execution, device readiness or physical/platform acceptance is
inferred. Twenty-two real-device rows remain NOT_RUN; authorization/selected
device/state/workflow and macOS/Windows/UNC conditions are missing. A03/L01 and
all original other-task criteria remain; Q04 and every child remain excluded.

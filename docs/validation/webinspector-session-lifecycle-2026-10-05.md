# L01 WebInspector correlated sessions and retained events

Host acceptance, 2026-10-05. Tested implementation:
`8011749571e2ae2a70ca8728984d0d93b56651bc`, based on
`4b8256684fbc15a125524f5a96d2616c0f5f3cbd`. Linux x86_64,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. This continues the
[client frame/start/event profile](webinspector-client-lifecycle-2026-10-05.md);
it does not complete L01, class4 or the all31-task goal.

Inspector send_command_and_wait and Automation's command helper now own one
checked absolute deadline across send/flush, unrelated events and matching
reply. Waiting no longer restarts the original duration after sending. The
shorter explicit/client budget applies. Inspector attach/unpaused owns setup
and optional target discovery; Automation attach owns request, two listings,
socket setup and final connection-ID discovery under one total. Automation's
page_id is assigned only after the complete attach returns successfully.

The directly owned logical guard commits after a complete outgoing command or
setup/request. Cancellation, expiry or error before the expected result then
invalidates the supplied client, even after complete unrelated frames followed
by an empty read. Owned metadata and pending events are dropped. Other sessions
and clones using that client reject late complete responses before IO; setter
None cannot revive it. The existing frame guard covers partial send/flush/read
failure. Unpolled futures, empty blocked writes and pre-IO admission can preserve
local reuse. No reader task, detached cleanup, reattach, ACK or retry is added.

Matching explicit business replies complete the wire obligation before ordinary
business parsing. Inspector still returns an error response object; Automation
still returns its original Protocol business error. Unrelated events are
restored to the front in their original order. Incomplete/cancelled owners
discard their skipped events when invalidating, rather than restoring them to
a connection that could consume a late reply. Original native/JSON/Protocol
failures and the primary error remain; a restore failure cannot replace an
already returned business error. Existing selector and response-match rules,
including transport-ACK discard behavior, remain unchanged.

Pending plus locally skipped events share fixed LOCAL caps of1024 events and
16MiB of compact JSON encoding of WebInspectorEvent. A counting writer includes
tags, keys, escaping and model payloads without allocating an encoded copy.
Existing pending events count against both budgets. Admission occurs before
appending; restoration checks the combined queue before retaining anything;
cached pop releases the byte budget and committed cancellation drops owned
queue capacity. These limits are separate from the unchanged16MiB wire cap.
They measure logical retained bytes, not RSS, allocator capacity, peak decoded
memory, latency or throughput. Application/page caches remain a subsequent
bounded-state owner; this profile does not certify all retained client state.

New private WebInspectorLimit retains precise ResourceLimit classification under
the existing Io variant through the feature-gated typed classifier. No new public
error variant, string classification or unrelated decoder-error substitution.
Both command counters and Inspector's transport counter permit u64::MAX once;
None records exhaustion. Exhaustion rejects before IO without wrapping/reusing
an ID. Inspector admits both counters before consuming either. This is the
existing local u64 wire domain, not evidence that a device accepts its maximum.

Scoped deadline inheritance prevents nested sends/automatic listings from
rechecking MAX configuration independently after an explicit finite cap has
already been admitted. It covers correlated helpers, startup, timed socket
reads and direct timed events including actual automatic listing writes.
Successful/error/cancelled scope exit restores the previous deadline policy;
independent sends with effective MAX still reject overflow. Timers reject due
nominal work and retain actual native Elapsed; no synthesized timer error.
Synchronous serialization/counting remains cooperative.

Session zero rejects before IO/ready input as source-free Timeout; effective
overflow is Io InvalidInput instead of the old internal Protocol overflow.
Actual expiry reports the original effective total rather than a restarted or
decreasing response remainder. Explicit finite budgets can clamp MAX, including
their nested wire operations. The old timing helper assertions remain test-only
and byte-identical after their production callers migrate. Six public variants,
constructors/signatures/root paths, default None, models/serde, wire/selectors,
startup Disabled mapping, business parsers, frontends and dependencies remain.
These timing/reuse/resource migrations require pre-1.0 release review; there is
no unsafe compatibility bypass or deprecation alias.

Send-only Inspector attach(false), send_command and bridge sends retain their
local write-completion contract; they do not own every future response after
return. Session getters/clones are not remote liveness. Discard the session when
its client becomes unusable and construct a new session with a new client;
transplanting cached target/page IDs is not certified recovery. Other public
Automation workflows can have multiple commands; their whole-flow frontend
budgets/cleanup remain subsequent work.

| Owned fixture | Observation and boundary |
| --- | --- |
| Actual command total | Inspector and public Automation get_title:125ms write block starting at first write poll plus125ms late reply under200ms; original total/Elapsed, unusable client and no further writes |
| Committed cancellation/expiry | Both sessions after a complete command and unrelated frame; empty wait, queued late complete reply and another session reject reuse |
| Business replies/order | Both original business policies, ordered first/second skipped messages, then successful next ID2 on the usable client |
| Attach | Paused/unpaused Inspector cancellation after setup/unrelated frame rejects late target; malformed target retains original Protocol; Automation request/listing cancellation invalidates without assigning page |
| Parsing | Unmatched malformed nested Target JSON retains its native serde_json source and invalidates |
| Count | Actual1025 unmatched wire events in each session reject ResourceLimit; pending plus skipped share1024 and restore rejection releases owned queue |
| Bytes | Exact16MiB admission, cached-pop release/re-admission, existing pending plus skipped, compact JSON escaping/Unicode and drop capacity |
| Admission/due work | Zero/MAX ready input stays unconsumed; unpolled/empty-write cancellation preserves client; due native timer does not poll ready work |
| Nested caps | Actual Inspector send/reply/idle, Automation command, startup identifier/state and automatic application-listing send inherit a finite cap over MAX; scope exit restores independent overflow policy |
| Counters | Actual Inspector direct/Target and Automation MAX bytes; no second write/wrap; exhausted transport does not consume a command ID |
| External consumer | Send futures, exhaustive six variants, actual command/native timeout, cloned session and queued late reply reject before IO |

Twenty-five new internal tests plus one external test. All old client/session/
root tests, previous client lifecycle assertions and original external consumers
remain byte-identical. Incoming routing, startup mapping, wire/models/old timing
assertions and protocol strings remain. Exactly twelve source/test files; no
frontend/dependency/reference edits.

First narrow54036 terminal0 passed21. Adding nested-cap fixtures exposed two
test-only E0283 type-inference diagnostics in18677 terminal101; that attempt ran
no tests. Concrete plist value type fixed compilation. Corrected72903 passed25;
first pipeline87754 passed minimum25/Core84/CLI40/stack5/strict gates. Direct
event inheritance and its actual listing fixture then changed production;
95858 passed26 and pipeline21871 terminal0 passed minimum26,
Core80+public4+integration1=85, CLI40, minimum CLI all-target check, original
stack5, strict Core/CLI Clippy and fmt. Finally only the new fixture file changed
to start the125ms write delay at first write poll, preventing scheduler setup
delay from shortening the exercised stage. Product/public hashes stayed identical
to21871. Supplement57845 terminal0 passed narrow26/minimum26/strict Clippy/fmt.
All build/test processes were terminal before source edits or starting another
build/test/check pipeline. Independent read-only audits did not mutate source.

Fixtures have3s outer bounds and20ms short waits; no old-source baseline-wide
negative control, device/browser/OS signal, benchmark or other-platform result
is claimed. With no live builds, initial focused CLI cache clean removed1657
files/6.6GiB (about1.1GiB free before); another pre-full clean removed778
files/3.0GiB (3.2GiB before,6.1GiB after). No source/reference removal or storage
failure. Full ended with about1.9GiB free.

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full70542 terminal0: **2340 Rust/13 Python**, CLI752/Core1427/public51,
zero FAIL/two existing ignored. Every section11 split gate passes: workspace
fmt/check/strict Clippy/MSRV;66 feature checks/four runtime groups;7+7 doctests/
three downstream snippets; C ABI;3.12 wheel/sdist build/install/import;
59 tooling/118 actual-help/38 raw/five host/55 actual CLI/three backup contracts.
Chromium/installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Frozen twelve-file source SHA256:
`93ce28b9c0926b4dc0b81991ca77f0f85e9495a17d89118957b823246cf3b8c2`.
Final ios debug binary SHA256:
`ec4f0d0e9b0eab456dd5e34e549ac372e8a41d90e5f0638c581f33a0d1cb534e`.
The actual implementation commit exactly matches the frozen patch through full.
Local evidence/audits use /tmp/ios-l01-webinspector-session-. Capability118
bodies/maturity/history and all31 tasks/subtasks remain. Next application/page
cache bounds, WebInspector frontend/server ownership, other L01, L02–L04/Q04
and classes5–9. No push/publication/history rewrite/subagent/reference edit.
Host acceptance does not replace platform/device/credentials/protocol conditions.

Documentation audit PASS: eight documents/seven Markdown/330 local links;
source and actual CLI-help capability inventory PASS. All31 task/subtask/history
prefix remains with only the L01 overview and final delivery appended. All118
capability bodies/maturity/prior evidence remain with one exact host entry.
Implementation/source/binary/full/narrow/compile/supplement/cache-clean provenance
and read-only references match; product source stayed unchanged during docs.

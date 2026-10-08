# Paired discovery error at the final epoch fence

Implementation `aa3c3a46c19dd19175c879bcb44f5856d24259ec`, parent `257aecc00f8a78a6be023132abe9b0769043eaed`; Linux x86_64,
Started 2026-10-07 UTC, accepted 2026-10-08 UTC; Rust1.97.1/MSRV1.80.1, project Python3.12.13.
This follows [the Source reply fix](tunnel-paired-error-retention-2026-10-07.md).

The first paired epoch check can pass, then clear/close can change the epoch
before the final candidate-handoff check. That second stale branch also returned
Ok and discarded an already acquired original paired error. It now maps only
successful stale replies to Ok and returns the original Err unchanged.
One production return branch changes; the boundary hook is test-only.
Public APIs/options/error enums/wire,clock budgets,Core/bindings/dependencies/
features and preceding assertions stay. The changed error outcome needs
pre-1.0 review. No whole-scan deadline or paired error persistence is added.

3 source files 158+/1-;four additive tests use the actual scanner and native userspace
runtime. Actual loopback TCP partial read obtains UnexpectedEof. The one-shot
hook runs only after the first epoch check succeeds, with no hook/table/watch
lock held. Clear is the actual manager clear future and must complete in its
first uncontended poll; close uses actual notify_shutdown. The clear-success
control supplies a unique nonempty paired snapshot,close-success an empty one.
This deterministic boundary fixture represents the concurrent epoch-change race;
it is not a threaded stress, mobdev2/DNS, pairing or physical-device test.

Initial exact published native Arc is explicitly joined;native shutdown_status,
Duplex EOF,registry closed-and-empty and runtime task baseline are checked before
regression assertions. One-shot state cycle is released both on invocation and
unexpected early return. Calls are initial connect1/USB1/paired1;no stale Source
connection or ready mutation. Error downcast/kind and established Io category stay;
error allocation address identity is not separately tested. Nonempty close can
return Cancelled at ticket admission before the final fence;clear-success can
create a coordinator before discarding its snapshot. No zero-coordinator mutation
or universal stale-success Ok result is claimed.

Old production75395 terminal101:2PASS/2FAIL,all actual native cleanup assertions
passed before the two Err regressions failed. Exact test-only sources/log and454
product hashes are frozen. Fixed matching run 60167 terminal0:15PASS.
Final52043 terminal0 freezes454 products before/after all seven serial gates.
Root alone runs Rust and edits source;no source/doc edits during validation.

| Check | Result |
| --- | --- |
| CLI tunnel |216PASS/1existing ignored |
| Minimum CLI all targets/original stack |PASS/5PASS |
| Strict workspace Clippy/format |PASS/PASS |
| Section11 full Rust/Python split |2740Rust/13Python,0FAIL/2existing ignored |
| Project .venv tooling |73PASS |

Full CLI1079/Core1496/public55;MSRV1.80.1,66features/four groups,7+7doctests,
three downstream snippets,C ABI,Python3.12 rebuilt wheel/sdist/install/import,
help118/raw38/host5/CLI95/backup3 PASS.22device NOT_RUN;Chromium/installed3.9/3.13
NOT_RERUN. No maturity/ACK/device/rollback/RSS/performance/long-run/platform proof.
Frozen patch SHA256 `7bc7fb5db19d2a9266fad866b1e31fed6dd750a02087be1300bfaff07429cecf`;actual ios SHA256 `707fd3603787a1ef2fa2b594fdf3c3343dbf86127691fc3242056e3872ef431d`.
Sources/command exits/log hashes and reviews: `target/validation/paired-final-fence-2026-10-07/`.
Reference HEAD/status unchanged;CARGO_INCREMENTAL=0,no cache deletion/push/publish/
history rewrite/device/credential/Apple/elevated TUN operation.

Both paired stale branches now preserve an acquired Err. After those branches,
reconciliation/candidates still await outside discovery setup clock;paired Err
only lives in the scan future and is not persisted across an outer caller drop.
Whole-scan total/cancellation/failure bookkeeping,metadata admission,HTTP/upgrades/
blocking/startup/physical freshness/reuse/restart/manual epochs,L03-L04/classes5-9
and every other original acceptance remain. Active30/allchildren excludingQ04,
9complete/21unfinished(9notstarted/12progress-or-conditions);UID untouched.

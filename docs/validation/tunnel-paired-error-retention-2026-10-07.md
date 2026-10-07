# Acquired paired discovery error across clear and close

Implementation `1ab05e3bcefd3926a402dc645f55c72d16aad6f4`, parent `6f9e4fd6187750a6a9a2f7416fd597990287adeb`; Linux x86_64,
2026-10-07, Rust1.97.1/MSRV1.80.1, project Python3.12.13.
All30 original main tasks and every child excluding Q04 remain required:
9complete/21unfinished (9notstarted/12inprogress-or-conditions).

## Original error and lifecycle behavior

The scanner already discards successful USB/paired snapshots from an old scan
epoch. Its paired branch previously returned Ok whenever clear/close changed
that epoch, even if the same Source poll had already returned a native IO error.
That swallowed an original acquired error. The stale paired branch now maps
only successful discovery to Ok and returns its original error unchanged.
Successful stale replies still cannot hand off candidates or mark ready.

The four fixtures cover clear/close completed inside Source before its reply.
Read-only follow-up review identified another open race: clear/close after the
first paired epoch check can still reach the later stale branch that returns Ok
and drops the acquired error. That second fence is not fixed or certified by
this commit; it requires a separate regression and implementation.

One production return branch changes; no new wrapper, public API, clock, error
enum, option, dependency, feature or binding. Original native error downcasts and
classification remain inspectable. Candidate/discovery timing and stop-A/active-B
isolation remain. Returned error rather than success requires pre-1.0 review.
Whole-scan post-paired totals/cancellation, out-of-clock failure bookkeeping,
metadata bytes,HTTP/upgrades/blocking/startup/physical freshness/reuse/restart/
manual epochs and all other original L01-L04/classes5-9 requirements remain.

## Actual host fixtures and wrong-old baseline

3 source files 134+/ 1-;4 additive tests use the actual scanner and private Source seam.
A loopback TCP peer writes one byte and closes; the source's real two-byte
read_exact acquires UnexpectedEof. Before the same discovery poll returns, it
executes completed manager clear or terminal notify_shutdown. Error cases return
that original IO; success controls return a supplied empty paired snapshot.
This is real local IO with private discovery metadata, not actual mobdev2/DNS,
pairing, device or Apple-service evidence.

Each case starts with one actual published userspace tunnel. Clear/close stops
its exact original Arc; the existing native registry explicitly joins it. Actual
DuplexStream EOF, original shutdown_status and task baseline recovery are checked
before the error regression assertion. Calls remain one initial connection,
one USB snapshot and one paired discovery; no stale candidate connects or marks
ready. The one-shot state owner is consumed before IO and any unconsumed control
is explicitly released before cleanup, avoiding retained state cycles.

The first run used unchanged accepted6f9e4fd production:82506 terminal101,
new2PASS/2FAIL plus2preceding matching tests PASS. The two acquired-error cases
incorrectly returned Ok; both success controls stayed passing. All four actual
native recoveries passed before regression. Exact test-only baseline sources,
453 product hashes and failure logs are retained. Fixed narrow result is recorded
with the final evidence; original tests/assertions remain.
Development13051 terminal101 retained original UnexpectedEof and completed all
native recovery assertions, but the new fixture incorrectly expected Disconnected.
The established direct-EOF classification is Io, including possible host-file
truncation. Only that new expected category was corrected after terminal exit;
Core classification and production fix stay.12590 terminal0,all6 matching tests
PASS (4new/2preceding). Exact failed sources/logs are archived.

## Accepted serial validation

Runner26492 terminal0; all seven serial gates freeze453 products before/after
each command. Root alone applies source and runs Rust; no source/doc edits while
validation lives. Exact sources, command/exit/log hashes, source review and frozen
patch are in `target/validation/paired-discovery-primary-2026-10-07/`.

| Check | Result |
| --- | --- |
| CLI tunnel regression |212PASS/1existing ignored,including4new |
| Minimum CLI all-targets/original stack |PASS/5PASS |
| Strict workspace Clippy/format |PASS/PASS |
| Section11 full Rust/Python split |2736Rust/13Python,0FAIL/2existing ignored |
| Project .venv tooling |73PASS |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full CLI1075/Core1496/public55;workspace check/fmt/strict Clippy,MSRV1.80.1,
66features/four groups,7+7doctests/three snippets,C ABI,Python3.12 rebuilt
wheel/sdist/install/import,tooling73/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. Maturity stays unchanged;
no OS TUN,device/ACK/rollback/replay,resource/performance/long-run/platform proof.

Frozen patch SHA256 `0d92fcf30c3fc44e62f4975228dcad692d14147de054cf8946816fe8b3f7e56b`.
Actual debug ios SHA256 `84127d56cd68799bbe0a66aec08b72763c1afefe346af32dcb67266f75d37bc0`.
Implementation equals tested freeze; reference HEADs/statuses remain unchanged.
CARGO_INCREMENTAL=0, no cache deletion/push/publication/history rewrite,
credential/device/Apple/elevated TUN action. Q04 and all its children are excluded;
UID risk is untouched. All other original acceptance remains. L01/L02/class4
and the goal stay active; no main-task count changes.

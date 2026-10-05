# L01 CDP browser page outcomes and explicit joins

Tested implementation: `d8cf32eb4ad09217c3f9962df658eea03fed12e9`, based on
`79d008b165824b78bdb8d30dc51b5d3551daaf48`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01, class4 and all31 tasks remain active.

The [browser child constructor](../../crates/ios-cli/src/cmd/webinspector/cdp.rs)
now returns its private task owner. One pinned local watch signal controls active
work outside the original unbiased inbound/outbound choice, with a yield each
turn. Usable empty idle still resumes. Native send/receive/decode failures and
invalidating timeouts return their original causes. The child records primary
before independent2s session cleanup, then records cleanup before publishing the
existing Closed event. Valid attached clients use the existing close selector;
invalid clients drop without a recovery write.

The [explicit supervisor](../../crates/ios-cli/src/cmd/webinspector/cdp/browser_tasks.rs)
owns those outcomes independently of the actor. Detach, Closed and failed input
sender paths stop and join their removed child, retaining failures in existing
safe diagnostics while keeping successful detach replies. Browser loop exit
cancels all remaining pages under one2s total; it aborts every unfinished task
before joining each handle. One failed child cannot skip another join. The
existing wrapper retains all known errors and an actually issued supervisor
expiry; primary survives an abort during cleanup. Native unexpected JoinError
remains owned. Normal completion without a cleanup outcome returns a local
Protocol error. An interrupted, unreturned cleanup cause is not fabricated.

The3-file implementation adds669 lines/removes76 and11 tests. Direct page owner,
browser constructor setup body, discovery helpers and every original assertion
stay; one existing Drop fixture gains the required outcome field. Core, public
bindings, access, dependencies, other owners and cleanup controller are unchanged.
No new wire keys, selectors, public source backend or credential/device operation.
Timing and diagnostic changes need pre-1.0 release review. Cooperative timers do
not preempt synchronous work. Drop remains fallback abort without async join.

| Eleven [fixtures](../../crates/ios-cli/src/cmd/webinspector/cdp/browser_task_tests.rs) | Actual evidence |
| --- | --- |
| Delayed flush | Actual80ms flush finishes despite50ms active idle |
| Pending flush | Native child cleanup waits the actual default2s |
| Failed flush | Supervisor retains original PermissionDenied IO |
| Invalid nested JSON plus failed flush | Both original serde_json and IO causes remain |
| Known JSON primary plus supervisor abort | Actual close frame proves primary recorded first; native50ms supervisor Elapsed, abort/join and release follow |
| Two pending children | Both cancel/abort/join and release under one default2s total; observation below3.5s distinguishes serial4s, not a performance baseline |
| Healthy empty idle | Multiple50ms idle periods resume the exact target reply, then normal joined stop |
| Observed partial frame | Actual2 delivered bytes before cancellation; original invalid client skips close/recovery |
| Full output queue | Actual64 queued messages plus additional observed input bytes; cancellation completes while the queue stays full, close/release/join occur without draining business output |
| Closed output receiver | Original mpsc SendError survives join and normal close |
| Panic/missing outcome | Two standalone supervisor actors, each alongside an actual forwarding child; native JoinError/local Protocol retained and the other child closes/joins |

Ten fixtures directly use the actual child constructor; the eleventh also
constructs two standalone actors to test only supervisor admission. There is no
new public route/admission or external Chromium claim. Actual close app/page/
sender, owned stream Drop/EOF and matching qualified page-lock reacquisition
are checked. Local write/flush is not an Apple ACK. Terminal try_send can miss
a full64 queue; the full-output fixture proves join/release independently and
does not claim terminal event delivery. Event retention remains resource work.

Before-product25725 exited101:0PASS/2FAIL, observing50ms cleanup and unfinished
80ms flush. Corrected55684 exited0:2PASS; expanded52734 exited0:6PASS and15147
exited0:11PASS. A test double-join was removed before expanded compilation;
completion observation leaves the actual join exclusively to its supervisor.
All changes preceded compilation; no source edit during live processes.

Frozen pipeline52191 exited0:all21 gates pass. Source remained frozen through
authoritative process completion; observation timeout never counted as exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_cdp_browser_child all features/minimum CLI | 11/11 PASS |
| Prior lease/WebDriver/direct cleanup/setup/page/finite/shell | 7/8/5/15/5/21/31 PASS |
| CLI WebInspector/access/core WebInspector | 150/29/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets | 17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt | 5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2467 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI862/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and the
separate browser gate stay ignored.22device NOT_RUN; Chromium and installed
Python3.9/3.13 not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`bf4149bca9f6545e8b7021a9d1a004319a7f013939e7a4d61dc4d6ae0950d7f3`.
Actual debug ios binary SHA256:
`358870ad1afb7f3f2422ecca8a5c9fb08d7a49cedb2fecbfe622b1474dbceaeb`.
Actual commit matches frozen source. Logs/copied development evidence with
original hashes/audits live under ignored
`target/validation/cdp-browser-tasks-2026-10-05/`; reference HEAD/status unchanged.

Explicit returned stop only. Browser outer Principal.run still drops the parent
on revocation. In-progress removed-child cleanup must remain owned outside a
raced active future before browser lease joins can be claimed. Generic caller/
server Drop, constructor primary cleanup, active request/output totals, stored
registry ownership, other L01, L02–L04/Q04 and classes5–9 continue. No Apple ACK/
rollback/retry/async Drop/browser lease join/platform/benchmark/RSS/decoder peak
acceptance. All31 tasks/subtasks/history,118 capability bodies/maturity/prior
evidence and counts9 complete/11 not started/11 in progress or pending conditions
remain. No subagent,push,publication,history rewrite,reference edit or real device/
Apple operation. This intermediate commit does not end the engineering goal.

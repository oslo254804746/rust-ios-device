# L01 stored session watcher ownership and joins

Tested implementation: `7fb827b1ee4ff17e29597123b23672184a20a0a5`, based on
`a0bf070aa52d14c57c1ee1981c0b5acdf2c6ff06`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01/class4/all31 tasks remain active.

The [registry](../../crates/ios-cli/src/cmd/webinspector/sessions.rs) now owns
active entries and retired [watcher owners](../../crates/ios-cli/src/cmd/webinspector/sessions/task_owner.rs).
Explicit removal/replacement retains the owner and issues abort before waiting
for its join mutex. If cancelled, the store still holds the task and its issued
abort. Borrowed joins record native completion/outcomes, never poll completed
handles again and transfer native errors without a further await. Original
unexpected panic/external-cancel JoinError stays downcastable, classified
Unknown/Cancelled; an explicitly issued abort's cancellation is neutral.
Registry joins use existing safe tracing, never raw panic payloads/new HTTP
fields. WebDriver DELETE's existing100ms total includes this removal join wait.
This pre-1.0 timing correction needs release review and device evidence.

The revoked actor verifies original weak resource identity before removal,
transfers its own handle to retirement and releases the strong store/global
lock before the existing100ms unit cleanup callback. It neither aborts nor
joins itself. A stale actor cannot remove a replacement resource. Registry
insert/get/principal/remove reap only finished retired actors, using try_lock
to avoid waiting on an active join. Explicit joins happen outside the global
map lock. No new actor, background reaper, source backend or protocol operation.
Idle completed handles stay until the next registry call or Drop. Generic
store/caller Drop remains abort fallback without asynchronous join.

Four files add687/remove22 lines and13 tests. Both original security/Drop
assertion functions and admission middleware are byte preserved. Private
registry signatures, unit cleanup policy, other owners/constructors, core,
public Rust/C/Python surfaces, access, dependencies, routes and successful
responses stay. No resource cap or performance acceptance is inferred.

| [Six protected loopback registry fixtures](../../crates/ios-cli/src/cmd/webinspector/sessions/lifecycle_tests.rs) | Actual result |
| --- | --- |
| Explicit remove |RequestedAbort native join before resource return; resource releases only after caller drops it |
| Cancelled removal wait |Actual join mutex held, entry gone and owner retired; another actual HTTP insertion completes without waiting on that mutex; API reap resumes the issued abort/join |
| Reload plus80ms cleanup |Addressability removed, original actor finishes normally rather than self-aborting; registry API reaps its join |
| Reload plus pending callback |Real default100ms expiry; callback does not finish but actor exits normally and is reaped |
| Same-ID replacement |Old watcher is aborted/joined; new resource generation remains addressable |
| Store Drop during cleanup |Actual AbortHandle proves actor pending and resource not dropped before store Drop; weak store becomes unavailable and abort/resource release completes |

These use actual HTTP admission/Host protection with synthetic policy, then
private Sessions operations and generic owned host Resource. They are not
Automation wire/device fixtures or actual Selenium creation routes. The live
actor-before-Drop check prevents a completed timeout being mistaken for Drop
release. Test listener joins are fixture supervision, not product server join.
Fallback abort/release does not certify asynchronous Drop cleanup or Apple ACK.

Seven [owned Tokio actor fixtures](../../crates/ios-cli/src/cmd/webinspector/sessions/task_owner/tests.rs)
cover normal/repeated join, explicit abort, original panic, unexpected external
abort, cancelled join/resumption, reap that never waits for an active actor and
Drop fallback. They retain native flags/classifications and observed actor guard
release. These standalone actors make no authorization/protocol/device claim.
The prior nine actual WebDriver DELETE, both original security scope/Drop and
all browser/page/cleanup profiles still pass.

First99572 exited0:13PASS. After that run, native Unknown/Cancelled assertions
and the actual pending-actor proof before Drop were added; frozen gates verify
the expanded source. First frozen63290 exited101:18 preceding gates passed,
strict Clippy manual_inspect failed on a map returning the same entry. Full
had NOT_STARTED. After authoritative exit/no live builds, the sole source
correction changed map to inspect and removed its redundant entry return.
All four archived source hashes and the exact correction are audited.

Corrected frozen82353 exited0, all24 gates pass. Source stayed frozen through
authoritative completion; observation timeout never counted as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_session_ all features/minimum CLI |13/13 PASS; actor7+registry6 |
| Prior actual DELETE/original scope/browser/page lease/runtime/direct/setup/page/finite/shell |9/2/23/7/8/5/15/5/21/31 PASS |
| CLI WebInspector/access/core WebInspector |184/29/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets |17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt |5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI |22/95 PASS |
| Section11 full Rust/Python split |2501 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI896/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and separate
browser gate stay ignored.22device NOT_RUN; Chromium/installedPython3.9/3.13
not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`05695680f3810a98b850c45eba0984130e406afdf334c9525e6e68d5e9212a34`.
Actual debug ios binary SHA256:
`f22253382855a70083544b0e29d4ef00d4787679fcaa5b346767f939620274a7`.
Actual commit matches frozen source. Durable first run/corrected logs/archived
failed attempt/four source bytes/native hashes/audits live under ignored
`target/validation/session-store-2026-10-05/`; reference HEAD/status unchanged.

Task ownership/joins only. Legacy unit callback/native protocol cleanup primary,
active request Arcs, idle retired handles without API calls, generic store/
caller/server Drop, constructor primary cleanup, active request/output totals,
terminal retention and resource caps remain. Other L01/L02–L04/Q04/classes5–9
continue. No Apple ACK/rollback/retry/async Drop/server join/platform/benchmark/
RSS/decoder peak acceptance. All31 tasks/subtasks/history,118 capability bodies/
maturity/prior evidence and9 complete/11 not started/11 in progress or conditions
remain. No subagent,push,publication,history rewrite,reference edit or real
device/Apple operation. This intermediate commit does not end the goal.

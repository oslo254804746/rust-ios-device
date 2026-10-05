# L01 CDP browser lease and retiring owners

Tested implementation: `d73514f4b83c956fe860f1051d27f0d912203a5d`, based on
`36d5a21f1d7e43926994bb7893bae8ad8f7e398a`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01, class4 and all31 tasks remain active.

The [browser parent](../../crates/ios-cli/src/cmd/webinspector/cdp.rs) now pins
one principal.revoked future through initial discovery and active work. Biased
outer control and a yield per turn preserve the original inner unbiased traffic
choice. Typed OperationCancelled remains parent primary. Cleanup does not poll
the completed lease again. Previously outer Principal.run dropped the whole
parent, bypassing its asynchronous cleanup. Browser revocation now reaches the
existing safe diagnostic formatter. Timing/diagnostic changes need pre-1.0
release review; no error frames/headers, public grammar or admission change.

Every removed page enters a [retiring owner](../../crates/ios-cli/src/cmd/webinspector/cdp/browser_tasks.rs)
outside the raced active future before cleanup awaits. Its borrowed stop keeps
the original deadline and issued waiter result. Completed native join outcomes
are recorded outside that future, so resumption skips completed handles.
Cancellation during abort/join keeps the actually issued Elapsed and known
primary/cleanup causes. Outcomes transfer together after all joins, with no
further await while taking them. On parent exit, retiring and remaining pages
share common independent2s cleanup; any previous retiring waiter error remains
owned. Every unfinished task is aborted before joining each handle.

The4-file change adds873 lines/removes70 and12 tests: three added child
resumption fixtures plus nine new browser fixtures. Direct page owner,
actual browser child constructor/active/cleanup, discovery helpers, every
previous assertion and extracted release assertions remain. Parent traffic
logic changes only to retain removed owners for borrowed cleanup. Core, public
bindings, access, dependencies, other owners and shared controller stay. Private
helper/import used only by tests gains cfg(test). No new protocol selector,
source backend, detached source worker or real device/credential operation.

| Nine [browser fixtures](../../crates/ios-cli/src/cmd/webinspector/cdp/browser_lease_tests.rs) | Actual evidence |
| --- | --- |
| Policy reload after attachment | Matching forwarding close, stream Drop/EOF and qualified lock reacquisition |
| Grant expiry after attachment | Same cleanup; an owned pending periodic discovery is checked if contacted |
| Reload plus delayed flush | Actual80ms flush completes before release |
| Reload after partial frame | Counter proves2 delivered bytes; invalid original client skips close/recovery |
| Reload plus pending flush | Real default2s cleanup completes by issued expiry, with release |
| Reload during detach cleanup | Actual close frame and unfinished flush prove retiring phase;80ms flush completes despite interrupted removal |
| Reload during initial source | Actual stream held during4s source delay drops before return or wire; one source call,0 received bytes |
| Reload during child source | Unpublished constructor releases stream and actual qualified page lock before source return/wire; only two source calls |
| Private owned-stop observer | Actual handler returns Cancelled primary plus original PermissionDenied cleanup IO after child cleanup/join |

Eight fixtures use actual cdp_router/admission, synthetic policy and browser
WebSocket. A queue adapter verifies authorized synthetic DEVICE before calling
the existing owned-source alias. Actual auto-attach and Runtime.enable forwarding
prove attachment. The ninth private result route observes the actual handler
with an owned stop; it makes no admission/revocation claim. Test listener joins
do not prove product server task supervision. Local close/flush is not Apple ACK.

Three [child resumption fixtures](../../crates/ios-cli/src/cmd/webinspector/cdp/browser_task_tests.rs)
cancel a borrowed cleanup after an actual completed join, retain completed
native JSON/IO errors while another child is pending, and reparent after an
actually issued50ms expiry during abort/join. Completed joins are not repolled;
native causes and all releases remain. Eleven previous child tests still pass,
including shared2s cleanup, panic/missing outcome and full queue. Terminal
try_send can still miss a full queue; no terminal delivery is invented.
Cooperative timers cannot preempt synchronous work. Generic Drop still aborts
without async join; product server/caller Drop remains separate.

Development61458 exited0:14 child tests pass. First browser compile34143
exited101:four E0283/E0608 fixture errors, no tests. Corrected68558 exited101:
1PASS/5FAIL; observed partial cancellation passed, four revocations missed close
and detach flush stayed unfinished. Parent correction90464 exited0:20PASS;
expanded11501 exited0:23PASS. One attempted helper-visibility patch applied
nothing because the helper was already visible; no existing helper was edited.

First frozen pipeline10010 exited101:15 preceding gates passed, strict Clippy
failed on a constant import now used only by cfg(test); full had not started.
After authoritative exit/no live builds, the import gained cfg(test). This is
the sole change from the archived first frozen source. Corrected frozen
pipeline26582 exited0, all21 gates pass. Source stayed frozen through process
completion; observation timeout never counted as exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_cdp_browser all features/minimum CLI | 23/23 PASS; child14+browser9 |
| Prior page lease/WebDriver/direct cleanup/setup/page/finite/shell | 7/8/5/15/5/21/31 PASS |
| CLI WebInspector/access/core WebInspector | 162/29/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets | 17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt | 5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2479 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI874/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and the
separate browser gate stay ignored.22device NOT_RUN; Chromium and installed
Python3.9/3.13 not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`9dfc360a55fc161b621a6352fa61d0d5c96cc69692052f175d083e1085ef517f`.
Actual debug ios binary SHA256:
`64c6adcef589a9a6c20cd71934a08de79193aed63ea31a9545b3a46b00bddaf7`.
Actual commit matches frozen source. Durable logs/copied development evidence,
archived failed attempt/native hashes and audits live under ignored
`target/validation/cdp-browser-lease-2026-10-05/`; reference HEAD/status unchanged.

Browser lease/retiring owner only. Generic caller/server Drop, constructor primary
cleanup, active request/output totals, stored registry ownership, terminal event
retention, other L01, L02–L04/Q04 and classes5–9 continue. No Apple ACK/rollback/
retry/async Drop/server join/platform/benchmark/RSS/decoder peak acceptance.
All31 tasks/subtasks/history,118 capability bodies/maturity/prior evidence and
counts9 complete/11 not started/11 in progress or pending conditions remain.
No subagent,push,publication,history rewrite,reference edit or real device/Apple
operation. This intermediate commit does not end the engineering goal.

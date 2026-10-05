# L01 WebInspector server setup totals

Tested implementation: `82db3a8699f99de4d7275baa896e408e543023ed`, based on
`3a5a1d9c1b6c5a697bc20f79a26db0ab08d30b9f`, Linux x86_64,2026-10-05.
Rust1.97.1/MSRV1.80.1,Python3.12.13. L01, class4 and all31 tasks remain active.

[CDP](../../crates/ios-cli/src/cmd/webinspector/cdp.rs) full-page discovery,
page/browser-page constructors and the
[WebDriver runtime constructor](../../crates/ios-cli/src/cmd/webinspector/webdriver.rs)
now each share a checked absolute total, starting before page-lock/source
contact. Startup, discovery, attachment and WebDriver context creation cannot
restart that total. Each stage caps the client to its remaining allowance.
The existing private controller retains native timer/failure behavior and
rejects late success; successful constructors clear the setup cap before
existing active operations. Cooperative timers do not provide hard preemption.

Exact CDP target discovery proceeds after an actual admitted match rather than
waiting for a full snapshot's quiet period. Missing targets wait within the
setup total. Only reusable empty idle retries; invalid errors remain primary.
Full CDP lists and WebDriver discovery use a separate500ms quiet window inside
the total. Short totals can therefore fail before quiescence. Incomplete browser
attachment drops without an attempted recovery selector. These timing/selection/
failure changes need pre-1.0 release review. Successful grammar/JSON formats,
core/public/binding APIs, protocol fields, dependencies and access policy stay.

The9-file commit adds15 tests and only expands the checked controller's private
visibility, plus a remaining-cap helper. Original controller logic and CDP/
WebDriver/task/finite/shell assertions remain; the5 prior CDP lifecycle functions
remain byte-identical. The shared bridge fixture's budget changes100ms→1s to
allow its500ms quiet collection; its assertions stay. Private cfg(test) source/
peer helpers are shared, using existing selectors and Automation create/get/close
behavior. Delay/stall extensions default to their previous behavior; no public
backend selector, forged ConnectedDevice or speculative protocol is added.

| 15 current owned tests | Verified behavior |
| --- | --- |
| Four source controls | Private source owns its stream before a200ms wait;50ms setup expiry drops the actual pending future before return, with one call/stream Drop/peer EOF/no wire |
| Zero/overflow groups | All four constructors reject private zero/MAX state before source contact; typed Timeout/InvalidInput, no issued Elapsed; unadmitted fixture streams stay caller-owned until fixture teardown |
| Source/startup totals | CDP discovery/browser-page and WebDriver600ms totals include200ms connection;500ms response delay exceeds the remainder while each stage is shorter than the total; release observed before a restarted startup cap could expire |
| Page discovery remainder | Actual connection/state/app-list/listing requests consume one600ms total; remaining wait ends inside its bounded fixture window without attach/retry/teardown bytes |
| Initial writer and page lock | Actual blocked write polls expire and release; held page lock expires before source contact and is reacquired after release |
| Browser caller cancellation | Cancellation after actual socket setup and before target creation drops stream, closes the output sender and releases/reacquires the page lock; no child was published or recovery write attempted |
| Successful WebDriver | Existing Automation peer creates context, permits idle after750ms setup and a400ms active reply, then stops context; cleared setup cap does not govern active use |
| Stalled/cancelled context creation | Real create frame is observed;650ms total limits its unreplied remainder, and caller cancellation after that frame drops the constructor/stream without retry/teardown |

Local test windows are not latency commitments or L04 baselines. Successful
WebDriver stop still closes only its browsing context: the fixture observes
zero forwarding teardowns. Setup failure after full attachment, primary and
independent cleanup, server signals, HTTP publication/output, active request
totals, revocation and joined child/stored task ownership remain. Existing
stored-session100ms and browser-page2s cleanup policies were untouched.
These private function/owned WebSocket fixtures add no new admission, external
browser, device, Apple ACK, rollback, retry, platform, RSS or decoder-peak proof.

Before-product69631 exited101:0 PASS/4 FAIL. All four injected sources returned
after their admitted budget. This demonstrates the missing frontend seam guard;
production DeviceSource already had a per-call cap. After-owner42493 exited0:
4 PASS. Initial regression91234:108 PASS; expanded33735:14 PASS; final18610:
15 PASS. No compile/test failure followed the product change. A patch context
failure applied nothing; a source-audit comparison initially needed normalization
of rustfmt's signature trailing comma, with no product change.

Frozen pipeline42886 exited0, all16 gates pass. Builds ended before source edits
or new validation; an observation timeout never counted as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_server_setup all features/minimum CLI | 15/15 PASS |
| Prior CDP page/finite/shell regressions | 5/21/31 PASS |
| CLI WebInspector/core WebInspector | 119/102 PASS; core96+public5+integration1 |
| Core-only webinspector cache minimum | 17 PASS; CLI still enables core full |
| Minimum CLI all-targets/original CLI stack tests | PASS/5 PASS |
| Strict core/CLI Clippy/workspace fmt | PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2436 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI831/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 feature checks/four runtime groups,7+7 doctests/three downstream snippets,
C ABI,Python3.12 wheel/sdist build/install/import all pass. Tooling67,actual
help118,raw38,five host workflows,95 CLI cases and3 backup contracts pass.
The2 ignored tests remain PBKDF2 and the separate browser gate.22device NOT_RUN;
Chromium and installed Python3.9/3.13 were not rerun. No maturity upgrade.

Frozen patch SHA256:
`46b1ebad6e7a3de57aa19ef2f2451621c905c1b274c9f0945eca41e9f8ed41c7`.
Actual debug ios binary SHA256:
`b05e520c44307f686a9b0f3de2565a07627583f8056bd1da8f87c84f8e83c499`.
Actual commit matches the frozen/tested patch. Evidence, copied development
logs with original hashes and audit live under ignored
`target/validation/webinspector-server-setup-2026-10-05/`. Reference HEAD/status
stay unchanged; no cache cleanup was needed.

This setup profile does not cover CLI listener zero admission or server startup;
the existing server argument parser still keeps its prior duration policy.
Continue remaining L01/cleanup/server ownership, L02–L04/Q04 and classes5–9.
All31 tasks/subtasks/history and118 capability bodies/maturity/prior evidence
remain; counts stay9 complete/11 not started/11 in progress or pending conditions.
No subagent, push, publication, history rewrite, reference edit or device/Apple
operation occurred. This intermediate commit does not end the engineering goal.

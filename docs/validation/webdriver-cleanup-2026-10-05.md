# L01 WebDriver runtime cleanup

Tested implementation: `8956866c4d095c5f3f0919a431b8b7e022f55e54`, based on
`37ab23affb041874b72cf5dc99cf9d41f662c989`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01, class4 and all31 tasks remain active.

[Runtime stop](../../crates/ios-cli/src/cmd/webinspector/webdriver.rs) now borrows
its original AutomationSession and application identity for context close then
forwarding socket close. Both use one100ms total, retaining the stored-session
limit. Previously stop closed only the context; its core call had a separate10s
limit and depended on the outer100ms timeout. The existing
[Automation cleanup body](../../crates/ios-cli/src/cmd/webinspector/tasks/session_owner.rs)
is moved unchanged into a borrowed helper; finite/shell5s and direct-page2s
policies remain. No session clone or command ID reset is introduced.

Starting stop disarms another attempt, including success, business failure and
caller cancellation. An invalid client skips socket recovery. Native IO/Elapsed
and both context/socket failures remain owned with original primary classification.
Device/client ownership remains with the runtime; explicit Drop releases the
stream in these fixtures. This is not async Drop or an Apple ACK guarantee.

The7-file commit adds8 tests,400 lines and removes83. Exactly one prior assertion
migrates: successful WebDriver stop observes forwarding teardown0→1. Other
assertion functions, active methods, constructor phases, HTTP route bodies,
CDP and controller logic remain. The private
cfg(test) peer gains WebInspector visibility, command ID counters and default-off
stop delay/stall controls. Its old behavior and assertions remain. The private
state builder accepts the existing InspectorSource trait for owned fixtures.
Core/public/binding APIs, dependencies, wire fields and access policy stay.

| Eight actual constructor/owned Automation plist fixtures | Observed result |
| --- | --- |
| Normal stop | Context close then exactly one forwarding close with existing identity; IDs1/2 and source call1 |
| Forwarding flush IO | Original PermissionDenied std::io::Error returned |
| Context business plus socket IO | Both errors owned; primary Protocol and native cleanup IO survive |
| Pending forwarding flush | Real100ms expiry returns native Elapsed |
| Context80ms plus flush30ms | Each stage is shorter than100ms, combined cleanup expires under one total |
| Pending context close | Real expiry invalidates client; no forwarding/retry before EOF |
| Repeated normal/business-failed stop | No duplicate context/socket request; original IDs1/2 stay |
| Caller cancellation after close frame | Actual stop future drops, client invalidates, repeated stop emits nothing, explicit runtime Drop yields stream Drop/peer EOF |

All fixtures join owned peer futures and observe stream Drop. They use private
sources, no forged ConnectedDevice or new public backend. They do not prove
physical-device cleanup, whole registry/lease joins or external-browser behavior.

First86097 exited101:14 test visibility compile errors, no tests ran. Expanding
cfg(test) visibility fixed those errors; pre-product23646 exited101:0PASS/5FAIL,
showing missing forwarding close, discarded native cleanup and missing dual
causes. After-owner3719 exited0:5PASS. Expanded12440 exited0:8PASS. No post-product
compile/test failure. A JS audit launcher typo was corrected before writing its
ignored script; source stayed unchanged.

Frozen pipeline96261 exited0, all18 gates pass. Source remained frozen; builds
ended before source edits or new validation. Observation timeout never counted
as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_webdriver_cleanup all features/minimum CLI | 8/8 PASS |
| Prior direct cleanup/setup/page/finite/shell | 5/15/5/21/31 PASS |
| CLI/core WebInspector | 132/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets | 17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt | 5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2449 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI844/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and the
separate browser gate stay ignored.22device NOT_RUN; Chromium and installed
Python3.9/3.13 not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`f821c0573294c3de72b9b4f545c567f63bd54ac7a62c2ced63c068115e398a57`.
Actual debug ios binary SHA256:
`10c40bafe12f2f1ec9eb11d80368710f22041a813d7a0f1f55458aea5efc6cae`.
Actual commit matches the frozen/tested patch. Logs/copied development evidence
with original hashes/audits live under ignored
`target/validation/webdriver-cleanup-2026-10-05/`; reference HEAD/status unchanged.

HTTP DELETE still discards its cleanup result, and outer100ms revocation handling
can preempt the runtime helper. Mutex wait admission, remaining budget across
that wait, active Arc races, registry revocation/join, constructor primary cleanup,
principal/server cancellation and active request/output totals remain. Existing
DELETE grammar/shape stay; this cleanup/timing correction needs pre-1.0 release
review and actual device evidence. No Apple ACK/rollback/retry/platform/benchmark/
RSS/decoder peak or async Drop acceptance.

Continue remaining L01, L02–L04/Q04 and classes5–9. All31 tasks/subtasks/history,
118 capability bodies/maturity/prior evidence and counts9 complete/11 not started/
11 in progress or pending conditions remain. No subagent, push, publication,
history rewrite, reference edit or device/Apple operation occurred. This
intermediate commit does not end the engineering goal.

# L01 WebDriver typed revocation cleanup

Tested implementation: `bf2da2dcef7106a16b19f0bd355e80b0e998b642`, based on
`f306e1228edfde78a0c9180f4be10aa580781101`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01/class4/all31 tasks remain active.

Actual [WebDriver registration](../../crates/ios-cli/src/cmd/webinspector/webdriver.rs)
now supplies the existing typed runtime helper to
[insert_with_budget](../../crates/ios-cli/src/cmd/webinspector/sessions.rs).
After entry retirement, the actor creates one checked100ms cleanup Budget.
Original runtime mutex, Automation context and forwarding close share that
absolute deadline. No competing outer timer can discard an already observed
business failure while forwarding flush is pending. Native
[JoinHandle<Result<()>>](../../crates/ios-cli/src/cmd/webinspector/sessions/task_owner.rs)
retains the callback result through normal task exit; the owner transfers it
on join/reap without a further await. Normal native exit can contain cleanup
failure. Unexpected native join flags/causes stay; issued abort cancellation
remains neutral. Original IO/Elapsed and business primary/secondary remain owned.

Previously the unit adapter logged errors, and outer100ms could preempt the
combined cleanup result. Safe tracing now receives the actual callback failure
with label automation session closed with error. No new HTTP/CLI error schema,
route/status/success payload, authorization or public Rust/C/Python change.
Timing/diagnostic correction needs pre-1.0 release review and device evidence.
The total starts after map retirement, excluding preceding map wait.

Six source files add380/remove20 lines and eight tests. Obsolete unit adapter
is removed; cfg(test) legacy unit insert drives its host callback with Budget.
Runtime.stop and the private duration cleanup wrapper gain cfg(test), with
bodies intact. Actual DELETE/stop_until/typed mutex helper, shared protocol
cleanup, constructors/active methods, retirement mechanics, original admission
and all previous assertion functions stay. Only the old Tokio helper adds Ok
for the typed native handle. Core/access/public/dependencies/other owners stay.
No new task/backend/protocol selector or real device/credential operation.

| [Eight actual product creation fixtures](../../crates/ios-cli/src/cmd/webinspector/webdriver/revocation_tests.rs) | Actual result |
| --- | --- |
| Actual HTTP policy reload |Original context and matching forwarding close, native Result Ok |
| Actual3s grant expiry |Same close and release without a reload request |
| Native forwarding flush IO |Original PermissionDenied remains downcastable |
| Pending forwarding flush |Real default100ms expiry retains original Elapsed |
| Business close plus flush IO |Original Protocol primary and native PermissionDenied secondary |
| Business close plus pending flush |Protocol primary survives100ms; original Elapsed secondary |
| Mutex held for total |Native Elapsed, no cleanup requests, cleanup_armed still true; explicit fixture Arc release |
| Pending context close |Native Elapsed, invalid original client skips forwarding recovery |

All use actual protected selenium_router POST and owned Automation peer/source;
seven use actual /auth/reload and one actual synthetic grant expiry. Private
observe snapshots the real actor owner and consumes its native Result; it
does not replace cleanup or certify production logging capture/server joins.
Safe error_message contains no synthetic DEVICE/TOKEN/session identifier.
Source call count1, original create/close IDs1/2, matching forwarding counts,
stream Drop, peer trace and fixture listener joins are observed. Fixture Arc
release is not cancellation of all in-flight requests. Local close/flush is not
Apple ACK; no actual browser/device/platform acceptance.

First82798 exited0 with8PASS. First frozen94556 exited101:19 preceding gates
passed, strict dead_code failed on the private duration cleanup wrapper now
used only by cfg(test) runtime.stop. Full had NOT_STARTED. After authoritative
exit/no live builds, the sole source correction added cfg(test) to that wrapper.
Archived five source hashes plus the unchanged shared-owner baseline and exact
correction are audited. No assertion/protocol body changed for this correction.

Corrected frozen98957 exited0, all25 gates pass. Source stayed frozen through
authoritative completion; observation timeout never counted as exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_webdriver_revocation full/minimum CLI |8/8 PASS |
| Prior store/DELETE/original scope/browser/page lease/runtime/direct/setup/page/finite/shell |13/9/2/23/7/8/5/15/5/21/31 PASS |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets |17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt |5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI |22/95 PASS |
| Section11 full Rust/Python split |2509 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI904/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and separate
browser gate stay ignored.22device NOT_RUN; Chromium/installedPython3.9/3.13
not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`8466c0b69a65828974ee3b14a72c7ce513a410a6b2b95a6aee27a609d1d43592`.
Actual debug ios binary SHA256:
`9a8b97ff270f422e9585129b4b6432812be812f0db976282d0ac4bcfdcc6e513`.
Actual commit matches frozen source. Durable first run/corrected gates/archived
failed attempt/source bytes/native hashes/audits live under ignored
`target/validation/webdriver-revocation-2026-10-05/`; reference HEAD/status unchanged.

Typed callback/returned-result only. Map wait before callback budget, known
primary before callback return if store aborts, active request Arcs, idle
completed handles, caller/server Drop, constructor primary cleanup, active
request/output totals, terminal retention and resource caps remain. Other
L01/L02–L04/Q04/classes5–9 continue. No Apple ACK/rollback/retry/async Drop/server
join/platform/benchmark/RSS/decoder peak acceptance. All31 tasks/subtasks/history,
118 capability bodies/maturity/prior evidence and9 complete/11 not started/11
in progress or conditions remain. No subagent,push,publication,history rewrite,
reference edit or real device/Apple operation. This intermediate commit does
not end the engineering goal.

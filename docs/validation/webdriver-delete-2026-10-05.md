# L01 WebDriver DELETE total and cleanup errors

Tested implementation: `731bdb0ac96d30013a17c083fd7c9f19c8a85a81`, based on
`e188f1f8bbf06b7a4c266ef05c75c580258c76f9`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01/class4/all31 tasks remain active.

The [DELETE handler](../../crates/ios-cli/src/cmd/webinspector/webdriver.rs)
now starts one checked100ms Budget before session removal. Map admission,
runtime mutex, context close and forwarding socket close consume the same
absolute deadline. Existing authorization middleware precedes this total;
response delivery is outside it. Removal happens before protocol I/O. The
original Automation session/IDs remain; cleanup disarms once at start and skips
an unusable client. The [shared cleanup body](../../crates/ios-cli/src/cmd/webinspector/tasks/session_owner.rs)
changes only private visibility. No competing outer cleanup timer can discard
an already observed business error while forwarding cleanup is pending.

Previously the outer100ms timeout was ignored and the unit callback only
logged cleanup errors, so DELETE still returned200/null. Failures now use
existing AppError500, safe unknown-error body and schema1 machine headers
with WebInspector phase/unknown effects. Original PermissionDenied flush IO
and issued Elapsed survive privately. Business-close Protocol remains primary
if forwarding close also fails/expires; both causes remain owned until HTTP
projection. Success/repeated deletion keep200/null and no error headers.
This pre-1.0 error/timing migration needs release review and device evidence.

Four source files add440/remove8 lines and nine tests. Accepted runtime.stop,
unit revocation callback, all active methods/constructor and original assertion
bodies stay. The [eight prior cleanup tests](../../crates/ios-cli/src/cmd/webinspector/webdriver/cleanup_tests.rs)
change only cfg fixture visibility. Core, public Rust/C/Python surfaces, access,
registry, dependencies, other owners, routes and successful payloads stay.
No new protocol selector, source backend or detached source worker.

| [Six actual admitted POST/DELETE fixtures](../../crates/ios-cli/src/cmd/webinspector/webdriver/delete_tests.rs) | Actual result |
| --- | --- |
| Normal/repeated DELETE |200/null, removal, no error headers; original create/close IDs1/2 and one matching forwarding close |
| Native forwarding flush IO |500/PermissionDenied machine class, safe body, original matching close |
| Pending forwarding flush |Real default100ms expiry;500/Timeout and release |
| Business error plus pending flush |500/Protocol primary; known business error survives actual expiry |
| Mutex held for whole total |500/Timeout, cleanup_armed remains true, zero close/forwarding requests |
| Mutex held85ms after observed removal |Remaining total cannot finish actual30ms forwarding flush;500/Timeout and matching close |

The mutex-whole-total fixture observes cleanup_armed=true: cleanup never
acquired the original runtime, so its once-only flag was not disarmed. Its
explicit fixture Arc release is not cancellation of all in-flight requests.
The protected actual router uses synthetic policy/DEVICE and an adapter that
verifies that DEVICE before delegating the owned-source alias. Every failed
case removes addressability and repeat DELETE succeeds. Source call count1,
stream Drop, peer trace and joined test listener/peer are observed. These
fixture joins do not certify product registry/server supervision. Local close
and flush are not an Apple acknowledgement.

Three additional private owned-result fixtures construct the real runtime and
call the actual Result helper. They retain native business+PermissionDenied
IO, business+Elapsed and mutex Elapsed, including original downcastable causes.
They make no HTTP/admission claim; the six routed cases cover that boundary.

First compile12436 exited101 with four E0599 Response::json fixture errors,
no tests. Current reqwest features stay: response bytes plus serde_json fix the
fixture. Corrected before-product log records1PASS/5FAIL, all failures because
DELETE returned200 instead of500. Its numeric tool handle/exit code was lost
across compaction; final failure log and absence of live build processes were
verified before source edits. No exact lost exit code is invented. Correction
45035 exited0 with6PASS; expanded40794 exited0 with9PASS.

Frozen pipeline91290 exited0, all22 gates pass. Source stayed frozen through
authoritative completion; observation timeout never counted as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_webdriver_delete all features/minimum CLI |9/9 PASS |
| Browser/page lease/WebDriver/direct cleanup/setup/page/finite/shell |23/7/8/5/15/5/21/31 PASS |
| CLI WebInspector/access/core WebInspector |171/29/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets |17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt |5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI |22/95 PASS |
| Section11 full Rust/Python split |2488 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI883/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and separate
browser gate stay ignored.22device NOT_RUN; Chromium/installedPython3.9/3.13
not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`c0d345de35ad32ba9d86bb8f62504f4f76e50fbaf2c12143b72da480a61cd54a`.
Actual debug ios binary SHA256:
`b0d4a5a156df6c9751d3ad3c8db4a06a97992ef9b4a95d3c6a7627423f89c3fb`.
Actual commit matches frozen source. Durable copied history/logs/native hashes
and audits live under ignored `target/validation/webdriver-delete-2026-10-05/`;
reference HEAD/status unchanged.

DELETE handler only. Stored task joins/revocation callback, active request Arcs,
generic caller/server Drop, constructor primary cleanup, active request/output
totals, terminal event retention, other L01/L02–L04/Q04 and classes5–9 continue.
No Apple ACK/rollback/retry/async Drop/server join/platform/benchmark/RSS/decoder
peak acceptance. All31 tasks/subtasks/history,118 capability bodies/maturity/
prior evidence and9 complete/11 not started/11 in progress or conditions remain.
No subagent,push,publication,history rewrite,reference edit or real device/Apple
operation. This intermediate commit does not end the engineering goal.

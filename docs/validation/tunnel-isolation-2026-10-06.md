# L02 tunnel manager device isolation

Tested source: `e0f52671396e6408fc38921f78f73a256a45ce65`, parent
`4afde1f5288e2ebec22577f0c07f62d9a6edfbe2`. Source frozen and validation started
2026-10-05; persisted results recovered and audited 2026-10-06. Linux x86_64,
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The actual [CLI manager](../../crates/ios-cli/src/cmd/tunnel.rs) previously held
one lifecycle mutex across network discovery and connections, blocking local
list/get/delete and every other device's start. Local queries and stops now
take the short tunnel table lock. A [private coordinator](../../crates/ios-cli/src/cmd/tunnel/lifecycle.rs)
serializes connections for one device and shares four connection permits across
manual starts and scans. The weak device gate table has a4096 entry cap and
reclaims expired entries at capacity; this is not a measured RSS bound.

Stop invalidates that device's active and queued generations under the same
write lock used to publish a tunnel. An old successful connection cannot publish
after removal. Stop keeps the existing200/404 response distinction: an absent
record returns404 even when a pending start was cancelled. Pending start returns
the original typed OperationCancelled via existing502/safe machine headers.
A subsequent explicit start is allowed. Global clear invalidates existing tickets
and drops records. This does not certify /shutdown supervision or native async joins.

Manual creation has one15s total after ticket acquisition: per-device queue,
permit wait, original source connect, optional existing-trust mobdev2 discovery
and fallback, validation and publication. Previously queue wait was unbounded
and separate connections could each spend15s. Real expiry preserves native
Tokio Elapsed. Explicit routes and configured native ConnectOptions remain;
an unmatched fallback now retains the original usbmux IO instead of formatting
it into a new message. These deadline/category changes need pre-1.0 release review.

The [private source](../../crates/ios-cli/src/cmd/tunnel/source.rs) invokes the
same four native Core connection functions and existing discovery. No SDK fake
constructor, public API, dependency, pairing or Apple wire change. A private
cfg(test) device owns a record, live flag and Drop counter. The manager boxes
native devices; this allocation is not a performance result.

Scans serialize separately and share the same device gates/permits. Candidate
generations are captured before waiting for the first connection. Stops fence
old discovery snapshots, including devices without a record or gate at scan
entry. A stale list failure removes only matching original publications;
records created while it waited survive. USB pruning keeps new publications
for the next scan. Connect-error versus list-error behavior stays distinct.
Backoff remains30/60/120/240/300s. Automatic candidate connects are still
sequential; scan discovery/total deadlines and in-flight detach/replug identity
need further work. Stops do not permanently exclude a device from later scans.

| [Fifteen actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests.rs) | Evidence |
| --- | --- |
| Protected HTTP start A held for native15s; local B list/get/delete | Combined B operations finish under1s while A remains active; B owned device drops, A stop404 cancels its start502 |
| Protected HTTP native failure | PermissionDenied machine header; safe body excludes synthetic private inputs |
| Same-device competing starts | One source call, one record, identical results, matching Drop |
| Active and queued generations stopped | Both native cancellation markers retained; no late record; new explicit generation succeeds |
| Eight owned request futures | Four source futures active/peak; clear cancels all and restores every permit |
| Held per-device queue | Original80ms test total expires before source poll; native Elapsed; next request works |
| Explicit routes and default fallback | Same usbmux/USB/Wi-Fi options and matching existing-trust mobdev2 host |
| Native failure and unmatched fallback | Original IO kinds downcastable; safe message hides synthetic inputs |
| Scan and manual start for same device | Single connection and ready state, matching owned release |
| Stop candidate behind stalled scan | Candidate never connects or publishes and acquires no failure backoff |
| Stop during discovery | Original unpublished snapshot rejected without source connect |
| Backoff then USB detach | Second scan skips; elapsed backoff retries once; detach releases record |
| Failed snapshot versus new publication | Old record removed, concurrently created record retained; original BrokenPipe |
| Dead record | Query removes it; repeated list/stop leave no record; one Drop |
| Caller drops before publication lock | Completed owned fake device released, permit/serial gate reusable |

These exercise actual TunnelAgentState, two protected product HTTP router cases,
and owned fake devices. Original tunnel assertions, native record grammar,
HTTP/auth/federation/bridge bodies, core/public/access/dependencies stay.
No Core lock microtest substitutes for CLI isolation; no physical tunnel, Apple
ACK, native worker join or socket/RSS proof follows from fake-device Drop.

First7782 exited101: private MuxDevice imports caused two visibility errors and
two inference errors; no tests ran. The private boundary was corrected to UDID
lists without exposing SDK types.21164 then exited0 with13PASS. Two more tests
and stronger old/new-publication assertions were included before final freeze.

Parent process9635 was live during the preceding turn. On recovery its handle
was missing and no validator/build remained. Its tool exit code is unknown.
Persisted evidence records all14 subprocess exits as0, including full; the final
binary hash was written after frozen-source assertions. A separate recovered
source/result audit exited0. This is not an invented parent terminal result.

| Persisted accepted gate | Actual result |
| --- | --- |
| New tests/full and minimum CLI |15/15 PASS |
| Tunnel regression |56 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Actual built CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2524 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI919/Core1443/public52 pass. Workspace fmt/check/strict Clippy,66 feature
checks/four runtime groups,MSRV1.80.1,7+7 doctests/three downstream snippets,
C ABI and Python3.12 wheel/sdist build/install/import pass. Tooling67/help118/
raw38/host5/CLI95/backup3 pass.22device NOT_RUN; Chromium and installed3.9/3.13
not rerun. No maturity upgrade. Four files add1133/remove303 lines.

Patch SHA256: `97fda9f0acc5ca57d037a3362227e141d42792ed0d5ccec17da595c37d51bc99`.
Actual debug ios SHA256: `871daaa54afaf6275a14bf490d1cef7d58783bf63db0e653317496645e444917`.
Commit matches frozen bytes. Durable logs, commands, fourteen native subprocess
statuses, hashes and audits are ignored under
`target/validation/tunnel-isolation-2026-10-05/`. Reference HEAD/status unchanged.

Required native connect/RSD/H2 and S01 host interfaces were already available;
L01's remaining owners are not declared complete. Continue bounded parallel
automatic scans, in-flight unplug/replug identity, scan totals and resource
checks; then L03/L04/Q04 and other L01/classes5–9. All31 main tasks/subtasks/
history and118 capability bodies/maturity remain. Current totals are9 complete,
10 not started,12 in progress or conditions. No subagent,push,publication,
history rewrite,reference edit,device/credential/Apple service operation.

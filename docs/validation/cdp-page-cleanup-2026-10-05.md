# L01 direct CDP page cleanup

Tested implementation: `221a0c4fc6ed90e864694a3ec7d634808329bc52`, based on
`f10d0152204d4bb12b42ee38efb402410b48dc13`, Linux x86_64, 2026-10-05.
Rust 1.97.1/MSRV 1.80.1, Python 3.12.13. L01, class4 and all31 tasks remain active.

The [direct page handler](../../crates/ios-cli/src/cmd/webinspector/cdp.rs) now
uses the existing [session owner](../../crates/ios-cli/src/cmd/webinspector/tasks/session_owner.rs)
and primary/cleanup result wrapper after its bridge loop returns. Device guard,
client and page lock stay owned through a separate 2s cleanup total. Previously
cleanup inherited the active page timeout and discarded its result. A normal
WebSocket close can now return cleanup failure; when both phases fail, both
causes remain owned and the original primary keeps its classification/source.
This error/timing correction needs pre-1.0 release review.

Only fully attached usable clients issue the existing forwarding close. Invalid
clients drop without guessed recovery. No protocol field, public API, successful
page message, CLI grammar, access policy or dependency changes. The shared
controller/cleanup logic only expands private visibility within WebInspector.
Original active page/browser/setup behavior and assertion functions remain;
the shared cfg(test) source adds native flush controls/counters with normal
defaults. The six-file commit adds five tests, 213 lines and removes19.

| Five actual page WebSocket/owned plist tests | Observed result |
| --- | --- |
| Normal close, native flush failure | Returned PermissionDenied with original std::io::Error; actual close frame followed by stream Drop/peer EOF |
| JSON primary plus native flush failure | Private wrapper owns serde_json primary and native IO cleanup; original Protocol classification survives |
| Normal close, stalled flush | Real default2s expiry returns native Elapsed; does not inherit50ms active idle policy |
| JSON primary plus stalled flush | Original JSON error and native cleanup Elapsed both survive |
| Delayed successful flush | Real80ms flush completes under independent2s, despite50ms active idle policy; completion counter increases once |

Every fixture observes one owned source call, the existing close selector and
no extra requests before EOF. This is local write/flush evidence, not an Apple
ACK. The private route observes the actual handler result; it adds no product
route, public source selector, forged device or new admission proof.

Before-product88897 exited101:0 PASS/5 FAIL. Normal close wrongly returned Ok,
dual failures lost cleanup, and the delayed flush never completed. Corrected
15775 exited0:5 PASS. No compile/test failure after the product change. Test
timing starts before close admission to avoid an observation race. A document
patch context mismatch applied nothing and was corrected without source edits.

Frozen pipeline81951 exited0, all17 gates pass. Source stayed frozen; every
build terminated before edits or another validation. Observation timeout never
counted as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_cdp_cleanup all features/minimum CLI | 5/5 PASS |
| Prior setup/page/finite/shell | 15/5/21/31 PASS |
| CLI/core WebInspector | 124/102 PASS; core96+public5+integration1 |
| Core-only webinspector cache minimum | 17 PASS; CLI still enables core full |
| Minimum CLI all-targets/original CLI stack | PASS/5 PASS |
| Strict core/CLI Clippy/workspace fmt | PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2441 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI836/Core1443/public52 pass. Workspace fmt/check/strict Clippy, MSRV1.80.1,
66 feature checks/four runtime groups,7+7 doctests/three downstream snippets,
C ABI and Python3.12 wheel/sdist build/install/import pass. Tooling67, actual
help118, raw38, five host workflows,95 CLI cases and3 backup contracts pass.
PBKDF2 and the separate browser gate stay ignored.22device NOT_RUN; Chromium
and installed Python3.9/3.13 not rerun. No maturity upgrade.

Frozen source patch SHA256:
`fcc622d1602b58f3fb7d5acfb4a9869a526c3366cf5857cb43a3c073b8780499`.
Actual debug ios binary SHA256:
`794a218696162073a0779d466d408d1fac534df1eb6431b6446a56f418862b03`.
Actual commit matches the frozen/tested patch. Logs, copied development evidence
with original hashes and audits live under ignored
`target/validation/cdp-page-cleanup-2026-10-05/`. Reference HEAD/status unchanged;
no cache cleanup needed.

This path runs only after a returned bridge result. Outer principal/server
cancellation still drops the handler; async cleanup after that cancellation is
not proved. Constructor cleanup, active request/output totals, HTTP publication,
server signals and joined browser/stored-session ownership remain. Existing
stored100ms/browser2s policies stay. No device/browser/platform/rollback/retry,
performance/RSS/decoder peak or async Drop acceptance.

Continue remaining L01, L02–L04/Q04 and classes5–9. All31 tasks/subtasks/history,
118 capability bodies/maturity/prior evidence and counts9 complete/11 not started/
11 in progress or pending conditions remain. No subagent, push, publication,
history rewrite, reference edit or device/Apple operation occurred. This
intermediate commit does not end the engineering goal.

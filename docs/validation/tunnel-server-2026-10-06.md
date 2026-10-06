# L01/L02 tunnel server shutdown ownership

Tested source: `5120fa90e06a92f4734603382b98779b69f1ca38`, parent
`b052752bed4c1b1d26d3fa2f70a27aabc7218b33`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. All31 tasks/class4 remain active.

The [server owner](../../crates/ios-cli/src/cmd/tunnel/server.rs) retains shutdown
for late waiters and owns a boxed scanner/candidate future instead of a detached
monitor. After existing startup, one pinned Ctrl-C future, HTTP shutdown or server
completion stops that scanner. Returned cleanup shares an independent2s budget
across existing table clear and graceful HTTP drain. Normal native primary or
cleanup Elapsed remains; dual failure stores both and exposes the primary source.
BrokenPipe still classifies as Disconnected. Wrapper Debug and CLI diagnostic
output stay safe; raw Anyhow Debug follows its deliberately retained native chain.

Caller Drop signals shutdown/epochs and clears unlocked tables without spawning
cleanup. It cannot await locked tables. Axum0.7.9 source was inspected read-only:
HTTP/signal delegate tasks remain separate; timeout/drop does not forcibly abort
or join pending handlers/upgrades. Startup/configuration totals, later admission/
check races, Core workers and generic caller-drop primary reporting remain.
Exit/timing corrections need pre-1.0 release review.

Five files add498/remove34 lines. Eight
[actual fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/server_tests.rs)
cover:

- Late shutdown waiter observing the retained signal.
- Preclosed server returning without source polling.
- Actual HTTP POST shutdown releasing pending scan A, owned B, permits/listener;
  one fixed signal future enters once.
- Caller future drop owning/releasing its scanner and unlocked device table.
- Typed signal PermissionDenied surviving successful cleanup.
- Locked-table30ms cleanup expiry preserving native BrokenPipe and Elapsed.
- Pending HTTP handler30ms drain expiry/listener close; fixture then releases
  the delegated handler and observes response/EOF.
- Real Linux SIGINT sent only to a created private fake-source child after its
  actual Tokio signal poll/source entry, followed by cleanup and runtime exit.

The held-table fixture explicitly releases its lock and performs the remaining
clear. Those fixture actions are not successful native cleanup by the server.
The SIGINT case certifies Linux signal/parent behavior with a fake source, not
device workers, actual CLI startup or other platforms. Original64 assertions,
native source/controller/retention/Core/HTTP/auth/trust/dependencies and startup
through listener/router/info stay. Only an old method fixture adapts its private
waiter; its assertions stay. No SDK fake constructor/protocol/pairing is added.

Control18746 exited101 with0PASS/1FAIL.34085 exited101 with E0599/no tests:
response parsing moved to existing bytes/serde.55083 exited101 with71PASS/1FAIL
because a new assertion expected Io for BrokenPipe; the existing category is
Disconnected. Frozen71101 exited101 with71PASS/1FAIL because a new assertion
treated raw Anyhow Debug as the diagnostic boundary. It now checks safe wrapper
Debug/Display and actual CLI error_message. The failed freeze's source hashes and
logs remain. Final86684 exited0/all14 gates/hash assertions; source audit exited0.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |72/72 PASS |
| Tunnel regression |113 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2581 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI976/Core1443/public52 PASS. Workspace fmt/check/strict Clippy, MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI/Python3.12
wheel-sdist build/install/import PASS. Tooling67/help118/raw38/host5/CLI95/backup3
PASS.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN; no maturity upgrade.

Patch SHA256: `d067f8f9ae5cf0a92b48926042d4281683ae0ca2989077e01c0e1ea7f40c2fc5`.
Actual debug ios SHA256: `3823aedc1950ad79587f8701ed6ceede9f1b9a1320688d6d5f79e0f69e79980c`.
Commit matches frozen source; ignored logs/commands/statuses/hashes/audits are in
`target/validation/tunnel-server-2026-10-06/`. Reference HEAD/status unchanged.

Continue startup/output/admission/whole-scan/metadata bounds, locked/delegated
server/Core worker joins, after-check races, other L01/L02, L03–L04/Q04 and
classes5–9. No physical ACK/native join/RSS/performance claim. All31 task/subtask/
history/118 bodies and9 complete/10 not started/12 in progress or conditions stay.
No subagent/push/publication/history rewrite/reference edit/device/Apple operation.

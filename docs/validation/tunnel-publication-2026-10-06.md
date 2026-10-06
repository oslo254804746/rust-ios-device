# L02 attachment query before publication

Tested source: `86fecfe3331e9efa74dc76fbc48b47ad5c3df8f1`, parent
`12e6b17757a99f66ff506835712560a2c88ffbe4`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

After initial expected-ID and record validation, the
[actual manager](../../crates/ios-cli/src/cmd/tunnel.rs) checks the completed
usbmux/direct-USB device against a new existing Source.usb_devices result.
Current(UDID,ID) membership permits publication; absence releases the owned
device with local OperationCancelled. Connect/list errors retain original
Anyhow/native IO with static safe context. The completed device stays owned
through the query. It shares the original15s budget, serial guard, permit and
cancellation; no global table lock covers the network wait. Wi-Fi/mobdev2 skip
this query, and initial expected-ID mismatch still returns before it.

This adds one daemon connection/list roundtrip for each successful USB creation,
including manual creation. It may increase startup time; pre-1.0 release review
and device/platform evidence remain. The existing native source/API/protocol
codec stay. Equality at the reply is only a point-in-time observation. Changes
after the query, reused IDs/daemon restart, native liveness before publication,
event-driven generation tracking and native worker/server joins are not solved.

Production changes are one17-line block; all other production bodies remain
byte-identical. Three files add198/remove3 lines and six tests. The private fake
registers successful legacy USB results in its default attachment list; explicit
metadata remains independently controlled. Query count is observed, and errors
are captured before the query gate as per-request outcomes. One old failed-scan
fixture now starts A before the failure and permits a new healthy query for B
while the old query keeps its captured failure. Its original assertions and all
other30 previous assertion bodies stay; no production skip-check hook exists.

| [Six actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/publication_tests.rs) | Result |
| --- | --- |
| Manual71 detaches while connection is pending | Returned device releases; no record; local cancellation |
| Initial71/completed71, current snapshot72 | Old matching result rejected; release/no failure backoff |
| Query never completes | Original80ms total retains native Elapsed; owned device/permit release |
| Stop while query holds a completed device | Fast absent-record result, native cancellation, no late publication |
| Query BrokenPipe after successful connect | Original IO downcast retained; safe message; matching release |
| Another attached transport still has returned71 | Accepted with one query; normal owned release |

These use actual TunnelAgentState and private owned fake devices/source futures.
No physical daemon/device or Core worker join is certified. Original native
source/lifecycle coordinator/core/public/access/dependencies, HTTP/auth/record/
federation/bridge and prior identity/parallel fixtures stay.

Control26945 exited101:0PASS/6FAIL. Post14072 exited0 with36PASS. Frozen20077
exited0 with all14 gates/frozen hashes; source/result audit exited0. Source was
unchanged during validation; observation timeouts never counted as exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |36/36 PASS |
| Tunnel regression |77 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2545 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI940/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI/Python3.12
wheel-sdist build/install/import pass. Tooling67/help118/raw38/host5/CLI95/backup3
pass.22device NOT_RUN; Chromium/installed3.9/3.13 not rerun. No maturity upgrade.

Patch SHA256: `ed585c77da2af2dbcd7dcd9e6cef4d0370eabfcb0c65552e4d46bfa6f83432fc`.
Actual debug ios SHA256: `0bf5bce817434a2c0c9af4679f2e68f246166d1b05636c07d80fa33c0176eaee`.
Commit matches frozen source. Durable logs,commands,statuses,hashes/audits live
under ignored `target/validation/tunnel-publication-2026-10-06/`; reference
HEAD/status unchanged.

Continue generation/route-specific failure backoff (currently keyed only by
UDID), after-query changes/reuse/restart, manual epochs, native liveness/joins,
retained metadata/resource bounds, discovery/whole-scan totals and L04 evidence.
The extra query is not a performance/RSS bound or remote ACK. Other L01/L02/
L03–L04/Q04/classes5–9 continue. All31 main tasks/subtasks/history and9 complete/
10 not started/12 in progress or conditions remain. No subagent,push,publication,
history rewrite,reference edit or device/Apple service operation. Goal remains active.

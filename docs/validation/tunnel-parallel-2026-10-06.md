# L02 bounded parallel tunnel scans

Tested implementation: `66bf7311020763b8a29b175a117cdf1b70d4e22f`, parent
`b96ede822e52611d2be97be624baacb2317af608`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The [actual CLI scanner](../../crates/ios-cli/src/cmd/tunnel.rs) now merges
usbmux and existing-trust mobdev2 candidates into one window of at most four
connection futures. The scanner owns those futures directly with
buffer_unordered; it creates no detached connection task. Existing per-device
serial gates, generation cancellation, shared four permits, native connections,
15s candidate budget, backoff and safe error reporting remain. Parent cancellation
drops every active candidate, restoring its owned serial guard and permit.

Candidates are deduplicated across both families before entering the window.
The first usbmux candidate wins over a matching mobdev2 target; otherwise the
first paired host wins, preserving the previous effective route choice. Repeated
A entries cannot fill the window with waits on the same device and block B.
Stopping A during an active scan leaves B's generation running.

Existing-trust paired discovery now precedes USB connection attempts so a stalled
USB batch cannot prevent a known mobdev2 device from connecting. That discovery
wait can delay USB startup. No total discovery/whole-scan deadline is claimed.
If discovery fails, its original error remains held while USB candidates are
processed, then returns; successful USB records remain published and ready is
not newly set. Caller cancellation while that error is held still drops the
parent; stronger primary retention during arbitrary parent Drop remains outside
this profile. This scheduling change requires pre-1.0 release review.

Three source files add319/remove17 lines and eight tests. Private NativeSource,
Lifecycle and refresh_connect bodies remain byte-identical. Manual creation,
table publication/removal, original HTTP/auth/federation/bridge/record grammar,
core/public/access/dependencies remain. The private fake adds paired_failure.
One previous queued-stop fixture now fills four slots instead of one: A/C/D/E
stall and B is queued. Its original assertions are preserved; the other previous
assertion functions are unchanged.

| [Eight actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/parallel_tests.rs) | Observed result |
| --- | --- |
| Stalled A, healthy USB B | B publishes under1s while A remains pending |
| Four duplicate A candidates before B | B publishes under1s; A connects once |
| Sixteen distinct candidates | Four active/peak source futures; remaining twelve not polled before cancellation |
| Caller drops the scan directly | All four active futures drop; every permit restored; next scan succeeds |
| Stop A with B still connecting | A cancelled without failure backoff; B publishes |
| Stalled USB A, paired mobdev2 B | B publishes under1s using its original host route |
| Paired discovery PermissionDenied | Original IO downcast retained; USB result survives; ready remains false |
| Duplicates across families/paired hosts | One source call per device; original first USB/host route; matching releases |

These are actual TunnelAgentState fixtures with private owned fake devices and
source futures. Fixture scan tasks are explicitly aborted/joined on negative
observations. Fake future/device Drop proves local ownership, not Core worker
join, physical tunnel/device ACK, live sockets, RSS or L04 performance.

Baseline control53715 exited101 with0PASS/5FAIL. USB parallel post25737 exited0
with20PASS. Additional cross-family control88793 exited101 with0PASS/1FAIL;
the mobdev2 candidate still waited for the USB batch. After the combined window
and two route/error regression fixtures,99805 exited0 with23PASS. Historical
logs and process statuses are retained; no historical source hash is invented.

Frozen87432 exited0 with all14 gates. Source stayed frozen through authoritative
completion, and source/result/compatibility audit exited0. No observation timeout
was treated as completion.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |23/23 PASS |
| Tunnel regression |64 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2532 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI927/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI and Python3.12
wheel/sdist build/install/import pass. Tooling67/help118/raw38/host5/CLI95/
backup3 pass.22device NOT_RUN; Chromium/installed3.9/3.13 not rerun. Capability
bodies/maturity and prior evidence stay.

Patch SHA256: `1cb613e6083c8c68f134628c3fd9cbc11194381732cfa7c0d108fdf26a95da39`.
Actual debug ios SHA256: `188538ef87689ab47b29392c56f757e5905890c25de576ed4a0f28e58d40590c`.
Commit matches frozen source. Durable control/current logs, commands, native
statuses, hashes and audits live under ignored
`target/validation/tunnel-parallel-2026-10-06/`; reference HEAD/status unchanged.

Remaining: in-flight detach/replug identity and stale attachment snapshots,
discovery/whole-scan totals, retained metadata/resource/native worker/server
joins, platform/device acceptance and L04 measurement. The four permits do not
bound every Core-owned task or process allocation. L01/L02/L03–L04/Q04/classes5–9
continue. All31 tasks/subtasks/history and9 complete/10 not started/12 in progress
or conditions remain. No subagent,push,publication,history rewrite,reference edit
or device/Apple service operation. The intermediate gate does not end the goal.

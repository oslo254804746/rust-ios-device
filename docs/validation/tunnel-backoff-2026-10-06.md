# L02 backoff for known connection identity

Tested source: `d0b04bed79ecec5c3f724ff003122f66b1159216`, parent
`cef374bdbc583004e53b0a40193ca69886ea52eb`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The [actual CLI manager](../../crates/ios-cli/src/cmd/tunnel.rs) previously
keyed failure backoff only by UDID, suppressing a newly observed device ID or
known paired host after another route failed. Failure records now retain a
private identity comprising the known route and optional expected native ID.
Backoff skips only the same identity. Repeated failures increment only that
identity's counter; a changed identity's failure starts at count1. Success
removes the record as before. The existing30/60/120/240/300s schedule and time
calculation stay. A known ID, host or route change can attempt immediately.

The scanner still uses existing native connection/discovery/trust functions.
No new pairing, configurable retry budget/state stream, actor, dependency,
protocol frame or repeat of side-effect operations is introduced. L03 remains
pending. Same ID/host after an unobserved change or daemon restart is not new
identity evidence. Retained failure entries remain unbounded across UDIDs;
the device gate limit does not prove a failure-map/RSS bound.

Custom identity Debug prints a static label and never its retained host. Five
new [actual manager/security fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/backoff_tests.rs)
cover:

- ID71 fails, same71 skips, newly observed72 immediately succeeds and removes failure.
- Known paired IP1 fails/skips, known IP2 immediately succeeds through the same trust path.
- Changed72 failure resets to1; same72 skips; expiry retries and increments to2.
- Same UDID switches from failed usbmux to an existing-trust mobdev2 route and succeeds.
- Formatting the whole failure record exposes no synthetic private host or route payload.

Original36 lifecycle assertion bodies/source fixtures remain byte-identical.
Two old time-test struct literals add only identity fields; original assertions
and timer function remain. All other production/native source/lifecycle/ID and
publication guards/manual/HTTP/auth/federation/bridge/record/core/public/access/
dependencies stay. Three files add203/remove4 lines. Known-change retry behavior
needs pre-1.0 release review and scoped device evidence.

Control34585 exited101 with0PASS/4FAIL. Post63939 exited0 with40PASS; Debug
fixture was added before freeze. Frozen79705 exited0 with all14 gates and
frozen-source assertions; source/result audit exited0. No observation timeout
was treated as exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |41/41 PASS |
| Tunnel regression |82 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2550 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI945/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI/Python3.12
wheel-sdist build/install/import pass. Tooling67/help118/raw38/host5/CLI95/backup3
pass.22device NOT_RUN; Chromium/installed3.9/3.13 not rerun. No maturity upgrade.

Patch SHA256: `b2479b78b7f65d429c8b626534bfe528f72b6f34a2f3171a9c52e77e6861fdeb`.
Actual debug ios SHA256: `a6679568cd1d2b990d35c1d5afa2d6035521a3945d6f67d5fc15d7c03bf5f95f`.
Commit matches frozen source. Durable logs,commands,statuses,hashes/audits live
under ignored `target/validation/tunnel-backoff-2026-10-06/`; reference
HEAD/status unchanged.

Continue failure-map/other retained metadata bounds, after-query changes/reused
IDs/restart/manual epochs, native liveness/worker-server joins, discovery/whole-
scan totals, L03 reconnection/L04 resources/performance/Q04 and remaining L01/
classes5–9. Logical known identity and owned fake release do not certify physical
device, ACK, RSS, throughput or native task bounds. All31 tasks/subtasks/history
and9 complete/10 not started/12 in progress or conditions remain. No subagent,
push,publication,history rewrite,reference edit or device/Apple service operation.
The engineering goal remains active.

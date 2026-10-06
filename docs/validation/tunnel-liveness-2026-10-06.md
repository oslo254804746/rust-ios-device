# L02 local liveness before tunnel publication

Tested source: `7a77000cdbd0a8e6d2fbaaf6ac5bd630af47d6c5`, parent
`d96cb4220eff078fec8e0b09d7a0c92ccd540527`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The [actual manager](../../crates/ios-cli/src/cmd/tunnel.rs) rejects known
inactive completed devices after expected-ID/record validation, before the
daemon query, and after publication lock/epoch/policy checks immediately before
insertion. Local status changes during query/lock waits are covered. Every route
participates, including Wi-Fi without a daemon query. Known false returns local
OperationCancelled and releases the owned result without failure backoff; it
does not reconstruct native IO.

Existing SDK semantics were inspected in
[TunnelHandle](../../crates/ios-core/src/tunnel/manager.rs) and
[userspace runtime](../../crates/ios-core/src/tunnel/tun/userspace.rs): kernel
checks receiver_count()>0, userspace checks every task is unfinished. These
implementations stay. True is not physical connectivity, Apple ACK or joined
worker proof. Receiver clones may outlive a kernel worker and a task can exit
after the final check. Those limits remain.

Production adds two checks/six lines. Three files add103/remove0 lines and four
[actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/liveness_tests.rs):

- Inactive at native result: reject before query; one owned release/permit restored.
- Dies during current-ID query: reject despite matching attachment response.
- Dies while publication lock waits: reject after acquiring the lock.
- Inactive Wi-Fi: same local check without a daemon query.

All previous41 assertion bodies/helpers/source fixtures and every other
production native source/lifecycle/ID/query/backoff/serial/watch/permit/manual/
HTTP/auth/federation/bridge/record/core/public/access/dependency stay byte-identical.
Private status is independently controlled, not an actual Core task or physical
fixture. Admission correction needs pre-1.0 release review.

Control52217 exited101 with0PASS/4FAIL; post30358 exited0 with45PASS. Frozen
79395 exited0 with all14 gates/frozen hashes; source/result audit exited0.
Source stayed frozen until authoritative completion; observation timeouts never
counted as exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |45/45 PASS |
| Tunnel regression |86 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2554 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI949/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI/Python3.12
wheel-sdist build/install/import pass. Tooling67/help118/raw38/host5/CLI95/backup3
pass.22device NOT_RUN; Chromium/installed3.9/3.13 not rerun. No maturity upgrade.

Patch SHA256: `06cd466d7036ffe0256130289678df299b72dde19d642a3be0ea12549f06396f`.
Actual debug ios SHA256: `8e7d076d086635b42dcd26102c5c1a6c816834dde91ec223b63423da0d535b3e`.
Commit matches frozen source. Durable logs,commands,statuses,hashes/audits live
under ignored `target/validation/tunnel-liveness-2026-10-06/`; reference
HEAD/status unchanged.

Continue after-check task/attachment changes,kernel receiver false positives,
unobserved same-ID/host reuse/restart/manual epochs,native worker/server joins,
retained metadata/failure-map bounds,discovery/whole-scan deadlines and L03/
L04/Q04/other L01/classes5–9. Owned fake release is not RSS/task bound or physical
proof. All31 tasks/subtasks/history and9 complete/10 not started/12 in progress
or conditions remain. No subagent,push,publication,history rewrite,reference edit
or device/Apple service operation. The engineering goal stays active.

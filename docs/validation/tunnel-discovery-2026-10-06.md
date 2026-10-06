# L02 scan discovery deadline and cancellation

Tested source: `a47648442d5b7707ad41beeb58a6ab1ad764ea9a`, parent
`653a1a3021f4e429a1263f9aea997e6ea2f63e00`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The [actual scanner](../../crates/ios-cli/src/cmd/tunnel.rs) now shares one
checked15s discovery deadline across scan-lock queue, usbmux snapshot and
existing-trust paired snapshot. Previously those waits could remain unbounded.
The [private control](../../crates/ios-cli/src/cmd/tunnel/lifecycle.rs) subscribes
to the scan epoch before queueing and marks its captured generation seen.
Stop/clear of the old generation drops its queued/phase future and returns
quiet scan Ok; it does not add connection-failure backoff or mark a cancelled
setup ready. A later generation can scan normally.

Real expiry retains original Tokio Elapsed with safe static context. A due/zero
deadline obtains native Elapsed from a pending timer future without polling real
work. Overflow reports the existing invalid_input decision before source or map
mutation. No CLI/SDK configuration is added; the existing private timeout is15s
in production and short/zero/overflow in host fixtures.

This deadline ends when paired discovery returns. Candidate15s budgets, four
connection window and whole-scan behavior remain. It is not an entire scan,
candidate handoff/metadata CPU or server-join deadline. Existing pair-record
loading may use blocking workers; dropping the wrapper is not cancellation or
join of those workers. Known paired IO still returns after USB candidate outcomes
as before. Timing/quiet cancellation corrections need pre-1.0 release review.

Four files add268/remove9 lines and nine
[actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/discovery_tests.rs):

- Stalled USB: product80ms expiry precedes outer1s guard; native Elapsed/probe drops.
- Scan queue: product60ms expiry without polling device sources.
- USB65ms then paired pending: original80ms deadline, no fresh per-phase timer.
- Stop during USB: quiet completion before reply, source future drops, next scan works.
- Clear during paired: quiet completion/source release, new generation works.
- Zero: no source poll/ready state, original Elapsed.
- Overflow: no source/ready state and machine invalid_input.
- Stop a queued scan: quiet completion without source polling.
- Candidate boundary: its normal timeout/failure/ready behavior survives setup completion.

Private probe counters/drop guards and a paired gate observe actual source
future ownership; they are not Core worker/device evidence. All previous45
assertion functions and nested fixtures remain byte-identical. Existing device
ticket/serial/permit/stop/clear bodies, native source/SDK/Core/public/dependencies,
connection/ID/query/liveness/backoff/HTTP/auth/record bodies stay. No protocol,
pairing or detached worker is added.

Control79873 exited101 with0PASS/7FAIL. Post58753 exited0 with52PASS; queued
cancellation/budget boundary and stronger overflow classification added before
final freeze. Frozen32405 exited0 with all14 gates/hash assertions; source/result
audit exited0. No observation timeout was treated as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |54/54 PASS |
| Tunnel regression |95 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2563 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI958/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI/Python3.12
wheel-sdist build/install/import pass. Tooling67/help118/raw38/host5/CLI95/backup3
pass.22device NOT_RUN; Chromium/installed3.9/3.13 not rerun. No maturity upgrade.

Patch SHA256: `0afafe77afebd925b6afc42c93790d0729d56f65a93423cb70391f44815b3705`.
Actual debug ios SHA256: `047970a9711ef1ee5d91f7ff8cc83af1ea0a74bb3841ba3106293f99d27ed969`.
Commit matches frozen source. Durable logs,commands,statuses,hashes/audits live
under ignored `target/validation/tunnel-discovery-2026-10-06/`; reference
HEAD/status unchanged.

Continue whole-scan/CPU/handoff budgets, blocking/native worker/server joins,
retained maps/metadata, after-check task/transport changes, receiver false
positives/reused IDs/restart/manual epochs,L03/L04/Q04/other L01/classes5–9.
Source future Drop is not ACK/native task/RSS/physical proof. All31 tasks/
subtasks/history and9 complete/10 not started/12 in progress or conditions remain.
No subagent,push,publication,history rewrite,reference edit or device/Apple service
operation. The engineering goal stays active.

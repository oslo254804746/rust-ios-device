# L02 failure retention and reserved capacity

Tested source: `26b89951b52dd690bcc0e994541007b5426fba93`, parent
`405bb0d89b2b5e428cceae8b23924db9dff464af`, Linux x86_64, 2026-10-06.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L02/class4/all31 tasks remain active.

The [actual scanner](../../crates/ios-cli/src/cmd/tunnel.rs) reconciles rows
against current authorized candidates before connecting. It removes unauthorized,
observed absent or known changed identities. Incomplete paired discovery preserves
unknown mobdev2 rows while authoritative USB absence removes USB failures.
Present expired identities keep their counter and30/60/120/240/300s progression.
Successful empty discovery is observed absence, not physical ACK. Reappearance
after cleanup can start at initial backoff; cache/admission timing needs review.

4096 semaphore permits cover retained records plus pending failure reservations.
Existing rows share their lease during retry; fresh scanned attempts reserve
before native work or a connection permit. Exhaustion reports existing source-free
resource_limit without evicting an active backoff or polling a fresh native
connection. Success/cancel releases reservations; failures own them. Recording
rechecks the ticket under its write lock to reject clear/stop-invalidated outcomes.

Four files add474/remove3 lines. Ten
[actual manager fixtures](../../crates/ios-cli/src/cmd/tunnel/tests/lifecycle_tests/retention_tests.rs)
cover:

- Twelve historical failures released by complete empty snapshots.
- Paired discovery error retaining unknown mobdev2 and removing absent USB.
- Actual protected HTTP policy reload from ManageTunnels to List releasing rows.
- Known changed ID removing old metadata before the new connection waits.
- Present expired identity retaining count2 and subsequent skip.
- 4095 rows plus pending A exhausting capacity: B has no native IO, failed A fills
  row4096, next scan still defers B, and complete absence restores permits.
- Full capacity allowing existing A retry and count increment.
- Caller drop releasing a pending reservation/source future.
- Clear racing queued failure recording, without row resurrection.
- Successful publication releasing reserved failure capacity.

Capacity prefill uses real private reserve/record calls, not4095 native attempts.
Fake source/owned-device Drop is not Core worker cancellation/join evidence.
Original54 assertions and fixtures remain, except three old timer/Debug struct
literals add slot:None. Native source/lifecycle/Core/HTTP/auth/trust/dependencies
stay unchanged; no new protocol, pairing or public API.

Control43601 exited101 with1PASS/4FAIL. Recovered after.log reports59PASS,
but parent handle/exit was unavailable: no exit0 is inferred. Expanded9659 exited101
with61PASS/2FAIL because filling rows consumed Tokio cooperative budget before
the first source poll. Fixtures now wait under an outer1s guard for actual entry
before capacity assertions; production code did not change for that correction.
Frozen94282 exited0/all14 gates/hash assertions; source/result audit exited0.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_tunnel full/minimum CLI |64/64 PASS |
| Tunnel regression |105 PASS/1 existing browser ignored |
| CLI WebInspector/access/core WebInspector |192/29/102 PASS |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict core-CLI Clippy/fmt/.venv targeted tooling |PASS/PASS/22 PASS |
| Built actual CLI fixtures |95 PASS |
| Section11 full Rust/Python split |2573 Rust/13 Python PASS,0FAIL,2 existing ignored |

Full command: `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI968/Core1443/public52 PASS. Workspace fmt/check/strict Clippy, MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI/Python3.12
wheel-sdist build/install/import PASS. Tooling67/help118/raw38/host5/CLI95/backup3
PASS.22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity upgrade.

Patch SHA256: `324521a21287418fc406e9aeb1445196a16c47f8fa9dd2b3dd7ecdfa22ed8cfd`.
Actual debug ios SHA256: `ef6ad9d64e17f6468043c31a940a64ac8927dadf252343726d7f11aaaf856bd0`.
Commit matches frozen source. Logs/commands/statuses/hashes/audits live under
ignored `target/validation/tunnel-retention-2026-10-06/`; references unchanged.

4096 bounds logical rows/reservations only. Bytes, candidate metadata/CPU,
native workers, sockets/RSS, whole-scan/server joins and physical proof remain.
Continue those, last-check/reused-ID/restart/manual-epoch races, other L01/L02,
L03–L04/Q04/classes5–9. All31 tasks/subtasks/history and9 complete/10 not started/
12 in progress or conditions stay. No subagent, push, publication, history
rewrite, reference edit or device/Apple operation. The engineering goal is active.

# L01/L02 coordinator wait cancellation

Implementation: `a4beec7fa01b605933b1966b79ef3ee738ae41fd`, parent
`8bd99de979495a7ea710925c072546015b0e2a80`. Linux x86_64,2026-10-06,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. All active30 tasks/subtasks exclude
Q04; L01/L02/class4 and the overall goal remain active.

The preceding shutdown profile captured the clear epoch before coordinator
locking, but the lock await itself could not observe cancellation. A held
coordinator therefore kept an old admission waiting until release. A directly
owned biased select now waits for clear/shutdown alongside that mutex, returning
the existing OperationCancelled without acquiring it or creating a row.
The post-lock epoch check remains; fresh admission after ordinary clear works.
No task, protocol, public API, permission or native source behavior is added.

One file adds40/removes2 lines, including one new test with two runtime paths.
The test holds the real private mutex through the entire cancellation result,
checks no coordinator mutation and original typed class, then releases it.
The reusable-clear path subsequently admits a fresh ticket. Before product
change, session35560 exited101:0PASS/1FAIL, blocked at the first clear path's
one-second outer fixture bound; terminal close was not reached in that failed
run. Final full/minimum runs complete both paths while the lock is held.
This is a causal cancellation check, not an L04 latency measurement.

Frozen pipeline14142 exited0, all eight gates PASS. Source stayed frozen until
its authoritative exit. The production change is only the mutable watch receiver
and mutex/select replacement; all preceding assertion functions remain byte
identical. All other product sources, Core/SDK/bindings/dependencies and excluded
Q04 sources stay unchanged; reference HEAD/status stay unchanged.

| Accepted check | Result |
| --- | --- |
| New full/minimum behavior |1/1 PASS, both cancellation paths |
| Tunnel regression |123 PASS/1 existing browser ignored |
| Minimum CLI all-targets/original stack |PASS/5 PASS |
| Strict CLI Clippy/format |PASS/PASS |
| Section11 full Rust/Python split |2591 Rust/13 Python,0FAIL/2existing ignored |
| Extra project .venv tooling |73 PASS, session90657 exit0 |

Full command:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

CLI986/Core1443/public52 PASS. Workspace fmt/check/strict Clippy/MSRV,
66 features/four runtime groups,7+7 doctests/three snippets,C ABI,3.12 wheel-sdist
build/install/import,tooling73/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN. No maturity upgrade,
physical/Apple/worker join/resource/performance or async Drop certification.

Frozen patch SHA256:
`0573916f2586d6509a1ce2ecf749670bf994df9bd3349a8ff3bb9bd44692a8d5`.
Actual debug ios SHA256:
`5ea60df57061c294c6b8252baf28376b02de6f8491365ceef054faa502284232`.
The committed patch matches the tested freeze. Ignored status/hash/source copies
and review: `target/validation/tunnel-coordinator-cancel-2026-10-06/`.
No cache clean,subagent,push,publication,history rewrite,reference edit or device
operation occurred. Cancellation timing needs pre-1.0 release review.

Coordinator acquisition is still outside the existing manual connection total;
ordinary contention has no separate acquisition deadline. Startup/whole-scan/
metadata/post-check races, delegated server/Core joins, other L01/L02, L03–L04
and classes5–9 remain. The known UID resource finding is unfixed, all Q04 subtasks
stay excluded, and other tasks retain their original acceptance conditions.

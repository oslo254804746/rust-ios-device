# L01 direct CDP page lease ownership

Tested implementation: `5ecb565633179185c77ecce9a688eb148ad62235`, based on
`fded0b0e50b8a8d858babae3261ab04a3cfdb1b7`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1, Python3.12.13. L01, class4 and all31 tasks remain active.

The [direct page owner](../../crates/ios-cli/src/cmd/webinspector/cdp.rs) now pins
one principal.revoked future through checked setup and active work. Previously
outer Principal.run discarded ScopeDenied and dropped the handler, bypassing its
asynchronous cleanup. Setup now races the same directly owned signal; active
work sits behind biased lease control and yields each message turn, retaining
the original inner unbiased traffic choice. Typed OperationCancelled remains
primary while existing independent2s cleanup runs. A completed lease future is
not polled during cleanup; valid attached clients close once, invalid clients
drop without guessed recovery.

The device/client/page lock remain owned through cleanup. A revoked page may
remain connected while that cleanup runs, processing no further business messages.
Revocation now reaches the existing safe diagnostic formatter instead of the old
silent outer discard. No new WebSocket errors/headers, protocol fields, public
grammar, admission/tickets/policy or dependencies. This timing/diagnostic
correction needs pre-1.0 release review. Cooperative timers do not preempt
synchronous decode/output or create an async Drop guarantee.

The3-file commit adds7 tests,399 lines and removes10. Original assertion functions,
inner traffic branch bodies, setup operation and returned cleanup remain.
Browser/other owners/controller/core/public/bindings/access code stays. The
private cfg(test) source counts delivered read bytes; its observer accepts a
directly owned stop future with the prior pending default. No detached source
worker, forged device or public backend is added.

| Seven [owned tests](../../crates/ios-cli/src/cmd/webinspector/cdp/lease_tests.rs) | Observed result |
| --- | --- |
| Actual policy reload after attachment | Existing forwarding close with matching app/page/sender, stream Drop/EOF and actual page-lock reacquisition |
| Actual grant expiry after attachment | Same cleanup/release through actual admitted cdp_router |
| Reload plus delayed flush | Actual80ms flush completes despite50ms active idle policy |
| Reload after partial frame | Counter proves2bytes consumed before reload; invalid cancellation emits no recovery/extra request |
| Reload during pending connection | Source owns stream before its1s wait; cancellation before return yields Drop/EOF, one call, zero wire/received bytes |
| Reload plus pending cleanup flush | Real2s cleanup completes by expiry instead of repolling the completed lease; release/EOF observed |
| Private stop plus native cleanup IO | Actual handler retains typed Cancelled primary and original PermissionDenied std::io::Error; matching qualified lock is reacquired |

Six fixtures use actual page routes, admission and synthetic policy files; the
source adapter verifies the authorized synthetic DEVICE then delegates an owned
test stream alias. The seventh private route observes actual handler errors and
makes no new admission claim. Actual Runtime.enable forwarding after target
creation proves attachment before revocation. Only existing cleanup follows;
no guessed Apple selectors, real credentials or device contacts. Joined test
listener/peer futures do not prove product WebSocket/task supervisor joins.

Before-product24664 exited101:1PASS/3FAIL. Observed partial-frame cancellation
passed; all three attached reload/expiry controls missed the close frame.
Corrected84618 exited0:4PASS; expanded98632 exited0:7PASS. No compile/test failure
after the product change. A source patch context mismatch applied nothing and
was corrected. The outer/inner selection layout was settled before the first
post-product compile; original traffic choice remains.

Frozen pipeline53535 exited0, all20 gates pass. Source remained frozen; builds
terminated before edits or new validation. Observation timeout never counted
as process exit.

| Accepted gate | Actual result |
| --- | --- |
| lifecycle_cdp_lease all features/minimum CLI | 7/7 PASS |
| Prior WebDriver/direct cleanup/setup/page/finite/shell | 8/5/15/5/21/31 PASS |
| CLI WebInspector/access/core WebInspector | 139/29/102 PASS; core96+public5+integration1 |
| Core-only cache minimum/minimum CLI all-targets | 17 PASS/PASS; CLI core remains full |
| Original CLI stack/strict core-CLI Clippy/fmt | 5 PASS/PASS/PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2456 Rust/13 Python PASS,0FAIL,2existing ignored |

Complete command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI851/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 features/four runtime groups,7+7 doctests/three downstream snippets,C ABI,
Python3.12 wheel/sdist build/install/import pass. Tooling67,actual help118,raw38,
five host workflows,95 CLI cases and3 backup contracts pass. PBKDF2 and the
separate browser gate stay ignored.22device NOT_RUN; Chromium and installed
Python3.9/3.13 not rerun. No maturity upgrade or cache cleanup.

Frozen source patch SHA256:
`d54f0345641659b337a1411bf35b0e9ad02afcee909d4201a7e76d8847bcb2ec`.
Actual debug ios binary SHA256:
`1ce215b1d7e9bb3c6e7e77a65fc925ffc56812527eb5fe09b96278b415543071`.
Actual commit matches the frozen/tested patch. Logs/copied development evidence
with original hashes/audits live under ignored
`target/validation/cdp-page-lease-2026-10-05/`; reference HEAD/status unchanged.

Direct page lease only. Generic caller/server Drop, constructor primary cleanup,
active request/output totals, joined browser children and stored-session owners
remain. Other L01, L02–L04/Q04 and classes5–9 continue. No Apple ACK/rollback/
retry/async Drop/join/platform/benchmark/RSS/decoder peak acceptance. All31
tasks/subtasks/history,118 capability bodies/maturity/prior evidence and counts9
complete/11 not started/11 in progress or pending conditions remain. No subagent,
push, publication, history rewrite, reference edit or real device/Apple operation.
This intermediate commit does not end the engineering goal.

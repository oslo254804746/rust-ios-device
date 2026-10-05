# L01 CDP page idle and terminal timeout retention

Tested implementation: `2e22aaf7da2c47fff1654c557882e8cb9a121517`, based on
`4de5c00df11ca72f963f1814d643610b0ebe97f3`, Linux x86_64, 2026-10-05.
Rust1.97.1/MSRV1.80.1 and Python3.12.13. L01, class4 and the entire31-task
goal remain active; this profile completes only the page loop's idle/error scope.

The [page bridge](../../crates/ios-cli/src/cmd/webinspector/cdp.rs) previously
ignored every timeout from its next raw-message read. Partial input or an
automatic listing write can invalidate the core client. Its next loop then
rejected local reuse with Protocol, replacing the original native timeout.
The bridge now continues only when that timed-out client is still usable.
Invalidating expiry returns the original Elapsed/Timeout and drops the stream
without retry or teardown bytes. Healthy empty idle still resumes and relays
the next target message; normal WebSocket close emits the existing close selector.

The three-file source commit contains one production guard, a test-module
declaration, five existing cfg(test) helper visibility changes and five new
tests. All original bodies/assertions remain byte-identical. Core, public APIs,
bindings, dependencies, security admission, cleanup and browser loops stay.
Protocol keys/selectors come from existing core/bridge fixtures; no protocol is
guessed. The host correction to the returned error needs pre-1.0 release review.

The [owned fixtures](../../crates/ios-cli/src/cmd/webinspector/cdp/lifecycle_tests.rs)
use an actual Axum localhost WebSocket and actual private page handler over a
duplex plist stream, through the existing private InspectorSource. Device=None
does not forge ConnectedDevice. A cfg(test) route observes the returned Rust
error; no public route/source argument is added. This verifies neither new
admission/revocation behavior nor an external CDP client's device workflow.
The source connection count, dropped stream and peer EOF are observed; teardown
and retry bytes are absent on terminal failures. The owned server joins on
success, with abort fallback for fixture failure.

| Five current tests | Actual observation |
| --- | --- |
| Partial header | Two real frame-prefix bytes then expiry retain native Elapsed/Timeout |
| Partial body | Complete prefix and incomplete body retain native Elapsed/Timeout |
| Automatic listing | A complete connected-application-list event routes a write into an owned pending writer; an observed poll proves the actual attempt; expiry retains Elapsed/Timeout |
| Healthy idle | Several empty idle caps leave the bridge open; a nested Target dispatch message reaches the actual WebSocket, and normal close writes existing teardown |
| Native failures | Actual EOF retains UnexpectedEof/IO, malformed plist retains plist::Error/IO, and the existing oversized frame rejection remains Protocol; all release the stream without more writes |

The50ms idle cap and4s fixture bound are local test controls, not latency
commitments. Logical frame limits do not prove decoder expansion/depth/CPU/peak,
RSS, or overall resource release. This profile adds no retry or recovery
authority, Apple ACK, rollback or proof of remote teardown.

Development results are preserved accurately in ignored
`target/validation/cdp-page-idle-2026-10-05/`:

- First23098 exited101 with3 test-only E0283 diagnostics; no tests ran.
  Explicit String keys fixed inference. The healthy reply fixture uses the
  existing nested Target dispatch shape; raw ACKs are intentionally filtered.
- Before-product80012 exited101:1 PASS/3 FAIL. Header/body observed Protocol
  instead of Timeout. The third fixture reached its outer bound because an
  applicationUpdated event does not cause a listing; this was a fixture error.
- Corrected before-product56035 exited101:1 PASS/3 FAIL. All three terminal
  cases observed Protocol instead of Timeout; healthy idle passed. This supplies
  the real negative control for the guard, not a baseline-wide acceptance claim.
- After-product7527 exited101:4 PASS/1 FAIL. The new native-failure group
  assumed malformed plist classified as Protocol; its existing class is IO.
  Only that assertion changed. Corrected19929 exited0:5 PASS.
- Frozen pipeline36134 exited0 with all15 gates; no product edits occurred
  during its execution. Timeouts while observing a process never counted as exit.

| Accepted gate | Result |
| --- | --- |
| New lifecycle_cdp_page, all features/minimum CLI | 5/5 PASS |
| Original finite/shell regressions | 21/31 PASS |
| CLI WebInspector/core WebInspector | 104/102 PASS; core96+public5+integration1 |
| Core-only webinspector cache minimum | 17 PASS; CLI dependency still enables core full |
| Minimum CLI all-targets/original CLI stack tests | PASS/5 PASS |
| Strict core/CLI Clippy and workspace fmt | PASS |
| .venv targeted tooling/actual CLI | 22/95 PASS |
| Section11 full Rust/Python split | 2421 Rust/13 Python PASS,0FAIL,2existing ignored |

The complete entry was
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
CLI816/Core1443/public52 pass. Workspace fmt/check/strict Clippy,MSRV1.80.1,
66 feature checks/four runtime groups,7+7 doctests/three downstream snippets,
C ABI and Python3.12 wheel/sdist build/install/import all pass. Tooling67,
actual help118,raw38,five host workflows,95 CLI cases and3 backup contracts pass.
The two ignored tests remain the existing PBKDF2 and separate browser gate.
Twenty-two device rows remain NOT_RUN; Chromium and installed Python3.9/3.13
were not rerun. Host evidence does not upgrade capability maturity.

Frozen patch SHA256:
`9b45ef230001de695e42149a97f0a3eb217c517059b33b8664e2160a5689a42d`.
Actual debug ios binary SHA256:
`2fe7cc98689301125679d1776244f17e8412e8b9d5d449fd1910fd8f0df48031`.
The actual commit patch matches the frozen tested patch. Evidence metadata,
logs and source audit are in the ignored directory above; read-only reference
HEAD/status remain unchanged. No cache cleanup was needed.

Remaining L01 includes page/browser/WebDriver/server setup and request totals,
finite sends/output, independent primary-preserving cleanup, server cancellation,
revocation and joined child/stored tasks, other protocols and long streams.
Existing stored-session100ms and browser-page2s cleanup policies were untouched.
Other-platform stdin, decoded plist bounds/fuzz, L02–L04/Q04 and classes5–9 remain.
All31 main tasks and their subtasks/history are retained; counts stay
9 complete/11 not started/11 in progress or pending acceptance/conditions.
No push, publication, history rewrite, subagent, reference edit or device/Apple
operation occurred. Continue independent work; this commit does not end the goal.

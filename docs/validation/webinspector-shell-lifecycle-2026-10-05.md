# L01 interactive WebInspector shell lifecycle

Host acceptance, 2026-10-05. Tested implementation:
`8ac37b7d52ce74dc29c6f11e06c82bcffba879fc`, based on
`26de9e25b93be089832e843fc012ab4766dbd842`. Linux x86_64,
Rust 1.97.1 / MSRV 1.80.1 / project Python 3.12.13. Continues the
[finite command profile](webinspector-command-lifecycle-2026-10-05.md).
L01, class 4 and all 31 tasks remain active; physical device acceptance is separate.

The former js-shell selected a default device outside its setup timer and used
two Ctrl+C futures. Input used Tokio's uncancellable blocking stdin read.
Metadata and result output escaped the frontend timer; wire errors could
continue on an invalid client, and normal/error exits omitted session teardown.

The [private shell owner](../../crates/ios-cli/src/cmd/webinspector/tasks/interactive.rs)
now shares one checked absolute setup total across input acquisition, default
selection, connection, startup, optional Safari launch, discovery, attachment,
navigation and metadata. One pinned signal spans all stages. The first default
device still wins, including network-first ordering; explicit selection bypasses
discovery. Zero rejects before stdin acquisition/discovery as source-free
Timeout, replacing the old 100ms fallback. Positive inputs keep that minimum;
invalid duration/clock inputs preserve their existing guards/sources.

After setup, idle input is unbounded and cancellable. Each expression receives
a fresh total covering command, interpretation and output, with the owned
client's cap refreshed from its remaining budget. Prompts/EOF output are finite
and cancellable. Continuous ready blank lines yield to the same signal.
Synchronous serialization/output and regular-file reads remain cooperative;
there is no hard preemption, measured CPU bound or remote filesystem guarantee.

Setup cancellation is typed Cancelled and exits 1. After setup, successful
directly owned Ctrl+C is a normal operator stop; failed signal registration
retains native IO. Completed JavaScript exceptions and remote-response business
errors may continue under --continue-on-error. Native wire/deadline/codec/input/
output failures now terminate even when that flag is true. No request retry
consumes an invalid client's late response. Already returned native errors
remain primary, including the observed native IO classification for EOF.

The [shared session owner](../../crates/ios-cli/src/cmd/webinspector/tasks/session_owner.rs)
is a move of the accepted finite cleanup logic. A fully attached usable
Inspector sends its existing forwarding-socket close under an independent 5s
total. Incomplete or invalid owners drop without guessing recovery requests.
Device and stream stay owned through cleanup, which never repolls a completed
signal. Primary and cleanup errors remain available, as do explicit stdin
restoration failures. A successful stop followed by cleanup failure returns
exit 1; previously emitted records and remote effects are not rolled back.
Local write/flush completion is not a remote ACK or proof of Apple teardown.

The [Linux input adapter](../../crates/ios-cli/src/cmd/webinspector/tasks/input/linux.rs)
owns pipe/terminal readiness directly and spawns no blocking read task.
Duplicated stdin shares file-description status flags: changes to nonblocking
mode restore explicitly, with best-effort Drop for abandonment/admission failure.
Preexisting nonblocking mode stays. Regular files use cooperative owned reads;
dev-null remains EOF. Other unpollable nonregular descriptors, including
dev-zero, now return their native readiness-registration error.
The already locked libc package gains only a Linux CLI dependency edge; no
package version/checksum changes. Other platforms retain Tokio stdin, and their
input cancellation/runtime shutdown still need implementation and acceptance.

Successful compact session/result/error JSON records stay, including the full
Runtime.evaluate response envelope. Human values use stdout, business errors
use stderr, and prompts/EOF/exit commands stay. Explicit page-key precedence,
Safari filtering and escaped navigation URL encoding stay. These zero/timing/
continue-on-error/cancellation/cleanup exit changes need pre-1.0 release review.
There is no core/binding public API, error variant, wire selector or server
access-policy change. Read-only reference HEAD/status stay.

| Current owned host fixtures | Verified observation |
| --- | --- |
| 23 shell tests | JSON/human formats, default/explicit selection, exact identities, Safari/navigation/page-key precedence, prompt/blank/EOF/exit behavior and cleanup |
| Checked setup | Zero/clock overflow and ready signal before input/default IO; admission/empty/native list failures restore acquired input; connection/startup share one total; incomplete attachment cancellation is typed and never guesses teardown |
| Active input/evaluation | Idle exceeds the request cap; successive delayed evaluations each get a fresh total; continuously ready blank input yields to cancellation |
| Business versus native failures | Completed JS/protocol errors continue with existing safe records; native EOF/plist/deadline/cancel stops committed work without retry/teardown |
| Primary and cleanup | Writer, UTF-8/input-limit/native input and signal IO preserve sources; native/stalled cleanup retains primary; late output expiry gets fresh cleanup; active cancellation and restoration failures have explicit outcomes |
| Eight Linux input tests | Owned pipe bytes/EOF, cancellation, saved/preexisting flags, regular file/dev-null, failed dev-zero admission restoration, actual PTY readiness and flags |
| Owned process exits | Stdin-only cancellation permits runtime exit with writer open; whole private shell receives actual SIGINT after its first input poll, cleans once and exits while the parent still owns the idle writer |
| Actual ios binary | Ten new js-shell cases: empty/stalled default and EOF, zero/two overflow kinds before default or explicit contact, human overflow and actual SIGINT during owned fake ListDevices; no actual service/pairing/Apple operation supplied |

31 new Rust tests use bounded private source/reader/writer injection and actual
core plist sessions; synthetic connections carry no forged ConnectedDevice.
The subprocess fixtures execute the actual test binary/owner/production stdin
adapter; no public CLI/HTTP argument selects a synthetic backend. Four inherited
admission-tooling tests cover expected error, prohibited contact/private raw
diagnostics, and rejected incorrect zero classification. Ten new actual CLI
cases raise 85 to 95; tooling rises 63 to 67 (targeted 18 to 22).

Historical development: first narrow compile exited 101 on seven test temporary
borrow errors before any tests ran. Second narrow compiled, 23 PASS / 2 FAIL;
the fixtures assumed Disconnected for native EOF and attempted a direct
downcast through a core cleanup wrapper. Correct native classification/source
traversal fixed them. Third narrow passed all 31. Initial pipeline 74954 passed
its pre-Clippy gates, then exited 101 on an unconditional test-only JSON import;
cfg(test) corrected it. These failures are not acceptance.

Pipeline 69263 completed all 14 gates/full 2416 Rust / 13 Python for the preceding
fixture snapshot. Review found the child ready marker was emitted at metadata
publication. It moved to the first actual input poll to remove a setup/active
race in the test criterion. Only interactive/tests.rs changed; product files
were byte-identical. The current complete result is pipeline **44479**, terminal 0,
with separately frozen evidence under reviewed/; older logs remain historical.

Current gates: shell31/minimum31, unchanged finite regression21, CLI99,
Core96+public5+integration1=102, core-only minimum17, minimum CLI all-targets,
original stack5, strict Core/CLI Clippy/fmt, targeted tooling22, binary build,
actual CLI95 and the section 11 full split all PASS.

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Current full: **2416 Rust / 13 Python**, CLI811/Core1443/public52, zero FAIL /
two existing ignored. Every section 11 split gate PASS: workspace fmt/check/
strict Clippy/MSRV1.80.1;66 feature checks/four runtime groups;7+7 doctests/three
snippets; C ABI;3.12 wheel/sdist build/install/import;67 tooling/118 actual help/
38 raw/five host/95 actual CLI/three backup contracts. Chromium and installed
3.9/3.13 were not rerun. 22 device rows remain NOT_RUN. Linux does not substitute
for other platforms, device conditions or credentials.

Source audit preserves original task/shell test modules and all fourteen finite
assertion functions byte-for-byte. Control source/tests stay byte-for-byte.
Shared session/cap/cleanup differ only by location, private visibility,
formatting and a trailing parameter comma; their logic stays. Cargo.lock
comparison permits only the existing libc CLI dependency edge.

Seventeen changed files (18 frozen paths including a rename),
2751 insertions / 865 deletions. Exact tested patch SHA256:
`9385b45600f00f84590743dab1a8abbdfc18a29cb286d88784e50a81cd9553d4`.
Actual ios debug binary SHA256:
`ebebc83ddc3cfe282d3a4447fa1269636b7a7aa3701e45db47bbcf194abe7d7c`.
The actual source commit matches that patch. Durable ignored evidence:
target/validation/webinspector-shell-2026-10-05/reviewed/.
No build-cache clean, subagent, push, publication, history rewrite or reference
checkout edit occurred.

CDP/WebDriver/server totals, revocation, primary cleanup and joined child owners
remain. Other L01 protocols/cleanup, L02–L04/Q04 resource/decoder/fuzz/manager
isolation and classes 5–9 continue. All 31 tasks/subtasks, 118 capability bodies/
maturity/prior evidence and 9 complete / 11 not started / 11 progress-or-condition
counts stay. No physical device/browser/other-platform/remote ACK/retry,
baseline-wide old-source control, long-run/resource/RSS/decoder peak acceptance.
This is a class 4 host subprofile, not completion of the overall goal.

Documentation audit PASS: eight documents/seven Markdown/363 local links and
118 source/actual-help records. All 31 task/subtask/history prefixes and 118
capability bodies/maturity/prior evidence stay. Frozen source/binary and all
14 current gates match; no product edit occurred during documentation, and
reference HEAD/status remain unchanged. The overall goal stays active.

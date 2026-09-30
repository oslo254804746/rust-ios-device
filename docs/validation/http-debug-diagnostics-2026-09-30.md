# S04 HTTP, Debug and lifecycle diagnostic evidence

Date: 2026-09-30. Tested implementation:
`1953f8b4273ea61703c22245e4da5b2e2bedc591` on Linux x86_64, Rust 1.97.1,
MSRV 1.80.1 and CPython 3.12.13. This follows the
[error formatter](error-diagnostics-2026-09-30.md) and
[automatic boundary](diagnostic-boundaries-2026-09-30.md) checks.
The [diagnostic contract](../diagnostic-contract.md) defines explicit data use.

| Check | Result and scope |
| --- | --- |
| Real HTTP framework rejections | PASS; three manager/WebDriver/CDP shapes, invalid JSON/query/path input, method mismatch, Allow, HEAD and reviewed-helper preservation |
| Owned error Debug | PASS; 38 named errors and service_error! types use safe formatting, original typed variants/Display/sources retained; source guard rejects derived payload Debug |
| Credential/tunnel Debug | PASS; FullPairRecord, SupervisionIdentity, VerifyPairSession and both TunnelInfo configurations omit synthetic material/addresses |
| Promise and shell output | PASS; clipboard promise failure correlation and safe shell protocol errors; requested JS exceptions and clipboard value models retain data purpose |
| Original tracing capture | PASS; positive control detects actual synthetic secret; bounded UTF-8 overflow rejected; real rollback failure and partial-preamble timeout/EOF require actual phase and no secret |
| CLI/core focused suites | PASS; CLI 542; core full-feature lib 953, one existing PBKDF2 ignored |
| Full split gate | PASS; 1589 Rust, eight Python, 66 feature configurations/four test combinations, seven default/seven full doctests and three downstream snippets |
| Bindings/artifacts/tools | PASS; linked C ABI and raw failure capture, wheel/sdist rebuild/install/import, 17 tool tests, 118 capability records/actual CLI help and ten CLI capture cases |
| Workflow lint/diff review | PASS; actionlint, git diff --check and git diff --cached --check |
| Physical-device and other-host execution | NOT_RUN; this evidence is host adapter/formatter behavior, not real-device or macOS/Windows observations |

Commands:

```sh
cargo test -p ios-core --lib --all-features --locked
cargo test -p ios-cli --bin ios --locked
python3 scripts/validate.py tooling-tests
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
/tmp/ios-validation-tools/actionlint .github/workflows/ci.yml
git diff --check
git diff --cached --check
```

The absolute interpreter and actionlint paths record this run; portable setup
remains in [build instructions](../build.md). Final full log:
`/tmp/ios-s04-http-debug-final-full.log`. Focused logs:
`/tmp/ios-s04-log-capture-isolated-suite.log`,
`/tmp/ios-s04-http-debug-cli.log`, `/tmp/ios-s04-http-debug-tools.log`.
Actual tested `target/debug/ios` SHA-256:
`e6547bd96bd283f810cb328b27ee871b2943168e90c228bba370cefc868d7d1d`.

Original stdout/stderr and HTTP/log fixtures are checked in RAM before any
masking; no raw diagnostic archive is produced. The ten CLI cases and C smoke
have zero original hits and independently zero masked-copy hits, correct exits,
nonempty required output and no timeout/overflow/incomplete capture. C capture
contains 19 bytes. Fixture assertions require the actual expected phase.

The first full gate failed because the async proxy tracing fixture captured no
phase under parallel execution; two subscriber-context adjustments also failed
the full core suite. The fixture now uses one owned child test process with a
global subscriber and a ten-second limit, killing/reaping only that child on
timeout. Its output streams are discarded; captured fields remain bounded RAM.
The full core suite and then every full gate passed. Empty capture was never
accepted. Earlier compile errors in the key fixture/fallback cfg and a strict
lint rejection of test placement were corrected before the final gate.

Source review distinguishes automatic diagnostics from explicit data models and
successful user results; the derive guard is a convention check, not a Rust AST
proof. Authentication material added by S01–S03 must extend the same raw-output
checks. No actual pairing, DDI/TSS/device operation, remote CI, push or publication
was performed. The roadmap retains Q02 device/platform conditions and unfinished
access control; this evidence does not close the second category.

Subsequent finding: while preparing S01, source inspection confirmed that the
CLI's global maximum level also enables tungstenite 0.24.0's request/frame/message
TRACE and close-reason DEBUG. This gate did not capture that dependency protocol
path. S04 was reopened for CLI subscriber filtering and a real WebSocket fixture;
the successful checks above remain evidence for the listed owned surfaces only.

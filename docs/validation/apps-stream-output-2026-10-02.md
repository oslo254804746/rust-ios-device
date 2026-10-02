# A02 application mutation results and syslog bounds — 2026-10-02

Implementation `60d5cb080f03d56194d9e9ea4d1ce8fd81dc49cf`, eight related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[usage](../usage.md), [roadmap](../engineering-roadmap-2026-09-30.md).

Application mutations now honor global JSON mode. Install (including streaming),
upgrade, archive, restore, kill, signal, launch and uninstall emit a compact
newline-terminated result with integer `schema_version: 1` and an `operation`
tag. The linked error contract lists the exact operation fields. Emission follows
the existing service and applicable staging-cleanup checks. Human mode preserves
the old bytes; other application query JSON formats are unchanged.

Pkill emits NDJSON progress after each successful signal request, followed by a
complete summary only when all matching requests succeed. A zero-match result
has zero counts and sends nothing. Matching, process order, serial calls and
per-process connections are retained. Counts describe completed signal requests,
not observed process termination. A request or output failure stops subsequent
calls and emits no complete summary; prior records remain and a write failure
can leave a partial last record. Flush can fail after a full record is visible.
Final errors retain sources, exit 1 and unknown effects. Install or upgrade can
affect the device before later cleanup/output failure; no rollback/retry promise
follows from these result records.

The existing typed ServiceNotFound now selects Instruments fallback only for
canonical appservice or its named shim, alongside retained legacy Unsupported
compatibility cases. Another missing service or native timeout, permission,
refusal or broken-pipe failure does not select fallback. Ten DTX adapters preserve
sources under fixed anyhow contexts, two timeout wrappers preserve Tokio Elapsed,
and signal prerequisites preserve the original routing error. Error enum
definitions, deadlines, wire requests and SIGKILL-only fallback are unchanged.
Source/class, Display context and safe error correlation can change as documented.

Syslog's existing 1 MiB input budget now applies before copying a final chunk
containing NUL, as well as unterminated chunks. The count excludes the delimiter
and precedes vis/UTF-8 decoding. Exact-limit messages and the following message
are accepted; one extra byte is InvalidData/Protocol. CLI retains a prior valid
NDJSON record and emits no over-limit record. This does not impose a serialized
JSON output budget or establish error/cancellation-safe reader reuse. The reader
must be discarded after a size error; stronger lifecycle work remains in L01.

| Check | Result |
| --- | --- |
| Core syslog all-feature and syslog-only tests | Seven PASS in each configuration, zero FAIL/ignored; `/tmp/ios-a02-syslog-limit-after.log` and `/tmp/ios-a02-syslog-limit-minimal.log`, terminal exit 0 |
| CLI syslog tests | 17 PASS, zero FAIL/ignored; `/tmp/ios-a02-syslog-limit-cli.log`, terminal exit 0 |
| CLI application tests | Final 43 PASS, zero FAIL/ignored; `/tmp/ios-a02-apps-json-final.log`, terminal exit 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-stream-apps-clippy.log`, terminal exit 0; final full includes strict workspace Clippy after the last fixture simplification |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-stream-apps-full.log`, terminal exit 0 |
| Review | New rendering/call helper is private; public Rust error enums, signatures and wire keys preserved. Migration documented; 114 changed local Markdown links and whitespace PASS, catalog 118 records PASS. This is not a general ABI or physical workflow proof |

Nine new tests cover the defect boundaries: core limit with/without final NUL and
exact-limit/next message; CLI prior-record/oversize/class/ownership; named typed
fallback; and six application renderer/ordered-call cases. Application cases
assert ten single-result human/JSON variants, escaped and Unicode paths, full
unsigned PID, signed signal and null PID; ordered progress/summary; second-call
permission failure with no third call; output BrokenPipe with no next call;
zero-match success; and exact prior human pkill bytes. Actual shared error
rendering asserts native source/class, phase apps, exit field 1, unknown effects
and safe diagnostics. Syslog exercises the real core decoder and CLI driver and
observes one reader drop.

These new fixtures exercise owned output/call callbacks and decoder/driver
boundaries; application dispatch wiring and error adapters were source-reviewed.
They do not run actual paired-TLS mutation CLI processes, physical appservice/
DTX requests, process termination, device staging cleanup or Apple cleanup.
Streaming-install stderr callbacks remain a pending IO boundary. Other JSON
commands, bindings, lossy adapters, L01/L03 and platform/device conditions remain.

Failure history is retained: the new core regression genuinely failed before
the fix because the oversized NUL-terminated message was accepted
(`/tmp/ios-a02-syslog-limit-before.log`, terminal exit 101). After the fix both
oversize forms reject before excessive buffer growth. One documentation patch
had a mismatched usage context and made no writes; inspection and a corrected
patch resolved it. No unreachable-fixture allowance remains in the final
zero-match test. Reduced-feature pre-existing dead-code warnings are retained.

Full gate: **1778 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
batch. The ios 0.1.13 SHA-256 is
`24c44a2288cd7c2fa9ccd3d82a8426be576d81ee089ceb81010478a90d02043b`.

This host slice is accepted; A02/third class and the 31-task goal remain active.
No capability maturity upgrade, device operation, push, publication, history
rewrite or reference-checkout modification occurred. Remaining JSON/stream,
binding and source boundaries are next; fourth class has not started.

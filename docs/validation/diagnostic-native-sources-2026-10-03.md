# A02 diagnostic service native sources — 2026-10-03

Implementation `1c8174275aadff24a1cb3001504e7451efc95fcf`, four source files,
starting clean HEAD `d993ecf` on `feat/device-service-updates-20260927`.
Linux x86_64, Rust 1.97.1/MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twenty-two native adapter sites now preserve their original causes:

| Boundary | Sources |
| --- | --- |
| CrashReport | Five Elapsed, two Utf8Error and two serde_json::Error: nine |
| OS Trace | Two Elapsed, five TryFromIntError, four native IO and one regex::Error: twelve |
| OS Trace CLI | One PID TryFromIntError |

Native service timers now use existing Io with timeout classification instead
of the unit Timeout variants. Crash-report native parsing and received trace
integer conversions remain protocol through InvalidData; local request-length,
PID and regex failures become invalid_input. Archive header/tar/extraction
adapters preserve original IO kinds and native OS chains instead of Protocol
prose. Truncated host header IO is io, not an inferred disconnect. Native tar IO
can also classify io rather than its old protocol string; source-free archive
format/path/link/size guards remain Protocol. Returned variants, Display,
immediate payloads and safe correlations can change. Migrate Timeout/Protocol
or immediate get_ref/text matches to typed class/source inspection. Native
sources can contain private diagnostic data; explicit access follows the
diagnostic policy. Fixed operation context and outer Debug stay safe.

Public CrashReportError's five and OsTraceError's four variants/fields remain
exhaustive. Existing public signatures/root exports, features, dependencies/
versions/lockfile and classifier stay. MessageFilterOptions adds
`compile_preserving_errors() -> Result<CompiledMessageFilter, OsTraceError>`.
Both compilers share the original bounded filter implementation. Legacy
`compile() -> Result<CompiledMessageFilter, String>` intentionally keeps its
exact formatting and lossy string behavior. Core next_filtered_options and CLI
stream preflight select the typed method. Term count/length guards become
source-free InvalidInput through this method, without inventing a regex failure;
actual builder failures retain regex::Error. Filter limits, case/Unicode/text
predicates and successful results stay. Invalid CLI PID/regex now classify
invalid_input instead of unknown, with existing exit/output/envelope shapes.
Preflight still runs before connecting or creating the stop signal.

CrashReport watch retries a report that may still be written by its producer.
A private parse marker retains each original UTF-8/JSON source; a bounded
32-source predicate recognizes that marker or the existing parser Protocol
guards. It never formats errors or treats every generic protocol/IO category
as a report parse failure. The old three retries, pending/known snapshots,
polling/signature rules, yield limit and terminal error behavior stay. An
admission watch timeout can finish normally, while an in-flight timeout remains
an error. Checked-add overflow still returns its created source-free Timeout.
No native Elapsed is fabricated for that outcome.

The private request-length conversion was equivalently extracted before baseline
tests, still using the old eraser. Conversion still precedes the existing 8 MiB
guard and all writes; direct helper overflow avoids giant allocations. Four
received usize conversions are source-reviewed on this 64-bit host, without
claiming 32-bit overflow injection. Trace EOF/framing/limits, archive validation
passes, path/link/type checks, staging/install, file modes and cleanup stay.
Partial-read/write timeout source retention does not certify stream reuse;
the supplied owner must still handle cancellation/cleanup under its existing
contract. No new L01 invalidation, retry or remote-release guarantee.

Production comparison passes: CrashReport's nine mapping sites and same-scope
parse-retry predicate are the only changes outside private helpers. OS Trace's
shared filter body equals the old compiler after two guard callbacks/one native
regex callback normalization; its remaining thirteen mapping sites include the
migrated preserving consumer and unchanged move-file adapter. CLI's five sites
are compared after native PID/preserving-filter normalization; argument/preflight/
deadline/signal/stream bodies remain. Cargo/lock/root exports/classifier/test_native
are byte-identical. Private CLI PID wrapping is reviewed separately: an IO custom
payload must expose the original TryFromIntError through its source, because
standard IO source forwarding otherwise skips that bare error. Logs/snapshot:
`/tmp/ios-a02-diagnostic-native-code-audit.log`,
`/tmp/ios-a02-diagnostic-native-source-diff.patch`.

| Check | Result |
| --- | --- |
| Final native diagnostic library group | 9 PASS; `/tmp/ios-a02-diagnostic-native-after-final.log`, terminal 0 |
| All-feature CrashReport / OS Trace | 28 / 31 PASS; `/tmp/ios-a02-diagnostic-native-{crash,trace}.log`, terminal 0 |
| Corresponding no-default service features | 28 / 31 PASS; `/tmp/ios-a02-diagnostic-native-{crash-minimal,trace-minimal}.log`, terminal 0 |
| External public_contract all / no-default | 27 / 13 PASS; `/tmp/ios-a02-diagnostic-native-{public-final,public-minimal}.log`, terminal 0 |
| Existing OS Trace integration target | 5 PASS, including shared stream fixtures; `/tmp/ios-a02-diagnostic-native-trace-public.log`, terminal 0 |
| Final CLI OS Trace tests | 11 PASS; `/tmp/ios-a02-diagnostic-native-cli-final2.log`, terminal 0 |
| Core/CLI strict Clippy | PASS; `/tmp/ios-a02-diagnostic-native-clippy.log`, terminal 0 |
| Full validation with workspace .venv Python | First full PASS; `/tmp/ios-a02-diagnostic-native-full.log`, terminal 0 |

Nine new internal tests, one external consumer and one CLI test establish actual
owned duplex timers, report UTF-8/JSON faults, regex errors/guards and native
archive IO/OS sources. An owned AFC peer withholds initial listing, later polling
or file data to exercise all three watch timers; another returns malformed JSON
through four complete file opens/reads/closes, verifying the unchanged three
retries and final original parser source. The tests discard their client, observe
peer EOF and join their own peer; this is local fixture cleanup, not device file
close/remote cancellation acknowledgement. The parser-scope test also rejects an
unissued JSON error under IO, recognizes source-free shape guards, and bounds a
cyclic source whose Display panics. Owned short/checksum-corrupt archives cover
both entry-validation passes; extracting a valid archive into an owned regular
file retains a real native OS error. Owned files are removed, including baseline
failure paths by their Drop fixture. No real backup/diagnostic data, credentials,
device/daemon or Apple operation; no every-OS/backend/allocator fault, physical
stream, partial-operation reuse or device cleanup certification.

Before-fix service/public logs show **zero PASS/nine FAIL**, terminal 101:
`/tmp/ios-a02-diagnostic-native-before.log` (eight internal) and
`/tmp/ios-a02-diagnostic-native-before-public.log` (one external). These are
genuine native-source/class regressions; the request converter was equivalently
extracted before them. First public post-fix compile failed E0433 for an unqualified
ErrorClass in a new assertion; using its existing root path fixed only the fixture,
log `/tmp/ios-a02-diagnostic-native-public.log`.

The original CLI baseline and first post-fix CLI run used unsupported level
`all`, stopping before PID/regex preflight. Their logs
`/tmp/ios-a02-diagnostic-native-before-cli.log` and
`/tmp/ios-a02-diagnostic-native-cli.log` do **not** prove native loss. The corrected
empty-default fixture then exposed a real implementation issue: Input class was
present but bare IO payload source forwarding skipped TryFromIntError, log
`/tmp/ios-a02-diagnostic-native-cli-final.log` (ten PASS/one FAIL). A private fixed
Debug/Display wrapper exposes that original cause; final CLI sources/categories
pass without weakening the assertion. The first audit guessed three CLI mappings;
the actual five are compared, including unchanged extra mappings. Production
guards, budgets, gates and lints were not weakened; failure logs remain.

Full gate: **1956 Rust/13 Python PASS, zero FAIL, two existing ignored**. Fmt,
check, strict workspace Clippy, MSRV, 66 feature builds/four groups, 7+7 doctests/
three snippets, C ABI/owned malformed/checked list, Python 3.12 wheel/sdist
rebuild/install, 53 tooling tests, 118 help records, 38 raw cases, five host
workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass. No separate
Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug SHA256
`9778f708eb92b42c85f1b61a29dca79c14ccdd8b0617d6ca3a89ccc4059295b5`.
Source stayed frozen throughout full and afterward. About 13 GiB free at full
start and finish; no storage failure or cleanup.

Documentation closeout: 118 capability records, 139 local links across six
changed Markdown files (baseline d993ecf), unchanged capability implementation/
maturity and historical evidence, and whitespace PASS. Logs:
`/tmp/ios-a02-diagnostic-native-docs-catalog.log`,
`/tmp/ios-a02-diagnostic-native-docs-links.log`. The seven-file documentation
commit leaves the tested source/full gate unchanged.

A02/class 3/overall goal remain active; class 4 has not started. Read-only next
scan finds 33 anonymous candidates in 19 service files, not 33 confirmed native
losses. `/tmp/ios-a02-diagnostic-native-next-services.log` and
`/tmp/ios-a02-diagnostic-native-next-consumers.log` record remaining Cryptex,
activation, InstallCoordination and pasteboard timers/native causes, old phase/
Duration/seconds values, admission/sleep markers and state rules. Other adapters,
typed guards/created outcomes, DProxy text, stream/JSON/callback/binding and
L01/L03/A03/platform/device conditions remain. All 31 tasks/subtasks and capability
maturity stay. No push/publication, history rewrite or reference checkout edits.

# A02 service deadlines and native sources — 2026-10-03

Implementation `bd6ee91459fd34b2b2f42f419f12b90dccd259da`.
Started from clean `3144ca5` on `feat/device-service-updates-20260927`, Linux
x86_64, Rust 1.97.1/MSRV 1.80.1 and Python 3.12.13. Nine source files.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twenty-one native boundaries now retain their original causes:

| Boundary | Sources |
| --- | --- |
| Cryptex routines and DDI assets | Four Elapsed, three reqwest::Error and one cache-publish IO: eight |
| MobileActivation | Two Elapsed |
| InstallCoordination connect | One Elapsed and one original CoreError |
| Pasteboard and its monitor | Seven Elapsed, one TryFromIntError and one uuid::Error: nine |

Cryptex and MobileActivation use their existing Io variants. InstallCoordination
and Pasteboard have no Io variant and retain sources through existing Xpc(Io).
Public families remain exhaustive, with 11/5/3/7 variants and their original
fields. Additive getters report the former Timeout payloads:
`CryptexError::timeout_phase()`, `MobileActivationError::timeout_duration()`,
`InstallCoordinationError::timeout_duration()` and
`PasteboardError::timeout_seconds()`. They accept their legacy Timeout value or
their private service-issued deadline, inspect at most 32 typed sources and
never format errors. Generic IO TimedOut and unissued Elapsed return None even
when their class is timeout. None does not rule out a timeout. Whole seconds
retain the existing as_secs truncation, including Some(0) for subsecond budgets.
Budget/phase recognition does not establish retry safety or remote cleanup.

Native service timers remain timeout. Received pasteboard integer/UUID faults
remain protocol through InvalidData. Cryptex HTTP timeout/build failures now
classify timeout/invalid_input rather than an opaque Download string; other HTTP
causes retain their own category. Cache publication retains original IO kind/OS
cause instead of Protocol prose, so it can classify io. InstallCoordination
connect retains the original CoreError/category instead of Protocol prose.
Returned variants, Display, immediate get_ref payloads, IO kinds and safe
diagnostic correlation can change. Migrate variant/text/immediate payload
matches to typed class/source inspection and the issuer getters. Explicit
native sources may contain private URLs/input and follow the diagnostic
redaction policy; wrapper Debug/context remain fixed and safe.

Source-free activation admission/poll/sleep and InstallCoordination query/select
deadline outcomes keep their legacy Timeout markers. The former uses deadline
minus its internal started instant, not the original caller's requested total.
No fabricated Elapsed. Activation legacy fallback still recognizes only its
existing initial unsupported probe; timers/malformed probes still propagate.

Private budget/connect/fetch/client-builder/cache-publication helpers were
equivalently extracted before the native-loss baseline. Public Cryptex routine
and preflight remain 30 seconds and installation 600; its next routine still
reconnects after used becomes true. HTTP connect/total stay 30/600 seconds.
Activation self/absolute deadlines and InstallCoordination connect/query budgets
stay. Canonical service metadata checks, command spellings, features, transfer
order/lengths and successful results stay. Pasteboard public operations and
monitor remain 10 seconds. Timed-out clients/subscriptions still take their XPC
owner after the dropped timed future, retain closed/idempotent unsubscribe state,
and the monitor still becomes unusable after failed/cancelled reads. Its idle
notification wait remains unbounded. Other pre-existing errors/reuse policies
and best-effort Drop are unchanged; no new L01 invalidation/cleanup guarantee.

One explicit guard correction compares nonnegative promised size as u64 against
the data limit. The old failed usize conversion could skip the upper bound on
32-bit hosts. Negative size and over-limit values retain their source-free
Protocol/Limit outcomes. Tests cover exact finite limit, excess and i64::MAX;
32-bit behavior is source-reviewed, without a 32-bit runtime claim. Cryptex's
fixed four-path Vec-to-array failure is not a native std::error::Error; that
guard stays. The previous next-consumers log's TryFromSlice label was incorrect.
Optional/lossy metadata/date observers and other guards require separate roles.

Production audit compares the extracted baseline bodies after the named native
callback normalization, preserves five pasteboard owner/state bodies and cache
concurrent-load/removal order, and isolates the one promise comparison change.
MobileActivation send_command normalization includes its two unchanged frame
adapters. Public enums/macro families are exact; Cargo/lock/root exports and
classifier are byte-identical. The shared XPC fixture's feature gates expand
only in test code. Extraction equivalence and fixed wrappers were also reviewed
in the full diff. Evidence:
`/tmp/ios-a02-service-deadline-extracted-before.patch`,
`/tmp/ios-a02-service-deadline-code-audit.log`,
`/tmp/ios-a02-service-deadline-source-diff.patch`.

Sixteen new internal tests and one external consumer cover actual owned H2/XPC
bootstrap with stalled local writes/reads, real service command/session expiry,
owned loopback HTTP request/body timeouts and builder failure, original native
cache rename IO/OS source and concurrent-valid-cache fallback, received integer/
UUID errors, source-free guards, all five pasteboard timer owner transitions,
monitor baseline/read fail-closed state, and bounded cyclic getter sources with
panicking Display. Public exhaustive matches compile for every changed family;
an external real activation request retains its Elapsed and reported duration.
Cryptex connect/preflight and InstallCoordination connect use the exact private
boundary with owned pending/Core futures; their public device entry-point
placement is source-reviewed. Cryptex invoke/install exercise the actual XPC
client and synthetic on-disk assets. Local peer EOF/join and owned temporary-file
Drop establish fixture cleanup only. No external repository download, real DDI,
TSS, pairing, credentials, physical device/daemon/Apple operation, partial-write
reuse, remote stop acknowledgement, every OS/backend fault or cleanup/retry
certification. Cache guards/bounded writes/staging/fallback remain.

Before-fix `/tmp/ios-a02-service-deadline-before.log` records all 13 new native
regression tests FAIL, terminal 101, plus three pre-existing selected tests PASS.
The first attempt did not compile: new fixture keys needed explicit String
conversion (E0283), and a borrowed path temporary needed a binding (E0716),
`/tmp/ios-a02-service-deadline-before-compile.log`. Those fixes changed fixtures
only; the subsequent baseline failures are genuine native losses. The first
all-feature activation group later had 32 PASS/one FAIL because its migrated
test incorrectly expected exactly 10 ms from an internally started absolute
deadline, `/tmp/ios-a02-service-deadline-mobileactivation.log`. The corrected
assertion requires >0 and <=10 ms plus original Elapsed; production budget and
fallback logic remain. No guard or lint weakened, no failure logs removed.

| Check | Result |
| --- | --- |
| Final native_source library group | 19 PASS: sixteen new and three pre-existing; after-final log, terminal 0 |
| All-feature Cryptex/Activation/InstallCoordination/Pasteboard | 20/33/28/30 PASS |
| Corresponding individual no-default features | 20/29/28/30 PASS; activation's four additional TLS fixtures require other features |
| External public_contract all/no-default | 28/13 PASS; public/public-minimal logs |
| Each service's minimum-feature external getter | 1 PASS per feature |
| Strict core Clippy | PASS; clippy-final log |

Logs use `/tmp/ios-a02-service-deadline-` followed by `cryptex`,
`mobileactivation-final`, `installcoordination-final`, `pasteboard-final`,
`{service}-minimal`, `{service}-public-minimal`, `public`, `public-minimal`,
`after-final` or `clippy-final`, plus `.log`. The corrected activation assertion
is additionally exercised in its final service/minimal groups and full gate.

Full validation: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`, first full PASS, terminal
0, `/tmp/ios-a02-service-deadline-full.log`. **1973 Rust/13 Python PASS, zero
FAIL, two existing ignored**. Fmt, check, strict workspace Clippy, MSRV, 66 feature
builds/four test groups, 7+7 doctests/three snippets, C ABI/owned malformed/checked
list, Python 3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records,
38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI and three backup
contracts pass. No separate Chromium or installed 3.9/3.13 rerun. Tested ios
0.1.13 debug SHA256
`735b7a46d62d9b705062febd25c0302cc1a870afa092194b93ee0756fb9f9172`.
Source diff SHA256
`1c2a6d64dab344e1e681c616ae117d47c6e51f45f784d40b7a525ec262d497de`.
Source remained frozen throughout full and afterward. About 8.1 GiB free at
full start and 7.3 GiB at finish; no storage failure or cleanup.

Documentation closeout: 118 capability records, 142 local links across six
changed Markdown files (baseline 3144ca5), unchanged capability implementation/
maturity and historical evidence, one new host evidence entry for the tested
source commit, and whitespace PASS. Logs:
`/tmp/ios-a02-service-deadline-docs-catalog.log`,
`/tmp/ios-a02-service-deadline-docs-links.log`. Seven documentation files are
committed separately; the accepted source/full gate stay unchanged.

A02/class 3/overall goal stay active; class 4 has not started. All 31 tasks and
subtasks remain. A broader read-only service search records 90 candidates in
28 files, including 36 anonymous candidates in 22 files, not confirmed losses;
it includes test branches and deliberate optional/string observers. Logs
`/tmp/ios-a02-service-deadline-next-services.log`, `next-anonymous.log` and
`next-dproxy.log` and `next-consumers.log` share that prefix. ImageMounter/restore HTTP/TSS/ZIP adapters,
MobileBackup2 operation deadlines, AFC/DTX/integer conversions, DProxy text,
typed guards/created outcomes, stream/JSON/callback/binding, L01/L03/A03/platform/
device work remain. No capability maturity upgrade, push/publication, history
rewrite or reference checkout edit.

Follow-up: [DDI/TSS native evidence](image-assets-native-sources-2026-10-03.md)
records `120b07cbfeba1665af62aad0bdc8ad34d47104f6`, 34 original ImageMounter
native boundaries and first full 1991 Rust/13 Python PASS. Download/TSS proxy,
wire, budget/guard and partial-file behavior remain; corrected fixture/negative
control scopes are explicit. Historical service deadline evidence remains;
A02 and the full goal stay active.

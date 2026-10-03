# A02 MobileBackup2 deadlines and retained operation failures — 2026-10-03

Implementation `145d0c41f839a22e757e4aef616885b5303f4d6f`, six source files,
starting clean `01a72928930d6e2ba02582a29d09434b7274dae9` on
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV 1.80.1,
Python 3.12.13. [Error contract](../error-contract.md),
[public contract](../public-contract.md), [roadmap](../engineering-roadmap-2026-09-30.md).

Six original native boundaries retain their actual causes:

| Boundary | Original sites | Final category |
| --- | --- | --- |
| Unback/Extract operation and finish timers | Four Elapsed; equivalent private dispatch shares two callbacks | timeout, formerly protocol |
| Received length-prefixed path | One FromUtf8Error | protocol, unchanged |
| Transfer byte count usize-to-u64 | One TryFromIntError boundary | protocol, unchanged; failure source-reviewed |

Errors use the existing Mobilebackup2Error::Io with fixed retained context.
TimedOut/InvalidData kinds, returned variant, Display/immediate payload and safe
diagnostic correlation can change. Native causes can expose received path bytes
or other private data; explicit inspection follows diagnostic redaction policy.
No native error is fabricated for source-free validation/shape/count/size guards.
The count conversion cannot fail for a usize on this 64-bit host; its callback
is source-reviewed, not fault-tested or claimed as a 128-bit runtime result.

`Mobilebackup2Error::device_operation_timeout()` additively borrows issued
`DeviceOperationTimeout` within at most 32 typed sources, without formatting.
The new type has private fields and no public constructor. `operation()` returns
Unback or Extract; `phase()` returns Operation or Disconnect; both new enums are
non-exhaustive. `duration()` returns the configured total budget shared by both
awaits, not an independently renewed disconnect allowance. Generic IO TimedOut,
foreign Elapsed, protocol prose and cyclic unrelated sources return None. None
is not proof that no timeout occurred.

Previously the finish future owned the operation result. Dropping it on timeout
dropped an earlier failure as well. The result now stays outside that timed
disconnect future. On disconnect expiry, standard Error::source follows the
final actual Elapsed, and classify_error reports timeout. `operation_error()`
separately borrows the earlier Mobilebackup2Error, preserving its native chain
and category. A primary operation timeout followed by disconnect timeout retains
two real Elapsed values on separate branches. This does not pretend that a
linear standard source chain can expose both independent causes.

When the operation succeeded, a later disconnect timeout still returns failure
without the successful Content value; there is no successful-content getter or
new partial-result output. Ordinary best-effort disconnect errors still use
the existing suppression/warning rules and leave the operation result unchanged.
Backup/restore/info/list/password `finish_session` remains byte-identical.

Public error enum remains four variants with unchanged fields; client fields,
public signatures/root paths, features/dependencies/classifier remain. Unback
and Extract use an equivalent private dispatch extracted before the negative
control, with the same public 300-second constant and one absolute deadline.
Local identifier/domain/path validation and layout still precede negotiation;
request field names/optional omission, Hello/version, transfer loop/counters,
free-space state reset and transfer/resource guards stay. There is no new
cleanup budget, retry or fail-closed/reuse rule.

Blocking host work may outlive a dropped awaiter and partial file/stream effects
may remain. Sending a best-effort disconnect is not an Apple acknowledgement.
Remote cleanup, partial-success recovery, cancellation and connection reuse
remain separate L01 work. Source retention and this specific primary-error fix
do not complete that lifecycle task or certify physical backup operations.

Production audit compares the complete Unback/Extract inner bodies, best-effort
disconnect/finish bodies and preceding backup/restore/info/list/password state
orchestration. The entire transfer loop/helper tail and entire production
protocol compare equal after normalizing their sole native mapping callbacks.
The shared dispatch extraction and intentional result-ownership repair are
reviewed separately, including both unchanged absolute timers/public wrappers.
Cargo/lock/root exports/classifier/shared native fixture, DeviceLink and backup
filesystem/manifest/crypto/MBDB are byte-identical. Facade differs only by the
three additive type exports. Evidence uses `/tmp/ios-a02-backup-deadline-`:
`original-{session,protocol,mod}.rs`, `extracted-{session,protocol}.rs`,
`extracted-before.patch`, `code-audit-final.log`, `source-diff.patch`.
Source diff SHA256
`e50dfedd077ef40c0cb989a67a01cc32e5deefc3cbc6c20ffa43da21fe79b749`.

Nine new internal tests cover both operation timer sites, zero budget, both
disconnect sites with primary protocol failure, native peer EOF before a stalled
disconnect, successful Content followed by disconnect expiry, double expiry,
and received malformed UTF-8. One new external consumer verifies the exhaustive
four-variant match, public observation types/borrow/Send+Sync shape and bounded
nonformatting cyclic/foreign/prose issuer behavior. It does not issue a public
five-minute timer. Existing exact request/Unicode/redaction/disconnect and
transfer/resource tests also pass.

Timer fixtures use the actual private dispatch and Tokio timeout_at with owned
temporary layouts and duplex peers. Gated AsyncWrite returns Pending to stall
disconnect; this is an owned transport fixture, not an actual OS write timeout.
The EOF is actual closure of that owned peer. Peer/client futures join within
three seconds; the immediately writable disconnect is read within one second.
Owned fixture layouts are removed on Drop. Production spawn_blocking is not
newly stopped by expiry. Public five-minute placement is source-reviewed, not
waited through on a device or public service fixture.

The baseline includes equivalent private dispatch and the new observation types,
but old native/finish adapters issue none of that evidence. Before.log reports
87 PASS/nine genuine FAIL/one existing ignored, terminal 101. After retaining
native sources but leaving result ownership unchanged, before-primary.log
reports 92 PASS/four genuine FAIL/one ignored, terminal 101: Unback/Extract
protocol failures, native EOF and earlier operation timeout are lost at finish.
Moving result ownership outside the timed future fixes all four. There were no
compile errors, invalid fixtures or weakened guards/lints/budgets; failure logs
remain. This two-stage control distinguishes native-source loss from primary
failure loss rather than counting the same failures twice as new regressions.

| Check | Result |
| --- | --- |
| Final all-feature backup module | 96 PASS/one existing ignored; after-final.log, terminal 0 |
| Independent no-default backup module | 51 PASS; minimal.log, terminal 0 |
| External public_contract | 30 PASS; public.log, terminal 0 |
| No-default public_contract | 14 PASS; public-minimal.log, terminal 0 |
| Core strict Clippy | PASS; clippy.log, terminal 0 |
| Full with workspace .venv Python | First full PASS; full.log, terminal 0 |

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. **2001 Rust/13 Python PASS,
zero FAIL, two existing ignored**. Fmt/check/strict workspace Clippy/MSRV,
66 feature builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/
checked list, Python 3.12 wheel/sdist rebuild/install, 53 tooling tests,
118 help records, 38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI
and three backup contracts pass. No separate Chromium or installed 3.9/3.13
rerun. Tested ios 0.1.13 debug SHA256
`a4f9ab1280248dfdf85145fd7b45dcb781c544fdef6ecebace46d48e06bba826`.
Source remained frozen through full and afterward. About 5.7 GiB free at full
start and 4.4 GiB at finish; no storage failure or cleanup.

Documentation closeout: 118 capability records, 153 local links across seven
changed Markdown files (baseline 01a7292), unchanged implementation/maturity
and historical evidence, one new tested-source host entry and whitespace PASS.
Logs `docs-catalog.log` and `docs-links.log` use this topic's prefix. Eight
documentation files are committed separately; accepted source/full stays.

A02/class 3/overall goal remain active; class 4 has not started. All 31 tasks/
subtasks and capability maturity stay. The next confirmed sources are restore
TSS/IPSW/preflight and H2/XPC control/bootstrap adapters; read-only context logs
`next-restore.log` and `next-restore-context.log` use this topic's prefix. AFC/
DTX/file/symbol/app/test conversions, DProxy text, typed guards/created outcomes,
stream/JSON/callback/binding and L01/L03/A03/platform/device work remain.
No physical backup, credentials, ambient daemon, Apple/device operation,
push/publication, history rewrite or reference checkout edit.

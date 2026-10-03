# A02 DDI download and TSS native sources — 2026-10-03

Implementation `120b07cbfeba1665af62aad0bdc8ad34d47104f6`, seven source files,
starting clean `818dd69704c10006783c1544308be6aa52af8be1` on
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV 1.80.1,
Python 3.12.13. [Error contract](../error-contract.md),
[public contract](../public-contract.md), [roadmap](../engineering-roadmap-2026-09-30.md).

Thirty-four original native boundaries now preserve their causes through the
existing ImageMounterError::Io and fixed private context:

| Boundary | Original sites |
| --- | --- |
| DDI download/cache/extraction | Seventeen native IO, six reqwest::Error, four zip::result::ZipError, one JoinError: 28 |
| TSS serialization/HTTP/response | Two plist::Error, three reqwest::Error, one Utf8Error: six |

The two download GET boundaries share one equivalent private request helper,
so the final production callback counts are 27 download plus six TSS. This
consolidation accounts for the difference from 34 original sites.

ImageMounterError's six variants/fields, public signatures/root exports,
Cargo/features/versions/lockfile, classifier, protocol and manifest remain.
Native IO keeps its original kind/OS cause and category; ZIP format/password
errors classify protocol, with nested ZIP IO preserving its IO kind/source.
HTTP timeout/builder faults classify timeout/invalid_input; other HTTP sources
keep their own category, including unknown. TSS local serialization becomes
invalid_input, received UTF-8/plist becomes protocol, replacing opaque Tss
service_failure. Host JoinError cancellation becomes cancelled; panic remains
unknown rather than inferred cancellation. This is host task status, not device
cleanup acknowledgement. Source-free Tss status/shape/ticket/size guards still
classify service_failure; Download cache/build/HTML/magic/missing-entry guards
stay opaque. No native error is invented for those outcomes.

Affected failures now return Io instead of Download/Tss strings. Display,
immediate get_ref payloads, IO kind, native category and safe diagnostic
correlation can change. Migrate variant/text/immediate payload matches to typed
classification and bounded source traversal. Explicit native causes can contain
private URLs, proxy/input data and paths; follow the diagnostic redaction
policy. Outer Debug and private context are fixed/safe; native causes are not
sanitized or replaced with guessed prose. No new public getter or enum variant.

Private request/body/builder/serialization/extraction-result/file/body-copy
helpers were extracted with old adapters before the regression baseline.
Original header/body, GET/POST, URL/build constants, version/cache selection,
fallback/abort and warn-only cache-write behavior stay. Client construction
still precedes reading environment configuration. HTTPS_PROXY then HTTP_PROXY
precedence stays; invalid explicit proxy still fails in downloads and is ignored
by TSS under its existing permissive policy. Tests supply private builders and
never mutate process environment. TSS public connect/total budgets remain
30/120 seconds and response cap 16 MiB; download builder policy stays. No new
total download/extraction/output/resource bound or L01 cleanup guarantee.

Content-type refusal still precedes creating the spool. ZIP leading-magic check,
archive directory/MACOSX/path filtering and extraction order stay. Personalized
load/build validation, temporary ZIP deletion after successful extraction and
best-effort cache warnings stay. Partial spool/extracted files can still remain
on failure, and blocking work is not newly stopped by dropped awaiters. These
existing lifecycle/resource conditions require separate work; source retention
does not certify retry, rollback or remote release.

Production comparison normalizes only the 27+6 named mapping callbacks across
the entire extracted download/TSS sources, not selected happy paths. This keeps
status/size/ticket/HTML/magic guards, proxy policies and all other bodies exact
after whitespace normalization. Extractions and new fixed wrappers were also
reviewed in the full diff. Protocol/manifest/Cargo/lock/root exports/classifier/
shared native fixture are byte-identical. Evidence:
`/tmp/ios-a02-image-assets-extracted-before.patch`,
`/tmp/ios-a02-image-assets-baseline-{download,tss,mod}.rs`,
`/tmp/ios-a02-image-assets-code-audit.log`,
`/tmp/ios-a02-image-assets-source-diff.patch`; source diff SHA256
`2d82729e2acc6ce02951fde62ab6ce39a2973cc22db5497c8b90df3971fa4fcd`.

Seventeen new internal tests and one external consumer exercise owned standard
and personalized cache IO, cache write failures, extraction file/parent/output
faults, malformed and encrypted ZIP open/entry errors, corrupted stored DMG/
signature/Restore CRC copies, nested ZIP IO, proxy/builder failures, owned HTTP
request/standard-body/spool-body deadlines, original TSS UID serialization and
response UTF-8/plist faults, host join cancellation/panic, source-free refusals
and successful HTTP ticket/header/body behavior. The external cache call uses
the actual public downloader and exhaustive error match without remote fallback.

Body write/flush and ZIP reader faults inject a real error captured from owned
OS file creation into private generic writer/reader fixtures; these are not
actual disk-full/write/flush syscall or every-backend tests. The real file-backed
spool, cache and extraction paths also have native OS fault evidence. HTTP peers
and callers join within a local fixture deadline; peer read errors are tolerated,
so this proves bounded fixture completion, not per-peer EOF. Owned directories
are removed by fixture Drop, including failure cases. Public device/Apple/DDI
host entry placement is source-reviewed; tests use loopback and synthetic
archives/tickets only. No credentials, external download, real TSS, pairing,
mount, device/daemon/Apple operation or remote cleanup/retry certification.

| Check | Result |
| --- | --- |
| Final all-feature ImageMounter group | 30 PASS; after-final.log, terminal 0 |
| Independent no-default imagemounter | 30 PASS; minimal.log, terminal 0 |
| External public_contract | 29 PASS; public.log, terminal 0 |
| Minimum-feature external cache consumer | 1 PASS; public-minimal.log, terminal 0 |
| Core strict Clippy | PASS; clippy.log, terminal 0 |
| Full with workspace .venv Python | First full PASS; full.log, terminal 0 |

Logs above use `/tmp/ios-a02-image-assets-` as their prefix. Before-final.log has
15 PASS/13 FAIL, terminal 101; twelve failures prove native loss, while one ZIP
fixture incorrectly damaged its local header before archive construction and
is excluded from native-loss evidence. Initial before.log had the same invalid
ZIP fixture plus twelve total failures before adding the file-open test.
Before-public.log proves one external native loss, zero PASS/one FAIL.

First after.log has 26 PASS/two FAIL: that ZIP fixture and a TSS sample that did
not reach native plist parsing. The corrected ZIP fixture sets encryption flags
so archive construction succeeds and entry access fails; the corrected TSS
sample includes an XML declaration and an incomplete dictionary. Targeted
before-corrected-zip/plist.log temporarily restore only four old ZIP adapters
and the old response-plist adapter: two existing native IO tests PASS and three
source-loss tests FAIL, terminal 101. These are genuine ZIP/nested IO/plist
negative controls, not a full old-source rerun. All five adapters are restored
to their fixed implementation before final checks. Final native fixtures and
successful HTTP exchange pass without changing production guards/budgets/lints.
Failure logs remain. No compile failure or source/lint weakening.

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. **1991 Rust/13 Python PASS,
zero FAIL, two existing ignored**. Fmt/check/strict workspace Clippy/MSRV,
66 feature builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/
checked list, Python 3.12 wheel/sdist rebuild/install, 53 tooling tests,
118 help records, 38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI
and three backup contracts pass. No separate Chromium or installed 3.9/3.13
rerun. Tested ios 0.1.13 debug SHA256
`828fb95e0be4628f64ee6723e1016a3da771c973a6363354870ab3b4ad623600`.
Source stayed frozen throughout full and afterward. About 6.5 GiB free at full
start and 6.1 GiB at finish; no storage failure or cleanup.

Documentation closeout: 118 capability records, 150 local links across seven
changed Markdown files (baseline 818dd69), unchanged implementation/maturity and
historical evidence, one new tested-source host evidence entry, and whitespace
PASS. Logs `/tmp/ios-a02-image-assets-docs-catalog.log` and
`/tmp/ios-a02-image-assets-docs-links.log`. Eight documentation files are
committed separately. Unreleased records this DDI/TSS migration and catches up
the preceding service getter/promise and diagnostic filter/source API changes;
accepted source/full evidence stays unchanged.

A02/class 3/overall goal remain active; class 4 has not started. All 31 tasks/
subtasks and capability maturity stay. A broader read-only service scan records
59 candidates in 26 files, including tests and deliberate optional/text
observers, not confirmed losses. Logs `next-services.log`, `next-backup.log`
and `next-conversions.log` share the same prefix. MobileBackup2 Unback/Extract
operation and finish deadlines need their primary-error/disconnect-state audit;
restore TSS/preflight, AFC/DTX/file/symbol/app/test conversions, DProxy text,
typed guards/created outcomes, stream/JSON/callback/binding and L01/L03/A03/
platform/device work remain. No push/publication, history rewrite or reference
checkout edit.

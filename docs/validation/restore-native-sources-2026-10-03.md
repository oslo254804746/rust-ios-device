# A02 restore H2/XPC, IPSW and TSS native sources — 2026-10-03

Implementation `d1a210f2ff466ae8f9e5f281e87ebb69823092fa`, nine source files,
starting clean `a5bb2f409eb22f73ad83dee4a6b13121a2bdcb68` on
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV 1.80.1,
Python 3.12.13. [Error contract](../error-contract.md),
[public contract](../public-contract.md), [roadmap](../engineering-roadmap-2026-09-30.md).

Twenty-five original returned-error boundaries retain their objects:

| Boundary | Original sites |
| --- | --- |
| Restore H2/XPC connect/control/bootstrap | Seven H2Error and six XpcError: 13 |
| Local IPSW/preflight | Three ZipError, one plist::Error, one native IO and one TryFromIntError: six |
| TSS build/encode/HTTP/response | Three reqwest::Error, two plist::Error and one Utf8Error: six |

Private fixed context reaches the existing RestoreError::Io. Native IO and
nested ZIP/H2 IO retain their kind/category/OS cause. H2 protocol faults and
received TSS UTF-8/plist remain protocol. XPC categories survive, including input
and opaque unknown. Local IPSW ZIP format/password/manifest and TSS serialization
become invalid_input. Native HTTP builder/expiry becomes invalid_input/timeout;
other HTTP categories remain, including unknown. The firmware size conversion
maps local input. This conversion cannot fail for a valid slice length on this
64-bit host; it is source-reviewed, not manufactured with a fake giant slice.

Former Protocol failures return Io. Display, immediate get_ref payload, kind,
class and safe diagnostic correlation can change. Migrate text/variant/immediate
payload dependencies to typed class and bounded source traversal. Explicit
sources may contain input keys, paths, URLs or received data; follow redaction
policy. Outer Debug/private context stay safe. Original typed H2/XPC guards are
retained as those error objects; no OS cause is fabricated for protocol guards.

The public three-variant error family, client/IPSW/TSS/result fields and public
signatures/root paths stay. No public getter, enum variant or field is added.
Cargo/features/dependencies/versions/lockfile/classifier stay. The private TSS
builder and control-buffer push were extracted with old adapters before the
baseline. Production comparison normalizes only the 13/6/6 named callbacks over
the entire extracted production sources and requires every final callback to
retain through a native helper. Other bodies/guards/policies compare equal;
private extraction and fixed kind/source wrappers receive separate diff review.
H2/XPC codecs and shared native fixture are byte-identical.

Evidence files use `/tmp/ios-a02-restore-native-`: `original-{mod,preflight,tss}.rs`,
`baseline-{mod,preflight,tss}.rs`, `extracted-before.patch`, `code-audit-final.log`,
`source-diff.patch`; source diff SHA256
`8a52dcf3e4502ccca04836275672a40e375b249be6c6a68866d4807708610bd8`.

H2 preface/settings/bootstrap frames/flags/stream IDs, message IDs, control
filters/buffering and reply/one-use preflight behavior stay. Tests confirm the
message ID still increments only after successful request write. No new H2
total/request/cleanup deadline, retry, rollback or fail-closed rule is added.
Failure after a partial write may still leave a client requiring separate
lifecycle handling; this source migration does not certify reuse or remote
cleanup. Scripted connect/bootstrap writes do not prove device acknowledgement.

TSS default connect/total budgets stay 30/120 seconds; Content-Type/User-Agent/
POST/URL, 4 MiB request and 16 MiB response bounds, status/plist/ticket validation,
batch/per-rider fallback and nonce matching stay. Source-free status/body/shape
guards keep Protocol. The request-overflow guard deliberately remains before
serialization-error mapping. Legacy batch fallback consumes combined/individual
errors into failed updater names and needs a separate preserving observation
contract. This work fixes returned adapters and does not claim that partial
batch outcomes now expose every error.

IPSW exact identity/preferred variant, 100,000 entry/256 identity/8 MiB manifest/
64 MiB firmware/depth 64 limits, safe path/duplicate/missing guards, signed XPC
fields and bounded 256 KiB firmware chunk iterator stay. Local malformed package
categories change, while direct selector/path/size/identity guards keep their
old Protocol representation. No new archive allocation, blocking-work stop or
resource bound is claimed.

Fifteen new internal tests and one external consumer cover returned native
failures. H2 tests feed owned scripted SETTINGS/control bytes, accept writes or
inject actual IO errors captured from owned OS file-open failure. Six write
positions cover connect and every bootstrap write/open boundary. The exhausted
scripted reader causes actual read_exact UnexpectedEof; malformed settings keep
H2Error. Request write/read faults keep H2/IO and ID behavior. A malformed XPC
dictionary key keeps XpcError plus native Utf8Error. The buffer guard is tested
through the exact private push helper with the actual bounded buffer limit,
not via an entire live 64 MiB restore response.

Synthetic IPSW tests cover malformed/encrypted ZIP, archive-open and entry-scan
IO injection, corrupted stored-member CRC IO and malformed BuildManifest plist.
Reader/seek fixtures capture native owned OS file-open errors; they are not
actual IPSW filesystem read/seek syscall failures or every backend. CRC/native
serialization/decoding errors are produced by real ZIP/plist codecs. Request
and three fixed bootstrap encode failures are source-reviewed; no multi-GiB
request or unreachable bootstrap error is manufactured. The actual received
decode and buffer boundaries are exercised separately.

TSS tests produce UID serialization, received UTF-8/plist and explicit builder/
endpoint failures, plus real loopback request and body timers. New clients use
no_proxy without changing process environment. Existing successful XML POST/
ticket, HTTP/status/body/request limits and per-rider fallback tests also pass.
HTTP client/peer futures join within two seconds; peer reads tolerate reset,
so completion does not claim per-peer EOF. Owned fixture directories are removed
on Drop. The external consumer calls public local IPSW and TSS APIs with an
exhaustive three-variant match; UID fails before contacting the supplied endpoint.
No real firmware/ticket/backup/credentials/device/daemon/Apple operation occurs.

Before-final.log has 29 PASS/15 FAIL, terminal 101. Fourteen failures prove native
loss; the scan fixture only gated Read and hit the later member-read adapter,
so its expected ZIP scan-source assertion was invalid and is excluded. Initial
before.log used the same fixture before arranging multi-case inputs to execute
before source assertions. Before-public.log proves zero PASS/one genuine FAIL.
First after.log has 43 PASS/one FAIL from that scan fixture. Zip 2.4.2 caches
data offsets while opening; by_index_raw seeks rather than reading the header.
Gating Seek too makes it fail at the scan boundary. Before-corrected-scan.log
temporarily restores only the old scan adapter, reporting zero PASS/one genuine
FAIL, terminal 101. This is a targeted mixed-source control, not a full original
rerun. The fixed scan adapter is restored before final checks. Failure logs
remain; there were no compile errors or weakened production guards/budgets/lints.

| Check | Result |
| --- | --- |
| Final all-feature restore module | 44 PASS; after-final.log, terminal 0 |
| Independent no-default restore feature | 44 PASS; minimal.log, terminal 0 |
| External public_contract | 31 PASS; public.log, terminal 0 |
| No-default restore public_contract | 15 PASS; public-minimal.log, terminal 0 |
| Core strict Clippy | PASS; clippy.log, terminal 0 |
| Full with workspace .venv Python | First full PASS; full.log, terminal 0 |

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. **2017 Rust/13 Python PASS,
zero FAIL, two existing ignored**. Fmt/check/strict workspace Clippy/MSRV,
66 feature builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/
checked list, Python 3.12 wheel/sdist rebuild/install, 53 tooling tests,
118 help records, 38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI
and three backup contracts pass. No separate Chromium or installed 3.9/3.13
rerun. Tested ios 0.1.13 debug SHA256
`675c638f8836ab8e4bfba1aa11beb4960f1498cbe731643b33e2fdea754b7ff4`.
Source remained frozen through full and afterward. About 4.1 GiB free at full
start and 3.6 GiB at finish; no storage failure or cleanup.

Documentation closeout: 118 capability records, 156 local links across seven
changed Markdown files (baseline a5bb2f4), unchanged implementation/maturity
and historical evidence, one new tested-source host entry and whitespace PASS.
Logs `docs-catalog.log` and `docs-links.log` use this topic's prefix. Eight
documentation files are committed separately; accepted source/full stays.

A02/class 3/overall goal remain active; class 4 has not started. All 31 tasks/
subtasks and capability maturity stay. Next: retain combined and per-updater
TSS batch failure observations without changing existing constructible result
fields or retry/nonce/wire policies. Read-only `next-batch-observation.log` and
`next-converters.log` use this topic's prefix. AFC/DTX/file/symbol/app/test/ZIP/GPX
conversions, DProxy text, typed guards/created outcomes, stream/JSON/callback/
binding and L01/L03/A03/platform/device work remain. No push/publication,
history rewrite, reference checkout edit or physical restore/signing acceptance.

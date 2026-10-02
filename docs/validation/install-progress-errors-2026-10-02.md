# A02 fallible streaming-install progress — 2026-10-02

Implementation `5810d16fa3eeaaf949b4226b80dfbc8c1871219d`, eleven related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[usage](../usage.md), [roadmap](../engineering-roadmap-2026-09-30.md).

`apps::install_ipa_with_checked_progress` adds an optional fallible observer.
Its original IO error immediately becomes the existing ZipConduitError::Io,
preserving native kind/source without reading the next reply or reporting success.
The old function/void callback and exhaustive error enum remain available. The
legacy adapter delegates to the same transfer body; no wire key, request,
completion decision or remote-error precedence changes. Completion replies do
not invoke the observer. Returned errors are handled; callback panics are not
caught by this addition.

CLI streaming install uses checked write/flush for startup and progress stderr,
through the shared writer. Existing progress text and diagnostic-ID projection
stay unchanged; successful stdout keeps the versioned install JSON/human result.
Startup failure precedes payload transfer but follows service connection. Progress
failure follows payload transfer and can leave prior or partial stderr lines;
a full line can precede a flush error. No success stdout result follows failure.
Final errors use the safe exit-1 path, phase apps, native class and unknown effects.
Final stderr reporting remains best effort when stderr is unavailable. A host
output BrokenPipe category does not identify a disconnected device.

Observer failure does not establish rollback, absence of device effects or safe
retry. Discard the failed service connection. The core API borrows its stream
and does not close/join it; CLI wiring returns through its existing owner drop.
This is not orderly cancellation or Apple cleanup acknowledgement. Stronger
L01/L03, other callbacks, JSON commands, bindings and lossy adapters remain pending.

| Check | Result |
| --- | --- |
| Core all-feature progress fixtures | Final four PASS, zero FAIL/ignored; `/tmp/ios-a02-install-progress-core-final.log`, terminal exit 0 |
| Core apps-only progress fixtures | Final four PASS, zero FAIL/ignored; `/tmp/ios-a02-install-progress-minimal-final.log`, terminal exit 0; pre-existing reduced-feature warnings remain |
| CLI application fixtures | 45 PASS, zero FAIL/ignored; `/tmp/ios-a02-install-progress-cli.log`, terminal exit 0 |
| External public consumer | 14 PASS, zero FAIL/ignored; `/tmp/ios-a02-install-progress-public.log`, terminal exit 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-install-progress-clippy.log`, terminal exit 0; final full repeats strict workspace Clippy after the final fixture correction |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-install-progress-full.log`, terminal exit 0 |

Seven new tests cover real public install functions over tiny local ZIP IPAs and
owned service byte streams, CLI writer/envelope boundaries and an external API
consumer. Core callback failures (BrokenPipe, PermissionDenied, TimedOut and
Other) occur after two progress replies and the full payload/terminator. Tests
assert original native kind/private source, stable class/safe Debug, two prior
callback observations and exact read position before the buffered completion.
Legacy/checked/no-observer invocations emit identical transfer bytes on the
current implementation, with both DataComplete and nested Complete decisions
and callback order retained. Source diff independently confirms the old transfer
body/wire keys remain unchanged; this is not a prior-release binary comparison.

Remote Error after prior progress still takes precedence over Complete and is
ServiceFailure without inferring timeout/unsupported from its words. Truncated
reply IO/UnexpectedEof and invalid XML integer/Plist/Protocol retain their typed
failures after progress. Temporary IPA directories are explicitly checked absent
after owned cleanup. CLI writer tests preserve the old safe bytes, partial prefix,
native PermissionDenied and full-line/flush BrokenPipe; actual safe envelope
rendering checks class/phase/exit field 1/unknown effects and no private text.
The external consumer retains old/new callback types and exhaustive enum matches,
with original local NotFound before transfer. These prove host source/output/
transfer behavior, not actual paired-TLS CLI startup, physical installation,
stream reuse, cancellation completion or Apple cleanup.

Failure history is retained. First core fixture compilation failed E0283 because
Dictionary key `.into()` left its type ambiguous (`/tmp/ios-a02-install-progress-core.log`).
Using literal str keys corrected the fixture without changing product code.
The next all-feature/apps-only runs had three PASS/one FAIL: a single `?` plist
body contains an inner parser EOF and correctly retains Io, contrary to the
fixture's Protocol expectation (`/tmp/ios-a02-install-progress-core-fixed.log`,
`/tmp/ios-a02-install-progress-minimal.log`). Inspection of locked plist 1.7.1's
`Reader::is_binary` confirms its eight-byte `read_exact` produces the inner IO
EOF for this one-byte body. The final explicit invalid XML
integer tests a genuine Protocol parse error, alongside separate transport
truncation. Native-first classification was not changed or weakened. One docs
patch context mismatch made no writes; inspection and correction resolved it.
Catalog 118 records, 121 changed local Markdown links and whitespace PASS.

Full gate: **1793 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
callback change. The ios 0.1.13 SHA-256 is
`fb786aad30988485cee86ebb5882a1018495befcbfc704a658f784c7f640277a`.

This host slice is accepted; A02/third class and the 31-task goal remain active.
No capability maturity upgrade, device operation, push, publication, history
rewrite or reference-checkout modification occurred. Next registered source
boundary is IPA extraction: ZIP archive/entry and entry-read errors still become
strings. It requires original-source/type/class/migration fixtures, not a claim
of current acceptance. Other JSON/callback/binding/source, L01/L03/A03 and
platform/device conditions remain; fourth class has not started.

# A02 default lockdown sources — 2026-10-02

Implementation `d81fa3f5517dcfbd008dccc4bda963cff8e193c0`, twelve related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python 3.12.13. Starting HEAD `de84394`, clean workspace.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Nine adapters now retain original errors instead of text: request/response
plist, product-version semver, certificate/key PEM IO, rustls client-auth
configuration, server-name parsing, TLS handshake IO and local pairing-record
plist parsing. The missing-file branch also preserves the original filesystem
IO error and OS code instead of replacing it with an identifier-only variant.
Eight adapters use existing LockdownError::Io; parsing and all file-read failures
use existing PairRecordError::Read. Both public enums retain their three
exhaustive variants; no API, dependency, lockfile or feature addition occurs.

The existing private retained-cause implementation moves from device into an
always-compiled error submodule. Its source, fixed context, Debug label and
bounded cached category behavior are preserved; device keeps an internal
re-export. Three generic tests move with it and now run without mdns/tunnel.
The classifier recognizes this private context unconditionally, retaining
Unknown rather than inventing IO evidence for opaque backend errors. Pairing
type classification still follows its existing tunnel gate.

| Failure role | Returned observation |
| --- | --- |
| Local request serialization, certificate/key PEM, invalid server name | IO InvalidInput / `invalid_input`, original plist/IO/DNS source |
| Received plist or ProductVersion syntax | IO InvalidData / `protocol`, original plist/semver source |
| rustls client-auth backend error | IO Other and original category; observed malformed DER is `unknown`, original rustls::Error |
| TLS handshake | Original IO kind/category, original IO object and any backend payload |
| Missing pairing file | Read with original NotFound IO and OS code; `not_found` |
| Malformed local pairing plist | Read with InvalidInput context and original plist::Error; `invalid_input` |

Local-input contexts intentionally express the caller's input role; the original
PEM IO still exposes its InvalidData kind through source access. Filesystem and
TLS kinds remain native. Missing/parse file failures migrate from NotFound/Parse
to Read, and affected lockdown Protocol failures migrate to Io. Source/class
inspection replaces variant/message assumptions. Display prefixes and safe
correlations can change; PairRecordError Display/Debug still hide paths and
credential data. Context Debug hides payloads. Explicit native source/Display
access can expose private data and is not a sanitized logging surface.

TLS provider/verifier, names, parameters and handshake behavior stay. No trust
policy changes, new timeout/retry, request/selector changes or success changes.
ProductVersion still normalizes one/two components to three. File loading still
uses the explicit path, 1 MiB bound and Zeroizing buffer/key. Existing frame
limits and created validation outcomes (empty certificate, absent key, missing
fields, disabled SSL, read-ahead rejection) keep their original behavior and
Protocol strings; no native cause is invented for them.

| Check | Result |
| --- | --- |
| Default core `lockdown::` | 31 PASS; `/tmp/ios-a02-default-lockdown-core-final.log`, terminal 0 |
| All-feature core `lockdown::` | 52 PASS; `/tmp/ios-a02-default-lockdown-core-full.log`, terminal 0 |
| Shared context, default/all-feature | 3/3 PASS; `/tmp/ios-a02-default-lockdown-context.log`, `/tmp/ios-a02-default-lockdown-context-full.log`, terminal 0 |
| Existing device/manual transport, all-feature | 2/18 PASS; `/tmp/ios-a02-default-lockdown-device-context.log`, `/tmp/ios-a02-default-lockdown-manual.log`, terminal 0 |
| External `public_contract`, full/default | 17/10 PASS; `/tmp/ios-a02-default-lockdown-public.log`, `/tmp/ios-a02-default-lockdown-public-default.log`, terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-default-lockdown-clippy.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-default-lockdown-full.log`, terminal 0 |

Nine new core fixtures and one external consumer exercise actual serializers,
framed malformed plist and preserved framing, owned temporary missing/malformed/
valid pairing files, malformed PEM and DER, invalid DNS name, closed TLS write,
complete invalid TLS record/backend source, and a real TLS handshake/application
ping/pong over owned duplexes. Existing offline synthetic certificate/key
fixtures are reused; the existing verifier accepts that certificate as before.
The malformed-peer fixture bounds its ClientHello read to 4096 bytes. Both TLS
peers are joined, and streams drop.
ProductVersion uses the actual method with owned framed responses and an
internally constructed client; this does not establish a full lockdown session.
The external consumer uses root TLS/receive APIs, exhaustive error matches and
Error/Send/Sync. The root constructor future is only type-checked, never polled.

Moved generic tests retain nine native IO kinds, opaque Unknown, at-most-32
cyclic source reads with no formatting, real async Join cancellation and real
blocking Join panic. These do not prove cancellation of already-running blocking
work. No real credential store, device, Apple endpoint, new trust policy or full
pairing/session workflow is exercised. Local join/drop does not certify remote
cleanup, retry safety or stream invalidation/reuse.

The first fixture runs failed with missing borrow lifetime/type inference
annotations (`/tmp/ios-a02-default-lockdown-before.log`,
`/tmp/ios-a02-default-lockdown-before-final.log`, terminal 101). After fixing the
fixtures, unchanged production code yielded **2 PASS / 7 FAIL** across nine
runtime tests (`/tmp/ios-a02-default-lockdown-before-runtime.log`, terminal 101),
showing native source flattening. The first post-fix narrow run was 30 PASS /
1 FAIL: its bad TLS peer sent an incomplete handshake header and waited for an
alert (`/tmp/ios-a02-default-lockdown-core.log`, terminal 101). Replacing that
fixture with a complete record header containing an invalid content type resolved
the timeout. No production timeout, gate or lint requirement was weakened.
Default narrow builds retain five existing tunnel-MTU dead-code warnings; the
all-feature strict Clippy gates pass without new allowances.

Full gate: **1864 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13, built debug
binary SHA256 `754e4c9fb9618a340af415ccf0afb8d5393a3a1ce714eb64927a81c1e64d543b`.
Source remained unchanged after the full gate. No storage failure/cleanup this
slice; the filesystem had about 39 GiB free initially and 28 GiB after full.

Documentation closeout: capability catalog 118 records, 103 local Markdown links
across seven changed Markdown files, and whitespace PASS. Logs:
`/tmp/ios-a02-default-lockdown-docs-catalog.log` and
`/tmp/ios-a02-default-lockdown-docs-links.log`; baseline `de84394` includes this
implementation and documentation follow-up. Source remained unchanged after
the full gate.

A02/class 3/overall goal remain active; class 4 has not started. Supervised-pair
OpenSSL/record-save sources are next, followed by other codec conversions,
legacy created outcomes, stream/JSON/callback/binding work and L01/L03/A03.
Platform/device conditions remain; the full 31-task scope and maturity are
unchanged. No push/publication, history rewrite or reference checkout edit.

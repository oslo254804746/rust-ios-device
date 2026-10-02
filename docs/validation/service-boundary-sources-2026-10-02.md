# A02 symbols, activation and pasteboard sources — 2026-10-02

Implementation `817da67f9442e61f89e676a3dcfe93b3af71741a`, thirteen related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Sixteen remote-symbols H2/XPC error adapters now retain original typed causes
through private fixed-phase wrappers and existing FetchSymbolsError::Io,
replacing flattened Protocol strings. Native IO retains its kind/class; genuine
H2/XPC protocol errors remain protocol. The CLI keeps FetchSymbolsError under
fixed connection context. Public enums, signatures, requests, fields, file/frame
budgets and successful results are unchanged. Returned variants, Display prefixes
and safe correlations can change; migrate affected Protocol matches to typed
sources or classify_error. The raw XPC module remains private; public consumers
can inspect standard IO causes and stable classifications.

Modern ConnectedDevice.activate retains MobileActivationError through existing
CoreError::Io instead of Other(String). Native IO, typed Timeout/Http/Protocol
and nested Plist/native IO causes survive; the legacy route, service command and
subsequent acknowledgement remain unchanged. CLI record application passes its
original CoreError through the existing absolute-deadline wrapper. Expiry keeps
Tokio Elapsed, observed Ctrl-C keeps OperationCancelled, and signal-registration
failure keeps native IO under context. Timeout/cancellation summaries retain
their text; registration context/correlations can change. Selection, deadlines
and fallback conditions are unchanged. Plain EINTR is not cancellation.

Pasteboard notification snapshot connection failures retain CoreError under
existing PasteboardError::Xpc/XpcError::Io rather than a protocol string. Native,
named service-missing and explicit unsupported causes remain distinct. The
same callback supplies baseline and later snapshots; tests below exercise its
baseline failure. Registration can already have occurred before this failure.
CLI success output and error envelope fields remain unchanged; native categories
can replace formerly unknown/protocol classifications. Final errors still have
phase symbols/activation/pasteboard, exit field 1 and unknown effects. Main uses
the parser's canonical command name for phase, independently checked in source.

Owner/new wrapper Debug and safe CLI envelopes hide private data; original
payloads remain accessible to explicit Rust cause inspection. Failure is not
retry/rollback authority or cleanup acknowledgement. Discard failed symbol clients
and notification monitors. These changes do not add borrowed-stream invalidation,
close/join, reuse guarantees or new resource limits.

| Check | Result |
| --- | --- |
| `cargo test -p ios-core --all-features --locked services::fetchsymbols::` | Final 10 PASS, zero FAIL/ignored; `/tmp/ios-a02-service-boundary-core-final.log`, terminal exit 0 |
| Core no-default `--features fetchsymbols`, same filter | 10 PASS, zero FAIL/ignored; `/tmp/ios-a02-service-boundary-fetch-minimal.log`, terminal exit 0; existing reduced-feature warnings remain |
| Core all-feature `device::activation_error::` | Final three PASS, zero FAIL/ignored; `/tmp/ios-a02-service-boundary-activation-core-final.log`, terminal exit 0 |
| Core no-default `--features mobileactivation`, same filter | Three PASS, zero FAIL/ignored; `/tmp/ios-a02-service-boundary-activation-minimal.log`, terminal exit 0 |
| CLI all-feature `cmd::activation::` / `cmd::symbols::` / `cmd::pasteboard::` | 11 / 4 / 12 PASS, zero FAIL/ignored; `/tmp/ios-a02-service-boundary-activation-cli-final.log`, `/tmp/ios-a02-service-boundary-symbols-cli.log`, `/tmp/ios-a02-service-boundary-pasteboard-cli.log`, terminal exits 0 |
| External `public_contract` consumer | Existing 14 PASS, zero FAIL/ignored; `/tmp/ios-a02-service-boundary-public.log`, terminal exit 0; exhaustive enums/signatures retained |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-service-boundary-clippy.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-service-boundary-full.log`, terminal exit 0 |

Fifteen new tests cover seven core symbols, three core activation and five CLI
boundaries. Symbols adapter tests retain seven native kinds, original H2/XPC
variants/payloads, genuine protocol classes and safe Debug without classifying
words. Public remote client fixtures cover closed-duplex write failure, settings
EOF, bootstrap step-1 PermissionDenied, catalog EOF and real malformed control
decode, partial file EOF and successful requested-limit bytes. Other adapter
stages are source-reviewed and share the tested typed mappers; the fixtures do
not inject every one of the sixteen stages separately.

The file fixture uses a complete 10 MiB chunk encoded in legal 16 KiB DATA frames,
then two bytes of an incomplete second chunk followed by EOF. The sink retains
exactly the first chunk and the call returns the original H2/IO failure without
a success byte count. Limited success returns the requested three bytes. Owned
service drop counters show one local release after connect error or explicit
client drop; failed borrowed clients do not automatically certify invalidation.

Activation mapper fixtures retain native IO, typed non-IO variants and both
actual malformed-plist and inner-parser-EOF causes. A real modern service call
over a closed owned duplex produces BrokenPipe through the same mapper used by
ConnectedDevice. This does not execute paired-TLS ConnectedDevice routing or
the acknowledgement step. CLI controlled futures use the production deadline/
signal helper: original CoreError variants/native payloads, actual Elapsed after
a started future, local guard drop, native registration error and observed
cancellation marker. Signal inputs are controlled; no real OS signal is sent.

The actual pasteboard monitor registers before its baseline callback returns
five distinct typed CoreError cases through the production connection/snapshot
path. It attempts one baseline, returns no monitor result, retains the source
and safe envelope, and leaves registration bytes followed by local peer EOF.
This is not a physical snapshot, later-notification/CLI stream workflow or daemon
cleanup acknowledgement. CLI symbols uses the real core connect failure and
public IO cause, with actual safe rendering. All three CLI envelopes retain
class/phase/exit field 1/unknown effects and hide private fixture text.

Failure history is retained. Before-fix closed-duplex regression had zero
PASS/one FAIL: Protocol replaced native Disconnected
(`/tmp/ios-a02-service-boundary-before.log`, terminal 101). First expanded symbols
run had eight PASS/two FAIL (`/tmp/ios-a02-service-boundary-core.log`, terminal
101): the fixture incorrectly supplied one 10 MiB frame, violating the existing
16 KiB inbound limit, and expected malformed control to fail at buffer push
instead of decode. Legal chunked frames and the actual decode phase corrected
the fixtures without weakening product limits/classification. Initial CLI
compilation failed E0603 when its test referenced the private raw XPC namespace
(`/tmp/ios-a02-service-boundary-activation-cli.log`, terminal 101). It now inspects
public native IO and FetchSymbolsError causes; no private module was exposed.
The initial combined source check passed before these test additions; its log is
`/tmp/ios-a02-service-boundary-check.log`, terminal 0.

Full gate: **1817 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
source change. Final ios 0.1.13 SHA-256:
`3f99a3449ee1bd86b00131c08808c186b49307ae4f9eec3b67d907b0a8301f3d`.

Documentation closeout: 118 catalog records, 108 local Markdown links in six
changed Markdown files, and whitespace checks PASS.

This host slice is accepted; A02/third class and the 31-task goal remain active.
Class four has not started. Deactivation's combined fallback and activation HTTP
sources are confirmed remaining losses; other source/JSON/callback/binding work,
L01/L03/A03 and platform/device conditions remain. No capability maturity
upgrade, device operation, push, publication, history rewrite or reference
checkout modification was performed.

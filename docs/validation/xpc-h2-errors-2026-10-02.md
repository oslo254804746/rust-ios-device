# A02 retained H2/RSD transport errors — 2026-10-02

Implementation `96f23e91dbbdd79656586217bff766eff313801b`, nine related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twenty-four H2-to-XPC adapters in RSD and RemoteXPC setup, bootstrap, header/body
reads, message writes and cryptex transfer writes retain the original H2Error
instead of formatting it into Tls(String). A private fixed-context payload lives
inside the existing XpcError::Io variant. Native IO kinds are preserved; H2
protocol failures use InvalidData while retaining the typed protocol source.
Debug hides source payloads. Explicit Display/source access remains subject to
the diagnostic policy; Display gains the existing IO prefix and returned failure
variants/class/correlation can change. The linked migration records this.

Existing public enums/signatures, successful frames, bootstrap ordering, retry
counts/budgets and pending memory limits remain unchanged. The classifier reads
typed categories and custom IO payloads, without matching prose. Protocol/input
strings that have no original source retain their existing Tls representation.

| Check | Result |
| --- | --- |
| `cargo test -p ios-core --all-features --locked xpc::` | Final 88 PASS, zero FAIL/ignored; `/tmp/ios-a02-xpc-h2-source-fixed.log`, terminal exit 0 |
| `cargo test -p ios-core --locked --no-default-features --features tunnel xpc::` | 88 PASS, zero FAIL/ignored; `/tmp/ios-a02-xpc-h2-tunnel-only.log`, terminal exit 0; existing reduced-feature dead-code warnings retained |
| `cargo test -p ios-core --all-features --locked --test public_contract` | 13 PASS; `/tmp/ios-a02-xpc-h2-public-final.log`, terminal exit 0 |
| Core strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-xpc-h2-clippy.log`, terminal exit 0 |
| `python3 scripts/validate.py tooling-tests` | 52 PASS; `/tmp/ios-a02-xpc-h2-tooling.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-xpc-h2-source-full.log`, terminal exit 0 |
| Review | Existing XpcError declaration compared unchanged; 87 changed local Markdown links and whitespace PASS. Representative source checks are not a universal ABI proof |

Five new module tests cover native timeout/permission/broken-pipe/EINTR/EOF kinds,
typed protocol source, fixed context and safe Debug. Controlled AsyncRead/Write
faults exercise setup read/write, XPC/RSD bootstrap and host handshake writes,
message send, RSD header/body reads, and all three connection receive methods.
Partial-body fixtures supply a complete wrapper then fail before its declared
body arrives. Original custom transport errors remain inspectable, including
through std::io::Error::get_ref. The unit framer APIs remain crate-private.

The new independent external consumer calls the public root XpcClient over a
closed owned duplex stream. It retains exhaustive XpcError matching and observes
Disconnected, the adapter IO and original BrokenPipe IO through the source chain.
Owned fault streams drop exactly once; pending-reply accounting is zero after
the tested receive failures. These checks do not establish connection reuse,
partial-write retry safety, physical device reachability or Apple cleanup.
Cryptex adapter wiring is source-reviewed and existing host regressions pass;
this is not new physical transfer/TSS or exhaustive phase-injection evidence.

Two development failures are retained: an initial external test referenced the
private framer module (`/tmp/ios-a02-xpc-h2-public.log`, compile failure), then four
module assertions initially traversed source() without inspecting custom IO
get_ref (`/tmp/ios-a02-xpc-h2-source-final.log`, 84 PASS/four FAIL). Moving private
tests into their module, keeping the separate root consumer, and fixing the
source observation corrected these tests. Public visibility and diagnostic
guards were not weakened; the full gate passed on its first run.

Full gate: **1763 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
Rust adapter change. The ios 0.1.13 SHA-256 is
`8f13a3de4261deaeb3e95a84fffd9b48de3dd5075661cc1086cae1371a6f23c8`.

This host slice is accepted; A02/third class and the 31-task goal remain active.
Remaining binding/void/port, other lossy adapters and stderr/stream/partial
contracts continue. Next is syslog's read-error success exit and scoped stream
fixtures; application mutation JSON remains pending. L01/L03, platform and
device acceptance remain separate. No capability maturity upgrade, device action,
push, publication, history rewrite or reference-checkout change.

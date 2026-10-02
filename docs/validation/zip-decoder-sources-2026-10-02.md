# A02 streaming IPA ZIP decoder sources — 2026-10-02

Implementation `b986ae1547bb16c2c90b4ee09b67817e6ab79d0a`, eight related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Three local extraction adapters retain the original returned ZipError or IO
object through private fixed-context wrappers and existing ZipConduitError::Io.
Previously archive-open, entry-open and entry-read failures became Zip(String).
Known non-IO package reasons keep InvalidInput/invalid_input; native IO retains
its kind and class. InvalidData CRC failures now classify as protocol, rather
than flattened invalid_input; UnexpectedEof/Interrupted/Other remain io. Local
UnsupportedArchive/FileNotFound do not imply device unsupported/service missing.
Future non-exhaustive ZIP reasons retain the cause under Other/generic io;
this is a fallback policy, not a constructed future-variant test.

The public exhaustive enum, legacy callback and both install function signatures
remain unchanged. Existing manually constructed Zip(String) remains invalid
input. Decoder-specific Zip matches must migrate to Io with typed source
inspection or classify_error. Display prefixes and safe correlation can change.
Fixed context is open IPA ZIP archive, open IPA ZIP entry or read IPA ZIP entry;
entry names are no longer added to read-failure context. Owner/new wrapper Debug
and CLI machine diagnostics hide private data; native Rust cause access retains
original payloads. No error text determines classification.

Extraction completes before either public install writes InitTransfer/payload,
flushes, reads replies or invokes the observer. CLI service setup precedes local
decoding, so the real decoder error rendered by CLI keeps phase apps, exit field
1 and unknown effects. Source diff retains successful extraction/wire keys,
metadata, transfer body and completion behavior. Existing public transfer tests
continue comparing legacy/checked/no-observer bytes on the current implementation;
this is not a prior-release binary comparison. Whole-file/in-memory extraction
and resource budgets are unchanged. Host faults do not establish physical
installation, retry, reuse, task join, rollback or Apple cleanup.

| Check | Result |
| --- | --- |
| `cargo test -p ios-core --all-features --locked services::apps::zipconduit::` | 19 PASS, zero FAIL/ignored; `/tmp/ios-a02-zip-source-core-final.log`, terminal exit 0 |
| `cargo test -p ios-core --no-default-features --features apps --locked services::apps::zipconduit::` | 19 PASS, zero FAIL/ignored; `/tmp/ios-a02-zip-source-minimal-final.log`, terminal exit 0; pre-existing reduced-feature warnings remain |
| `cargo test -p ios-cli --all-features --locked cmd::apps::` | 46 PASS, zero FAIL/ignored; `/tmp/ios-a02-zip-source-cli.log`, terminal exit 0 |
| `cargo test -p ios-core --all-features --locked --test public_contract` | Existing 14 PASS, zero FAIL/ignored; `/tmp/ios-a02-zip-source-public.log`, terminal exit 0; exhaustive enum and old/new signatures retained |
| `cargo clippy -p ios-core -p ios-cli --all-targets --all-features --locked -- -D warnings` | PASS; `/tmp/ios-a02-zip-source-clippy.log`, terminal exit 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-zip-source-full.log`, terminal exit 0 |

Nine new tests cover two adapters, five real decoder fixtures, both public
install entries and the CLI error renderer. Adapter tests retain original typed
causes/private native payloads, fixed context, safe Debug and classifications
for seven IO kinds and all four known non-IO ZIP reasons. The real decoder uses
tiny generated ZIPs: malformed archive, damaged local header, password-required
metadata, an actual stored-payload CRC mismatch and valid/empty archives. The
success fixture checks names, directory flags, order and contents. Password
metadata deliberately triggers the decoder's entry-open refusal; it is not a
real encrypted-payload or password-decryption acceptance test.

Both entire public install entries read actual temporary IPA files for all four
failure cases over an owned service. Tests assert retained source/class/context,
zero written bytes/flushes/reply reads/observer calls and no success. The CLI
fixture drives the real public decode failure into the actual safe error
renderer, checking invalid_input/apps/exit field 1/unknown effects and absent
private paths. It does not start a paired-TLS CLI process. Temporary directories
are explicitly checked absent after cleanup.

Failure history is retained. The before-fix archive regression failed zero
PASS/one FAIL because its chain contained no typed ZipError
(`/tmp/ios-a02-zip-source-before.log`, terminal 101). After source retention,
the first expanded run had 16 PASS/two FAIL
(`/tmp/ios-a02-zip-source-core-expanded.log`, terminal 101): the fixture expected
local-header corruption to survive archive construction and fail at entry-open.
Locked zip 2.4.2's central_header_to_zip_file calls find_data_start during new;
its metadata search ultimately returns InvalidArchive for this corruption.
The final fixture uses the actual archive phase and separately covers entry-open
with password-required metadata. Product classification was not weakened to
fit the fixture. The retained source is the original error returned by zip;
library-internal errors absent from that returned cause are not reconstructed.
Primary sources inspected read-only were the locked zip result/read/spec/CRC
implementations and this project's classifier. No Apple wire change or reference
checkout modification occurred.

Documentation closeout: 118 catalog records, 106 local Markdown links in six
changed Markdown files, and whitespace checks PASS.

Full gate: **1802 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 52 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed for this
decoder change. The final ios 0.1.13 SHA-256 is
`edfc4ad10aa102c37ca07b888a50e85690ecb7f521c85b0fb7bffd7a8ef508aa`.

This host slice is accepted; A02/third class and the 31-task goal remain active.
Class four has not started. Registered next losses include CLI remote symbols
connect, activation record application (including inner ConnectedDevice mapping)
and pasteboard XPC startup. Their typed/native fixtures and migrations remain
pending, together with other source/JSON/callback/binding work, L01/L03/L04/Q04,
A03 and platform/device conditions. No capability maturity upgrade, device
operation, push, publication or history rewrite was performed.

# A02 manual-pairing transport sources — 2026-10-02

Implementation `d694cfafa392c3e465bef6bc2eb5e570bbbcaf75`, eight related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Sixteen adapters now retain original objects under existing public
PairingTransportError::Io and existing fixed context/bounded category snapshots:
RSD/bootstrap/encode/decode XpcError, connect/write/header/body H2Error, four
inner PairingError paths, header TryFromSliceError, unlock seal/open AEAD Error
and JSON decode Error. Native IO kinds remain. Named XPC service misses/closed
states preserve their categories; received authentication/JSON/header faults
are protocol. Opaque crypto/encryption stays unknown. The four inner paths,
including M6 verification, retain the already-preserved native inner chain.
Earlier string-only fields cannot be reconstructed; original error objects may
contain them. Wrapper Debug hides payloads; explicit source access is private data.

One additional service dial adapter retains original Tokio Elapsed under
TimedOut, using fixed context instead of an address-bearing string. The source
constant remains 15 seconds. Short private header-length input now returns
native InvalidData/protocol instead of an indexing panic; actual H2 reads remain
24 bytes. Existing body-length guard remains Xpc(String) with its diagnostic:
the shared checked-length helper returns String rather than native Error.
That guard and other created legacy failure/proof strings are not given invented
causes. Further codec/typed guard and lifecycle work remains.

Affected Xpc/Rsd/Crypto-to-Io, Display/correlation and category migration is
recorded. All nine public exhaustive variants, root function/future/signature
and credential fields remain. Requests, sequence/nonces, salts/derivation/signing,
bootstrap/stream choices, limits, trust, retry schedule, remote-error behavior
and successful/optional unlock-key results stay. No dependency, lockfile,
service-feature, binding or CLI schema changes.

| Check | Result |
| --- | --- |
| Core all-feature `pairing_transport::` | 18 PASS; `/tmp/ios-a02-manual-pairing-core.log`, terminal 0 |
| Core no-default tunnel,mdns, same filter | 18 PASS; `/tmp/ios-a02-manual-pairing-minimal.log`, terminal 0 |
| External `public_contract`, full/minimal tunnel,mdns | 16/11 PASS; `/tmp/ios-a02-manual-pairing-public.log`, `/tmp/ios-a02-manual-pairing-public-minimal.log`; terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-manual-pairing-clippy.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-manual-pairing-full.log`, terminal 0 |

Eight new core fixtures and one external consumer cover actual H2 closed write,
header/body read, malformed XPC decode and bootstrap faults; actual H2 connect
failure passed through the production adapter; actual unlock RPC; inner M6/seed/
SRP errors through their adapter; nine native IO kinds, named XPC categories and
opaque encryption; real owned RSD refusal/Elapsed; and short header conversion.
The root API future and exhaustive enum/Error/Send/Sync are externally checked
without polling or a device connection. Minimal mode includes both tunnel and
mdns because this existing public API requires both.

The unlock peer bounds frames at 4096 bytes/four reads, verifies message ID 5,
sequence 9 and the authenticated plaintext request, then sends failed AEAD,
authenticated malformed JSON, valid host key or optional-empty reply. It shuts
down and is joined; the local framer is dropped and sequence remains 10 as before.
Failures return no partial host key. Other fixtures use an owned non-listening
IPv6 socket for actual RSD refusal and an expired pending future for Elapsed.
Typed encryption error projection does not fault-inject Vec encryption. These
do not inject every stage of the public full SRP flow, huge encoder-size failure
or a 15-second network dial timeout. They do not prove stream reuse, rollback
or remote cleanup. No real credential directory, device trust/Apple request or
physical pairing operation.

Failure history: before production changes all four actual write/read/unlock/
short-header regressions failed, zero PASS/four FAIL, terminal 101,
`/tmp/ios-a02-manual-pairing-before.log`. Write/read were misclassified as
protocol, authentication as unknown, and short header panicked on indexing.
The fixes retain source/category and return a checked header error. Narrow
tests and the first full run passed without weakened lint/gate requirements.

Before testing, the filesystem had 25 GiB free and 48 GiB incremental cache;
no cargo/rustc/validate/maturin process was running. Official
`cargo clean -p ios-core` removed 28948 generated files/34.1 GiB, terminal 0,
restoring 58 GiB free (`/tmp/ios-a02-manual-pairing-clean.log`). Source, reference
checkouts and validation records remained. Full ended with about 40 GiB free;
this run had no storage failure.

Full gate: **1854 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun. Tested ios 0.1.13 SHA256:
`fa4e97f1b3447e6584ad452dc259efd9f1a300c2eaf59bd443dff1f7b7c658c0`.

Documentation closeout: capability catalog 118 records, 123 local Markdown
links across eight changed Markdown files and whitespace PASS. Logs:
`/tmp/ios-a02-manual-pairing-docs-catalog.log` and
`/tmp/ios-a02-manual-pairing-docs-links.log`; baseline `94373c1` includes this
implementation and documentation follow-up. Source remained unchanged after
the full gate.

A02/class 3/overall goal remain active; class 4 has not started. At this checkpoint,
default lockdown plist/PEM/rustls/version/pair-record sources were next. The
[default-profile follow-up](default-lockdown-sources-2026-10-02.md) now retains
those sources and missing-file IO, with unchanged TLS/trust/success behavior.
Supervised OpenSSL/record-save sources and other codec conversions,
legacy created outcomes, A02 stream/output/binding, L01/L03/A03 and platform/
device conditions remain. Classification does not authorize retry or certify
cleanup, invalidation/reuse or cancellation of running blocking work. No maturity
upgrade, push/publication, history rewrite or reference checkout change.

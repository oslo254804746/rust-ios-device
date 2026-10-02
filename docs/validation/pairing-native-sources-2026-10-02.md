# A02 connection and pairing native sources — 2026-10-02

Implementation `98aeccb5b4bab66d8431c1eeb9d91bbdacf02b62`, sixteen related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Twenty adapters now retain native causes through existing CoreError::Io and
private fixed context: four discovery/credential JoinError paths, two verification
PairingError paths, two PSK paths, three local credential identity/hex paths,
two remote Base64 paths, four listener AEAD paths, two received public-key slice
conversions and tunnel IPv6 parsing. Three additional dial deadline adapters
retain Tokio Elapsed under TimedOut: endpoint, PSK and control-channel dial.
The existing PSK backend-unsupported variant now exposes its OpenSSL source.

Private context snapshots use the existing bounded typed classification before
wrapping; original opaque Unknown does not become IO merely from the container.
Native Pairing/PSK IO preserves its kind and category. Local credential syntax
is invalid_input; malformed remote encoding/key length/address and failed
response authentication are protocol. Encryption, opaque crypto/backend and
worker panic retain unknown when no precise native category exists. Explicit
Rust source access retains original objects and can reveal private material;
safe Debug hides payloads. Returned Other/Protocol-to-Io and Display/correlation
migration is recorded, with unchanged enums/signatures, success, requests,
routing/retry schedule, timeouts and trust policy.

The same-version chacha20poly1305 dependency enables std so native AEAD Error
implements standard Error. This already std/Tokio workspace does not offer
no_std support. Dependency versions, lockfile, service feature names/defaults/
groups and crypto parameters remain. Minimal/MSRV/full/downstream gates verify
this dependency-feature migration.

| Check | Result |
| --- | --- |
| Core all-feature `device::error_context::` | Five PASS; `/tmp/ios-a02-device-context-core.log`, terminal 0 |
| Core no-default tunnel, same filter | Five PASS; `/tmp/ios-a02-device-context-minimal.log`, terminal 0 |
| Core no-default mdns, same filter | Three PASS; `/tmp/ios-a02-device-context-mdns.log`, terminal 0; tunnel-specific tests absent |
| Core all-feature `device::tunnel_connect::` | Final 17 PASS; `/tmp/ios-a02-pairing-sources-core-final.log`, terminal 0 |
| Core no-default tunnel, same filter | Final 15 PASS; `/tmp/ios-a02-pairing-sources-minimal-final.log`, terminal 0; mdns-specific fixtures absent |
| Core all-feature `device::tunnel_endpoint::` | Six PASS; `/tmp/ios-a02-pairing-endpoint-core.log`, terminal 0 |
| External `public_contract` | Existing 15 PASS; `/tmp/ios-a02-pairing-sources-public.log`, terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | Final PASS; `/tmp/ios-a02-pairing-sources-clippy-final2.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | Final PASS; `/tmp/ios-a02-pairing-sources-full-final.log`, terminal 0 |

Nine new Rust tests comprise five context tests and four tunnel source tests;
two existing endpoint tests now check native address/Elapsed sources. Context
fixtures cover nine IO kinds, original native/opaque sources, cyclic source
classification bounded to 32 reads, cached subsequent classification and zero
source formatting. Real async-task cancellation and real blocking-worker panic
keep JoinError flags. The panic uses a non-string payload, not private error
text. It does not inject failure into full discovery or prove already-running
blocking work can be cancelled. Typed Pairing/PSK mappers retain native kinds,
opaque crypto/protocol and backend-unsupported source. Backend-unsupported is a
typed fixture built from an actual invalid-cipher OpenSSL error; it does not
demonstrate an installed backend lacking PSK.

Actual PSK calls use an owned non-listening TCP socket for refusal and a closed
duplex for handshake write failure, retaining original IO/PSK causes. In-memory
credential loaders exercise native malformed hex, both wrong-length identities
and decoded key mismatch; no real credential directory is read or modified.
The identity error remains the original PairingError::Crypto(String), because
that inner constructor already discards its native slice cause. Both Base64
extraction functions retain DecodeError; received key conversion tests 0/31/33
failure and exact 32-byte success through the helper used by both flows.

The actual RemotePairing listener runs over owned loopback TCP with its existing
framed control channel. Three cases yield malformed Base64, failed AEAD response
authentication, or a valid encrypted port 1234. The peer reads a bounded request,
checks sequence number, writes its frame, shuts down and is joined; the local
control channel is dropped. No partial successful result is returned on failure.
This proves the local native decoding/authentication path and preserved success,
not an actual device listener, direct-XPC authentication, or every listener stage
individually injected. Endpoint timeout uses an actual expired pending future;
the other two dial timer adapters are source-reviewed rather than forced network
timeouts. No pairing, new trust, actual device/Apple call or OS trust change.

Failure history is retained. The initial before-fix fixture had one extra JSON
brace and failed compilation (`/tmp/ios-a02-pairing-source-before.log`, terminal
101). After correcting only the fixture, zero PASS/one FAIL reproduced Unknown
instead of protocol and discarded Base64 source
(`/tmp/ios-a02-pairing-source-before-final.log`, terminal 101). Initial all-target
core/CLI check passed (`/tmp/ios-a02-pairing-sources-check.log`, terminal 0).
Initial strict Clippy rejected two err().expect() calls for Debug-capable PSK
results (`/tmp/ios-a02-pairing-sources-clippy.log`, terminal 101); expect_err fixed
them without suppressing a lint or changing runtime behavior. Final Clippy after
the public-key conversion addition passed.

The first full gate passed Rust/Python/MSRV, all feature/group tests and docs,
then failed building the FFI archive with NoSpace/os error 28
(`/tmp/ios-a02-pairing-sources-full.log`, terminal 1). It does not count as C ABI
acceptance. The 194 GiB filesystem had 159 MiB free; target used 105 GiB,
including 68 GiB incremental cache. No cargo/rustc process remained. Direct
rm-f cleanup was rejected by execution policy and did not execute. Official
`cargo clean -p ios-core` removed 64625 generated files/67.9 GiB, terminal 0,
restoring 65 GiB free without source/reference/validation-record changes
(`/tmp/ios-a02-pairing-sources-disk-clean.log`,
`/tmp/ios-a02-pairing-sources-disk-recovery.log`). The full rerun passed at
`/tmp/ios-a02-pairing-sources-full-final.log`, terminal 0. No source or gate
requirement was weakened; the original failure remains recorded.

Full gate: **1840 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed. Tested
ios 0.1.13 SHA256:
`833df7056404ca8dbf7cd6f16a1d15cf6759360cde49f11020f0eb1b8a4ee927`.

Documentation closeout: capability catalog 118 records, 112 local Markdown
links across six changed Markdown files, and whitespace PASS. Link log:
`/tmp/ios-a02-pairing-sources-docs-links.log`; baseline `a36853b` includes the
implementation and documentation changes.

A02/class 3/overall goal remain active; class 4 has not started. At this checkpoint,
inner credential slice, HKDF, cipher key-init/seal/open and OPACK adapters still
discarded native sources. Subsequent source review corrected the earlier
signature-validation hypothesis: signing is infallible and SRP proof returns
bool here; the pairing module/type itself is tunnel-gated. The
[inner-source follow-up](inner-pairing-sources-2026-10-02.md) retains these seven
adapters; at that checkpoint the public manual-pairing transport still flattened
four inner causes. The later [manual transport follow-up](manual-pairing-sources-2026-10-02.md)
retains those and other original transport/codec objects. The later
[default-profile follow-up](default-lockdown-sources-2026-10-02.md) moves the
shared context and three generic tests into the always-compiled error module,
retaining default lockdown native sources and missing-file IO. Supervised
OpenSSL/record-save and other A02 JSON/stream/callback/binding boundaries,
L01/L03/A03 and platform/device acceptance remain. Error classification does
not authorize retry or prove remote cleanup, client invalidation or reuse; outer
cancellation still does not stop already-running blocking work. No capability
maturity upgrade, push/publication, history rewrite or reference checkout edit.

# A02 inner pairing native sources — 2026-10-02

Implementation `9fa4ff78be9b30199188250ab63abb1869add4f6`, eight related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Seven adapters in the crate-private lockdown pairing module retain native
objects under existing PairingError::Io and the existing fixed-context,
bounded-category snapshot. They cover private-seed slice conversion, HKDF
expansion, both cipher key initializations, AEAD seal/open and device-info
OPACK encoding. Native length errors are invalid_input; received authentication
failure is protocol, without inferring permission rejection. Opaque encryption
stays unknown, and OPACK keeps its typed category. The two in-memory credential
loader fixtures now require the original TryFromSliceError below PairingError.
Wrapper Debug hides payloads; explicit Rust source access remains private data.

HKDF enables its same-version std feature so InvalidLength implements standard
Error. No dependency version/lockfile/service feature or crypto parameter
changes. Fixed 32-byte derivation, salts/info, nonces, AEAD/signing, successful
wire fields, optional M6 handling and trust behavior remain. Internal returned
Crypto/Protocol-to-Io, Display/correlation and category migration is documented.
PairingError's three variants remain; its now-unused legacy Protocol variant has
a targeted explained dead-code allowance, with all other warnings checked.

The module and type exist only with tunnel, so the classifier's existing gate
is correct; it was not removed. Default builds do not have this type. Ed25519
signing here is infallible and SRP proof verification returns bool; the prior
signature-source hypothesis was incorrect. The actual extra loss was OPACK.
At this checkpoint, the separate public manual-pairing transport still flattened
four inner errors into PairingTransportError::Crypto(String), including M6. This slice does not
claim native categories survive that full handshake; those outer adapters are
next work, followed by default-profile lockdown sources. The subsequent
[manual transport follow-up](manual-pairing-sources-2026-10-02.md) retains those
four outer chains and other native transport/codec errors; full device pairing
and remaining default/codec/lifecycle conditions still require separate evidence.
The later [default-profile follow-up](default-lockdown-sources-2026-10-02.md)
retains default lockdown sources, while supervised/codec/lifecycle work remains.

| Check | Result |
| --- | --- |
| Core all-feature `lockdown::pairing::` | Final 16 PASS; `/tmp/ios-a02-inner-pairing-core-final.log`, terminal 0 |
| Core no-default tunnel, same filter | Final 16 PASS; `/tmp/ios-a02-inner-pairing-minimal-final.log`, terminal 0 |
| Core all-feature `device::tunnel_connect::` | Final 17 PASS; `/tmp/ios-a02-inner-pairing-tunnel-core-final.log`, terminal 0 |
| Core no-default tunnel, same filter | Final 15 PASS; `/tmp/ios-a02-inner-pairing-tunnel-minimal-final.log`, terminal 0 |
| Existing external `public_contract`, full/minimal tunnel/default | 15/10/9 PASS; `/tmp/ios-a02-inner-pairing-public-final.log`, `/tmp/ios-a02-inner-pairing-public-minimal-final.log`, `/tmp/ios-a02-inner-pairing-public-default-final.log`; terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-inner-pairing-clippy-final.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-inner-pairing-full.log`, terminal 0 |

Five new Rust fixtures test actual constructor rejection at 0/31/33 bytes and
32-byte identity/signing success; actual M6 wrong key, modified ciphertext,
truncated tag and wrong nonce plus valid altIRK extraction; actual oversized
identity OPACK rejection; native crypto adapter behavior; and fixed success
values. HKDF values are independently computed with Python hmac. Authenticated
M5 plaintext preserves state, identity, public key, seven info fields and a
verified Ed25519 signature. Existing optional malformed M6 cases still succeed
without an optional key. Original enum matching and Error/Send/Sync are checked
inside the module. Existing external consumers retain their public API checks.

Real library HKDF/key-length errors and AEAD encryption failure from an owned
buffer without tag space exercise the production native-context adapter. They
do not fault-inject the fixed 32-byte production HKDF/key initialization or its
Vec encryption call. Fixtures use only bounded synthetic in-memory data; no
real credential directory, paired device, Apple request or trust modification.
They do not establish full manual pairing, stream reuse or remote cleanup.

Failure history is retained. Before production changes, three actual constructor/
M6/OPACK regressions all failed: zero PASS/three FAIL, terminal 101,
`/tmp/ios-a02-inner-pairing-before.log`. Initial pairing tests subsequently
passed 16 (`/tmp/ios-a02-inner-pairing-core.log`, terminal 0), but an added
external test incorrectly imported crate-private lockdown and failed E0603
(`/tmp/ios-a02-inner-pairing-public.log`, terminal 101). That invalid fixture
was removed without widening API visibility; internal type checks and the
existing external consumer passed on the final source. No runtime requirement
or full gate was weakened. The first full run passed.

Full gate: **1845 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun. Tested ios 0.1.13 SHA256:
`f7ba544e466b1c089bd86caeb652af0a02b3db99ae58f623a9b32dad1097dc3c`.

Documentation closeout: capability catalog 118 records, 117 local Markdown
links across seven changed Markdown files, and whitespace PASS. Logs:
`/tmp/ios-a02-inner-pairing-docs-catalog.log` and
`/tmp/ios-a02-inner-pairing-docs-links.log`; baseline `a256d37` includes this
implementation and documentation follow-up. Source remained unchanged after
the full gate.

A02/class 3/overall goal remain active; class 4 has not started. At this checkpoint,
outer manual pairing RSD/H2/XPC/bootstrap/crypto/JSON needed retention; the
subsequent transport evidence above records that follow-up. Default lockdown
PEM/TLS/plist/version/pair-record sources remained at that checkpoint; the
default-profile evidence above records their follow-up. Supervised OpenSSL/
record-save and other codec/legacy sources remain.
Other A02 stream/output/binding,
L01/L03/A03 and platform/device conditions remain. Classification does not
authorize retry or prove cleanup, invalidation/reuse or cancellation of running
blocking work. No maturity upgrade, push/publication, history rewrite or
reference checkout modification.

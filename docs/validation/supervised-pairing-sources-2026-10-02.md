# A02 supervised pairing sources — 2026-10-02

Implementation `b642afef32d8d575d45cbbdfd0a0ba7df8193b3a`, three related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python 3.12.13. Starting HEAD `e452ea2`, clean workspace.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

All 53 native-to-string adapters in supervised pairing now retain their original
objects through existing LockdownError::Io and the shared private fixed context:
51 OpenSSL ErrorStack, one plist::Error and one filesystem IO error. The stacks
retain original codes and entries; wrapper Debug omits payloads. Of the OpenSSL
adapters, 48 backend/certificate/key/signing paths use Other with cached Unknown,
two local PKCS12 parse/unlock paths use InvalidInput, and the received device
public-key parser uses InvalidData/protocol. The local save serializer preserves
its native category; file writing preserves original IO kind/category/object.
No category is inferred from OpenSSL prose or guessed to be permission denial.

An additional native NulError adapter rejects interior NUL in a password after
DER parsing and before OpenSSL parse2. The dependency's Rust wrapper otherwise
unwraps CString construction and panics. Valid passwords take the existing
parse2 path without a second CString allocation; invalid input is InvalidInput
with fixed context and the original NulError. The native error retains password
bytes, so explicit source/Debug access is private credential data, not sanitized
logging. Context Debug and the existing outer Debug remain safe.

Affected Protocol failures now return Io; Display prefixes/correlations and
categories change. Opaque backend/signing failures move from protocol to unknown;
local PKCS12 credentials become invalid_input, malformed received public keys
remain protocol, and filesystem failures use native categories. The public
three-variant exhaustive LockdownError, root pair_supervised/save_pair_record,
FullPairRecord fields, signatures and feature gates remain. No dependencies,
lockfile, bindings or CLI schema change. Earlier string-only outcomes cannot
recover native objects; created missing certificate/key and device-response
validation failures keep their existing Protocol outcomes.

Success/control-flow source comparison is identical after removing error
adapters and the new NUL-password guard. RSA-2048, certificate fields/serial/
validity/extensions/SHA1 signing, PKCS7 BINARY, request keys/order, private-key
stripping from wire, generated IDs, escrow results and save format remain.
The existing owner-only file helper, replacement/temp cleanup and parent
directory policy stay; no new retry, timeout, TLS/trust or lifecycle policy.
Raw Rust source/Display inspection may expose private paths/material; safe
machine output surfaces and their contracts remain separately validated.

| Check | Result |
| --- | --- |
| Core no-default supervised-pair `lockdown::supervised_pair::` | Initial post-fix 13 PASS, final 14 PASS; `/tmp/ios-a02-supervised-pairing-core.log`, `/tmp/ios-a02-supervised-pairing-core-final.log`, terminal 0 |
| Core all-feature `lockdown::` | 61 PASS; `/tmp/ios-a02-supervised-pairing-core-full.log`, terminal 0 |
| External `public_contract`, all-feature/minimal supervised-pair | 18/11 PASS; `/tmp/ios-a02-supervised-pairing-public-final.log`, `/tmp/ios-a02-supervised-pairing-public-minimal.log`, terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | PASS; `/tmp/ios-a02-supervised-pairing-clippy.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-supervised-pairing-full.log`, terminal 0 |

Nine new core fixtures and one external consumer cover actual DER/password/
NUL rejection before writes, malformed received public key with the actual
GetValue request, real root/signed certificate builder errors, a real PKCS7
failure after the first Pair request, save errors/success and a complete public
supervised exchange over owned duplexes. Backend faults use synthetic Ed25519
keys with incompatible signing operations; this tests actual builder/PKCS7
boundaries, not injected failure into fixed production RSA generation. No
allocator, each extension/PEM/DER conversion, fixed save serializer or every
filesystem stage is independently fault-injected.

The complete root exchange bounds each fixture frame at 64 KiB, reads exactly
three requests and verifies GetValue, first Pair and signed Pair. It checks
certificate version/serial/empty subjects, RSA certificate signatures against
the generated root, device key, unchanged PairRecord across requests, absence
of private keys on wire, supervisor DER and exact signed binary challenge.
PKCS7 mathematical verification uses the known ephemeral synthetic signer,
NOINTERN and test-only NOVERIFY; it still verifies the signature, without
certifying chain trust. The production flags remain BINARY. Returned escrow and
host certificate/key match; the peer shuts down and is joined, then host drops.

Actual save tests use UUID-owned directories, preserve obstructing file/directory
targets, observe staging cleanup, reject NUL paths, load the saved record, check
extra escrow/root-key fields and verify Unix 0600/0700. The external consumer
uses only root APIs and existing public record fields, with native source checks.
It does not establish a usbmux/physical supervised-device session, enrollment,
Trust dialog behavior, remote cleanup, stream reuse or retry safety. No real
credential directory, device or Apple endpoint is touched. FullPairRecord's
existing Vec key storage/lifecycle remains separate A03/L01 work.

Before production changes, eight fixtures yielded **2 PASS / 6 FAIL**, including
the actual dependency NUL-password panic (`/tmp/ios-a02-supervised-pairing-before.log`,
terminal 101). The ninth core fixture then added an actual PKCS7 backend fault.
Initial external validation yielded 17 PASS / 1 FAIL because its NUL-path save
used shared /tmp: the existing parent permission policy rejected that directory
before reaching the intended filename validation. `/tmp` remained root-owned
1777; no permission change occurred. The fixture was corrected to a UUID-owned
directory, and final external tests passed. Original failure log:
`/tmp/ios-a02-supervised-pairing-public.log`, terminal 101. No production policy,
gate, lint or timeout was weakened.

Full gate: **1874 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug binary
SHA256 `a8718b15ee20032bfe1e053bdf5255add2aa9c369d533a8c66bb9872bd328673`.
Source remained unchanged after full. No storage failure/cleanup; about
28 GiB free at the start and 27 GiB after full.

Documentation closeout: capability catalog 118 records, 94 local Markdown links
across five changed Markdown files, and whitespace PASS. Logs:
`/tmp/ios-a02-supervised-pairing-docs-catalog.log` and
`/tmp/ios-a02-supervised-pairing-docs-links.log`; baseline `e452ea2` includes this
implementation and documentation follow-up. Source remained unchanged after
the full gate.

A02/class 3/overall goal remain active; class 4 has not started. Subsequent
[MCInstall source coverage](mcinstall-sources-2026-10-02.md) addresses its analogous
PKCS12 password boundary and native-source adapters; this page retains the
supervised-pairing gate counts above. Base mux/archive/codec integer sources, typed guards,
created outcomes, stream/JSON/callback/binding, L01/L03/A03 and platform/device
conditions remain. All 31 tasks/subtasks and maturity stay. No push/publication,
history rewrite or reference checkout change.

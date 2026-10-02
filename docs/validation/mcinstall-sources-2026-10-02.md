# A02 MCInstall native sources — 2026-10-02

Implementation `f1f9d37665d969011bd3b69970b03febe1ee0f45`, three related files.
Branch `feat/device-service-updates-20260927`, starting clean HEAD `1def819`.
Linux x86_64, Rust 1.97.1, MSRV 1.80.1, Python 3.12.13.
[Error contract](../error-contract.md), [public compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

Six native-to-string adapters are removed: five OpenSSL ErrorStack adapters now
retain the original object under existing McInstallError::Io and private fixed
context; the remove-profile identifier encoder propagates existing Plist with
its original plist::Error. PKCS12 DER/password errors are invalid_input. Certificate
DER, signing-stack allocation and PKCS7 signing/encoding errors remain unknown
through cached native classification. No backend prose determines a category.

After DER parsing, interior NUL passwords return invalid_input with the original
NulError before the dependency's parse2 CString unwrap can panic. Valid passwords
take the existing path without an extra CString copy. NulError owns the original
password bytes: explicit source/Debug access is private credential data. Context
Debug and the existing outer diagnostic Debug omit these bytes.

Two feature-disabled public entry points retain a typed CoreError::Unsupported
decision through Io. This is an explicit feature admission outcome, not a
discarded backend error. Two parsed missing certificate/key predicates return
InvalidInput IO without inventing an underlying ErrorStack. Incomplete-P12
coverage observes a real missing field; it does not independently force both
Option branches. Existing PasscodeSet is precondition, and manually constructed
legacy Crypto(String) remains unknown. Public McInstallError still has exactly
five variants; root exports, signatures, dependencies and feature definitions stay.

Affected Crypto matches should migrate to class/source observations: returned
failures now use Io; remove-profile encoder Protocol becomes Plist (still protocol).
Display prefixes, safe correlations and categories can change. Earlier strings
cannot recover native objects. Raw native Display/source access remains private
data; machine diagnostics retain their separate safe-output contract.

Source comparison of the escalation flow is identical after excluding error
adapters, the NUL guard and optional trailing commas. PKCS7 BINARY, certificate
DER, request keys/order, challenge handling, existing zeroization and ACK gates
stay. Existing best-effort profile-list fallback remains. No timeout, retry,
trust, stream lifecycle, CLI schema, binding, allocation budget or wire policy
is added. The existing profile metadata `u64 as i64` conversion remains pending
integer/typed-guard work; this slice does not guess a wire representation.

| Check | Result |
| --- | --- |
| Core MCInstall, no-default mcinstall / mcinstall+supervised-pair / all features | 25 / 31 / 31 PASS; `/tmp/ios-a02-mcinstall-core-{basic,supervised,full}.log`, terminal 0 |
| External public_contract, all / mcinstall / mcinstall+supervised-pair | 19 / 11 / 12 PASS; `/tmp/ios-a02-mcinstall-public{,-basic,-supervised}.log`, terminal 0 |
| Core/CLI all-target/all-feature strict Clippy | PASS; `/tmp/ios-a02-mcinstall-clippy.log`, terminal 0 |
| Success/control-flow source comparison | PASS; `/tmp/ios-a02-mcinstall-code-audit.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | First full PASS; `/tmp/ios-a02-mcinstall-full.log`, terminal 0 |

Nine distinct new internal fixtures compile as eight with supervised-pair and
two without (the serializer fixture is shared); one external root consumer is
also added. Actual cases cover disabled-feature admission without writes,
serializer failure, malformed DER/wrong password/NUL on both entry points,
incomplete parsed P12, real PKCS7 failure with an ephemeral Ed25519 identity,
closed duplex IO, received malformed plist and failure at each of four ACK gates.
The generic serializer fixture preserves an already-native send_plist path;
it does not inject failure into the fixed Value remove-profile encoder. Certificate
DER, allocator and each combined signing/encoding substage are source-reviewed,
not independently fault-injected.

The complete public silent-install fixture uses an owned duplex, 64 KiB fixture
frame bounds and a 15-second test bound. It observes Escalate, EscalateResponse,
ProceedWithKeybagMigration and InstallProfileSilent in order, verifies supervisor
DER and the exact binary challenge signed by the known ephemeral signer, checks
payload and ACK gates, shuts down/joins the peer and drops the host. Test-only
PKCS7 NOVERIFY/NOINTERN verifies the mathematical signature without certifying
chain trust; production flags remain BINARY. This is not physical supervision,
profile installation, Trust, remote cleanup, stream reuse, retry safety or new
memory-wiping evidence. No real credentials, devices or Apple endpoints are used.

Before the production fix, final regression fixtures yielded **5 PASS / 5 FAIL**
across both feature modes (ten executions of nine distinct fixtures), including
the actual dependency NUL panic: `/tmp/ios-a02-mcinstall-before-final.log`, terminal
101. Earlier `/tmp/ios-a02-mcinstall-before.log` also ended 101 but included an
invalid empty-P12 fixture rejected by OpenSSL's builder. That fixture was corrected
to a real certificate-only bundle before the final pre-fix run. The fixture
construction failure is not counted as a production regression; original logs
remain. No production policy, gate, lint or timeout was weakened.

Full gate: **1883 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four test groups,
7+7 doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 rerun. Tested ios 0.1.13 debug binary
SHA256 `2319b64f23b52af9a96d3c820ed37ce189a4606b41c7b80163a64b0ef49e3034`.
Source remained frozen through full and unchanged afterward. No storage failure
or cleanup; about 26 GiB remains free.

Documentation closeout: capability catalog 118 records, 96 local Markdown links
across five changed Markdown files (baseline `1def819`), and whitespace PASS.
Logs: `/tmp/ios-a02-mcinstall-docs-catalog.log` and
`/tmp/ios-a02-mcinstall-docs-links.log`. Documentation changes do not alter source
or the accepted full gate.

A02/class 3/overall goal remain active; class 4 has not started. Subsequent
[usbmux source coverage](usbmux-sources-2026-10-02.md) addresses base codec/TCP
adapters; this page retains the MCInstall gate counts above. Archive native
sources, integer conversions/typed guards/created outcomes, other
stream/JSON/callback/binding and L01/L03/A03/platform/device conditions remain.
All 31 tasks/subtasks and maturity remain. No push/publication, history rewrite
or reference checkout changes.

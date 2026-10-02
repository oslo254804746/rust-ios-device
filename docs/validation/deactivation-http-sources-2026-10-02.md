# A02 deactivation fallback and activation HTTP sources — 2026-10-02

Implementation `87e2b4789d1334ffe44e7b666d17c01eff8d949a`, seventeen related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, full-gate Python 3.12.13.
[Error contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

ConnectedDevice.deactivate preserves the original one-fallback schedule. Modern
service failure uses the existing source-preserving activation adapter. When
both attempts fail, existing CoreError::Io retains the additive private-field
DeactivationFallbackError, available at root/device with mobileactivation.
The IO custom payload provides borrowed original CoreError getters and cached
classifications. Its standard source is the final fallback failure; the earlier
failure remains separately observable. Classification uses the final category,
including Unknown, rather than overriding it with the earlier precise error.
Fallback connection failure also retains the earlier cause. Primary success
skips fallback construction; fallback success still returns Ok without the
earlier-failure observation. No additional retry or device cleanup guarantee.

Activation HTTP build/send/chunk failures retain original reqwest objects under
private fixed-phase wrappers and the existing MobileActivationError::Io. Endpoint
URL parse keeps its original cause; HTTPS/credentials/custom-opt-in rejections
use private typed InvalidInput payloads. Native timeout and builder evidence
and underlying typed sources classify before generic IO fallback. HTTP status
and request/response body-budget rejection remain Http/service_failure. Client
Debug hashes both URLs. Original Rust Display/source access can reveal private
URLs, while safe Debug/CLI error envelopes hide them.

Returned variants, Display prefixes and safe correlations migrate explicitly:
combined Other becomes Io, native HTTP Http becomes Io, timeout becomes timeout,
and endpoint rejection becomes invalid_input. Public exhaustive enums,
signatures, requests, fallback conditions, HTTPS verification, exact official
endpoint policy, custom opt-in, redirects, timeouts, budgets, headers and success
response fields remain. Classification is not retry or rollback authority.

| Check | Result |
| --- | --- |
| Core all-feature `deactivation_error` | Four PASS, zero FAIL/ignored; `/tmp/ios-a02-deactivation-host-final2.log`, terminal 0 |
| Core no-default `--features mobileactivation`, same filter | Four PASS, zero FAIL/ignored; `/tmp/ios-a02-deactivation-minimal.log`, terminal 0 |
| Core all-feature `services::mobileactivation::` | 30 PASS, zero FAIL/ignored; `/tmp/ios-a02-activation-http-core-final.log`, terminal 0 |
| Core no-default mobileactivation, same service filter | 26 PASS, zero FAIL/ignored; `/tmp/ios-a02-activation-http-minimal.log`, terminal 0; four TLS fixtures require tunnel/OpenSSL and are absent in this build |
| External `public_contract` | 15 PASS, zero FAIL/ignored; `/tmp/ios-a02-deactivation-http-public.log`, terminal 0 |
| CLI all-feature `cmd::activation::` | 12 PASS, zero FAIL/ignored; `/tmp/ios-a02-deactivation-http-cli.log`, terminal 0 |
| Core/CLI strict all-target/all-feature Clippy | Final PASS; `/tmp/ios-a02-deactivation-http-clippy-final.log`, terminal 0 |
| Tooling regression suite / actual CLI inventory | 53 tests / 118 records PASS; `/tmp/ios-a02-deactivation-http-tooling.log`, `/tmp/ios-a02-deactivation-http-inventory.log`, terminal 0 |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | Final full PASS; `/tmp/ios-a02-deactivation-http-full-final.log`, terminal 0 |

Fourteen added Rust tests comprise four deactivation, eight HTTP, one public consumer
and one CLI fixture. Aggregate tests verify skipped/exactly-once fallback,
borrowed variants/source identity, final typed and opaque Unknown classes, safe
Debug, and owned non-listening TCP connection failure retaining both causes.
A real mobileactivation deactivate over closed duplex and the actual legacy
lockdown frame writer yield native BrokenPipe retained in both observations.
This fixture does not instantiate a paired LockdownClient/session or execute
ConnectedDevice's device/service routing. External consumer verifies public
getter signatures and Error/Send/Sync; it does not manufacture private fields.

Four HTTP fixtures without tunnel use public endpoint rejection, actual native
builder failure, owned HTTPS refusal, stalled handshake timeout and URL Debug.
Endpoint checks cover parse, plaintext HTTP, credentials and missing custom
opt-in. Refusal retains reqwest's original private URL and native connect flag;
stalled handshake retains its timeout flag. The listener task is aborted and
joined before assertions, so failed assertions do not leave an owned task.

Four tunnel-gated TLS fixtures generate an ephemeral CA/leaf/key in memory,
without writing/logging certificates or private keys. Positive fixtures add
their CA only to the private test client's trust roots, with verification still
enabled and the existing HTTPS/redirect policy. They test body truncation,
body timeout after two received bytes, complete body/header success and HTTP
403 rejection. No partial successful response is returned after read failure.
The public constructor with its default roots rejects the untrusted fixture
certificate and retains the native reqwest cause. This adds no production
custom-CA interface or OS trust modification and makes no Apple request.

The CLI fixture passes public HTTP input and timeout errors through the actual
safe renderer; the timeout also uses the production deadline helper. Original
MobileActivationError/native source survives, with activation phase, exit field
1, unknown effects and hidden private query/credential values. It does not run
an entire paired CLI/device process or inject the aggregate from outside core.

Failure history is retained. Before the HTTP fix, zero PASS/two FAIL reproduced
ServiceFailure replacing a real reqwest timeout and client Debug exposing the
private query (`/tmp/ios-a02-activation-http-before.log`, terminal 101). The first
aggregate fixture failed E0599 by assuming a LockdownClient::new constructor
(`/tmp/ios-a02-deactivation-host.log`); it now uses the real frame writer, without
adding a production constructor. E0283 on ambiguous Dictionary key conversion
was fixed with explicit string keys (`/tmp/ios-a02-deactivation-host-final.log`).
The first strict Clippy rejected items after the test module
(`/tmp/ios-a02-deactivation-http-clippy.log`); moving tests to the end fixed it
without suppressing the lint or changing runtime behavior. Initial core/CLI
all-target/all-feature check passed before fixture expansion
(`/tmp/ios-a02-deactivation-http-check.log`).

The first full gate failed at capability inventory after the earlier gates
passed (`/tmp/ios-a02-deactivation-http-full.log`, terminal 1). The scanner
assumed any same-name directory meant a service/mod.rs entry; mobileactivation.rs
remains the actual flat entry and now owns private submodules in its directory.
An added owned temporary source-tree regression reproduced the wrong path
(`/tmp/ios-a02-service-inventory-before.log`, zero PASS/one FAIL, terminal 1).
The scanner now checks the real .rs entry before directory/mod.rs, retaining
the existing inventory records and public service paths. One additional tooling
test covers both layouts. Final full rerun passed, terminal 0; the original
full/test failures remain recorded rather than relabeled as successes.

Full gate: **1831 Rust / 13 Python PASS, zero FAIL, two existing ignored**.
Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four groups, 7+7
doctests/three downstream snippets, C ABI/owned malformed/checked-list cases,
3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw
diagnostic cases, five host workflows/22 real-device NOT_RUN, 15 CLI and three
backup contracts pass. Existing PBKDF2/separate-browser ignored cases remain;
no separate Chromium or installed 3.9/3.13 wheel rerun was performed. Tested
ios 0.1.13 SHA256:
`a0f1202357c39346e8e858dd39eb20e150ab6d5d37b46df52542ad38fd4f6547`.

Documentation closeout: capability catalog 118 records, 110 local Markdown
links across six changed Markdown files, and diff whitespace PASS. Link check
log: `/tmp/ios-a02-deactivation-http-docs-links.log`; baseline `d23a052` includes
the implementation commit and the documentation changes.

A02/class 3 and the overall goal remain active; class 4 has not started. Native
HTTP source retention does not type status/body-budget reasons or prove partial
read-buffer zeroization, automatic failed-client invalidation, ordered cleanup,
physical activation/deactivation or live Apple acceptance. Other A02 sources,
JSON/callback/binding boundaries, L01/L03/A03 and platform/device conditions
remain. No capability maturity upgrade, device operation, push/publication,
history rewrite or reference-checkout modification.

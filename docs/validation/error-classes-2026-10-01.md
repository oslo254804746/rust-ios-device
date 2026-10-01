# A02 Rust classification/evidence slice — 2026-10-01

Implementation: `678fb9975fa3fe9747b8efb7e29996b31f33c745`, eleven related files.
Branch `feat/device-service-updates-20260927`, Linux x86_64, Rust 1.97.1,
MSRV 1.80.1, Python binding runtime 3.12.13. No physical device selected.
[Contract](../error-contract.md), [compatibility](../public-contract.md),
[roadmap](../engineering-roadmap-2026-09-30.md).

| Check | Actual result |
| --- | --- |
| `cargo test -p ios-core --all-features --locked error::classification` | Final eleven PASS, zero FAIL/ignored; `/tmp/ios-a02-classification-final.log` |
| Same filter with `--no-default-features` | Seven PASS before the additional nested-wrapper case; existing minimal warnings retained, not strict minimal Clippy certification |
| `cargo clippy -p ios-core --all-targets --all-features --locked -- -D warnings` | PASS; final workspace strict Clippy also passes in the full gate |
| Minimal/all-feature `--test public_contract` | Eight/ten PASS after classifier and RSD evidence consumer additions |
| Core all-feature `rsd` filter | 36 unit plus two selected integration PASS (one AX, one public consumer); other filtered targets do not contribute passing integration cases |
| `python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python` | PASS; `/tmp/ios-a02-classification-full.log` |
| Existing public declarations | CoreError variants, ServiceDescriptor and RsdHandshake fields/declarations byte-for-byte preserved; this is a source-review aid, not a general ABI proof |
| Changed local Markdown links and whitespace | 67 links and diff/staged checks PASS |

Full gate: **1706 Rust PASS, eight Python PASS, zero FAIL, two ignored**.
The existing PBKDF2/browser cases remain ignored. Check/strict Clippy/MSRV,
66 feature builds/four groups, default/full doctests 7+7, three downstream
snippets, linked C ABI/19-byte raw capture, Python 3.12 wheel/sdist rebuild/
install, 41 tool tests, 118 actual-help inventory records, 38 original diagnostic
cases, five host regression cases/22 real-device NOT_RUN, eight actual CLI
output contracts and three local backup contracts pass. CLI stays 0.1.13;
accepted binary SHA-256:
`7fbab03cfb87b8b75ae0a96978b59ddc4d861696c2c3e0ca5cf78abaebc16f1c`.
No separate browser gate or Python 3.9/3.13 installed-wheel run was repeated.

The new non-exhaustive ErrorClass, stable string codes, classify_error and
CoreError::class inspect types/IO kinds and at most 32 original sources.
Tests preserve original sensitive-source data for explicit callers, traverse
24 context wrappers and nested backup/DeviceLink/IO errors, check IO custom
payloads, distinguish codec data from writer failure, bound a cyclic source
without invoking Display/Debug, and observe a real owned aborted Tokio task
and elapsed timer. No error text is parsed for a category. Unknown wire statuses
remain ServiceFailure; EINTR/file EOF are not guessed as cancellation/disconnect.

RsdFeatureEvidence describes a successful directory snapshot only. Tests keep
empty-metadata permissiveness, distinguish metadata unavailable from a missing
service, preserve canonical-before-shim resolution and check fixed serialized
codes. NotAdvertised does not prove a daemon's explicit unsupported response.
The old vector cannot distinguish an omitted feature list from an empty one;
ConnectedDevice::rsd None still includes skipped and failed discovery.

The first check/test failed because MuxError and house_arrest do not have
crate-root reexports. Adapters now use their existing internal/service paths;
no module visibility was expanded. Corrected tests and the complete gate passed
before this implementation commit. Existing error variants/fields/source chains,
descriptor fields and CLI/HTTP/C/Python output/status shapes are unchanged.

This slice's host acceptance is complete; **A02 remains in progress**. Lost
upstream source types, truthful unattempted/failed probe tracking, machine-error
CLI/HTTP/Python/C mappings, ordinary stderr callbacks, streaming JSON and
partial-operation semantics remain work. Lifecycle/physical cleanup acceptance
stays with L01. No maturity upgrade, pairing, DDI/TSS/restore/erase, push,
publication, history rewrite or reference-checkout mutation occurred.

## CLI deadline source retention

Implementation `7fd97077ad67b474191a2ec64112a98c371fc980`, three private CLI
WebInspector files. Four real Tokio timeout wrappers retain Elapsed through
anyhow Context; an expired operation budget retains the existing typed
WebInspector Timeout. Context text, timer values, routes, normal outputs and
failure exits remain unchanged. Existing Core startup's Disabled remapping
and Ctrl+C cancellation strings are still separate pending work.

`cargo test -p ios-cli --all-features --locked cmd::webinspector::` passed
33 cases, zero FAIL/ignored (`/tmp/ios-a02-web-timeout-source.log`). The two new
cases exercise a known expired budget and an actual application-list write
blocked by an owned eight-byte duplex peer, retaining the original typed source
and Timeout classification. The latter observes a real elapsed timer, without
a device connection or an invented service response. Existing access and actual
Rust bridge regressions remain included.

The complete split gate, `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`, passed with **1708 Rust,
eight Python, zero FAIL, two existing ignored**
(`/tmp/ios-a02-web-source-full.log`, terminal exit 0). The remaining gate scopes
match the classifier gate above: 66 feature builds/four groups, 7+7 doctests/
three snippets, strict Clippy/MSRV, C ABI/19-byte capture, 3.12 wheel/sdist,
41 tooling, 118 help, 38 raw cases, five host/22 device NOT_RUN, eight CLI/three
backup contracts. Accepted ios 0.1.13 binary SHA-256:
`ace50f42c398f3f221bc53cab0fc9c4b8122826f6ec84f369ba0cb6d2d85cfb5`.
The source/context diff and whitespace were reviewed before the focused commit.
This host slice is accepted; A02, probe tracking, frontend machine fields,
stream/partial semantics and L01 device cleanup remain in progress.

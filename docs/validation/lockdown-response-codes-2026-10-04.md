# Lockdown response codes and CLI decisions

A02/class 3 host slice accepted 2026-10-04, source
`08df6533265656b394b58a829653b15dfa7d197f`, baseline
`b2f81b2a07dfac6e9c4a7ed9f2a3218a26c96eee`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Ten source files, 853 insertions/33 deletions.
Canonical full exited 0 against the frozen source diff SHA256
`55b7b8dade1e7de91cc9dd4fc62cd90bbaa0e5ce57b5752ee75ceeba0e3f37f0`.
[Error contract](../error-contract.md), [public contract](../public-contract.md).

`LockdownResponseError` is an additive opaque public error with private fields
and operation(), code(), class() and in_error_chain() accessors. The last scans
at most 32 typed sources, visiting IO custom payloads directly. It never formats
or matches diagnostic text. Debug/Display omit the raw code; explicit code()
access retains the exact received string, which can contain private data.
The object has no invented OS code or underlying native parser/timer cause.

The preserving entry points are LockdownClient::get_value_preserving_errors,
start_service_preserving_errors, and ConnectedDevice's
lockdown_get_value_preserving_errors / lockdown_get_value_in_domain_preserving_errors.
They return existing LockdownError::Io/CoreError wrappers around the new
response observation. Their shared transport still preserves ordinary native
IO/plist errors. Existing error families stay exhaustive; no field, signature,
CLI flag, output schema, dependency or C/Python ABI change.

| Received operation / exact Error code | Stable category |
| --- | --- |
| GetValue / MissingValue | not_found; requested value absence, not missing service |
| StartService / InvalidService | service_missing; no inference about DDI, device support or retry |
| Other code or the same code under the other operation | service_failure; explicit device response failure, without guessing its cause |
| Legacy Protocol string mentioning either code | protocol; observation absent, no reconstruction from prose |

Existing get_value/start_service and the LockdownClient start-service method
retain historical Protocol(String) errors and Display. Existing C value-query
paths remain on that legacy policy. Migrate Rust consumers needing response-code
evidence to the additive APIs; existing source access cannot reconstruct already
flattened codes. A missing observation does not mean success. Valid request
serialization, port/SSL and value results, limits and transport behavior stay.

Preserving GetValue rejects a present non-string Error as source-free Protocol;
legacy still ignores that field when Value exists. This is an explicit new
profile, not a silent change to the old method. Error takes precedence over a
contradictory Value/Port. Missing Value without a device Error and non-dictionary
GetValue replies remain malformed protocol; they do not become MissingValue.

Backup's three WillEncrypt queries use the preserving method. Its private
resolver defaults false only for observed GetValue/MissingValue; generic host
NotFound, legacy prose and similar code substrings propagate their original
typed error under a fixed safe context. Non-boolean-compatible received values
classify protocol. Bool/integer success aliases, preflight/lock/transfer order
and successful output remain. Some setup already happens before these checks;
the error is not proof that the overall command had no effects.

Debugserver keeps its existing two-name preference/order and falls back only
for observed StartService/InvalidService. An unknown response containing that
substring no longer selects fallback. Exhausted exact misses return the final
original typed response error under the old fixed summary context instead of
an opaque summary alone. Other native errors propagate. The first known miss
still selects fallback; it is not exported as a new multi-attempt error list.
Returned category, Display/source and safe diagnostic correlation can migrate;
failure exit 1, safe v1 fields and unknown effects remain.

Seven tests were added; one old backup text-default assertion migrated to
require propagation of its legacy Protocol error. Core all-feature and default
new tests each pass four (three internal/one external). CLI new tests pass three;
full CLI passes 689/one existing ignored, including the final malformed-field
guard in canonical full. Final strict Core/CLI Clippy passes. Preparatory E0283 key inference and
E0505 fixture borrowing were corrected before accepted checks; no product fault
or old-implementation negative control is inferred from those compile errors.

Actual owned duplex GetValue and root StartService entries verify request fields,
legacy/new code handling, error priority and native EOF. GetValue compares exact
request bytes; StartService compares parsed request objects. MockStream covers
unchanged successful/shape results and the explicit non-string Error migration.
Private contexts at depths 31/32 and a source cycle exercise bounded lookup/
classification without invoking panic-on-format methods. Unknown private codes,
safe Debug/Display and nested IO observations are checked.

CLI backup uses the public LockdownClient through QueryType/StartSession, an
actual owned in-memory rustls exchange and GetValue, then its actual resolver.
The TLS material is the existing offline public test fixture; the owned server
uses no client authentication. This proves the host client/query/resolver path,
not Apple pairing/trust or ConnectedDevice/usbmux/device workflow acceptance.
No production pairing store, daemon, actual credential or device is used.
The peer/client futures are joined under two seconds; no background child is
spawned. Debugserver's actual private service-start loop runs owned framed
responses in both preference orders, verifies one/two calls, exact versus
substring/wrong-operation codes, last-error retention, EOF/no extra requests and
safe final error rendering under a one-second joined budget.

The code-field equality was checked against
[pymobiledevice3's response verification](https://github.com/doronz88/pymobiledevice3/blob/38fbd227b4cdf6cafea7b1c289d004c622cbf00c/pymobiledevice3/lockdown.py#L1046).
Reference hashes/status snapshots stay in /tmp; both read-only checkout states,
including their pre-existing uncommitted files, remain unchanged. The actual
checkout directory here is raw-projects. No source was copied from the reference.

Logs use `/tmp/ios-a02-lockdown-codes-`: core.log (preparation), core-final.log,
minimum.log, cli-narrow.log, cli.log, clippy-final.log, full.log and
production-review.diff. The source freeze check and original exhaustive
LockdownError/CoreError plus C/Python/dependency checks pass. The recorded
production diff is a review artifact, not a token-equivalence or device proof.

Canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
Session 83948 ended with exit 0 and `[PASS] validation/full`. Fourteen Rust test
groups total 2108 PASS; Python 13 PASS, zero failures/two existing ignored.
CLI 689 PASS/one existing ignored, core 1271 PASS/one existing ignored, public
contract 38 PASS. Check/fmt/workspace strict Clippy, MSRV, 66 feature checks/four
feature test groups, 7+7 doctests/three downstream snippets, C ABI contracts,
Python 3.12 wheel/sdist, 53 tooling/118 help/38 raw captures, five host workflows,
15 CLI and three backup contracts pass. Twenty-two device rows remain NOT_RUN.
Chromium and installed Python 3.9/3.13 were not rerun.
Final ios debug binary SHA256
`2e0650178e80fb30b7f5fc6feb45388e0ea8713233d1c84319ed1fb0bd2c7616`.
All ten source files stayed frozen through full. About 20 GiB free afterwards;
no cleanup, storage failure or restart. Eight documentation files close this
slice; seven Markdown files/191 local links, whitespace, exact hashes/full log
and capability catalog checks pass. Capability count, implementation/maturity and historical evidence stay
unchanged, with one new host evidence entry. Documentation does not certify
device workflows.

A02/class 3/overall 31-task goal remain active. Known text decisions are removed;
next is the identified thirteen owner-issued timeout/input/precondition guards,
including accessibility's empty traversal timeout. This is not an exhaustive
inventory of all untyped legacy errors. Lifecycle/reuse/cleanup/resource and
platform/device conditions remain with L01/L03/A03 and their owners.

Follow-up: [CLI owner guards](cli-owner-guards-2026-10-04.md) delivers the
identified guard review and final Linux host acceptance, retaining this gate
and its device/legacy compatibility limits.

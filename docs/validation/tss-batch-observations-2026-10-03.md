# TSS batch failure observations: host validation

Date: 2026-10-03. A02/class 3 source:
`17f31a3efe91a77f6397c469cd3fd4ee3a65c0a4`, branch
`feat/device-service-updates-20260927`, clean baseline
`8d5dd2d0b6a139207765e4b05763d589c3e2f459`. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, workspace Python 3.12.13. This accepts the host
observation contract, not physical Restore or Apple signing.

The legacy `prefetch_tss_batch` returned final failed updater names while
discarding combined and individual signing errors. The additive
`prefetch_tss_batch_preserving_errors` shares its algorithm and returns opaque
`TssBatchOutcome`, with the original result and independent ordered failures.
Recovered failures remain observable; final failure still uses the old names.

| Observation | Phase / index / original error |
| --- | --- |
| Request merge fails | BuildCombinedRequest / None / original RestoreError |
| Combined signer returns Err | SignCombinedRequest / None / original RestoreError |
| Combined reply lacks usable rider ticket | SignCombinedRequest / input index / None |
| Individual signer returns Err | SignIndividualRequest / input index / original RestoreError |
| Individual reply lacks usable ticket | SignIndividualRequest / input index / None |

Missing/empty/non-data tickets have `MissingTicket` kind, no invented error and
no guessed ErrorClass. Returned errors have `Error` kind and preserve their
original variant, typed class and source chain. Input indices distinguish
duplicate updater names. New enums start non-exhaustive; opaque fields prevent
constructing inconsistent observations. Debug shows metadata/counts only.
Explicit errors are private source data under the [error contract](../error-contract.md).

`result()` borrows the legacy result; `failures()` borrows the observations.
`into_parts()` keeps both owned branches, `into_error()` takes the original
error, and `into_result()` deliberately drops observations. A nonempty failure
list can accompany complete success after fallback. Empty input has no calls or
observations; invalid riders return Err before any signer call.

Four source files changed: tss.rs, new tss/batch.rs and tss/batch_contract_tests.rs,
and the external public-contract consumer. Original rider/ticket/result fields,
three exhaustive RestoreError variants, signatures/root/features/dependencies
and classifier stay. Batch merging/UUID/AP flags, validation, ticket storage,
nonce matching, HTTP parsing/limits and pre-existing tests are byte-identical.
The equivalent extraction retains the original algorithm exactly after only
wrapping its result. Final control-flow review and five branch scenarios check
the shared legacy/observed call policy. The old path allocates no observation
storage and drops signer errors before the next call. The new path retains
those errors until consumed/dropped. Observation count follows the input/call
count; arbitrary caller-owned error payloads have no newly imposed size cap.
No new timeout, retry, cancellation, cleanup, rollback or reuse guarantee.

Source diff SHA256:
`d3d10bbe8dd576ea94121d35ce77bf69583a86716ace47b619ad46844dccbf05`.
All four files, including new files, were staged before hashing and full.
Logs use `/tmp/ios-a02-tss-batch-`; `code-audit-final.log`,
`baseline-audit.log`, `source-diff.patch`, `original.rs` and `baseline.rs`
record scope/extraction review. One audit assertion originally assumed
single-line formatting; whitespace normalization corrected that check without
changing accepted source.

Twelve internal and one external contract tests were added. Actual owned
file-open NotFound/OS codes, Tokio timeout of a pending future, malformed TSS
UTF-8 and XML serialization UID faults retain their original causes. The timer
is supplied by a fixture signer, not a new public TSS deadline. Real loopback
reqwest timeout remains observable after successful individual fallback; the
existing 50 ms no-proxy fixture joins both futures within two seconds, tolerating
local read reset. This is not per-peer EOF or device cleanup proof. Duplicates,
missing/empty/non-data tickets, build conflicts, six rider-validation cases,
empty/full success, exact/mismatched nonce, private Debug payloads and counted
error destruction are covered. Public tests construct/destructure the unchanged
result, construct a rider, exhaustively match RestoreError and take an actual
native plist source without contacting the supplied unused endpoint.

Negative control: the new outcome prototype used the exact extracted legacy
algorithm with no observations. `before-compiled.log`: **3 PASS / 8 FAIL**;
`before-public.log`: **0 PASS / 1 FAIL**, all failures at missing observation
assertions. Existing result/call/validation assertions passed. Two preliminary
compile attempts (`before.log`, `before-final.log`) failed on explicit key/type
inference and trait-object function coercion; corrected before behavioral
controls and excluded from regression evidence. The final destruction test was
added after those controls; it is not claimed as a separately run old baseline.

| Check | Result |
| --- | --- |
| All-feature Restore / after-final.log | 56 PASS |
| External all-feature / public.log | 32 PASS |
| Minimum restore / minimal.log | 56 PASS |
| Minimum external / public-minimal.log | 16 PASS |
| Core strict Clippy / clippy.log | exit 0 |
| First canonical full / full.log | terminal exit 0 |

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. **2030 Rust / 13 Python
PASS, zero FAIL, two existing ignored** (14 Rust result groups; separate Python
host group). Fmt/check/strict workspace Clippy/MSRV, 66 feature builds/four
groups, 7+7 doctests/three snippets, C ABI/malformed/checked list, Python 3.12
wheel/sdist rebuild/install, 53 tooling tests, 118 help records, 38 raw cases,
five host workflows/22 device NOT_RUN, 15 CLI and three backup contracts pass.
No separate Chromium or installed Python 3.9/3.13 rerun. Tested ios 0.1.13 debug
SHA256 `471a5dec1fbf907f30e86684fcec1e388368b9a413cc6c72722a421b107d8a3f`.
Source stayed frozen throughout full and documentation. About 3.0 GiB free at
full launch, 2.4 GiB at finish; no storage failure/cleanup.

The 118 capability records, implementation/maturity and historical evidence
stay; one new host entry is added. Seven changed Markdown files have 159 valid
local links (baseline 8d5dd2d). Audits are in docs-links.log/docs-catalog.log;
eight documentation files are committed separately. A02/class 3/overall goal
remain active; class 4 has not started and all 31 tasks/subtasks remain.
Next confirmed source losses include application PID/plist, test-runner PID,
ZIP integer and GPX parser/time errors (read-only next-input-sources.log).
Other native adapters, DProxy text, typed guards/created outcomes, stream/JSON/
callback/bindings and L01/L03/A03/platform/device work remain. No physical
request/ticket/firmware/daemon/credentials/Apple operation, reference edit,
push/publication or history rewrite. See [public contract](../public-contract.md).

Follow-up: [service input sources](service-input-native-sources-2026-10-03.md),
source `ab097900523e91f93e54249c42c214a7e1349ee6`, retains eleven app/test/IPA/GPX
native causes in existing families with input classification. Final complete
gate passes 2041 Rust/13 Python after an old integration assertion migration;
the initial full failure, official stopped-build cleanup and private-versus-
public/64-bit source-review limits are retained. This leaves the above batch
observation evidence and physical/lifecycle conditions intact.

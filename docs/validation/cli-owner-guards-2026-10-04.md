# CLI owner-issued guards and A02 host acceptance

A02/class 3 final host gate accepted 2026-10-04, source
`fe0c0927418157cff83e2ebb41934d80cd12fae1`. Baseline
`75b0f32dcdc375d8caa5fe9983193400f1da1709`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Nine CLI files, 284 insertions/29 deletions.
The corrected canonical full exited 0 with frozen source diff SHA256
`8299147db966f80a805ff1de460d4ad9e0a86c3dd70e6cd1dcc8fd4ec48819c3`.
[Error contract](../error-contract.md), [public contract](../public-contract.md),
[preceding response-code gate](lockdown-response-codes-2026-10-04.md).

Thirteen previously identified owner-issued guards were reviewed. Eleven
invalid timeout inputs now use source-free IO InvalidInput, class invalid_input;
the empty accessibility traversal uses source-free IO TimedOut, class timeout.
These are decisions, without native OS codes, parser causes or Tokio Elapsed.
Their original fixed human message remains. Existing local input guards do not
imply that every invalid input in every legacy API has a precise category.

| Owner / guard | Role and actual verification boundary |
| --- | --- |
| HID zero timeout | InvalidInput; actual command run after confirm/UDID validation, before operation |
| Display capture zero timeout | InvalidInput; actual capture entry before connection |
| Display capture checked_add | InvalidInput; actual private deadline helper. Production still checks after tunnel setup/address selection |
| Companion seconds_duration checked_add | InvalidInput; actual helper overflow, zero accepted |
| Companion operation_deadline checked_add | InvalidInput; helper first rejects the maximal input in seconds_duration; second addition guard source-reviewed, not forced to fail |
| Crash flush checked_add | InvalidInput; actual helper overflow, zero/one accepted |
| Crash missing operation deadline | CLI Precondition; unreachable under current Flush-to-Some construction, source-reviewed only |
| Activation checked_timeout checked_add | InvalidInput; actual helper overflow, zero accepted |
| Activation absolute_deadline checked_add | InvalidInput; maximal input fails the preceding checked_timeout; second addition guard source-reviewed |
| OS Trace checked_add | InvalidInput; actual helper overflow, None/zero policy retained |
| RunXCTest zero timeout | InvalidInput; actual command run before plan/connection |
| RunXCTest checked_add | InvalidInput; actual command run after valid plan/UDID, before connection |
| Accessibility empty traversal idle limit | Timeout; actual state-policy method, not the entire DTX/device inspection/cleanup flow |

The private OperationPrecondition marker has no native source. Only CLI machine
mapping recognizes it as precondition; the general Core classifier remains
Unknown for this private CLI type. It does not set confirmation_required or
add a --force hint. Human and machine confirmation/precondition inspection share
the existing bounded 32-source traversal. No public Core API/variant, C/Python
binding, dependency, feature, protocol, output schema or command flag changed.
Opaque strings still remain Unknown; no classification by natural-language text.

Accessibility still tolerates the same number of transient idle results. At
the fifth consecutive idle result an empty traversal fails, while a traversal
with collected items completes. A focus event resets the counter. The call to
move_focus, limit/dedup behavior and finish_inspection cleanup stay in place.
The new method merely centralizes that existing outcome policy and types its
error. Timeout clamping, cancellation, budgets and successful formatting stay.

Seven new tests cover the actual zero/overflow command entries and helpers,
the accessibility empty/partial/reset policy, private marker depth 31/32,
source-free outcomes, safe v1 envelopes and opaque-text nonclassification.
Two existing Companion/Crash overflow tests now verify categories as well as
rejection. The new seven initially passed; full CLI then exposed a real
regression in the existing cyclic-source budget test: an extra independent
precondition walk exceeded its 128-access aggregate budget. The first canonical
full likewise ended with exit 1 at Rust tests (CLI 695 PASS/one FAIL/one ignored).
The correction combines marker discovery into the original single walk and
keeps the existing assertion. All 17 output tests and corrected CLI 696 PASS/
one existing ignored, fmt and strict CLI Clippy then pass. This is regression
evidence, not an old-implementation negative control.

Logs use `/tmp/ios-a02-owner-guards-`: narrow.log, cli-first.log,
full-first.log, output-final.log, clippy-final.log, cli-final.log,
fmt-final.log and full.log. No production source is edited during an active
build/validator handle. The first full exited before the correction; the final
source stayed staged and frozen through its own complete gate.

Corrected canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
Session 12108 ended with exit 0 and final validation/full PASS. Fourteen Rust
groups total 2115 PASS, Python 13 PASS, zero failures/two existing ignored.
CLI 696 PASS/one existing ignored, core 1271 PASS/one existing ignored, public
38 PASS. Fmt/check/workspace strict Clippy/MSRV, 66 feature checks/four groups,
7+7 doctests/three downstream snippets, C ABI, Python 3.12 wheel/sdist,
53 tooling/118 help/38 raw captures, five host workflows, 15 CLI and three backup
contracts pass. Twenty-two device rows remain NOT_RUN; Chromium and installed
Python 3.9/3.13 were not rerun. Final debug ios SHA256
`933b59e94d44e209d525059bc11056da53369f340e29c7ba0da723a90adce78e`.
About 19 GiB free before/after full; no cleanup or storage failure.

Seven actual CLI subprocesses use the final binary: HID/display zero timeout,
RunXCTest zero/maximal timeout, Companion listen/Crash flush/OS Trace stream
maximal timeout. Each exits 1 with invalid_input, canonical phase and unknown
effects; stdout is empty, raw/private captures clear, zero owned loopback usbmux
trap connections and all children reaped. Display's maximal timeout remains
tested only through its helper because production evaluates it after setup.
These binary fixtures and the thirteen-site source review are recorded in
binary.log and source-review.log. They are local failure paths, not device flows.

A02's finite acceptance uses the mapped profiles in the error contract: typed
Rust categories/original causes and additive response/probe observations; safe
opt-in final CLI errors and recorded stream/partial-failure formats; native
Python metadata/built-in argument exceptions; caller-thread and checked C
profiles; protected HTTP machine headers and separate post-upgrade protocol
messages. Legacy opaque/void/zero/ALLOC ambiguities remain explicit profiles,
not invented failure evidence. Earlier device/platform conditions and L01/L03/
A03 cleanup/reuse/resources remain assigned to their owners. A host gate does
not prove device pairing, support, remote acknowledgement, retry or rollback.

Documentation audit passes for eight files, seven Markdown/198 local links,
exact hashes/full logs/binary cases and capability catalog. All 118 capability
implementation/maturity records and historical evidence remain unchanged; only
one host evidence entry is added. All 31 original task IDs and historical
status rows remain unchanged.

The overall 31-task goal and all subtasks remain active. Class 3's current Linux
host profiles are delivered; platform/device and legacy compatibility limits
remain recorded. Class 4 lifecycle work follows this gate. No device, daemon, credentials, Apple
action, reference checkout, push/publication or history rewrite is involved.

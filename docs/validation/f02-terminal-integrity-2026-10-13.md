# F02 terminal results and final receipts — 2026-10-13

This is an independently reviewable partial repair of the original F02 scope in
`docs/engineering-roadmap-2026-09-30.md:444–454`. It follows the read-only
`repair/f02-fullflow-audit/audit-20261013.md` audit. It does not close F02 or claim
physical-device, modern CoreDevice workflow, or complete platform acceptance.

## Implemented behavior

`TestRunSummary::terminal_outcome()` is the shared Rust check used by direct
`runxctest`, `.xctestrun` `runtest`, attempt classification, failed-selector
admission, and rerun merge. A supported terminal result requires actual observed
plan-start and plan-finish events. A started case without a final status, a stalled
case, an unmodeled status, or reported suite uncaught exceptions is incomplete
evidence and returns nonzero. Passed, skipped, and expected-failure cases remain
supported; retained failure descriptions alone do not change their final status.

`failure_count()` retains concrete failed cases and nonzero suite unexpected-failure
counters even when a suite reports zero failures, and protects callers that provide a summary with a contradictory zero
aggregate. The recorder's `failed_tests` uses the greater of each suite's counter
and its concrete failed-case and unexpected-failure counts. The raw suite counters remain unchanged. This
does not compare nested suite counters, infer selection coverage, prohibit zero
tests, or reject valid class selectors. A began-and-finished empty plan is valid.

Each attempt retains its actual summary before terminal validation or a subsequent
termination error. Unsupported terminal evidence is labeled `incomplete`, distinct
from supported `completed` and `failed_tests` attempts and interrupted transport or
cancellation attempts. Reruns continue to select explicitly failed cases only.
Unsupported rerun evidence is rejected before replacing the primary summary; the
first failed attempt and the invalid rerun summary remain separately available.
The existing selected-case duplicate/missing/unselected checks are preserved.
Package installation stays outside the rerun loop.

Wait-mode output is deferred until startup/session cleanup, log flush/join, owned
uninstall, crash finalization, original native shutdown/join, final JUnit reporting,
and attempt sidecar writing have produced their outcomes. `status: finished` means
an actual finish event was observed. `outcome: passed|failed` is the final workflow
result; `test_outcome: passed|failed_tests|incomplete` describes the retained test
summary. `terminal_error_class` agrees with the emitted final attempts object and
the successfully written sidecar. Human direct-run output reports `XCTest failed`
when the final workflow fails even if tests passed.

Sidecar-write failure is included in the final in-memory history and receipt, and
adds a diagnostic to an available JUnit report. A sidecar path that cannot be
written remains unavailable; existing files/directories are preserved. Receipt
output failure is returned after cleanup and combined with an existing primary
error using the original error objects. It reclassifies final history and attempts
one local sidecar rewrite. Initial publication and this sole rewrite share one
absolute 5-second artifact clock; a started atomic filesystem writer remains owned
until completion and is checked against that same clock. Expired admission refuses
the rewrite and retains the original file plus a secondary timeout. No output retry
or device mutation replay is added. Normal no-wait started receipts retain their acknowledged-startup and
unknown-post-disconnect-lifetime semantics.

`TestRunIncomplete` retains the original case/suite names and unsupported status
as structured evidence. Its Debug and Display expose fixed status categories and
counts rather than those raw values. Existing primary/secondary error ownership,
absolute deadlines, known-PID cleanup, partial results, original service EOF,
native owners, and worker/task joins are preserved.

## Validation prepared and actually performed

The implementation subagent was prohibited from invoking Cargo, rustc, maturin,
compiling tests, committing, pushing, or using a physical device. Root owns the
native build and actual CLI gates. The following results are the subagent's actual
checks only:

- PASS: CPython 3.13 UTF-8 AST parsing of `regression_dtx_fixture.py`,
  `test_regression_adapters.py`, and `test_regression_crash.py`.
- PASS: `git diff --check` at the source freeze.
- Native Rust tests, rebuilt actual CLI fixtures, physical devices, and other
  platforms: NOT_RUN by this subagent. Root-run results must be recorded separately.

Prepared Rust gates cover start/finish evidence and valid empty plans, supported
case outcomes with retained failure details, unfinished/stalled/unknown/uncaught
results, concrete failures with zero suite/aggregate counts, retained safe error
evidence, attempt classification, unchanged primary after rejected reruns, and
final receipt/test outcome separation with original cleanup errors retained.

Prepared actual CLI gates extend the existing localhost TLS-usbmux/Lockdown,
streamed ZIP, InstallationProxy, Instruments, DTX, syslog and crash peers. Both
direct and generated `.xctestrun` commands cover supported and invalid terminal
events, failure followed by invalid rerun, log EOF, owned uninstall failure, JUnit
directory refusal, attempt-sidecar directory refusal, and final crash failure.
The tests parse exactly one final JSON value, compare it with the actual sidecar,
retain raw summaries and first failure text, and preserve installation counts,
known-PID termination, original peer EOF, original worker joins, preserved artifact
markers, and absence of staging residue. The human-output fixture checks final
cleanup failure is not printed as a successful finished run. A hidden Windows
process with its original stdout reader closed verifies nonzero native exit,
sidecar reclassification, original stderr reader/job joins, and unchanged device
mutation/known-PID cleanup counts. A Rust gate also verifies an expired shared
artifact clock retains both the original output error and the original sidecar.

Suggested root-only gates are `cargo test -p ios-core --lib terminal_outcome_tests`,
the existing ios-cli `cmd::test_workflow`/`cmd::runxctest`/`cmd::runtest` tests, and the
existing `ActualCliWorkflowTests` and `ActualCrashWorkflowTests` Python unittest
classes against the newly built binary. Existing no-wait/native/platform gates
remain required separately.

Root actual results: core terminal5/0 plus original integration19/0; CLI workflow
15/0, runxctest10/0, runtest11/0. Both actual CLI fixture classes passed all 26
methods and their subcases, including closed original stdout, independent stderr
reader/job joins, both entry points, raw history/exit/receipt agreement, original
peer EOF and no repeated installation. The new receipt assertion was rechecked
after a literal-unwrap Clippy style repair; production behavior was unchanged.
Strict Core/CLI all-feature/all-target Clippy, CLI build and serial fmt/diff passed;
diagnostic safety tooling7 PASS/1 original platform skip. The shared build's
readonly manager1 PASS had zero Source connections; host runner5 PASS/22 NOT_RUN.

Source, binary, log and commit hashes are frozen in
`repair/f02-terminal-integrity/summary.json`; the common build also contains the
separately committed scan capsule work. This is isolated legacy wire/CLI fixture
evidence, not a physical device or modern CoreDevice full-flow PASS.

## Original F02 work still open

The recorder still lacks bounded admission for suite/case growth, names and failure
payloads, and explicit truncation/incomplete markers for limited logs. The 32 MiB
serialized history limit does not bound ingestion or prior clones. Full workflow
orchestration remains in the CLI; a reusable core workflow owner with explicit
stage outcomes and read-only Developer Mode/DDI/service-readiness preconditions
before optional installations is still missing.

An actual modern full CLI workflow remains unverified: owned tunnel/RSD service
directory, remote DTX, RemoteXPC AppService launch/signal, canonical/shim routes,
advertised capability rejection, interrupted committed calls and service-owner
close before native retirement require dedicated fixtures and pinned primary
protocol evidence. XCTest selector/object grammar and protocol negotiation also
remain bounded by that evidence. No cleanup RPC or remote acknowledgement is
invented by this repair.

Real Ctrl-C gates across installation, known-PID authorization, partial results,
and rerun startup remain open, along with terminal receipts for effects/recovery
and physical-device/platform workflows. Interrupted installation or launch may
have unknown remote effects; absent PID evidence does not prove no process was
created, and a new invocation must not assume no prior effects. This repair does
not automatically replay installations, launches, recovery writes, or uncertain
cleanup mutations.

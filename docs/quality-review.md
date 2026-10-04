# Quality review and release readiness

Review monthly and after releases or important protocol/dependency changes.
When cadence changes, record the reason and next review date; retain previous
conclusions. This template starts D02 governance, not 1.0 readiness or release
authorization. The roadmap retains every unfinished task and subtask.

Current implementation review: 2026-10-04. Monthly review remains planned for 2026-10-30.
The [capability catalog](capabilities.md) is the source for maturity/evidence;
[build evidence](validation/build-release-2026-09-30.md) records the first gate.

| Measure | Current evidence | Remaining condition |
| --- | --- | --- |
| Indexed CLI/service surfaces | 62 commands and 49 service families | CI checks drift against source and actual help |
| Stable device workflows | No current capability certified stable | Representative device PASS with scope and exact commit |
| Host build/tests | [A02 final owner-guard gate](validation/cli-owner-guards-2026-10-04.md), latest `fe0c092`, 2115 Rust/13 Python, CLI696 PASS/one existing ignored, zero FAIL/two existing ignored; corrected canonical full terminal0. Seven new tests/two strengthened existing, thirteen reviewed guards and seven actual CLI machine input failures/no trap connection. First cyclic-source regression corrected within original budget. Strict checks/MSRV, 66 features/four groups, 3.12 wheel/sdist, C ABI, 53 tooling/118 help/38 raw/five host/15 CLI/three backup contracts pass. [Response codes](validation/lockdown-response-codes-2026-10-04.md), [streams](validation/cli-stream-contract-2026-10-04.md), [native](validation/cli-native-sources-2026-10-04.md), [diagnostics](validation/cli-diagnostic-sources-2026-10-04.md), [connection](validation/connection-native-sources-2026-10-04.md) and earlier evidence retained | Class3 current Linux host profiles delivered; unreachable/secondary guards are source-reviewed, focus helper is not complete device inspection, display overflow occurs after setup. Legacy Unknown/void/zero/ALLOC profiles remain explicit. FPS queue/task ownership and remote cleanup/reuse/resources stay L01/L03/L04/A03. No separate Chromium/installed 3.9/3.13 rerun; 22 device rows NOT_RUN and platform/fixture/deployment conditions remain |
| Platform release candidates | Linux x86_64 validated | macOS, Windows, ARM Linux runner results |
| Post-release regression | NOT_RUN | No release made by this implementation |
| Critical p50/p95 latency and throughput | NOT_RUN | L04 reproducible before/after baseline |
| Resource release and bounded long-run checks | Debug host revocation/ownership fixtures PASS; bounded long-run NOT_RUN | Orderly device cleanup, L02/L03 fixtures and L04 sampling |
| Public compatibility contract | A01 candidate profiles and source/CLI/C/Python fixtures; A02 typed classifications/source/probe/mapped output profiles accepted for current Linux host scope, retaining exhaustive error matches and descriptor fields | Legacy compatibility/Unknown profiles remain; L01 cancellation, A03 SDK resources and cross-platform/1.0/device workflow evidence remain pending |
| Internal module boundaries | R03 device, R01 backup, R02 WebInspector and R04 explicit plist/result-output boundaries accepted; public paths preserved, A02 current error/stream profiles and actual Rust CDP routers with owned synthetic service streams accepted | L01 cleanup, A03 SDK and F03 external-client/physical-device workflows remain pending |
| 1.0 candidate workflow matrix | NOT_RUN | Cross-platform/device automation and long-run evidence |

Each review must record tested commits, host/device/iOS/transport/state, exact
commands, PASS/FAIL/INCONCLUSIVE/NOT_RUN, accessible sanitized evidence, changes
to compatibility, and next actions. Percentages must state their denominator;
unknown or NOT_RUN conditions cannot count as passing coverage. Performance
budgets follow measured baselines, not invented throughput or delay promises.

1.0 candidate entry criteria: explicit candidate APIs/commands/JSON formats;
error/ownership/cancellation/migration contracts for every proposed stable
interface; verified release gates and artifacts; at least one representative
platform/device workflow regression and bounded long-run check. Experimental
CDP, Cryptex, media, clipboard and restore boundaries remain separately listed.
Meeting these criteria does not automatically publish anything or complete the
remaining roadmap features. Choose an unused release version consistently
before registry packaging; do not overwrite already-published 0.1.13.

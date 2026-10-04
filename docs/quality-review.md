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
| Host build/tests | [L01 DTX gate](validation/dtx-lifecycle-2026-10-04.md), latest `940c4c6`, 2126 Rust/13 Python, CLI696/Core1281/public39 PASS, zero FAIL/two existing ignored; final full terminal0. Eleven new owned duplex/service/public-consumer tests (minimum dtx nine), CPU post-EOF stop expectation migrated and separate AFC diagnostic numeric-collision fix. Strict checks/MSRV, 66 features/four groups, 3.12 wheel/sdist, C ABI, 53 tooling/118 help/38 raw/five host/15 CLI/three backup contracts pass. [A02 host gate](validation/cli-owner-guards-2026-10-04.md), [response codes](validation/lockdown-response-codes-2026-10-04.md), [streams](validation/cli-stream-contract-2026-10-04.md), [native](validation/cli-native-sources-2026-10-04.md), [diagnostics](validation/cli-diagnostic-sources-2026-10-04.md), [connection](validation/connection-native-sources-2026-10-04.md) and earlier evidence retained | L01/class4 in progress: DTX local state and total budgets accepted; other protocols, FPS queue/task/timestamps, cleanup/reuse/resources and L02–L04/Q04/A03 remain. Service fixtures use private construction/warmed flags, not device setup/trust. Legacy compatibility and class3 device/platform limits remain explicit. No separate Chromium/installed 3.9/3.13 rerun; 22 device rows NOT_RUN and platform/fixture/deployment conditions remain |
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

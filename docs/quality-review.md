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
| Host build/tests | [L01 Lockdown gate](validation/lockdown-lifecycle-2026-10-04.md), latest `79f750e`, 2192 Rust/13 Python, CLI706/Core1332/public44 PASS, zero FAIL/two existing ignored; first full terminal0. Ten new tests pass in final full/default minimum; Lockdown73 and five original CLI stack-budget tests pass. Shared request deadline, sent/late/fragmented and buffered reads, empty/partial writes, genuine TLS constructor, complete business reuse and default five-second/manual session close ownership are covered. Source audit preserves assertions/envelopes/parsers/frame limit; six old private fixtures gain default state. Strict checks/MSRV, 66 features/four groups, 3.12 wheel/sdist, C ABI, 53 tooling/118 help/38 raw/five host/15 CLI/three backup contracts pass. [RSD gate](validation/rsd-lifecycle-2026-10-04.md), [XPC gate](validation/xpc-lifecycle-2026-10-04.md), [H2 gate](validation/h2-lifecycle-2026-10-04.md), [AFC gate](validation/afc-lifecycle-2026-10-04.md), [FPS gate](validation/fps-lifecycle-2026-10-04.md), [DTX gate](validation/dtx-lifecycle-2026-10-04.md), [A02 host gate](validation/cli-owner-guards-2026-10-04.md), [response codes](validation/lockdown-response-codes-2026-10-04.md), [streams](validation/cli-stream-contract-2026-10-04.md), [native](validation/cli-native-sources-2026-10-04.md), [diagnostics](validation/cli-diagnostic-sources-2026-10-04.md), [connection](validation/connection-native-sources-2026-10-04.md) and earlier evidence retained | L01/class4 in progress: DTX/FPS/AFC/H2/XPC/borrowed RSD/Lockdown client profiles accepted; DeviceLink/plist/backup raw ownership, borrowed session callers, setup/service/multiple-request totals, other cleanup/resources/long streams and L02–L04/Q04/A03 remain. Lockdown request default None, manual stop5s (shorter configured clamp), terminal local close without ACK, and separate manual primary/cleanup results. Constructor/raw helpers/device fresh-session paths unchanged. Opaque halves remain owned until Drop, no automatic stop/task/remote release/safe retry. Cooperative timers, legacy profiles, 22 device NOT_RUN rows and no separate Chromium/installed 3.9/3.13 rerun remain |
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

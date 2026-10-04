# Quality review and release readiness

Review monthly and after releases or important protocol/dependency changes.
When cadence changes, record the reason and next review date; retain previous
conclusions. This template starts D02 governance, not 1.0 readiness or release
authorization. The roadmap retains every unfinished task and subtask.

Current implementation review: 2026-10-03. Monthly review remains planned for 2026-10-30.
The [capability catalog](capabilities.md) is the source for maturity/evidence;
[build evidence](validation/build-release-2026-09-30.md) records the first gate.

| Measure | Current evidence | Remaining condition |
| --- | --- | --- |
| Indexed CLI/service surfaces | 62 commands and 49 service families | CI checks drift against source and actual help |
| Stable device workflows | No current capability certified stable | Representative device PASS with scope and exact commit |
| Host build/tests | [A02 connection source gate](validation/connection-native-sources-2026-10-04.md), latest `4b3a9cb`, 2078 Rust/13 Python, 53 tooling/118 help/38 raw/five host/15 CLI/three backup contracts; original backend/integer/JSON/elapsed sources with Unknown/Input/Protocol/Timeout owner roles, actual joined duplex requests/EOF, timer expiry and local proxy loopback; five new core tests, 15 handshake/9 discovery/11 routing/4 lockdown/16 userspace and 37/17 existing public tests; minimum features/strict core and first full PASS marker, 3.12 wheel/sdist; [preceding guarded gate](validation/guarded-native-sources-2026-10-03.md), [recorder gate](validation/dproxy-recording-observations-2026-10-03.md), [decoder gate](validation/dproxy-decoder-observations-2026-10-03.md), [input source gate](validation/service-input-native-sources-2026-10-03.md), [TSS batch gate](validation/tss-batch-observations-2026-10-03.md), [restore source gate](validation/restore-native-sources-2026-10-03.md) and earlier evidence retained | Narrower-host runtime, remaining native/created decisions and CLI diagnostics/stream/binding work remain; actual mDNS daemon/OS failure and TCP dial expiry are not runtime-certified; cancellation/reuse/cleanup and resource conditions stay L01/L03/L04, 22 real-device rows NOT_RUN and platform/fixture/deployment conditions remain |
| Platform release candidates | Linux x86_64 validated | macOS, Windows, ARM Linux runner results |
| Post-release regression | NOT_RUN | No release made by this implementation |
| Critical p50/p95 latency and throughput | NOT_RUN | L04 reproducible before/after baseline |
| Resource release and bounded long-run checks | Debug host revocation/ownership fixtures PASS; bounded long-run NOT_RUN | Orderly device cleanup, L02/L03 fixtures and L04 sampling |
| Public compatibility contract | A01 candidate profiles and source/CLI/C/Python fixtures; additive A02 typed classifications/RSD evidence with retained exhaustive error matches and descriptor fields accepted | A02 upstream source/probe tracking, machine mappings/streams; L01 cancellation, A03 SDK resources and cross-platform/1.0 workflow evidence remain pending |
| Internal module boundaries | R03 device, R01 backup, R02 WebInspector and R04 explicit plist/result-output boundaries accepted; public paths preserved, actual Rust CDP routers accept owned synthetic service streams | A02 error/stream contracts, L01 cleanup and F03 external-client/physical-device workflows remain pending |
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

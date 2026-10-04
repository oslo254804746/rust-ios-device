# Notification CLI control host acceptance (2026-10-04)

L01/class4 implementation `e3ab42b8e1d47155edd9e0ffd08c668b5e3a41ec`,
based on `7e901d6f8c38b8f856797c1a453c686d1c5e626f`.
Six CLI/source/verifier test files, 1295 insertions/141 deletions. Linux x86_64,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. This is host acceptance, including
actual Linux SIGINT at a synthetic discovery endpoint. Device service, stdout,
signal and cleanup behavior remain pending. See
[roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md),
[errors](../error-contract.md), preceding
[CLI cleanup profile](notification-cli-cleanup-2026-10-04.md) and
[core profile](notification-lifecycle-2026-10-04.md).

Finite zero/overflow timeouts reject as existing InvalidInput before explicit
or default device selection. One checked30s setup budget owns default discovery,
device connect and service connect. First-device/empty-default behavior stays.
One pinned Ctrl+C future is polled before setup work and survives into capture;
setup stop returns OperationCancelled. The device owner lives through cleanup.

Post and each complete subscription stage have a30s send cap. Roundtrip/Wait/
Springboard and Stream with --timeout share one absolute user budget from before
subscription through receive/output. Roundtrip includes observe/post; Stream
includes every subscription. Setup precedes this total and cleanup has the
existing independent5s cap. Observe retains per-event idle timing with unrelated
events; Stream without --timeout remains unbounded. A ready stop/due total wins
before ready operation traffic. Complete event/output boundaries yield without
an arbitrary tick cancelling partial framing.

Stream/Observe operator stop and Stream total expiry complete normally subject
to cleanup. Finite/setup stop retains typed cancellation; issued setup/send/
finite-total expiry retains real Elapsed. Stream's old idle expiry could fail
while between-frame expiry succeeded; the new consistent normal stop is an
explicit timing/exit migration. Output lines, filtering/order, parameters/help,
v1 fields and count0 policy stay: Observe emits0, Stream emits1 then stops.

The prior cleanup owner retains primary/native/output errors and dual cleanup
causes. Invalid/ProxyDeath connections skip unsafe late Shutdown. Directly owned
boxed futures add no task/queue/background reader/async Drop. Synchronous
encode/decode/output and running blocking jobs are cooperative. No remote ACK,
rollback, safe retry, physical-service or performance guarantee is inferred.

| Owned fixture | Observed behavior and limit |
| --- | --- |
| Admission/unpolled | All five finite variants reject zero/u64MAX without Elapsed; generic setup zero/MAX and unpolled work never poll operation |
| Shared setup |200ms across125ms+125ms stages expires after first;75ms later no detached continuation. Ready stop/native permission failure wins before work |
| Setup lifetime | Expiry/caller Drop after an accepted64-byte prefix gives owned peer EOF |
| Send ownership | Short expiry distinguishes blocked zero-byte send from accepted partial frame; only partial invalidates |
| Roundtrip total | Real --timeout1-equivalent command with650ms blocked observe/post and650ms late reply expires before output; exact Shutdown follows completed subscription |
| Stream total | Same1s total begins before both subscriptions;650ms send and650ms late relay yields0 records and normal stop/Shutdown |
| Observe idle | Unrelated then matching event each650ms apart succeeds beyond1s total; exact original JSON newline bytes |
| Infinite stream/signal | No timeout during125ms idle; owned stop sends Shutdown. Same pinned signal survives setup; queued stop emits no Observe |
| Ready traffic | Already due/ready stop consumes no event/output;20 queued events yield control after third callback |
| Partial vs idle stop | Idle stop attempts exact Shutdown; partial read sends no late control, then Drop gives EOF |
| Finite/native signal | Ready finite stop retains OperationCancelled; native signal PermissionDenied plus cleanup BrokenPipe retains primary/source with safe Debug |
| Injected writer | Actual Wait consumption invokes write_line_to with permission-failing owned writer; single/dual cleanup retains output primary. This is not OS stdout injection |
| Caller/count0 | Caller drops partial owned listener with peer EOF/no Shutdown. Original Observe0/Stream1 count0 policies preserved |

Nineteen new Rust tests. Final pipeline76076 terminal0: narrow19,
notification CLI36, minimum CLI all-target/no-default check, five original16KiB
stack checks, strict Core/CLI Clippy, fmt, verifier11 and actual CLI39 PASS.
Wire fixtures have2s bounds;20ms short timers exercise shared production helpers.
The30s constants are source-reviewed, not measured30s expiry runs.

Three new verifier tests reject contact even with valid JSON and reject private
diagnostics even with valid schema; positive0-contact passes. Twenty-four new
actual-binary cases extend the old15 to39:

- Five commands × zero/MAX × explicit/default =20 JSON admission cases, plus
  two human cases. All22 show zero accepted synthetic endpoint connections,
  safe diagnostics and unchanged class/phase/exit/v1 fields. Effects remains
  conservative unknown. This count is observed, not inferred from source.
- Valid empty-default command sends exactly one ListDevices and no additional
  connection; the original empty-default error contract stays.
- Actual Linux SIGINT is sent only to an owned CLI child blocked on its sole
  synthetic ListDevices endpoint. It returns cancelled/notify/exit1, closes the
  owned peer, and bounded process capture completes. No device/service/pairing
  reply or credentials are provided. Other platforms remain unverified.

All39 actual CLI cases have0 raw hits and complete bounded capture/cleanup.
Existing clap/help and old cleanup/stream helper/renderer/tests are byte-identical.
Main changes only notification preflight/delegated discovery; other command
bytes/tests and old Python verifier tests remain. Core APIs/error enums, wire
envelopes, limits, dependencies and read-only reference HEAD/status stay.
No old-source negative control or benchmark/device acceptance is claimed.

Preparation: with no live build, cargo clean -p ios-cli --profile dev terminal0
removed4238 cache files/10.9GiB; free space rose from2.6 to about12GiB. Initial
expanded37-case run had35PASS/2humanFAIL because the new verifier expected the
wrong human label. It was corrected to existing io/InvalidInput, without changing
product formatting; corrected37 and final expanded39 passed. Earlier18-test
narrow passed before the shared-signal fixture brought the final count to19.

Full section11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session13402 terminal0, validation/full PASS: **2257 Rust/13 Python**, zero FAIL,
two existing ignored; CLI734/Core1366/public47. Rust/Python checks split;
workspace fmt/check/strict Clippy/MSRV,66 features/four runtime groups,
7+7 doctests/three downstream snippets, C ABI,3.12 wheel/sdist/install,
56 tooling/118 actual-help/38 raw/five host/39 CLI/three backup contracts pass.
Chromium and installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Frozen source SHA256:
`5329b8ccbc98417797eb15f0ce3a5ab3a1ee01e3368d27a04e920ec4e70531c2`.
Final ios debug binary SHA256:
`50f09b7fb0fe78eca75e945485e4d3a09b231f61ce08dae0e1e5023dbcc7ef64`.
The six-file source patch stayed frozen through full and matches the actual
implementation commit. After full4.3GiB free, no storage failure. Local logs/
audits use /tmp/ios-l01-notification-cli-control-; outcomes are retained here.

Documentation audit PASS: eight documents, seven Markdown files/305 local links;
all31 tasks/subtasks/prior history retained with only the L01 row and final
delivery appended. Exact implementation, frozen source/binary, full/narrow,
preparation and read-only references match. Capability118 bodies/maturity/prior
evidence remain; source and actual CLI-help validation PASS. No product source
changed during documentation.

All31 tasks/subtasks remain active with9 complete/11 not started/11 in progress
or awaiting conditions/acceptance. Next: Companion shared request/listener/
forwarding cleanup, then other L01 protocols/raw borrowed sessions/multi-request
totals/long streams/manual pairing/blocking resources, L02–L04/Q04 and classes5–9.
Capability118 bodies/maturity/history and platform/device/credentials/protocol
conditions remain. No reference edit, subagent, push, publication or history
rewrite. Host evidence does not close the overall goal.

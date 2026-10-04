# Notification CLI cleanup host acceptance (2026-10-04)

L01/class4 implementation `9b2c488ec9052d2899de91a68622b00be207446c`,
based on `a8917656824347914d9affc51b42592f97071f03`.
Two CLI source/test files, 450 insertions/93 deletions. Linux x86_64,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. Host helper/source acceptance;
device command/output/OS signal behavior remains pending. See
[roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md),
[errors](../error-contract.md) and the preceding
[core notification profile](notification-lifecycle-2026-10-04.md).

The command body is directly owned and boxed, then its result is retained while
cleanup awaits. A usable core client attempts Shutdown after returned success or
failure, including output/render errors. Core's existing independent five-second
cap applies. An invalid/ProxyDeath stream skips unsafe control writes and remains
owned until command Drop. This unattempted path does not create a secondary error
or imply remote cleanup. Whole-future cancellation drops transport without a
detached task or asynchronous Drop.

Cleanup failure after success is now reported; previously it was suppressed.
Success output/device effects can already have happened, without rollback or
retry authorization. A single primary failure keeps its original object. Dual
failure owns both originals in a private NotificationCleanupFailure; source and
category follow the primary, with cleanup separately retained. Display is static
and Debug includes only categories; automatic safe v1 fields remain. Public core
APIs/error enums, wire, setup, parameters/help, event selection and success output
remain. Setup/total/signal ownership is not implemented by this profile.

| Owned helper fixture | Evidence and limit |
| --- | --- |
| Success | Exact post then one-key Shutdown, closed local state and peer EOF after Drop |
| Returned primary | Original permission IO/context survives successful exact Shutdown |
| Native cleanup | Success fails on native BrokenPipe; dual failure retains permission primary/native cleanup with primary classification and safe JSON/Display/Debug |
| Issued cleanup expiry | Short partial Shutdown expiry retains real Elapsed separately while primary chain/category remains permission |
| Default cap | Actual default five-second Shutdown expiry after success is reported under a six-second fixture bound; exact64-byte prefix then Drop/EOF |
| Invalid frame/death | Partial receive expiry and ProxyDeath retain original failure, send no late Shutdown and drop owned transport |
| Whole-future cancellation | Drop during accepted Shutdown prefix yields peer EOF without detached completion |

Nine new tests. Fixed narrow63198 terminal0 passes nine; final pipeline85220
terminal0 passes minimum CLI all-target/no-default check, CLI notification17,
five original16KiB stack checks, strict Core/CLI Clippy and fmt. Each fixture owns
futures/streams with one-second wire bounds, six seconds for default cleanup.
No device CLI, actual stdout fault injection, OS Ctrl+C, old-source negative
control, benchmark or platform acceptance is claimed. Source audit proves all
command match tokens/parameters/output/setup unchanged after fmt whitespace and
the single allowed tuple-pattern trailing comma; original stream helper/renderer/
tests remain byte-identical. Moving returns into the boxed body deliberately
lets the cleanup decision follow them. Core/root/APIs/protocol/deps and read-only
reference HEAD/status stay unchanged; no task/queue is added.

Preparation: first narrow75043 terminal101 was test-only E0599 from the wrong
ErrorClass name. It was corrected to existing PermissionDenied and the existing
v1 `error.class` field before fixed63198. Temporary source audit initially
rejected rustfmt's tuple-pattern comma; normalization corrected the audit without
source or assertion changes.

Full section11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session78503 terminal0, validation/full PASS: **2238 Rust/13 Python**, zero FAIL,
two existing ignored; CLI715/Core1366/public47. Rust/Python checks split;
workspace fmt/check/strict Clippy/MSRV, 66 feature checks/four runtime groups,
7+7 doctests/three downstream snippets, C ABI, 3.12 wheel/sdist/install,
53 tooling/118 actual-help/38 raw/five host/15 CLI/three backup contracts pass.
Chromium and installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Frozen source SHA256:
`e59de42bd588c30c6507c977745398e5ebc9636179a17af2d490218467b9d3e9`.
Final ios debug binary SHA256:
`56e8c8ae91f7b9c4887aa647a4ddc5268b80562722c3e79b937ed0703971377d`.
Source frozen through full/implementation commit. About4.0GiB free during narrow,
2.6GiB after full; no cleanup/storage failure. Local logs/audits use
`/tmp/ios-l01-notification-cli-cleanup-`; outcomes are retained here.

Documentation audit PASS: eight documents, seven Markdown files/295 local links;
all31 tasks/subtasks/prior evidence retained with only the L01 overview row and
appended delivery. Exact implementation/source/binary/full/narrow/preparation and
read-only references match. Capability118 bodies/maturity/history unchanged with
one host entry; source/actual CLI help validation PASS. No product source changed.

All31 tasks/subtasks remain active with9 complete/11 not started/11 in progress
or awaiting conditions/acceptance. Next: notification CLI checked setup/finite
totals/operator signal; then other L01 protocol/cleanup/borrowed raw sessions/
multi-request totals/long streams/manual pairing/blocking resources,
L02–L04/Q04 and classes5–9. Platform/device/credential/protocol conditions
remain. No device/Apple operation, reference edit, subagent, push, publication
or history rewrite.

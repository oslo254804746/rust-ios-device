# MobileBackup2 lifecycle host acceptance (2026-10-04)

L01/class 4 implementation `d2828b802e697ff4d1069744e89d7a1eb31214c7`,
based on `65cf7384eafac8fb36ecd0cd09b105faaf119830`.
Seven authorized source/test files, 1280 insertions/77 deletions. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, project Python 3.12.13. This is host acceptance;
device backup/restore workflows and capability maturity remain pending. See
[roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [public compatibility](../public-contract.md),
[errors](../error-contract.md) and [backup contract](../backup-contract.md).

Mobilebackup2Client adds is_usable and set_operation_timeout(Option<Duration>).
A private accepted-byte transport and public-operation guard cover version/Hello,
command, host work, raw upload/download and final cleanup. Default None keeps
ordinary operations unbounded; DeviceUnback/Extract keep the existing 300-second
ceiling, clamped by a shorter configured budget. One absolute budget spans all
participating awaits. Zero/overflow admission occurs before I/O or host work;
actual expiry retains Tokio Elapsed. Unback/Extract keep typed operation evidence.
Synchronous encoding/decoding and already running blocking work are cooperative
boundaries. Directly owned boxed futures add no background tasks or queues.

Cancellation/failure after progress before terminal completion and native
transport failure/EOF invalidate local reuse. All nine public operation paths
then reject before I/O, including late replies; private diagnostic/filter/
discarded-path state is cleared. Local pre-I/O failures, unpolled futures and
empty idle waits can preserve state. A complete DLMessageProcessMessage marks
terminal before dictionary/ErrorCode validation so the original bounded
disconnect attempt remains. Cleanup shares the total budget and DeviceLink's
five-second cap. Operation expiry starts no late disconnect, and abandoned raw
transfer does not receive interleaved control cleanup.

Expected disconnect BrokenPipe/ConnectionAborted/ConnectionReset/NotConnected/
UnexpectedEof after the terminal reply retain the old completion policy. The
transport remains unusable. Other cleanup errors propagate, including after
success. SessionCleanupFailure has private fields, borrowed operation_error and
cleanup_error accessors, and is reached through cleanup_failure's bounded
32-source typed search including IO payloads. Existing Io wraps dual failure;
source/class follows the original primary. The primary stays owned while cleanup
awaits so outer expiry dropping that body cannot discard it. This explicitly
replaces the older cleanup-timeout-first category for affected dual failures.
The four Mobilebackup2Error variants remain. No ACK, asynchronous Drop, rollback,
worker stop/join, safe retry or device cleanup guarantee is implied.

| Owned host fixture | Evidence and limit |
| --- | --- |
| Preflight/idle | Zero, overflow and unpolled operations preserve no-I/O state; empty issued idle expiry/cancellation can reuse |
| Negotiation | Version consumption followed by an empty blocked Hello write remains part of the interrupted owner |
| Sent/fragmented replies | Complete request cancellation and fragmented expiry reject late replies without more I/O |
| Total budget | Negotiation, host layout work and final reply consume one absolute deadline |
| Raw upload | Cancellation and malformed frame invalidate while preserving unrelated host file content |
| Raw download | Cancellation sends only the accepted file prefix; no late disconnect before transport Drop |
| Host wait | Controlled blocking-worker release preserves consumed-message ownership; it does not prove arbitrary worker termination |
| Cleanup | Native secondary failure, total expiry and host-cleanup expiry retain the earlier error; actual default five-second disconnect expiry retains business primary and Elapsed separately |
| Source lookup/public use | Bounded cyclic/context/IO source traversal; external getter/setter, nine Send futures and exhaustive four-variant consumer compile |

Twelve tests added: eleven internal/one external. Final all-feature and
no-default-feature new-test runs each pass twelve; backup lib regression passes
107 with one existing ignored, explicit backup integration passes eleven and
DeviceLink regression passes fourteen. Five original CLI 16-KiB stack checks,
21 backup CLI tests, strict Core/CLI Clippy and fmt pass. Fixtures own their
futures and controlled joins; no old-source negative control, benchmark,
physical device or other-platform acceptance was performed.

Preparation provenance is retained: the handoff's first backup regression was
95 PASS/one FAIL/one existing ignored, followed by moving the terminal marker
before business parsing. The handed-over latest source had only fmt/whitespace
checks. Resumed narrow session68597 exited101 with test-only E0283 at the new
dictionary value; explicit plist::Value fixed it. Session74098 exited0, twelve
PASS; lib session89356 exited0, 107 PASS/one ignored. Pipeline11316 then stopped
101 at explicit integration, three PASS/eight FAIL: expected peer closure after
complete terminal replies had become a reported disconnect error. Restoring
the existing exact suppressible IO-kind policy fixed production compatibility
without changing integration or original session assertions. Final pipeline82468
exited0 for all narrow/regression/minimum/integration/DeviceLink/CLI/Clippy/fmt
checks above. Source audit preserves the whole original transfer/host-command
tail except the reviewed terminal marker, Hello parser, old session/integration
assertions and prior public-consumer bytes. Native deadline fixture count stays,
with explicit migration to no late disconnect and primary-first dual failures.
DeviceLink gains only a private stream_ref; protocol/host_fs/manifest/root/enums/
classifiers/frontends/dependencies and reference HEAD/status remain unchanged.

Full section 11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session1410 terminal0, validation/full PASS: **2216 Rust/13 Python**, zero failures,
two existing ignored; CLI706/Core1354/public46. Rust and Python checks are split.
Workspace fmt/check/strict Clippy/MSRV, 66 feature checks/four runtime groups,
7+7 doctests/three downstream snippets, C ABI, Python 3.12 wheel/sdist/install,
53 tooling/118 actual-help/38 raw capture/five host/15 CLI/three local backup
contracts pass. Chromium and installed 3.9/3.13 were not rerun; 22 device rows
remain NOT_RUN. No maturity upgrade.

Frozen source SHA256:
`64e0009837d0f74c146897ab3f3e01312b088659f83d23efcdcd7d10fb61d36c`.
Final ios debug binary SHA256:
`235f77ff5cfd61afa1b3838c0b048fe929a499800f8a4c71d2f4cfa0027a45c1`.
Source stayed frozen through the full gate and implementation commit. Free disk
was about 6.0 GiB before full and 4.2 GiB after; no clean/storage failure.
Local logs/audits use `/tmp/ios-l01-backup-lifecycle-`; outcomes are retained here.

Documentation audit PASS: nine documents, eight Markdown files/280 local links;
all 31 tasks/subtasks/prior evidence retained with only the L01 overview row
changed and this delivery appended. Tested commit/frozen source/binary, full and
narrow provenance and read-only references match. Capability118 bodies/maturity/
history remain unchanged with one host evidence entry; source and actual CLI
help catalog validation PASS. No product source changed during documentation.

L01/class 4/all 31 tasks remain active: 9 complete, 11 not started and 11 in
progress/awaiting acceptance or conditions. Next independent owner is
NotificationProxy framing/idle/subscription/shutdown. Other protocol/cleanup,
borrowed raw sessions, setup/service/multi-request totals, long streams/manual
pairing, blocking-worker/resource bounds, L02–L04/Q04 and classes 5–9 remain.
Real devices/platforms/credentials/protocol evidence stay separate conditions.
No device/Apple operation, reference edit, push, publication or history rewrite.

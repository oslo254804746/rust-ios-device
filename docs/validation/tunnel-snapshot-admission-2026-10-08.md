# Managed discovery snapshot and candidate admission

Implementation `327b0ef38e43576f2313c51c597b5c16cb7abc01`, parent `061bf296653f8f1fcd8ab97bf4e29f33b6c44c65`; accepted 2026-10-08 UTC on
Linux x86_64,Rust1.97.1/MSRV1.80.1,project Python3.12.13.
6 source files 743+/5-;14 additive regressions,all preceding tests/assertions remain.
This is ordinary L01/L02 manager admission,not Q04 or its UID/resource experiment.

Private logical limits:

| Input | Ceiling |
| --- | --- |
| Raw USB or paired rows,each family |16,384 |
| USB unique IDs,including unauthorized |4096 |
| Each UDID or paired host |4096 UTF-8 bytes |
| Raw string bytes,each family |16MiB |
| Combined authorized scan candidates |4096 |
| Owned candidate UDID+route-host bytes |16MiB |

Raw USB bytes count all UDIDs;paired bytes count all UDIDs+hosts. All byte/count
additions are checked. Duplicate raw rows still consume raw count/bytes;selection
keeps first USB/last non-USB and first discovery position. Paired planning keeps
USB priority and first paired host. No new empty-value/NUL/format/wire rule.
These private ceilings are managed admission behavior,not Core protocol maxima.

Four Source consumption points validate entire snapshots:initial scan USB before
attached/eviction/selection clones;publication USB before any matching row;
paired scan before complete candidate planning;default manual fallback paired
before find. A matching first row never bypasses a later oversized tail.
USB raw refusal uses existing CLI typed resource_limit before attachment eviction
or new gates. Existing permission cleanup stays. A post-validator epoch check
continues to discard stale successful USB replies.

A complete borrowed paired plan uses indices and borrowed IDs before any paired
String clone/ticket. Permission recheck only deletes planned rows. Raw/combined
refusal rejects the entire paired batch and drops its targets immediately,then
retains unknown Mobdev2 backoff and processes legal USB candidates before returning
the resource error. Prior readiness is unchanged;new readiness is not published.
Original Source Err stays outside private Complete/Incomplete admission state;
stale Source success remains quiet Ok,including invalid paired metadata after
actual clear. Both acquired native paired error fences remain. USB tickets still
precede paired discovery;setup/candidate clocks,four-item window and per-Ticket
cancellation/default manual fallback behavior stay.

Previously accepted oversized snapshots now return resource_limit;this pre-1.0
behavior needs review. Public struct/error enum/options/wire/dependency/feature/
Core/bindings stay. Source materializes input before admission;vector capacity,
initial publication clones,arbitrary manual parameters/global gate and failure
strings,cached/native/list/federation metadata remain separate budgets. Candidate
seen ID and reconcile host copies each add a separate bound derived from the
candidate strings. These tests do not prove Source allocation,allocator capacity,
RSS/CPU/throughput/long-run resource release or whole-scan total/persistence.

Actual manager regressions cover USB raw rows/unique/UTF-8 field/aggregate rejection
and exact acceptance;paired raw reject before fresh prefix tickets while preserving
old backoff and legal USB publication;combined count/bytes reject without partial
handoff;combined exact count/bytes keep4active futures and restore permits;all raw
IDs counted under real configured authorization;actual HTTP policy reload removes
B without adding newly authorized unplanned C;default manual fallback validates
its matching-target tail. Two actual native userspace fixtures prove USB oversized
tail retires the new exact Arc while old Arc stays live,then original native joins,
status/DuplexEOF/taskbaseline recover before regressions;actual clear with invalid
paired Source success stays quiet. Private metadata is not mobdev2/DNS/device proof.

Old production47308 terminal101:19PASS5FAIL (7new=2PASS5FAIL,17oldPASS),exact455
product hashes. New row/field/aggregate/paired/manual cases returned Ok. The initial
unique test already passed by reaching existing coordinator capacity;its helper
was subsequently strengthened to require unchanged gate count,not claimed as an
old failure. Baseline helpers clear/drain fake no-native owners before regression.
V1 24740 all26PASS,v2 93542 all28PASS,v3 72932 all31PASS(14new17old).
Read-only peer review confirms production wiring;root added its missing exact-byte,
unauthorized-ID/reload/fresh-prefix assertions. No peer edits or Rust executions.

Final12016 terminal0;all seven SERIAL gates freeze457 products before/after:
CLI230PASS/1existing ignored,minCLIalltargets/originalstack5/strictworkspace/fmt/
section11 full/.venvtools73 PASS. Full2754Rust/13Python,CLI1093/Core1496/public55,
0FAIL/2existing ignored;MSRV1.80.1/66features/fourgroups/docs7+7/snippets3/C ABI/
Python3.12 rebuiltwheel-sdist/install/import/help118/raw38/host5/CLI95/backup3 PASS.
22device NOT_RUN;Chromium/installed3.9/3.13 NOT_RERUN,no maturity upgrade.
PatchSHA256 `7faa7b3f9ecab3a12900dbf6cfc5567804427b73922b3d0267b434d29079a0a8`;actualiosSHA256 `24fcb89c22b621d619df6cf5b3f53d567ee2ff6924f54fc4bc6c666a9ad5f9f7`.
Evidence `target/validation/tunnel-snapshot-admission-2026-10-08/`;source/command/
exit/log hashes and references verified. Root alone ALLRustserial;no source/docs
edits during live validation;CARGO_INCREMENTAL=0,no cache deletion/push/publish/
history rewrite/device/credential/Apple/elevated TUN actions.

Active all30 original main tasks and all children excluding Q04 ONLY:
9complete/21unfinished(9notstarted/12progress-or-conditions);UID untouched.
Whole-scan total/cancel/paired primary across outerdrop/out-of-clock failure writes,
global metadata retention/list/federation bytes,physical freshness/reuse/restart/
manual epochs/HTTP/upgrades/blocking/startup,L03-L04/classes5-9 and all original
other acceptance remain. No main task,class4 or goal completion is claimed.

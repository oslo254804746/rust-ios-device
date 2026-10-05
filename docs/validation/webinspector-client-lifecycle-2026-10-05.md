# WebInspector client lifecycle host acceptance (2026-10-05)

L01/class4 implementation `30fb63fe5d6a6530bb96270adbb59c27c96d1e27`,
based on `09d5fcd9eacc9f15dcda1a3b3bfc62e7d4fbd228`.
Four source/test files,1210 insertions/55 deletions. Linux x86_64,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. See
[roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md) and
[errors](../error-contract.md). This profile covers supplied-stream client frames,
startup and incoming event routing; correlated Inspector/Automation, retained
cache limits and whole frontend/server workflow owners remain separate work.

WebInspectorClient adds is_usable and set_operation_timeout(Option<Duration>).
Default None retains unbounded client sends/event reads. Startup/timed-event
operations use the shorter explicit/configured budget. Each send admits before
argument cloning/JSON encoding and shares one budget through XML send/flush.
An event total includes read/decode/routing and automatic application-listing
writes, without restarting after a completed incoming frame. Each operation
poll blocks work at a due nominal deadline while the native timer supplies the
actual Elapsed. No synthesized timer error, reader task or event queue is added.
Synchronous codecs and already running work remain cooperative.

Zero rejects before cached events or IO as source-free legacy Timeout(duration).
Unrepresentable effective budgets return Io InvalidInput; explicit shorter limits
can make a MAX configured cap valid. Startup overflow previously returned Protocol;
timed zero/ready-cache and page-discovery zero behavior deliberately migrate to
checked admission. Six error variants, their native duration wrapper/getter,
existing constructors/root paths, models, protocol strings and16MiB raw framing
stay. Startup now reports its effective operation budget instead of a varying
response remainder. No bypass/deprecated alias is introduced; release review must
include these pre-1.0 timing/source/reuse migrations.

Directly owned frame guards track accepted read/write bytes, including bytes
delivered from an enclosing BufReader, native errors, EOF and failed flushes.
Cancellation/expiry after progress, native failure or codec error invalidates
the client. Local XML failure before IO, unpolled futures and empty blocked
sends/idle reads can retain reuse. Invalid state drops owned application/page/
pending collections and automation metadata; every later guarded wire/cache-read
operation rejects before IO. Restore attempts on invalid state discard supplied
events; setter None cannot revive a transport. Connection identifier stays.
Opaque stream/buffers remain owned until client Drop. This is local admission,
not peer liveness, remote cleanup, retry permission or measured memory recovery.

A decoded incoming event remains owned until routing and automatic listing
writes finish. A cancellation while those writes have accepted zero bytes still
invalidates the already consumed event and clears partially updated caches.
Complete routing/shape/JSON/PID-conversion failure retains its original error
while invalidating. Existing correlated business parsing after a returned event
is outside this guard and is not changed by this profile.

Startup owns the entire report-to-CurrentState sequence. A sent report without
the expected state invalidates on caller cancellation or expiry, even after an
empty response read; late complete state cannot be consumed by a subsequent
call. Both APIs retain legacy mapping: blocked report outer-timer expiry remains
native Timeout, report transport EOF/BrokenPipe still maps to Disabled on start,
and response failures keep their existing Disabled mapping. The preserving API
retains originals. Disabled alone does not prove a device setting. Successful
CurrentState clears the logical owner, without guessing an ACK/teardown/retry.

open_application_pages retains ordinary snapshot completion only for an empty
idle expiry on a still usable client. Partial frames or interrupted routing now
return the original failure instead of a successful snapshot. An optional total
covers application request and subsequent reads; it stays an error independently
of per-event idle quiescence. Default None keeps the old idle-discovery profile.
Direct raw wire helpers remain caller-owned and unchanged; no implicit stop or
remote release is performed by guard/client Drop.

| Owned fixture | Observation and boundary |
| --- | --- |
| Admission/cache | Twelve client entries reject zero/MAX effective budgets without IO/cache removal; explicit zero/MAX retains ready cache; shorter explicit event limit safely clamps MAX configuration |
| Due timer | A nominal-now native timer suppresses ready-operation polling while retaining real Elapsed |
| Empty idle | Actual expiry/caller cancellation preserves metadata and a subsequent complete CurrentState event |
| Partial input | Header/body expiry/cancellation clears every metadata getter and pending collection; all twelve entries/restore reject reuse, all owners Drop gives EOF |
| Buffered input | A complete first event leaves two cached header bytes; a later zero-new-wire expiry still counts that progress and invalidates |
| Send ownership | Unpolled sends/start/read do nothing; cancelled empty prefilled write preserves later successful report, partial accepted write invalidates |
| Native/codec/routing | Native EOF/BrokenPipe, bad plist/typed non-dictionary,16MiB oversize, complete bad shape, JSON failure and PID ParseIntError retain original variants/native kinds/sources; invalid reuse rejects |
| Local XML | Unsupported local XML UID fails before IO while retaining ready state/cache |
| Flush | Complete report followed by blocked-flush cancellation or native PermissionDenied invalidates and releases local ownership |
| Startup/late reply | Legacy/preserving committed caller Drop or expiry invalidates; complete late CurrentState cannot be read |
| Startup total |200ms includes125ms blocked send plus125ms late complete response, with native Elapsed and unusable client |
| Automatic listing | Complete application event followed by empty blocked listing send invalidates on expiry or caller cancellation; no late write after buffer release |
| Event total |125ms incoming event plus125ms blocked automatic send shares200ms, with original total/native Elapsed and no late listing |
| Discovery | Empty idle returns snapshot/reuse; partial idle returns failure; configured total differs from idle quiescence and shortens explicit event timing |
| Successful routing | Original application event/cache and exact listing selector/two argument keys retained |
| External consumer | Setter/getter, thirteen unpolled Send futures, exhaustive six variants, zero no-contact and native EOF/invalid reuse |

Sixteen new internal and one external test. Initial compile43968 terminal101
failed on23 test-only E0283 type-inference diagnostics; concrete dictionary key/
value types corrected them. No Rust tests were counted as passing that attempt.
The admission fixture also corrected MAX input roles to test effective limits,
with a separate shorter-explicit-clamp case; no runtime failure is claimed for
that pre-execution correction. Narrow23094 then passed17.

After adding native-write and cancelled automatic-send branches, pipeline63338
terminal0 passed final full17/minimum17, Core WebInspector55 plus3 public/1
integration=59, CLI WebInspector40, minimum CLI all-target check, original stack5,
strict Core/CLI Clippy and fmt. Two more native JSON/PID branches changed only the
new fixture file; production/public hashes remained identical to that pipeline.
Supplement29765 terminal0 again passed narrow17/minimum17/strict Clippy/fmt,
then the four-file snapshot froze. All old client tests/assertions, startup
mapping, incoming routing/model/wire/timing/Inspector/Automation and original
external consumers remain byte-identical. Protocol strings/default None/16MiB,
frontends/dependencies/public families and read-only references stay.

Fixtures use2s outer bounds, SHORT20ms and125+125ms under200ms. They prove owned
duplex/buffer/source behavior, not device services/browser/OS signals or latency/
throughput/RSS. No baseline-wide old-source negative control is claimed. Every
process was terminal before source editing. Focused cargo clean -p ios-cli
--profile dev removed1657 cache files/6.6GiB without a live build; no source/
reference removal or storage failure. About2.9GiB free before cleanup,8.0GiB
after narrow and5.6GiB before full.

Full section11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session23783 terminal0, validation/full PASS: **2314 Rust/13 Python**, zero FAIL,
two existing ignored; CLI752/Core1402/public50. Rust/Python checks split;
workspace fmt/check/strict Clippy/MSRV,66 features/four runtime groups,
7+7 doctests/three downstream snippets, C ABI,3.12 wheel/sdist/install,
59 tooling/118 actual-help/38 raw/five host/55 CLI/three backup contracts pass.
Chromium and installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Frozen source SHA256:
`3db99be39a1149eab1064b21ac22630b6f0610a2dca23d738deabbb5e5b0570b`.
Final ios debug binary SHA256:
`503a27aa69f3cdff1637a4ccaaa0b08742798ad02a4c9ac29f2c8660635a52bf`.
The four-file patch stayed frozen through full and exactly matches the actual
implementation commit. Local logs/audits use /tmp/ios-l01-webinspector-; outcomes
are retained here.

Documentation audit PASS: eight documents, seven Markdown files/324 local links.
All31 tasks/subtasks/prior history prefix remain, with only the L01 overview row
and final delivery appended. Exact implementation/source/binary/full/narrow and
compile/supplement/cache-clean provenance match. Capability118 bodies/maturity/
prior evidence remain with one host entry; source/actual CLI-help validation PASS.
Read-only references and product source remain unchanged during documentation.

All31 tasks/subtasks remain with9 complete/11 not started/11 in progress or
awaiting conditions/acceptance. Next Inspector/Automation correlated totals/
attachment/late-reply ownership, bounded pending/skipped/application/page state,
then WebInspector frontend/server lifecycle. Other L01 protocols/raw borrowed
sessions/multi-request totals/long streams/manual pairing/blocking resources,
L02–L04/Q04 and classes5–9 remain. Capability118 bodies/maturity/history and
platform/device/credentials/protocol conditions stay. No reference edit,
subagent, push, publication or history rewrite. Host acceptance does not complete
L01, class4 or the overall goal.

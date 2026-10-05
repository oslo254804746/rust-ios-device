# L01 WebInspector metadata and flattened snapshot admission

Host acceptance,2026-10-05. Tested implementation:
`20d6fc5774e9f2bfdc243de194ee7e62f084967c`, based on
`602ef1d3780ab2e4a5f72aa62260ac2fa45c56d6`. Linux x86_64,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. Continues the
[client profile](webinspector-client-lifecycle-2026-10-05.md) and
[correlated session profile](webinspector-session-lifecycle-2026-10-05.md).
This does not complete L01, class4 or the all31-task goal.

| LOCAL policy | Admission |
| --- | --- |
| Application cache/raw application list |1024 applications/raw entries |
| Page application sets |1024, including empty/orphan sets |
| Aggregate cached pages/raw listing |4096 pages/4096 raw entries per listing |
| Combined retained metadata |16MiB compact JSON of availability/applications/application_pages, including map keys, duplicated model IDs, containers, escaping and UTF-8 |
| Staged maps/listing event Vec |Each capped at16MiB compact JSON before insert/clone/append; the Vec counts every duplicate event |
| Flattened snapshot |Separate16MiB compact JSON of the existing ApplicationPage sequence, including the application repeated for each page |

These fixed limits are local resource policies, not inferred Apple/device
limits. Raw counts precede collection reservation and individual model parsing.
Incremental staging uses checked replacement/subtraction/addition. A borrowed
projection admits the resulting combined cache and flattened snapshot before
mutation; it does not clone the full existing snapshot or allocate encoded JSON
solely to count. Scalar Unknown automation availability participates in the
same retained budget. Accounting replaces old values rather than summing old
and incoming temporary copies.

Page replacement drops its old map rather than clearing and retaining its
capacity. Connected snapshots retain only page sets for present applications,
drop empty tables and shrink the surviving outer table. Disconnect removes
both caches and drops empty table capacity. Removed payload/count/byte budget
becomes available to later updates. Original success behavior stays: listing
events contain all duplicates in original order; cached IDs retain the last
value in first-ID order; application snapshots preserve last-value semantics
and request one automatic listing per unique ID. Rejected candidate updates
cannot partially corrupt the preceding snapshot.

Complete receiving routing failure uses the already accepted event owner to
invalidate/drop metadata and pending state. It returns private typed
WebInspectorLimit under existing Io Other, classified ResourceLimit; no new
public error family/string matching/unrelated decoder substitution. Discovery
does not return successful quiescence on resource rejection. Later guarded IO
rejects before contacting the peer. Direct private handler fixtures bypass the
frame owner only to verify pre-mutation rejection; they are not public reuse
acceptance. Diagnostics contain resource names/limits, not app/title/URL data.

All old client assertions, startup mapping, previous client/session lifecycle
tests and public consumers remain byte-identical. Public signatures/root paths,
six variants/models/serde, wire/selectors/16MiB framing, business matching,
default None and frontends/dependencies remain. Existing open_pages_snapshot
returns the same Vec/order; its possible repeated application copies are now
admitted before cache retention. Fixed admission/ResourceLimit changes need
pre-1.0 release review. No task, remote ACK/teardown/retry or compatibility
bypass is added. Invalid clients/sessions require fresh ownership, not cached
target/page transplantation.

| Fixture | Actual observation |
| --- | --- |
| Raw counts |Oversized malformed app/list entries reject ResourceLimit before per-entry parsing or old-cache mutation |
| Repeated applications |1024 actual connected/updated frames admitted,1025th rejects and clears client; no additional peer writes |
| Empty/orphan page owners |1024 empty sets count; disconnect permits another set; invisible orphan payload still counts bytes |
| Aggregate pages |4096 exact pages, cross-app4097 rejection; actual two-frame rejection invalidates; replacing with empty releases old capacity/budget |
| Duplicate pages/apps |Original event order/first-ID cache order/last values; connected snapshot removes absent page owner and sends one unique listing |
| Exact bytes |Independent materialized JSON checks exact16MiB metadata with availability/keys/escaping/Unicode; one byte over rejects before mutation; smaller replacement/readmission |
| Combined bytes |Existing app plus orphan page and actual Unknown scalar plus app share retained budget; rejection clears the receiving owner |
| Flattened copies |Actual8MiB application plus two small pages rejects before repeated snapshot retention; exact boundary or one byte below admitted, next name character rejects |
| Staging duplicates |Two8MiB duplicate page titles reject event Vec growth even though final cache would have one ID |
| Binary wire evidence |Actual bplist00 frames share8MiB strings and fit below16MiB; duplicated listing/app expansion rejects staged byte budgets; no automatic write on rejection |
| Discovery/public consumer |Resource rejection cannot become idle success; external Send/exhaustive six variants, ResourceLimit/Io, empty metadata and later read/write rejection before IO |

Sixteen new internal tests plus one external test. First narrow75451 terminal0:
13 PASS. Additional actual binary/aggregate/public fixtures compiled and passed
in33329 terminal0:17 PASS. No compile or runtime test failure in this profile.
Sequential pipeline terminal0: minimum17, Core96+public5+integration1=102,
CLI40, minimum CLI all-target check, original stack5, strict Core/CLI Clippy
and fmt. Existing builds were terminal before edits or another pipeline;
observation timeouts were not treated as exit. Read-only audits did not change
source. Fixtures have10s bounds; no baseline-wide old-source negative control,
device/browser/OS signal/benchmark or other-platform result is claimed.

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Full45509 terminal0: **2357 Rust/13 Python**, CLI752/Core1443/public52,
zero FAIL/two existing ignored. Every section11 split gate passes: workspace
fmt/check/strict Clippy/MSRV;66 feature checks/four runtime groups;7+7 doctests/
three downstream snippets; C ABI;3.12 wheel/sdist build/install/import;
59 tooling/118 actual-help/38 raw/five host/55 actual CLI/three backup contracts.
Chromium/installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

With all builds terminal, focused CLI cache cleans removed1301files/5.1GiB
initially (1.9GiB free before,5.7GiB after), then446files/1.7GiB pre-full
(3.5GiB before,5.3GiB after). Full ended with about695MiB free. No source/
reference removal or storage failure. Frozen five-file source SHA256:
`987d5836c804bdba0cde773311c3726d2b08af787ca9bb1f61c96de4ee5f52b7`.
Final ios debug binary SHA256:
`75af15d7cb44043e82dc630583077d90b38c61e9cba73a5e7a50563c559b6fcd`.
Actual commit exactly equals the frozen tested patch. Local logs/audits use
/tmp/ios-l01-webinspector-cache-. References retain prior HEAD/status.

Logical bytes bound these retained/staged projections, not RSS, allocator
capacity, total simultaneous copies, decoder peak, recursion/object-reference
expansion, CPU, latency or throughput. Locked plist1.7.1 binary writer shares
scalar references; real small shared-string frames can expand during decode.
The existing raw16MiB bound precedes plist::Value decoding; model/cache admission
follows it. Decoder preflight/resource/fuzz evidence remains L01/Q04 work, and
synchronous codecs/counting stay cooperative. Do not claim all allocation is
bounded from these cache fixtures.

Next WebInspector CLI/CDP/WebDriver/server setup/whole-flow total/pinned signal/
joined tasks/output/primary cleanup, other L01/raw borrowed/multiple-request/
long-stream/manual-pairing/blocking owners, L02–L04/Q04 and classes5–9. All31
tasks/subtasks/history and capability118 bodies/maturity remain; one host
evidence entry added. No subagents/reference edits/push/publication/history
rewrite. Host acceptance does not replace platform/device/credentials/protocol
conditions or make the overall goal complete.

Documentation audit PASS: eight documents/seven Markdown/340 local links;
source/actual CLI-help inventory118 PASS. All31 task/subtask/history prefix and
capability118 bodies/maturity/prior evidence remain, with only the L01 overview,
final delivery and one host entry added. Committed/frozen source/binary and
full/narrow/gates/cache-clean provenance match; product source stayed unchanged
during documentation and reference HEAD/status remain unchanged.

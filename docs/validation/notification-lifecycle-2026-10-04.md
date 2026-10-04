# NotificationProxy lifecycle host acceptance (2026-10-04)

L01/class 4 implementation `db732437a945c75096d05f67b9e2063838756c97`,
based on `dd1090efcb58623c776aff51e10d3598321ba895`.
Five source/test files, 849 insertions/51 deletions. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1 and project Python 3.12.13. Host acceptance only;
no device capability maturity upgrade. See [roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md)
and [errors](../error-contract.md).

NotificationProxyClient adds is_usable/set_operation_timeout. Ordinary default
None preserves unbounded send/idle receive. Explicit waits use the shorter
configured budget. wait_for's absolute deadline now includes ObserveNotification
and unrelated events cannot restart it. Zero returns existing source-free Timeout
before I/O, including next_event's formerly issued zero timer. Ordinary overflow
returns Io InvalidInput. Actual expiry retains Elapsed through the original
retained context. Budget exhaustion between complete ready events polls an
already due Tokio timer before reading another frame, retaining real Elapsed.
Encoding/decoding is cooperative; boxing the owned future preserves stack bounds.

A per-frame guard tracks accepted bytes including caller-buffered reads. Partial
frame cancellation/expiry, native IO/EOF and typed plist decoding failure clear
subscription names and reject all seven public operations before I/O. Empty idle
cancellation/expiry preserves completed subscriptions and valid late events.
A complete one-way subscription is not an abandoned request/reply. Complete
business-shape errors retain existing parser/reuse behavior. ProxyDeath keeps its
existing event/error/category, then closes local reuse. Shutdown now caps
write/flush at five seconds with shorter configured clamp, including safe MAX.
Success closes reuse, including repeat shutdown; later calls return Protocol.
Empty blocked close can retain state; interrupted partial close cannot. No ACK,
automatic cleanup, asynchronous Drop, queue or background task is added. Opaque
transport/buffers remain owned until Drop. Outgoing length checks u32 conversion
before writing; its extreme allocation boundary is source-reviewed only.

| Owned fixture | Evidence and boundary |
| --- | --- |
| Admission | Seven unpolled/configured-zero paths; three explicit-zero waits; ordinary MAX rejects before I/O. First real observe proves reuse |
| Idle/subscription | Cancelled unbounded read and actual empty next/wait expiry retain subscription; cached observe emits no duplicate; late relay remains valid |
| Partial/buffered read | Two/four/six-byte receive prefixes, cancellation and expiry; all later paths reject and late remainder stays unread. Warmed BufStream header also commits ownership |
| Write progress | Prefilled64-byte direction gives empty timed-out post/reuse; next accepted exact64-byte post prefix invalidates |
| Shared deadline | Prefilled512-byte direction releases at125ms; observe completes and reply arrives125ms later. One200ms wait expires while completed subscription remains reusable |
| Caller wait cancellation | Drop after exact completed ObserveNotification preserves subscription and next wait reuses it |
| Complete events/errors | Unknown/missing command/missing name preserve framing; later valid relay succeeds. Both event APIs and wait return ProxyDeath then reject reuse |
| Native/codec | Native EOF/BrokenPipe, invalid plist, typed non-string Name and declared oversized frame retain original family and invalidate |
| Shutdown | Exact one-key envelope/local close, MAX/short clamp, empty vs partial cancellation/expiry and actual default five-second timer under six-second fixture bound |
| Event filtering/public use | Twenty unrelated events cannot reset configured100ms wait; late events stay valid. External getters/setter/seven Send futures/exhaustive five errors and two event enums compile |

Thirteen tests added: twelve internal/one external. Final all-feature and
no-default-feature `notificationproxy` new-test runs each pass thirteen;
notification regression22, CLI notification8 and five original 16-KiB stack
checks pass. Strict Core/CLI Clippy and fmt pass. Owned pinned futures/joins and
bounded wire/rejection waits; no old-source negative control, benchmark,
device or other-platform acceptance.

Preparation: narrow22673 terminal101 had test-only E0283 dictionary-key inference;
explicit String keys fixed it. Narrow7128 terminal101 was eleven PASS/one FAIL:
ready unrelated traffic exhausted the budget through a source-free loop decision.
Polling the already due actual timer fixed production source retention. Final
narrow91032 terminal0 passed thirteen; pipeline73242 terminal0 passed regression,
minimum, CLI, stack, Clippy and fmt. Source audit preserves old core tests/serde
types, both complete-message parsers and incoming framing after mechanical
tracked-stream substitution, including four-MiB cap/direct typed decoding.
Prior public-consumer bytes remain. CLI production stays byte-identical; the old
ProxyDeath fixture now asserts terminal rejection, then uses a fresh connection
for its original zero-issued-idle/EOF assertions. Root/error/event enums/other
protocols/dependencies/read-only reference HEAD/status remain unchanged.

Full section 11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session80002 terminal0, validation/full PASS: **2229 Rust/13 Python**, zero FAIL,
two existing ignored; CLI706/Core1366/public47. Rust/Python checks split;
workspace fmt/check/strict Clippy/MSRV, 66 feature checks/four runtime groups,
7+7 doctests/three downstream snippets, C ABI, 3.12 wheel/sdist/install,
53 tooling/118 actual-help/38 raw/five host/15 CLI/three backup contracts pass.
Chromium and installed 3.9/3.13 were not rerun; 22 device rows remain NOT_RUN.

Frozen source SHA256:
`2f0c36d901792fcff6e1efa409876f1ce59e55edfb3edf69e6db21c3e007fd38`.
Final ios debug binary SHA256:
`67ef78da06e6086178032c2b2906e1f50ec8e623c53c40a3a3d296ebea4cf4fa`.
Source stayed frozen through full and implementation commit. About3.6GiB free
before full/3.5GiB after; no cleanup or storage failure. Local logs/audits use
`/tmp/ios-l01-notification-`; outcomes are retained here.

Documentation audit PASS: eight documents, seven Markdown files/285 local links;
all31 tasks/subtasks/prior evidence retained, with only the L01 overview row and
an appended delivery. Tested commit/frozen source/binary/full/narrow/preparation
and read-only references match. Capability118 bodies/maturity/history remain
unchanged with one host entry; source/actual CLI help validation PASS. Product
source stayed unchanged during documentation.

L01/class4/all31 remains active; counts stay9 complete/11 not started/11 in
progress or awaiting conditions/acceptance. Next independent increment is CLI
notification setup/total/signal/cleanup ownership: returned errors currently
skip shutdown and successful paths suppress its result. This core profile does
not accept that owner. Other protocols/cleanup/borrowed raw sessions/setup/
multi-request totals/long streams/manual pairing/blocking resources,
L02–L04/Q04 and classes5–9 remain, with device/platform/credential/protocol
conditions retained. No device/Apple operation, reference edit, push,
publication or history rewrite.

# L01 finite WebInspector CLI command ownership

Host acceptance,2026-10-05. Tested implementation:
`fa3d3f7ec523bad235c2d3986e28bf229998d1c2`, based on
`418b51dbe199e1a1dd1d9a09f4554ffbda065db5`. Linux x86_64,
Rust1.97.1/MSRV1.80.1/project Python3.12.13. Continues the
[discovery profile](webinspector-discovery-lifecycle-2026-10-05.md).
L01/class4/all31 remain active; device evidence is separate.

Opened-tabs, eval and launch previously performed root default-device discovery
before command timeout/signal ownership. Opened-tabs/eval restarted budgets at
later stages. Launch emitted output after its driver and used the caller's
remaining budget for best-effort cleanup. The new private command owner covers
default selection, connection, startup, discovery, attachment, every command,
rendering and output with one checked absolute total and one pinned Ctrl+C
future. Nested stage caps never extend that total.

Ready signal wins the due timer and ready work. A failed signal wait retains
native IO. A due total suppresses ready work, retaining actual timer Elapsed;
a single cooperative poll cannot return success after the total. Already
returned native failures stay primary. Synchronous output/serialization remain
cooperative, with no forced preemption or measured CPU guarantee.

The existing first default device wins, including a network device listed first;
explicit selection bypasses default discovery. Empty defaults retain the
existing no-device diagnostic. Zero now rejects as source-free Timeout before
any device discovery, replacing finite commands' former100ms fallback. Positive
inputs keep the100ms minimum; invalid duration/clock inputs reject before
discovery with existing sources. Opened-tabs/eval use the existing500ms idle cap
inside the total. A total shorter than quiescence can fail; continuous events
cannot repeatedly extend it into a successful snapshot.

Only completed attachment on a usable client authorizes cleanup. The real
device guard and service stream remain owned until an independent5s cleanup
total finishes. Inspector sends the existing socket teardown; Automation closes
its owned context, then its forwarding socket under the same cleanup total.
Cleanup does not re-poll a completed signal. A cancelled/incomplete committed
core operation invalidates its client, so the owner drops without guessing
recovery/cleanup requests or consuming late replies.

Primary and cleanup errors are both retained. The first automation cleanup
failure also survives a second cleanup timeout. The private wrapper exposes its
primary source and formats only static text/classes. Successful output followed
by cleanup failure now returns exit1, replacing launch's ignored cleanup error;
already emitted records and remote effects are not rolled back. No automatic
retry or device-side cleanup ACK is claimed.

Existing external selectors/fields are verified against the read-only local
references and existing core APIs. WebKit's
[pinned target owner](https://github.com/WebKit/WebKit/blob/814ebe2cfe6ab3aa3039250361171f51d3847da4/Source/JavaScriptCore/inspector/remote/cocoa/RemoteInspectorCocoa.mm#L612)
admits inspection and automation in a shared target-connection map; its close
handler verifies target/connection identity before closing/removing. Applying
the existing forwarding-socket close to the fully attached automation owner
follows that shared mechanism. This is protocol-source reasoning, not physical
device proof.

Successful opened-tab rows, full eval response JSON, human values and launch
metadata stay. CLI grammar/default numeric inputs, selectors, core APIs/error
variants/models/dependencies and server access policy stay. Exactly one old
root default-selection predicate assertion intentionally changes because finite
commands now own discovery. Original task/shell test modules are byte-identical.
Eleven source/test files,2147insertions/208deletions. These timing, zero and
cleanup-exit migrations need pre-1.0 release review.

| Current owned host fixtures | Verified observation |
| --- | --- |
| Seven control tests | Zero/overflow/native input, due admission, signal priority/IO, the same pinned signal across stages, one total and dropped pending work, dual-error/source/privacy policy, late successful poll versus retained native failure |
| Normal actual plist sessions | Tabs/eval/launch each JSON and human output; first/default and explicit selection, exact page key and metadata, Runtime/Automation commands, owned context close and exact socket identity |
| Default/connect/startup failures | Empty/list/native connect sources; connection and startup share one total; partial EOF/native plist preserve their actual sources and release the owned stream |
| Committed evaluation | Cancellation or actual expiry closes the stream; no teardown/retry consumes an incomplete command's late reply |
| Completed business/output errors | Matched business failure still cleans once; native output BrokenPipe remains primary through successful, native-failing or stalled cleanup |
| Independent cleanup | Output cancellation/expiry does not abort the separate cleanup; delayed flush completes after the caller deadline; completed output followed by cleanup failure returns failure; automation stop business failure survives teardown expiry |
| Actual Linux CLI probes | Three finite commands: empty default,200ms unanswered ListDevices and EOF, zero/two overflow kinds before default or explicit contact, human invalid input, actual owned child SIGINT; no real device/pairing service supplied |

Twenty-one new Rust tests use boxed owned duplex ServiceStreams/private source
and writer injection with4s fixture bounds. Synthetic connections carry no
forged ConnectedDevice. Thirty new actual CLI cases raise the total from55
to85; four admission-tooling tests raise full tooling from59 to63.

Historical development results were19tests10PASS/9FAIL, then20tests18PASS/2FAIL,
then20tests19PASS/1FAIL. They exposed fixture assumptions about omitted pause
keys, complete eval response envelopes and native source/class traversal.
The late-success control subsequently failed0PASS/1FAIL before the driver fix.
Previous full30439/71014 and temporary logs are historical; those /tmp files
are unavailable after the environment continuation and are not used as this
checkpoint's acceptance. Source was preserved and all gates were rerun.

Current pipeline55428 is authoritatively terminal0. Narrow/minimum21,
CLI68/Core96+public5+integration1=102, core-only webinspector minimum17,
minimum CLI all-targets, original stack5, strict Core/CLI Clippy/fmt,
targeted tooling18 and actual CLI85 PASS. CLI enables core full as a dependency;
the core-only minimum check is separate.

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Current full: **2385 Rust/13 Python**, CLI780/Core1443/public52, zero FAIL/two
existing ignored. Every section11 split gate PASS: workspacefmt/check/strict
Clippy/MSRV1.80.1;66 feature checks/four runtime groups;7+7 doctests/three
snippets; C ABI;3.12 wheel/sdist build/install/import;63 tooling/118 actual help/
38 raw/five host/85 actual CLI/three backup contracts. Chromium/installed3.9/3.13
were not rerun;22 device rows remain NOT_RUN. No browser/other-platform/device,
baseline-wide old-source negative control, benchmark/RSS/decoded peak result.

Frozen eleven-file patch SHA256:
`a61bc9678b409a9264049a2d1db784fbc5ee52ada160487c215ffe1e19a0ce7f`.
Final ios debug binary SHA256:
`5169af3728eecdb8e281852b9ddaa517d00b6bfeba4f351b31a2f9e45e93e898`.
The actual source commit matches the frozen tested patch. Current ignored
evidence directory: target/validation/webinspector-command-2026-10-05/.
No build cache clean was needed. Local reference HEAD/status remain unchanged.

Interactive js-shell, CDP/WebDriver/server totals, signal/primary cleanup and
joined child/revocation owners remain. Continue other L01 protocols/cleanup,
L02–L04/Q04 resource/fuzz/manager isolation and classes5–9. All31 tasks/subtasks,
118 capability bodies/maturity/prior evidence and9complete/11notstarted/
11progress-or-conditions remain. No subagents/push/publication/history rewrite/
reference checkout edits. Host acceptance does not complete the overall goal.

Documentation audit PASS: eight documents/seven Markdown/354 local links and
source/actual-help inventory118. All31 task/subtask/history prefix and118
capability bodies/maturity/prior evidence remain; frozen source/binary/current
full13gates match, with no product edit during documentation.

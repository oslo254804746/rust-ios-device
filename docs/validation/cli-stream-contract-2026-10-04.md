# CLI stream failure and framing acceptance

A02/class 3 host slice accepted 2026-10-04, source
`64153aae931a2b2b147ef4611ba1d5f36f9ea661`, baseline
`bc8614ba29aaba30d1fe22635a4885c83e30b40d`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Nine CLI files, 340 insertions/53 deletions.
[Error contract](../error-contract.md), [public contract](../public-contract.md).

Pcap reader failures and FPS reader-task DTX failures now keep their original
error objects through anyhow context. FPS sends DtxError rather than String
through its existing channel. Actual Elapsed survives two XCTest wait branches,
notification stream expiry and CrashReport version-query expiry. OS Trace's
two signal-wait branches, crash watch and power-assert hold propagate signal IO
failure instead of reporting a clean operator stop. These are ten formerly
discarding native paths, including two source-reviewed signal branches.

Notification observe/stream ProxyDeath events become the existing typed
NotificationProxyError::ProxyDeath, class disconnected; an event is explicit
closed-proxy evidence, without an invented native IO cause. Companion and
OS Trace's issued deadline results use source-free IO TimedOut, class timeout,
with the existing human message. Their successful operator stops remain success.
These deadline markers have no OS code or Elapsed source. Returned Display,
source access, category and safe correlation migrate; ordinary failure stays
exit 1 and machine effects unknown. A stream's prior records remain valid data.

JSON WebInspector shell no longer writes its interactive `> ` prompt or
EOF presentation newline to stdout. Human interactive output is unchanged.
This is a source-reviewed two-guard correction; no physical interactive shell
or PTY/device workflow was run. JSON consumers should parse the existing session,
value and error records. There is no new output schema or command flag.

The following is a source inventory of existing formats and stop policies,
not a claim of end-to-end device acceptance for every row. Compact JSON records
have one terminating newline. Pretty documents can span multiple lines and
must be parsed as successive JSON values rather than one value per line.
Local `--json` flags remain independent of the global `--no-json` policy.

| Surface / JSON selection | Existing stdout and stop/failure meaning |
| --- | --- |
| syslog / global | Compact NDJSON; count/capture-window expiry succeeds; reader/output failure exits 1, retaining earlier records |
| listen / global | Compact attach/detach NDJSON; stream None completes; explicit stream error fails |
| notify observe/stream / local | Compact objects; count completes. Observe's per-read timeout fails. Stream's pre-read expired deadline completes, but expiry during recv still fails; this historical distinction is retained |
| Instruments notifications / always JSON | Successive pretty objects; count completes, per-read timeout fails |
| Instruments CPU / local | Compact objects; count/window/None completes, read failure fails after the existing stop attempt. Non-finite CPU JSON is rejected before that row |
| Instruments energy/network/GPU/activity trace/HAR/FPS/kdebug / local | Compact records; selected count ends sampling; native reads fail. FPS window expiry emits a sample rather than a failure |
| Instruments process/threshold / local | Compact snapshot arrays or per-matching-process objects; first snapshot is skipped, None/count ends; native failure fails |
| crash watch / local | Compact report NDJSON; watch completion or successful Ctrl+C completes; read/output/signal error fails |
| OS Trace activity / global | Compact NDJSON; count/None/operator stop completes; configured total deadline fails with timeout |
| pasteboard watch / global | Compact snapshot/data NDJSON; successful unsubscribe on Ctrl+C completes; read/output/unsubscribe failure fails. Per-representation error data is distinct from whole-operation failure |
| companion listen / global | Compact objects in JSON mode; operator stop completes, configured total deadline fails; native read failure fails, unobserved terminal None remains an opaque failure |
| WebInspector shell / global | Compact session/value/error records; stdin EOF or operator stop completes; continue-on-error emits error records and permits eventual success; terminal failure exits 1 |
| apps pkill / global | Compact per-request progress followed by explicit complete summary; a failure retains progress and emits no complete summary |
| heartbeat / global | One pretty array after all selected pings; a failure emits no result document |
| pcap and Bluetooth capture / local | Binary destination plus one pretty completion summary; count/capture-window ends normally. Bluetooth Pcapng deliberately counts and skips malformed records; skipped_records reports that policy |
| DProxy recording | Recorded event file is NDJSON, raw capture separate; observer/write failures may follow persisted data. CLI startup text has no JSON-output flag |

Serializers retaining the R04 legacy policy can encode non-finite floats as
null. That remains distinct from CPU's manual numeric renderer, which rejects
them. Result fields can intentionally contain device/application data; automatic
failure diagnostics retain redaction. Error-format=json describes the final
failure object on stderr, not every progress/tracing line on that stream.
Closed stderr can prevent delivery while preserving the unsuccessful status.

Eight tests were added. Actual owned duplex readers distinguish Pcap capture
expiry from native UnexpectedEof, and deliver a notification containing quotes/
newline followed by a real framed ProxyDeath. The notification test checks
compact JSON escaping, actual zero-budget Elapsed, subsequent EOF and safe v1
rendering. An actual DTX EOF passes through the owned FPS channel and its real
error adapter; this does not run CoreProfile setup, reader task or cleanup.
Actual zero-budget pending timers exercise private XCTest/CrashReport adapters.
Companion/OS Trace tests distinguish source-free markers from OS/Elapsed causes.
OS Trace injects an actual empty-path file-open IO failure into its owned signal
future under both deadline modes; no parent signal or real signal-registration
fault is induced. Crash/power signal branches and JSON prompt guards are
source-reviewed and covered by compilation, without a fabricated runtime fault.

The first test preparation referenced a private Core module and failed E0603;
the fixture was corrected to encode its own existing plist frame. Final tests
pass. No old-implementation negative control or new public test-only API.
Before the two final signal/prompt guards, full CLI: 686 PASS/one existing ignored.
Final fmt/strict CLI Clippy and first canonical full: terminal 0. Canonical full
includes 2101 Rust/13 Python, CLI 686 PASS/one existing ignored, zero FAIL/two
existing ignored, 66 features/four groups, 7+7 doctests/three snippets,
MSRV/check/strict workspace Clippy, linked C ABI/malformed/checked listing,
3.12 wheel/sdist, 53 tooling/118 help/38 raw/five host/15 CLI/three backup
contracts; 22 device rows NOT_RUN. No separate Chromium or installed 3.9/3.13 run.

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. Logs use
`/tmp/ios-a02-stream-native-`: narrow.log, cli.log, clippy-final.log,
full.log and production-review.diff. The staged nine-file diff stayed frozen
through full; Core/C/Python/dependencies are byte-identical. Production diff
review covers the native adapters, source-free outcomes and guard changes;
the hash check is a freeze check, not proof of every operation's behavior.

Source diff SHA256:
`2a95e4665c28c5215b0e1147d08845810e280410af99e5566b90c151188fb3c8`.
Tested ios 0.1.13 debug SHA256:
`837a8d2d7b609a745c2821665cc2f39fb989c0b64e01003378df8a59b817197f`.
About 21 GiB free before/after; no cleanup or storage failure, duplicate full,
reference edit, device/daemon/credential/Apple action, push or publication.

Budgets, requests, success data, collection/count policy and existing stop/join/
flush placement remain. A failed reader/output can follow file/device effects;
no retry, rollback, reusable connection or remote cleanup ACK is established.
FPS's existing unbounded channel and output-error task ownership remain L01 work.
Pcap read failure still returns before the final flush/summary. XCTest JUnit
reporting and other operation/cleanup failures retain their existing ordering.

Eight documentation files are reviewed separately. Seven Markdown files/187
local links, whitespace, exact source/binary hashes and the full log pass.
All 118 capability/implementation/maturity records and preceding roadmap/
evidence remain; one host entry is added. The doc audit and catalog logs use
the same temporary prefix. These checks do not upgrade device maturity.

A02/class 3 and the overall 31-task goal remain active; class 4 has not started.
Known next executable blockers are backup's MissingValue text decision,
debugserver's InvalidService text decision and the remaining owner-issued
source-free guard review (including accessibility's empty traversal timeout).
The finite binding/probe review confirms the existing scoped mapped contracts,
not platform/device/allocator exhaustion or post-upgrade lifecycle acceptance.
Keep the [preceding native gate](cli-native-sources-2026-10-04.md).

# Companion CLI lifecycle host acceptance (2026-10-05)

L01/class4 implementation `c2fa6fb60c9bdd2f56514b7b4844fe39f596bc3d`,
based on `968dabbacf9d45e024c87cf8670791d9374dd634`.
Nine source/test/verifier files, 1371 insertions/239 deletions. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, project Python 3.12.13. See
[roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md),
[errors](../error-contract.md) and the preceding
[Companion core profile](companion-lifecycle-2026-10-04.md).
Host evidence does not certify Watch/iPhone forwarding, device stop or another
platform. The actual Linux SIGINT check stops only an owned CLI on fake discovery.

Finite Listen/Forward/Stop zero or overflowing seconds reject before explicit
or default device selection. Clap declarations, flags, defaults, help, classic/
RSD routes and output serialization remain. Zero is now InvalidInput rather
than admitting an immediately expired command. No bypass or deprecated alias
is introduced; this pre-1.0 behavior migration must accompany release review.
Automatic error v1 fields, command phase, exit1 and conservative effects=unknown
stay, even when a host contact trap proves that particular rejection sent no IO.

One pinned Ctrl+C future spans default discovery, connection, service setup and
command work. Setup has a checked30s budget, shortened by the finite total.
The optional total begins before default discovery and is not restarted between
stages. List/Get have bounded setup and the existing core10s request default;
other commands use the finite remainder when supplied. Listen uses unbounded
next_event, not the diagnostic request timeout as an idle timer. None retains
unbounded listening/forwarding wait. Setup and finite request-stage stop return
typed OperationCancelled; after listener/forwarding start, ordinary Ctrl+C stays
a graceful stop. Native signal failure retains its original IO source/category.

Ready signal wins before timer/work. At every operation poll, an already due
nominal total blocks new work while the native Tokio timer supplies the actual
Elapsed. This also covers the timer-registration gap exposed by the first test.
Finite expiry still returns a failure; its old synthetic TimedOut payload is
replaced by actual Elapsed. Listen yields after complete events and output so
continuously ready events cannot starve the control owner. No new reader/task/
queue is added. Timers remain cooperative and do not preempt synchronous codec,
rendering, output or an already running worker.

After forwarding start, CLI creates the consuming close future before output or
waiting, disarming the legacy Drop task immediately. It awaits one independent
five-second Stop after graceful stop, total expiry or returned output/signal
failure, even when the command total has expired. It keeps the client/device
owned through cleanup. Invalid state skips an unsafe attempt without inventing
a second failure. Cleanup failure after success is returned; dual failure owns
both originals, with only the primary exposed through source/category. Private
Debug contains categories and Display is static. Native cleanup UnexpectedEof
keeps the existing Io category and original native object, not Disconnected.

Additive public ForwardingHandle::close/close_with_timeout consume the handle.
Disarm happens at future creation, including an unpolled future. Zero preserves
the core's source-free Timeout policy; MAX clamps safely to5s. Success, business
failure, committed cancellation and whole-command cancellation schedule no
detached fallback. Other shared owners retain the stream until all drop. Existing
borrowed stop/stop_with_timeout and handle Drop policies remain; those legacy
paths may still retry a business-failed explicit stop. No remote stop/rollback/
retry guarantee, automatic task join or new protocol acknowledgment is claimed.

| Owned fixture | Observation and boundary |
| --- | --- |
| Admission/setup | All finite zero/MAX inputs reject; setup zero/MAX/unpolled work starts nothing; same pinned signal carries from completed setup into command |
| Shared total |200ms covers125ms setup plus125ms later stage; actual Elapsed, no detached continuation |
| Ready control | Ready stop and nominal-now timer suppress ready operation polling; timer failure retains actual Elapsed |
| Setup Drop | Owned blocked setup after64 accepted bytes drops to peer EOF |
| Forward total | SHORT20ms expiry starts exact Stop only after total due; success or complete business-failed cleanup retains primary Elapsed and sends no fallback |
| Output/signal | Actual write_line_to PermissionDenied and native signal PermissionDenied survive successful, business-failed or native EOF cleanup; private errors remain retained/safe |
| Cleanup expiry | Actual default5s unanswered Stop has separate native Elapsed; output primary remains PermissionDenied and its source chain excludes cleanup Elapsed |
| Graceful forwarding | Original human start line and exact Stop; successful stop returns success, complete business failure returns its own Protocol |
| Listener | Twenty continuously ready raw events preserve exact JSON/newlines and stop after the third callback; empty/partial listener stop drops to EOF without guessed StopListening |
| Whole caller Drop | Pending setup, unanswered Start, completed forwarding wait and committed cleanup release owned streams; no detached follow-up Stop |
| Invalid cleanup | Helper-level invalid-state branch does not poll cleanup or fabricate a secondary error |
| Existing output | List pretty JSON, Get human string and Stop human command preserve original bytes/newlines |
| Public core close | Four internal and one external signature-Send test cover no fallback, zero/unpolled, empty/committed cancellation and successful shared-client reuse |

Twenty-three new Rust tests:18 CLI,4 core internal and1 external. Final narrow
80528 terminal0 passes40 (CLI18/Core20/public2); minimum companion22 includes
Core20/public2. Pipeline26213 terminal0 passes Core Companion36 plus2 public,
CLI Companion26 (25 command tests plus the existing main clap case), minimum
CLI all-target check, original stack5, strict Core/CLI Clippy and fmt.
Owned wire fixtures use2s bounds, SHORT20ms and6s for actual5s cleanup. The
200ms total fixture is helper-stage evidence, not a physical multi-stage route.
Companion output faults use the real local writer helper with an injected writer;
no Companion OS stdout fault, device signal or throughput benchmark is claimed.

Three additional verifier tests bring test_compatibility to14: expected rejection,
forbidden endpoint contact and private diagnostic negative controls. Actual CLI
contracts extend39 to55; the original39 remain passing. New16 comprise12 JSON
Listen/Forward/Stop zero/MAX explicit/default probes,2 human admission probes,
one valid empty discovery and one actual owned Linux SIGINT. Fourteen admission
probes contact no endpoint; empty discovery sends one ListDevices/no extra
connection. SIGINT retains cancelled/companion/exit1 with peer EOF; no service,
pairing, Watch or Apple endpoint is supplied.

Preparation records: first narrow50503 terminal101 failed compilation on a test
byte-array conditional; explicit slices corrected it. Next73066 compiled and
reported CLI13 PASS/2 FAIL, so Core/public tests were not counted as passing.
One failure exposed the due timer/ready operation bug, fixed with per-poll
admission while retaining the native timer; the other corrected the fixture's
UnexpectedEof category to existing Io and added a native-kind assertion. Three
more CLI cases then brought the final count to18. No baseline-wide old-source
negative control is claimed. Each process exited before the next source edit.
Focused cargo clean -p ios-cli --profile dev removed1917 cache files/7.3GiB with
no live build; no source/reference removal or storage failure. About3.7GiB free
before cleanup,8.7GiB after and6.2GiB before full.

Full section11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session62300 terminal0, validation/full PASS: **2297 Rust/13 Python**, zero FAIL,
two existing ignored; CLI752/Core1386/public49. Rust/Python checks split;
workspace fmt/check/strict Clippy/MSRV,66 features/four runtime groups,
7+7 doctests/three downstream snippets, C ABI,3.12 wheel/sdist/install,
59 tooling/118 actual-help/38 raw/five host/55 CLI/three backup contracts pass.
Chromium and installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Frozen source SHA256:
`2363413f3539ee2a33f627aafe383fc00f7890d8ae87669aac73e7dace23e4ae`.
Final ios debug binary SHA256:
`a3910bf0bf0656c18463b8904b98edbb29c0154fcaf4bb51e197c3ad54c1126c`.
The nine-file source patch stayed frozen through full and exactly matches the
implementation commit. Source audit preserves old core assertions after private
drop flag initialization, lifecycle helpers, legacy stop APIs, parsers, four
error variants/native wrapper,10s/4MiB defaults and original external consumers.
CLI clap/help/connect/route helpers, five unchanged legacy tests, unrelated
compatibility helpers and reference HEAD/status remain. The two old CLI timing
tests deliberately migrate zero admission and issued timeout source as above.
Local logs/audits use /tmp/ios-l01-companion-cli-; outcomes are retained here.

Documentation audit PASS: eight documents, seven Markdown files/319 local links;
all31 tasks/subtasks/prior history prefix retained with only the L01 overview
row and final delivery appended. Exact implementation/source/binary/full/narrow/
failure-preparation and read-only references match. Capability118 bodies/maturity/
prior evidence remain; source and actual CLI-help validation PASS. No product
source changed during documentation.

All31 tasks/subtasks remain with9 complete/11 not started/11 in progress or
awaiting conditions/acceptance. Next WebInspector frame/client/correlated sessions/
bounded caches, other L01 protocols/raw borrowed sessions/multi-request totals/
long streams/manual pairing/blocking resources, L02–L04/Q04 and classes5–9.
Capability118 bodies/maturity/history and platform/device/credentials/protocol
conditions remain. No reference edit, subagent, push, publication or history
rewrite. This host profile does not complete L01, class4 or the overall goal.

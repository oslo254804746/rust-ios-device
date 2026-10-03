# DProxy recording observations and stderr: host validation

Date: 2026-10-03. A02/class 3 source:
`4dda7a225a10f15e22d0bd5de47c7a5b6624ca83`, branch
`feat/device-service-updates-20260927`, clean baseline
`17abc82054b8a5480e9cf34b4e0bed949891339a`. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, workspace Python 3.12.13.
This is host recorder/callback/output acceptance, not a physical proxy workflow.

The preceding decoder API retained native failures, but recorder/CLI callers
used legacy push and could still panic on stderr failure. Additive
`ProxyRecorder::record_chunk_with_observer` and `proxy_bidirectional_with_observer`
deliver independently owned DecodeFailure values to FnMut callbacks. The CLI
uses the proxy observer, emitting safe metadata at DEBUG (`-vv`). Core event
and two CLI status writes return original IO errors instead of panic.

| Step | Result / observation / effects |
| --- | --- |
| Empty chunk | No writes/decoding/callback |
| Raw write fails | Original IO; no decode/callback; partial raw writes still possible |
| Nonfatal decode failures | Ordered owned callbacks before event writes |
| Decoder later fails fatally | All callbacks first, original fatal Result; no events from that push written |
| Event serialization/newline/diagnostic/flush fails | Existing primary Serde/Io; delivered native failures remain independently owned |
| Successful recording | Original forwarding/flush/shutdown policy continues |

Failure event indices refer to decoded events in one push, not persisted NDJSON
lines or global session indices. They can refer to events skipped by fatal
decoding or later write failure. Use StreamDecoder's outcome when owned partial
events are needed. Independent decoding and returned write/fatal errors are
separate branches, not one invented source chain. Existing error families and
typed classes remain. Explicit sources/events follow the
[error privacy contract](../error-contract.md).

The proxy shares one callback under its existing recorder mutex across both
directions, before forwarding the current chunk. Callback owns each failure;
the SDK holds no session failure list. Callers must bound retention and keep
callbacks quick. Blocking callbacks stall recording, and panics are not caught.
Existing record_chunk/proxy_bidirectional select unobserved mode without
observation allocations. No new timeout, cancellation, retry, reset, rollback,
per-error payload cap, disk durability, remote cleanup or safe reuse guarantee.
Ordinary decode errors retain their nonfatal recording/forwarding policy.

CLI DEBUG uses DecodeFailure's metadata-only Debug, never native Display,
source text or raw events. The existing project logging filter sends diagnostics
to stderr, with WARN default; `-vv` enables DEBUG. Tracing keeps its existing
logging behavior, rather than a new checked-output contract. Existing status
and event summary text/redaction, stdout and JSON fields remain. Three direct
stderr sites now propagate native IO. CLI's existing dispatcher returns exit 1
for an operation Err and best-effort final reporting; startup status occurs
after initial connection/listener bind, accepted status after capture creation.
Those public command positions are source-reviewed, not physical-device runs.

Four source files: dproxy/mod.rs, new recording_contract_tests.rs, the external
public-contract consumer and CLI cmd/dproxy.rs; 710 insertions/23 deletions.
No new public type/enum variant/field, constructor, feature or dependency.
Original four exhaustive DproxyError variants/ProxyEvent fields/root/classifier,
observation types and actual DTX/H2/XPC codecs stay. Complete decoder/parser/
old tests are byte-identical except one sentence describing current adoption.
Recorder construction/meta/file layout/JSON/redaction stay. The shared proxy
retains splits/try_join/mutex/16 KiB reads/record-before-forward/flush/shutdown;
recording keeps empty/raw/decode/event order and original fatal event omission.
Whole production CLI changes only observer adoption and two checked status
writes; connect/capture/admission/protocol/TLS paths remain.

Private event-writer extraction preserves serialization, newline, diagnostic,
flush order and exact diagnostic bytes. The prototype isolated the old
IO-panic policy with expect; final returns the original IO at that same step.
Actual parent stderr remains unchanged; a Linux child exercises global stderr.
Existing Serde/raw/newline/flush/transport variants remain. Diagnostic failure
can follow a complete NDJSON line and raw bytes, before explicit event flush
and forwarding; partial JSON/newline/raw failures can leave partial captures.
No rollback or durable-storage claim.

Source diff SHA256:
`a72d1ec7cec1ecaf164bbed70c704f2ffc25538a992db34d231798bcefb29771`.
All four files including new tests staged before full. Logs share
`/tmp/ios-a02-dproxy-recording-`: original.rs/original-cli.rs,
baseline.rs/baseline-cli.rs, source-diff.patch, code-audit-final.log,
full.log and full-counts.log. Source stayed frozen throughout full and docs.

Eleven internal, one CLI and one external tests were added. Actual malformed
binary plist/DTX/XPC sources reach owned callbacks before recording/fatal
return. Actual read-only raw/event files produce OS write errors: raw failure
precedes decoding; event failure follows an owned plist observation. Existing
Serde IO conversion retains the raw OS error when consumed. Private writers
inject captured real File::open NotFound errors at serialization, newline,
diagnostic and flush stages, preserving kind/OS/class and ordering; these are
adapter injections, not actual sink write syscalls. Diagnostic failure returns
without panic after one complete NDJSON line and before flush/second event.

The extra Linux test runs its owned test-binary child with stderr redirected
to /dev/full and --nocapture. Public record_meta_event returns actual ENOSPC,
raw OS code 28 and IO class, with one NDJSON line already written; the child
exits successfully and emits an explicit stdout marker. This is actual Core
stderr evidence, not an injected writer or CLI physical startup. It was added
after the negative controls and is not claimed as an old-algorithm control.
It is cfg Linux; no other-platform syscall acceptance follows.

Joined owned duplex peers check both-direction exact wire/raw bytes, callbacks
and actual EOF; a fatal native-XPC-then-H2 fixture checks zero forwarding and
empty event file while owning both observations. Both joins have one-second
bounds and test local stream destruction, not remote acknowledgements. Broken
DTX retains subsequent raw bytes without another decode source; empty input
skips even read-only writers. Actual CLI subscriber capture checks safe DEBUG
metadata/native class and excludes private payload/hex. The external consumer
actually uses public recording/native ownership; the async public signature
is compiled but its future dropped before polling. Core duplex tests separately
exercise the live async algorithm without devices.

Negative controls: initial ten internal tests before.log **3 PASS / 7 FAIL**;
before-public.log **0 PASS / 1 FAIL**; before-cli.log **0 PASS / 1 FAIL**.
Failures are missing observations/metadata or IO panic, with original wire/
file/error assertions otherwise passing. Prototype recorder/proxy/pump/CLI
algorithms match original bodies exactly; private writer extraction isolates
the original diagnostic panic policy. No invalid fixture or compile failure
counts as behavior evidence, and no guard/lint was weakened.

| Check | Result / log |
| --- | --- |
| DProxy all-feature / minimum | 36 / 36 PASS / after-final.log, minimal.log |
| CLI DProxy scope | 10 PASS / cli.log |
| External all-feature / minimum | 36 / 17 PASS / public.log, public-minimal.log |
| Core/CLI strict Clippy | exit 0 / clippy.log |
| First canonical full | terminal exit 0 / full.log |

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. **2068 Rust/13 Python
PASS, zero FAIL, two existing ignored**, fourteen Rust result groups and a
separate Python host group. Fmt/check/strict workspace Clippy/MSRV, 66 feature
builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/checked list,
Python 3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records,
38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI and three backup
contracts pass. No separate Chromium or installed 3.9/3.13 rerun. Tested ios
0.1.13 debug SHA256
`2f020e35dc0879d17d67afbc75844bd9779808a4beecb1939fd9353af08d248c`.
About 59 GiB free after narrow checks, 58 GiB at finish; no cleanup/storage
failure in this slice. No build/validator remains live after full.

Eight documents are committed separately; the 118 capability records retain
implementation/maturity/history with one new host entry. Seven changed Markdown
files have 171 valid local links; whitespace and accepted source/binary hashes
pass doc-audit.log. A02/class 3/overall goal remain
active, class 4 has not started, all 31 tasks/subtasks remain. Next: guarded
AFC/DTX/file/symbol source ownership and 32-bit conversion review, followed by
other typed guards/created outcomes/stream/JSON/callback/binding conditions;
L01/L03/A03/platform/device requirements remain. No physical proxy/device/
daemon/credentials/Apple operation, reference edit, push/publication, history
rewrite or maturity upgrade. See the [public contract](../public-contract.md)
and [preceding decoder gate](dproxy-decoder-observations-2026-10-03.md).

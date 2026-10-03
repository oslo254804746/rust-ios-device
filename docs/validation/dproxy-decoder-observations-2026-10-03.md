# DProxy decoder observations: host validation

Date: 2026-10-03. A02/class 3 source:
`a37a80b6eb7be4cc5f6d096e45eca1a65930e133`, branch
`feat/device-service-updates-20260927`, clean baseline
`dfeb4500badf0932fe4f2df90ba1a14572f2e503`. Linux x86_64,
Rust 1.97.1/MSRV 1.80.1, workspace Python 3.12.13.
This accepts a host decoder contract, not a physical proxy workflow.

Legacy StreamDecoder discarded errors into raw fallback/summary strings and
lost earlier events when a later frame failed fatally. The additive
`push_preserving_errors` shares decoding with `push` and returns an opaque
DecodeOutcome with events, independent DecodeFailure observations and the
original optional fatal DproxyError.

| Step | Observation / original error |
| --- | --- |
| Lockdown plist fallback | Error / original plist::Error / event index |
| DTX decode failure | Error / original DtxError / event index |
| XPC body-length check | Error / original std::io::Error / stream and event index |
| XPC message decoding | Error / original XpcError / stream and event index |
| Fixed XPC header/flags slices | Error / original TryFromSliceError; guarded unreachable |
| Decoder/Lockdown/H2/stream bounds and message-size addition | ResourceLimit / no fabricated error |
| XPC END_STREAM with pending bytes | IncompleteMessage / no fabricated error |
| Existing missing-stream decision, including empty DATA | InvalidState / no fabricated error |

Six original discard adapters retain their original objects. Four are exercised
at runtime; two fixed slice conversions cannot fail after the >=24-byte check.
The XPC length test uses u64::MAX on this 64-bit host: it preserves the existing
SDK limit IO object with InvalidData, not an OS fault or invented integer cause.
Arithmetic overflow paths and fixed slices are source-reviewed. The ordinary
admission limit test uses the existing 64 MiB+24 bound plus one byte; the
unknown-stream test uses 33 tiny frames, and H2 limit rejection needs only a
nine-byte header. No multi-GiB inputs or fabricated unreachable failure.

Failures expose direction, phase/kind, optional stream ID and an index in this
push's event list. Fatal guards have no corresponding event. Source-free local
decisions have neither original error nor ErrorClass; fatal_error_class uses
the unchanged legacy DproxyError independently. Native-source class/getters
use the existing bounded classifier. Independent failures are separate owners,
not a made-up linear Error::source chain. Metadata/count-only Debug omits raw
events, summaries and errors; explicit getters follow the
[source privacy contract](../error-contract.md).

`events`, `failures` and `fatal_error` borrow the branches. `into_parts` keeps
all three owned branches; a failure's `into_error` takes its original object.
`into_result` deliberately drops observations and, on fatal error, preceding
events, matching legacy push. Read `input_ignored` before decomposition when
needed: true means a previous DTX failure caused this call to skip input, not
an ordinary empty result, partial frame, Binary mode or an error in this call.

Four source files changed: dproxy/mod.rs, new observations.rs and
observation_contract_tests.rs, and the external public-contract consumer.
ProxyEvent's constructible fields/JSON and four exhaustive DproxyError variants
stay; new phase/kind enums start non-exhaustive and observation fields are
private. Direction/ProxyProtocol/root/features/dependencies/classifier and the
actual DTX/H2/XPC codecs are byte-identical. The recorder/pump, stream
admission/counting/clearing, H2 parser, preface handling, summaries/JSON and old
tests are byte-identical. Whole decoder bodies match after observation and
result-container normalization; the private XPC parser changes only error
adapters and preserves exact legacy summary/overflow text. Timestamps are
wall-clock values, excluded from event comparison.

The old path allocates no observation vector/box storage. The new path retains
errors until consumed/dropped, with count following decoded error events plus
a terminal guard. No new per-error payload cap, timeout, reset, retry,
cancellation, rollback, forwarding, cleanup or reuse guarantee. Existing DTX
stop/ignore, XPC failed-stream clear and empty DATA fatal policy stay. Empty
DATA behavior is not newly certified correct. Recorder and CLI still use old
push; their observers and three unchecked stderr sites remain pending.

Source diff SHA256:
`9fe368edc4102b30acb4ea579462fb9e9ab173c59034b1571a33e18c207eb730`.
All four files, including new files, staged before full. Logs share
`/tmp/ios-a02-dproxy-observations-`: original.rs, baseline.rs,
source-diff.patch, code-audit-final.log and full-counts.log. The negative-control
prototype keeps original push/decoder/parser bodies exactly. Audit normalization
initially mishandled match-arm callbacks and formatter trailing commas; its
correction changed no source. Sources stayed frozen throughout full and docs.

Thirteen internal and one external tests were added. Malformed binary plist,
DTX bad magic and real XPC UTF-8 faults preserve original objects; bounded
source traversal finds the nested Utf8Error. Fallback bytes, subsequent XPC
frame recovery, native error then fatal H2, successful event then fatal H2,
native ownership, source-free decisions and explicit ignored input are covered.
Fragmented all-protocol comparisons check legacy/observed result text/events
and every decoder buffer/count/preface/broken state. That comparison shares the
final engine; independent old-algorithm controls and whole-body audit supply
the original-behavior evidence. The external consumer constructs/destructures
ProxyEvent, exhaustively matches DproxyError and actually uses owned sources
and fatal decisions through the root public path.

Corrected negative controls: before-final.log **2 PASS / 11 FAIL**;
before-public-final.log **0 PASS / 1 FAIL**, all missing observations or partial
events. The initial plist fixture was plain text accepted by the plist parser,
so its raw-fallback assertion was invalid and excluded. It became malformed
binary plist with an independent parser-is_err assertion before both controls
were rerun. Initial before.log/before-public.log remain; no compile failure
is counted as behavioral evidence and no production guard/lint was weakened.

| Check | Result / log |
| --- | --- |
| All-feature DProxy scope | 25 PASS / after.log |
| Minimum dproxy scope | 25 PASS / minimal.log |
| External all-feature / minimum | 35 / 16 PASS / public.log, public-minimal.log |
| Strict Core Clippy | exit 0 / clippy.log |
| First canonical full | terminal exit 0 / full.log |

Full command: `python3 scripts/validate.py full --python
/home/oslo/projects/rust-ios-device/.venv/bin/python`. **2055 Rust/13 Python
PASS, zero FAIL, two existing ignored**, fourteen Rust result groups and a
separate Python host group. Fmt/check/strict workspace Clippy/MSRV, 66 feature
builds/four groups, 7+7 doctests/three snippets, C ABI/malformed/checked list,
Python 3.12 wheel/sdist rebuild/install, 53 tooling tests, 118 help records,
38 raw cases, five host workflows/22 device NOT_RUN, 15 CLI and three backup
contracts pass. No separate Chromium or installed 3.9/3.13 rerun. Tested ios
0.1.13 debug SHA256
`64ae2015597f0d01173a41bc135837faadebac53616e0eaa264000860e8ea096`.
About 67 GiB free after early controls, 60 GiB at finish; no storage failure or
cleanup in this slice. The preceding official cleanup remains in its own record.

Eight documents are committed separately. The 118 capability records keep
implementation/maturity and historical evidence, adding one host entry. Seven
changed Markdown files have 166 valid local links; whitespace and accepted
source/binary hashes pass doc-audit.log. A02/class 3 and
the overall goal remain active; class 4 has not started, all 31 tasks/subtasks
remain. Next: recorder/CLI error observations and checked stderr, then guarded
AFC/DTX/file/symbol converter ownership/32-bit coverage. Read-only
next-recording.log records actual caller/diagnostic placement. Other typed
guards/created outcomes/stream/JSON/callback/binding and L01/L03/A03/platform/
device conditions remain. No physical device/proxy/daemon/credentials/Apple
operation, reference edit, push/publication, history rewrite or maturity upgrade.
See the [public contract](../public-contract.md) and
[preceding input source acceptance](service-input-native-sources-2026-10-03.md).

Follow-up: [recorder/CLI observation and stderr acceptance](dproxy-recording-observations-2026-10-03.md)
at source `4dda7a225a10f15e22d0bd5de47c7a5b6624ca83` adds owned streaming
callbacks and uses them in CLI DEBUG metadata, with three checked stderr sites.
The decoder engine and this preceding source/gate stay; guarded conversions
and other A02/platform/lifecycle/device conditions remain pending.

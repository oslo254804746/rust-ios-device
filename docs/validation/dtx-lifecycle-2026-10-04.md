# DTX cancellation state and shared deadlines

L01/class 4 host slice accepted 2026-10-04, source
`940c4c673e7fe023503b97197777354c7f44e253`, baseline
`fc204bf6a60edf15e0c54d47777091ec6034d425`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Four core source/test files and one CLI test migration,
694 insertions/20 deletions. First canonical full failed on the old CPU fixture's
post-EOF stop expectation. Second full hit an unrelated AFC diagnostic-fixture
false positive; the corrected canonical full exited 0 against frozen source
SHA256 `e483b09329f51c4462d8f7de79d23b366b9ca3ca742dbad857e02fd5f602826d`.
[Lifecycle profile](../lifecycle-contract.md), [public contract](../public-contract.md),
[error contract](../error-contract.md).

DtxConnection tracks bytes reported by its owned AsyncRead/AsyncWrite stream.
Every public raw send/ACK/recv/channel/synchronous/asynchronous method-call
operation guards its local connection state. A completed IO/codec error or a
cancelled operation after byte progress invalidates the connection and clears
all receive queues, pending/outstanding IDs, fragments and memory accounting.
Later calls reject before I/O. The owned transport remains until client Drop;
this is not a remote stop/close acknowledgment.

Cancelled zero-byte waits remain usable, preserving ordinary listener idle
polling. An unpolled future also remains harmless; complete successful calls are
reusable. The new is_usable getter is local framing/correlation evidence, not
permission to retry an operation with unknown device effects. Low-level public
read_dtx_frame and service business logic have their own owner boundaries.

Additive request_channel_with_timeout, method_call_with_timeout and
recv_with_timeout use one absolute Tokio budget. Send, unrelated messages,
ACK writes, reply reads and reassembly share that budget. Zero returns source-free
Timeout before polling/identifier changes/wire writes; unrepresentable Duration
is source-free Input. Real expiry retains its original Elapsed inside existing
DtxError::Io. Timers are cooperative, not a hard real-time CPU limit. No new
public error variant, root export, dependency, feature, CLI flag/schema or C/
Python implementation change. CLI production is unchanged; its CPU fixture
migrates the post-failure cleanup expectation. Existing error enum definitions stay exhaustive.

CoreProfile start still accepts its existing quiet 500 ms notice slot; after
partial notice bytes it now returns the original timeout and invalid state.
Accessibility keeps its empty 300 ms initialization slots/idle None behavior;
partial initial/focus frames now fail immediately. Three owner branches no
longer return an apparent success/None for an unusable connection. Successful
request bytes, channels, metadata, filters, limits and existing timings stay.

Eleven tests added: eight codec, one CoreProfile, one accessibility and one
external public consumer. All-feature ten internal/one external and minimum
dtx eight internal/one external pass, as do strict Core/CLI Clippy and fmt.

| Actual owned host fixture | Evidence / boundary |
| --- | --- |
| Zero/Duration::MAX/unpolled calls | No peer bytes, identifiers unchanged, connection usable; no fabricated native cause |
| Channel and repeated correlated replies | Actual public requests/replies, sequential IDs and complete success reuse |
| Caller timeout of legacy method_call | Request and unrelated message read, final late reply queued on peer; every follow-up public operation rejects with no extra request |
| Partial header / fragment body | Actual peer bytes and pending read expiry, native Elapsed and cleared maps/accounting; no old bytes consumed by a later request |
| Blocked partial send | Actual 16-byte duplex backpressure; timer expires with header prefix sent, retry rejected without extra bytes |
| Unrelated notification requiring ACK | Actual request/notification; ACK blocks in 16-byte duplex and shares the original timer |
| Native EOF / bad magic | Original UnexpectedEof/BadMagic survives; state invalid and cleared |
| Empty idle then a complete frame | Actual read expiry keeps connection reusable and next valid frame succeeds |
| CoreProfile public start | Private client construction, actual setConfig/start wire and quiet/partial notice; no full device connect/handshake |
| Accessibility public idle / initial stage | Private warmed flags for idle entry; actual empty/partial read and separate initial flush stage; not Apple capability/DTX setup or full CLI cleanup |
| External consumer | Actual public API and exhaustive three-variant match under minimum dtx; zero then successful correlated call |

Duplex peer/client operations are owned; join budgets bound test exchanges and
no background fixture task is spawned. Preparatory E0599 test-helper bounds and
E0716 fixture-config lifetime were corrected. A wrong channel sign caused the
CoreProfile fixture's follow-up read to wait; the identified test process was
terminated, fixture corrected and a one-second follow-up budget added before
accepted checks. Production channel rules/assertions remain. This is fixture
preparation, not an old-implementation negative control or device defect.

Logs use `/tmp/ios-l01-dtx-`: narrow.log, narrow-final.log, consumers.log,
consumers-interrupted.log, consumers-final.log, minimum.log, clippy.log,
fmt-final.log, source.patch and full.log. Source is staged/frozen throughout
canonical full. Wire vectors and broad consumer compatibility are checked by
the complete gate, not claimed from the narrow tests alone.

First full's CLI result was 695 PASS/one FAIL/one existing ignored. The existing
owned CPU fixture expected a stop request after native EOF; an unusable DTX
connection now rejects that write. The migrated fixture still requires stop for
normal count completion, and preserves its original EOF/source/category,
preceding valid JSON row and primary error despite a failing diagnostic sink.
This is an intentional lifecycle migration, not removal of primary/output
assertions. Logs retain full-first.log and cli-cpu.log. On resumption the first
validator handle was missing; final FAIL marker and no active workspace
cargo/validator/maturin process confirmed it stopped before source edits.

Second full exited 1 at core tests (1280 PASS/one FAIL/one existing ignored).
The old AFC fixture forbade the three-digit substring 129, which can appear in
the process-keyed diagnostic correlation; AFC production was unchanged. The
separate test-only commit `fc204bf6a60edf15e0c54d47777091ec6034d425` checks the
complete owned 39-byte source representation and retains the original typed
source/bytes/Protocol assertions. A preparatory assertion incorrectly assumed
zerocopy's Debug exposes those bytes; its actual SizeError Debug omits source.
That assertion was removed; explicit into_src access still verifies original
ownership. The corrected AFC fixture passes. Full-second.log and
afc-fixture-final.log preserve evidence; no product error/cause or source debug
policy changed. This commit is the source baseline for the final L01 gate.

Final canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
Session 85615 ended with exit 0/final validation/full PASS. Fourteen Rust test
groups total 2126 PASS; Python 13 PASS, zero failures/two existing ignored.
CLI 696 PASS/one existing ignored, core 1281 PASS/one existing ignored, public
39 PASS. Fmt/check/workspace strict Clippy/MSRV, 66 feature checks/four groups,
7+7 doctests/three downstream snippets, C ABI contracts, Python 3.12 wheel/sdist,
53 tooling/118 help/38 raw captures, five host workflows, 15 CLI and three backup
contracts pass. Twenty-two device rows remain NOT_RUN. Chromium and installed
Python 3.9/3.13 were not rerun. Final debug ios SHA256
`88511ed0a9f8b7d4fe5c286c418f5593c697d1bd21bba185611baf9f201c1e3d`.
Five source/test files stayed frozen through full; about 18 GiB before/17 GiB
after, no cleanup or storage failure. Codec constants/functions before the
connection type, root/classifier/dependencies/C/Python and CLI production are
byte-identical to the source baseline; the standalone AFC fixture fix precedes
it. Source hashes prove freeze, not all device operations or CPU timing bounds.

Eight documentation files close this host slice. Seven Markdown/206 local
links, whitespace, exact source/binary/full logs and capability catalog checks
pass. All 118 capability implementation/maturity records and historical
evidence remain unchanged; one host evidence entry is added.

L01/class 4/overall 31-task goal remain active. Other protocols, setup/total/
cleanup budgets, task joins and bounded queues (notably FPS) remain executable
work. L02–L04/Q04 and all platform/device conditions remain; no maturity upgrade,
physical device, daemon, credentials, Apple action, reference edits, push/
publication or history rewrite.

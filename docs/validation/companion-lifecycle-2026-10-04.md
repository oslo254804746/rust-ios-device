# Companion lifecycle host acceptance (2026-10-04)

L01/class4 implementation `7a0ed63d99f04d99971004b177dd68e1cdf3b3bb`,
based on `976f58c2f03586467e9031be49d1ffab678a67a7`.
Four core source/test files,918 insertions/62 deletions. Linux x86_64,
Rust1.97.1/MSRV1.80.1, project Python3.12.13. See
[roadmap](../engineering-roadmap-2026-09-30.md),
[lifecycle](../lifecycle-contract.md), [compatibility](../public-contract.md) and
[errors](../error-contract.md). Host fixtures do not certify Watch/iPhone,
service/start-forwarding/remote-stop, OS signal or other-platform behavior.

Shared local usability spans client/listener/forwarding handles. The additive
client getter is local admission, not peer liveness or retry authorization.
One checked request budget includes mutex wait/send/flush/reply; default10s
stays. Zero rejects before polling as legacy Timeout(duration), overflow is
existing Io InvalidInput, actual expiry retains the original Elapsed/duration
wrapper. State is rechecked after locking to reject owners queued before an
abandoned request invalidates the connection.

Accepted buffered/wire bytes, a committed unanswered request, native transport
failure/EOF or codec error invalidate all shared owners. Empty blocked writes/
lock waits, unpolled futures and pre-I/O local failure can preserve state.
Complete business replies parse outside the guard and retain original reuse.
Opaque stream/buffers remain until all shared owners drop. No event queue or
background reader is added; synchronous encoding/decoding remains cooperative.

A listener holds one boxed read across cancelled next polls, including partial
headers. Actual idle expiry keeps the existing terminal policy even for empty
reads, now drops pending/mutex immediately and invalidates shared state. Whole
listener Drop also invalidates, preventing another shared owner consuming a
subscription event as a reply. Finished reads keep None; active zero/MAX idle
admission starts no read and does not finish the listener.

All forwarding stop paths have an independent5s cap with shorter configured/
explicit clamp. MAX safely clamps; successful explicit stop remains cached/
idempotent even for later zero/MAX. Complete business failure can retain reuse;
abandoned/native/codec stop requires a fresh transport. The existing handle Drop
still schedules a bounded best-effort task when runtime/state allow, skips
stopped/invalid/zero-configured state and checks state again after lock. There is
no new task or awaited join/remote cleanup guarantee. Complete business-failed
explicit stop can still trigger its old Drop attempt; frontend disarm/task and
primary-retention ownership remain separate pending work. This profile adds no
cleanup aggregate, rollback, protocol ACK or safe retry.

| Owned fixture | Observation and boundary |
| --- | --- |
| Admission/unpolled | Zero/MAX client requests/sends reject with original families/no Elapsed/no bytes; client remains locally usable |
| Shared request total |200ms includes125ms blocked send then125ms late reply; real Elapsed and invalid state, no late reply consumption |
| Accepted send | Caller cancellation with empty blocked send preserves reuse; accepted partial frame invalidates |
| Shared mutex/late reply | Cancel complete sent request while forwarding stop waits on lock; waiting stop rejects after lock, no second request/Drop write, owned EOF |
| Buffered replies | Complete first reply plus cached partial next header; subsequent expiry invalidates and third request rejects |
| All paths | Six public client wire entries reject invalid state; handle stop and Drop cannot send late control |
| Mutex budget | Expiry before lock acquisition retains Elapsed without progress; later complete request succeeds |
| Native/codec | Original EOF/BrokenPipe, malformed/typed dictionary plist and4MiB oversize families remain; reuse rejects |
| Business/cached stop | Complete business errors retain reuse; explicit later successful stop stays cached for zero/MAX and Drop sends nothing |
| Stop admission/expiry | Zero handle stop sends nothing and permits explicit completed stop. Actual short/default5s expiry keeps native duration/source and prevents second Drop request |
| Existing Drop | Synthetic active handle with MAX config emits exact stop; actual5s expiry invalidates, releases mutex and transport after all owners drop |
| Held listener | Cancelled next poll retains partial header/owned future; completion returns original future/unknown event dictionary |
| Terminal listener | Empty/partial actual idle expiry clears pending/mutex and shared state; later stop rejects, all owners Drop yields EOF |
| Listener input/Drop | Active zero/MAX rejection preserves listener for a complete event; whole partial listener Drop releases lock and prevents shared stop/Drop write |

Sixteen new internal and one external signature/Send/exhaustive-family test.
Initial narrow59893 terminal0 passed15 before two more internal cases brought
final full/minimum count to17. Pipeline65141 terminal0 passes full17/minimum17,
core Companion32, CLI7, minimum CLI all-target/no-default check, strict Core/CLI
Clippy and fmt. Initial stack-final filter matched0; that is not acceptance.
Corrected fits_within_windows_main_thread_budget matches the original5 and
passes in stack-corrected.log. Wire fixtures use2s bounds; actual5s cleanup uses
6s bounds. Active forwarding handles are private synthetic state; actual start
flow is not claimed by these new stop fixtures. No old-source negative control,
device/OS signal or benchmark is claimed.

Source audit retains every old core test/assertion byte after three private
shared-state fixture initializations, original external consumer bytes, command/
port/business/frame parsers, protocol strings, request default10s and4MiB cap.
Public variants/native duration wrapper/getter, CLI production, dependencies and
read-only reference HEAD/status stay. No product source edits during verification.

Preparation: no live build when cargo clean -p ios-cli --profile dev terminal0
removed2193 build cache files/8.2GiB; no source/reference removal or storage
failure. About4.1GiB free before cleanup,8.2GiB during final narrow gates,
3.8GiB after full. The first15-test narrow and expanded17 both passed; no Rust
failure was observed. The incorrect zero-match stack observation is retained
above, not counted as five passing tests.

Full section11 gate:

```sh
python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python
```

Session76233 terminal0, validation/full PASS: **2274 Rust/13 Python**, zero FAIL,
two existing ignored; CLI734/Core1382/public48. Rust/Python checks split;
workspace fmt/check/strict Clippy/MSRV,66 features/four runtime groups,
7+7 doctests/three downstream snippets, C ABI,3.12 wheel/sdist/install,
56 tooling/118 actual-help/38 raw/five host/39 CLI/three backup contracts pass.
Chromium and installed3.9/3.13 were not rerun;22 device rows remain NOT_RUN.

Frozen source SHA256:
`0bbeca9d7402da4d2f1b900df8479c00ac3d8678a6e7fef2d7946f9978fb3287`.
Final ios debug binary SHA256:
`2773252713e3e238a9032dadde71e4d63cc7da6d24db5420a228a9c6f8f54165`.
The four-file source patch stayed frozen through full and matches the actual
implementation commit. Local logs/audits use /tmp/ios-l01-companion-; outcomes
are retained here.

Documentation audit PASS: eight documents, seven Markdown files/312 local links;
all31 tasks/subtasks/prior history retained with only the L01 row and final
delivery appended. Exact implementation/source/binary/full/narrow/preparation
and read-only references match. Capability118 bodies/maturity/prior evidence
remain; source and actual CLI-help validation PASS. No product source changed
during documentation.

All31 tasks/subtasks remain in scope with9 complete/11 not started/11 in progress
or awaiting conditions/acceptance. Next Companion CLI setup/total/signal/output/
primary cleanup/Drop disarm, then WebInspector frame/client/correlated sessions/
bounded cache, other L01 protocols/raw borrowed sessions/multi-request totals/
long streams/manual pairing/blocking resources, L02–L04/Q04 and classes5–9.
Capability118 bodies/maturity/history and platform/device/credentials/protocol
conditions remain. No reference edit, subagent, push, publication or history
rewrite. Host acceptance does not complete the overall goal.

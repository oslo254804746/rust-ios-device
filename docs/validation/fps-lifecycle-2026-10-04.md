# FPS capture ownership, windows and cleanup

L01/class 4 host slice accepted 2026-10-04, source
`dcec5a2032f2994ed45678cb838447607abee65b`, baseline
`f3bbf4afcd523de6b6d94b7d73e5c898eebabecc`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Five source/test files, 869 insertions/119 deletions.
[Lifecycle contract](../lifecycle-contract.md), [error contract](../error-contract.md),
[public compatibility](../public-contract.md).

FPS previously spawned a reader feeding an unbounded event channel, retained an
unbounded timestamp window and could detach the reader on output failure.
Receiving continuously ready events could postpone an already due window.
The CLI now owns the CoreProfile session in the capture future, with no reader
task or event queue. A pinned next_event future survives sampling ticks, so a
tick does not discard a partly read DTX frame. Biased selection checks operator
stop and the absolute sample deadline before the event; events cannot require
draining the backlog before emitting a due sample.

The FPS setup budget is 30 seconds across connect_instruments, CoreProfile
connect and start. The outer default-device resolver runs before run_fps and is
outside this budget. Zero --window-ms fails before run_fps connects; an
unrepresentable sample deadline is InvalidInput. Cleanup has a separate five
second budget for stop. Actual expiry retains Tokio Elapsed. Successful stop
means the asynchronous DTX write completed, not a device acknowledgment.

Each window admits at most 262,144 frame-commit timestamps (2 MiB of timestamp
elements). The next matching timestamp fails with a private CLI ResourceLimit
decision, without silently dropping frames or allocating a second timestamp
collection. The additive instruments::iter_frame_commit_timestamps borrows the
chunk and copies the timebase scalars. The existing Vec-returning parser keeps
its filter, conversion, zero-denominator and truncated-record behavior and has
no new collection cap. Existing DTX message/retained-state limits still apply;
the timestamp bound is not a bound on total decoder memory.

Count completion, operator stop, read/admission/signal/output failure all enter
bounded cleanup. A cleanup failure after success is returned. If both stages
fail, the original primary error is the source of a safe private wrapper, and
the original cleanup error remains owned separately. The final v1 envelope
classifies the primary, keeps effects unknown and exports no unsafe source
details. The private resource decision maps only at the CLI's existing bounded
32-source scan; general Core classification remains Unknown for that marker.

Ending capture may cancel a partly read frame, which makes DTX unusable under
the preceding lifecycle profile. stop then rejects and capture reports the
cleanup failure; it does not claim a successful stop. Cancelling the entire
capture future drops the client/transport without executing asynchronous
cleanup. No reader task remains to join. Synchronous stdout writes and parsing
remain cooperative/blocking work: these budgets do not interrupt a blocked
stdout syscall or certify a hard CPU deadline. Sampling calculation, NDJSON
fields and human row format stay; stdout is flushed after each sample.

Twelve tests added: nine instruments CLI, one CLI error boundary, one core
iterator and one external consumer. Ten CLI tests pass together; output's
18-test regression suite retains its cyclic-source access assertion. Core and
external iterator tests pass at full and minimum instruments features.

| Actual owned host fixture | Evidence / boundary |
| --- | --- |
| CoreProfile setup | Public connect/start, both channel requests, machTimeInfo array and setConfig/start messages; no usbmux/pairing/device startup |
| Setup expiry | Withheld first channel reply, actual Elapsed and peer EOF after the owned setup future drops |
| Count and partial frame | Exact two empty JSON rows and stop; a 16-byte frame prefix spans a sampling tick, remainder yields the expected frame/FPS and stop |
| Ready backlog | 10,000 actual peer frames and counted AsyncRead; three sampling ticks occur while unread bytes remain. Sampler tested separately because terminal partial-frame cancellation need not permit stop |
| Timestamp admission | Actual parser/appender at exact cap and next-record refusal, last admitted timestamp retained. Prefilled vector boundary test, not a full 16 MiB oversized wire capture |
| Output and cleanup failure | Actual Linux /dev/full ENOSPC after one valid row; stop blocks after 64 wire bytes, actual cleanup Elapsed retained alongside original output error |
| EOF and cleanup-only failure | Actual native EOF stays primary despite invalid-state cleanup; successful count followed by blocked stop returns Timeout instead of success |
| Stop and cancellation | Successful injected operator stop sends stop; injected actual file-open IO retains its source (not a real signal-registration fault). Outer caller cancellation during partial read closes the peer with no detached reader |
| Iterator/public consumer | Expected filter/qualifier/truncation/saturating numeric results, zero timebase and short take; public iterator outlives the separate timebase value, legacy parser stays callable |

Fixture exchanges are owned joins with one/two-second guards, without spawned
fixture tasks. Preparation corrected a Bytes/Vec return type, a type-0 ACK used
where machTimeInfo needs type-3 Response, and a source-free IO assertion that
incorrectly checked get_ref instead of source. The first complete narrow run
had two PASS/seven FAIL; after those fixes eight PASS/one FAIL exposed the
backlog test's inappropriate successful-stop expectation after partial-read
cancellation. The corrected sampler fixture checks three ticks/unread bytes
and transport drop. These are fixture corrections, not device evidence or
old-implementation negative controls. Production DTX invalidation stayed.

Canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
Session 88111 ended exit 0 with validation/full PASS. Frozen source SHA256
`3a597ba373e6999143308542a1c93a10d1b036cb28711ded50654263403920a5`;
debug ios SHA256
`ddd91930c66b50ed5dffb69c1e4f2083146b758982a0823b818acef5a44b7b16`.
Fourteen Rust groups total 2138 PASS, Python 13 PASS, zero failures/two existing
ignored. CLI706/Core1282/public40 PASS. Fmt/check/workspace strict Clippy/MSRV,
66 feature checks/four groups, 7+7 doctests/three downstream snippets, C ABI,
Python 3.12 wheel/sdist, 53 tooling/118 help/38 raw/five host/15 CLI/three backup
contracts PASS. A separate final-binary zero-window capture with explicit UDID
returns safe InvalidInput, exit 1, empty stdout, zero trap connections and
complete child cleanup. Default-device resolution is not covered by that trap.

Logs/scripts use /tmp/ios-l01-fps-: check.log, narrow.log (compile failure),
narrow-final.log, narrow-corrected.log, narrow-accepted.log, narrow-terminal.log,
core.log, public.log, min-core.log, min-public.log, output.log, clippy.log,
clippy-final.log, fmt.log, source.patch, full.log and binary.py/binary.log.
Other instruments production, the FPS calculator, root/error families,
dependencies/C/Python source stay unchanged. Twenty-two device rows are NOT_RUN;
Chromium and installed Python 3.9/3.13 were not rerun. No maturity upgrade.
L01/class 4 and all 31 tasks/subtasks stay active; other protocol request,
cancellation/cleanup profiles, L02–L04/Q04 and platform/device conditions remain.

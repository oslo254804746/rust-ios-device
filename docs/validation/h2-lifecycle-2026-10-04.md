# H2 frame cancellation and flow-control budgets

L01/class 4 host slice accepted 2026-10-04, source
`ddd3620f058c6c23649a083d209dceb41f0eb472`, baseline
`529f65e0dd81a7bfa86170cfd6ac3a986cf3b6b9`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Two source/test files, 578 insertions/nine deletions.
[Lifecycle contract](../lifecycle-contract.md), [public compatibility](../public-contract.md),
[error contract](../error-contract.md).

H2Framer now guards read_stream, read_next_data_frame, buffer_next_data_stream,
write_stream, open_stream, finish_stream and the borrowed finish_connect stage.
Fixed-stream helpers delegate to these entries; nested stream opening uses an
internal implementation so it cannot reset the enclosing operation's progress
or budget. A private transport wrapper observes reported read/write bytes,
native poll errors, read EOF and WriteZero. It forwards flush/shutdown directly.

Cancellation after reported bytes, including partial raw headers/payloads and
split sends while waiting for credit, invalidates the connection. Native
transport failures/EOF also invalidate it while returning the original IO.
Retained H2 buffers, queued DATA, byte/window accounting and tracked stream
collections are cleared. Later framer entries reject before I/O; use a fresh
transport. The transport handle stays owned until framer Drop. Drop cannot
await shutdown or confirm remote cleanup; borrowed transports must not bypass
invalid state. No reader or cleanup task is spawned.

Zero-byte cancellation/expiry, including an already-open stream waiting for
initial flow credit, remains usable. Unpolled futures change nothing. Complete
RST_STREAM, END_STREAM and GOAWAY retain their existing stream-local rules;
the guard does not convert every Protocol error into connection invalidation.
Existing fatal protocol errors retain their earlier connection state policy.

Internal set_operation_timeout(Some(budget)) shares one absolute cooperative
Tokio deadline across HEADERS, DATA splitting, flush and flow-control reads.
Default None and all current production callers preserve unbounded framer
timing. Zero rejects before I/O/state mutation with source-free TimedOut;
unrepresentable deadlines return InvalidInput; actual expiry retains Elapsed
in existing H2Error::Io.get_ref. H2's parent module is private: no new public
root export, XpcClient option, CLI/C/Python API or constructor budget is added.
Timers cannot preempt synchronous parsing/allocation or blocking work.

The operation future is boxed before constructing the async timeout wrapper.
It remains directly owned and cancelled on Drop. This bounds nested future
storage at the wrapper boundary; it is not a throughput/latency measurement.

| Owned host fixture | Evidence / boundary |
| --- | --- |
| Zero/Duration::MAX/unpolled | Internal framer entries reject without peer bytes or stream opening, then a real frame succeeds |
| Empty idle expiry/caller cancellation | Original Elapsed retained on configured timeout, framing reusable for the next complete DATA frame |
| Partial raw header/body | Actual four/eleven-byte prefixes, configured expiry and caller cancellation; seven subsequent framer entries reject, retained data clears, complete late frame verified unread through the fixture's owned transport, Drop produces peer EOF |
| Native EOF/BrokenPipe | Closed duplex peer returns original IO kind in existing enum and invalidates framing |
| Blocked HEADERS/DATA | Four/sixteen-byte backpressure; exact wire prefix remains and no follow-up bytes are sent; outbound credit is not charged for the incomplete frame |
| Flow-control send/receive | Injected two-byte windows, actual outgoing DATA then incoming business DATA and partial WINDOW_UPDATE; shared expiry invalidates and clears the queued business frame |
| Zero-byte credit wait | Injected zero windows, empty expiry remains usable; actual connection/stream WINDOW_UPDATE lets the next send succeed |
| Complete stream shutdown | Actual RST and END_STREAM frames reject affected reads while another stream still sends/receives; existing GOAWAY regression assertions remain |
| External public consumer | Public XpcClient::connect_stream performs the full H2/RemoteXPC bootstrap; partial raw frame cancellation makes subsequent public recv/send fail without new wire operations |

Eight internal and one external test added; full feature runs all nine, minimum
dproxy runs eight internal tests. XPC regression suite: 102 PASS. Exchanges use
owned joins, with a one-second bound on concurrent peer operations; no new
fixture task is spawned. Private framer construction/window injection is not
physical peer negotiation; the external bootstrap uses only public APIs.

Preparatory compilation failed because the first external test imported the
private H2 path; it was replaced by the public XPC fixture without exporting
raw APIs. Next narrow result: seven PASS/one FAIL from assuming native Tokio
read_exact EOF had no custom IO payload; the helper now checks the original
kind/OS-code contract. No production error was flattened to satisfy the test.

First full session 57967 ended exit 1 at Rust tests with a CLI dispatch-file
stack overflow/SIGABRT. The inline async timeout wrapper amplified nested
futures. Boxing the owned operation before the async wrapper restored the
existing dispatch_file_future_fits_within_windows_main_thread_budget test;
the CLI test, 16 KiB assertion and environment were not relaxed. This is a
Linux host regression check, not Windows platform acceptance. Final narrow,
minimum, 102 regressions and strict Core/CLI Clippy/fmt pass. No old-source
negative control or benchmark was run.

Canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
Final session 10807 ended exit 0 with validation/full PASS. Frozen source SHA256
`e7a60ac94d78611310df7e4fb326b6f7b98b481594707b1e67e15b442d7a3e20`;
debug ios SHA256
`33da543e42db13bcc0454d2a3deaa6784834d6421f650498b17b61a6ff690d0c`.
Fourteen Rust groups: 2159 PASS, Python 13 PASS, zero failures/two existing
ignored; CLI706/Core1301/public42 PASS. Fmt/check/workspace strict Clippy/MSRV,
66 features/four groups, 7+7 doctests/three downstream snippets, C ABI,
Python 3.12 wheel/sdist, 53 tooling/118 help/38 raw/five host/15 CLI/three backup
contracts PASS. Source frozen until full terminal; approximately 11 GiB free,
no clean/storage failure. Logs use /tmp/ios-l01-h2-: narrow.log,
narrow-final.log, narrow-accepted.log, full.log, stack.log, narrow-boxed.log,
regression-boxed.log, minimum-boxed.log, clippy-boxed.log, fmt-boxed.log,
source-audit.log, source.patch and full-final.log.

Source audit confirms wire constants/preface/bootstrap/raw decoder/builders,
error enum and all existing H2 tests byte-identical. Root/classifier/frontend/
dependencies unchanged. Reference checkouts remain at the previously recorded
hashes/status in /tmp/ios-l01-h2-reference.log; no new protocol comparison was
needed for unchanged wire encoding and no reference source was edited/copied.

The upper XPC layer still needs guards for a consumed wrapper header with a
pending body, a sent call awaiting its first reply byte, pending-reply clearing,
stream invocation and file-transfer ownership. H2 state cannot certify those
logical boundaries; this slice does not claim every XpcClient entry rejects
queued upper-layer replies or shares one call deadline. Other L01 protocols/
cleanup, L02–L04/Q04 and all 31 tasks/subtasks remain active. Twenty-two device
rows are NOT_RUN; Chromium and installed Python 3.9/3.13 were not rerun.
Capability maturity remains unchanged; no device/credentials/Apple actions,
push/publish/history rewrite or reference changes occurred.

# AFC request cancellation and file cleanup

L01/class 4 host slice accepted 2026-10-04, source
`c4b471d33b1260736a295e8718d3c47ee6562165`, baseline
`24663c7327238461e4d67e8ddc6a7fdd02da1de0`, branch
`feat/device-service-updates-20260927`. Linux x86_64, Rust 1.97.1/MSRV
1.80.1, Python 3.12.13. Two source/test files, 966 insertions/52 deletions.
[Lifecycle contract](../lifecycle-contract.md), [public compatibility](../public-contract.md),
[error contract](../error-contract.md).

All twelve AFC request sites now share one send/flush/reply operation and local
ownership guard. AfcClient::set_request_timeout(Some(budget)) configures one
absolute cooperative Tokio budget per request; None, including the constructor
default, retains legacy unbounded request timing. This excludes argument
preparation, whole-file duration, host reader work and service setup. It is not
an idle timeout for a listener. No CLI/C/Python option or default was added.

Zero request budget returns source-free TimedOut before polling or changing the
packet identifier. Unrepresentable deadlines return source-free InvalidInput.
Actual timer expiry retains Elapsed in the existing IO custom payload, available
through get_ref. A cancelled request with no reported read/write bytes remains
usable; an already polled zero-byte send may have skipped a packet identifier.
Dropping an unpolled future changes nothing. Packet counter exhaustion returns
Protocol without wrapping/panicking or I/O.

Completed IO/frame-codec failures and cancelled requests after reported byte
progress mark the client unusable. This includes a completely sent request
awaiting its first reply byte. Every later public request rejects before wire
I/O, so a late reply is not consumed as the next operation's response.
is_usable reports local framing/request/transfer ownership, not remote recovery.
The transport remains owned until client Drop; reconnect using a fresh transport.
The existing response packet-number acceptance policy is unchanged; this slice
does not introduce a multiplexer or new numeric correlation validation.

Complete ordinary device Status errors consume their frame and preserve reuse.
The returned Status variant/category remains. A short FileRefOpenResult cannot
provide a handle for cleanup and now also invalidates the client. Existing frame
codec, 256 MiB frame/whole-file ceilings, read/write chunk sizes, opcodes, modes,
path encoding and successful serialization stay.

read_file now attempts close after a returned read failure as well as success
and the memory limit. write_file_from_reader still attempts close after host
reader/device-write failures. Both use an independent five-second cleanup
budget, clamped by a shorter configured request budget if present. A failed
cleanup marks the transfer owner unusable. Cleanup after successful transfer
returns its original error. With one failure, the original operation error is
returned; if both fail, existing AfcError::Io owns a safe private wrapper with
both original AfcError objects, whose source is the primary. General Core
classification follows that primary; the cleanup object remains owned
separately. A primary Status can therefore become an outer Io on dual failure,
an intentional returned-variant/Display/source migration with the same public
three-variant error definition.

The automatic file helpers also guard ownership between requests. Cancelling
after open while waiting for a host reader invalidates the client even with no
new AFC bytes, because close cannot be confirmed. Drop does not await close or
spawn a cleanup task. Manually opened handles remain caller-owned. No confirmed
remote handle release, rollback, safe retry, filesystem mutation or socket
shutdown is implied. A host reader implementation may own its own work; this
library does not spawn a transfer reader. Timers do not preempt synchronous
parsing, allocation or blocking work. Whole-file total budgets and other
protocol/device lifecycle profiles remain separate tasks.

Twelve tests added: eleven internal and one external public consumer. Full and
minimum afc configurations both pass all twelve; the complete AFC regression
suite has 31 PASS. Strict Core/CLI Clippy and fmt pass. Old tests/assertions are
retained without relaxation.

| Actual owned host fixture | Evidence / boundary |
| --- | --- |
| Zero/Duration::MAX/unpolled/counter boundary | Public queries and private counter injection; no peer bytes, zero/overflow deadlines keep state/counter, exhausted counter rejects without wrap. Counter injection is not a reachable short runtime rollover |
| Zero-byte send cancellation/expiry | Owned duplex with a private stalled-write adapter; native timer expiry and caller timeout retain usability, followed by a real successful request with skipped identifier |
| Sent request then caller cancellation | Actual public list request; fourteen public entries reject reuse, peer sees no second request, complete late reply verified unread through the owned raw stream after rejection |
| Partial header/body, native EOF, bad magic | Actual peer bytes/read failure and original Elapsed/UnexpectedEof/Protocol retained; follow-up rejected and safe Debug omits trace contents |
| Blocked send | Sixteen-byte duplex backpressure; actual expiry leaves exactly the request header prefix, no follow-up bytes, transport Drop closes peer |
| Complete business failure then success | ObjectNotFound response followed by a real list reply and sequential identifiers; a subsequent seven-byte open-result handle invalidates state |
| Public read_file | Open/read/close wire exchange, unchanged mode/chunk size; read Status failure still closes, dual device statuses both retained, successful content returned only with successful cleanup |
| Public write_file_from_reader | Actual host file-open IO injected by the owned reader, original OS code/kind retained; actual close on success/failure and no unexpected FileRefWrite |
| Independent cleanup expiry | Public read_file with legacy unbounded requests, actual close sent and reply withheld; the production five-second cleanup timer expires, retaining primary ObjectNotFound and secondary Elapsed |
| Transfer cancellation between requests | Actual open then pending owned duplex host reader; caller cancels, no async close/task, later query rejected and client Drop produces peer EOF |
| External consumer | Public setter/is_usable and exhaustive Io/Status/Protocol match; zero then actual successful request using only public imports |

Exchanges are owned joins, bounded at one/two seconds or six seconds for the
production cleanup deadline; no fixture task is spawned. First narrow result
was seven PASS/four FAIL: the helper incorrectly searched io::Error::source()
for Elapsed instead of its get_ref custom payload. The corrected helper verifies
the original typed timer object. Production timer objects/state were not changed
to accommodate it; no old-implementation negative control was run.

Read-only references: pymobiledevice3
`38fbd227b4cdf6cafea7b1c289d004c622cbf00c`, go-ios
`ced7e53d94a255ee064f9773a43fceaad03c546c`. Compared AFC call ordering and
ownership/correlation behavior, without copying source or adding a background
dispatcher. Existing uv.lock/go.mod local changes remain untouched; exact
reference hashes/status recorded in /tmp/ios-l01-afc-reference.log.

Canonical command:
`python3 scripts/validate.py full --python /home/oslo/projects/rust-ios-device/.venv/bin/python`.
Session 55322 ended exit 0 with validation/full PASS. Frozen source SHA256
`edf675e1b7eaa0a4c514ac1d8d9475f75010af2934ced87c5c714e4d556d1a17`;
debug ios SHA256
`c506bf709ac1db3aca9adeada84d7bc6042fe5ffb42781764df918486481750c`.
Fourteen Rust groups total 2150 PASS, Python 13 PASS, zero failures/two existing
ignored. CLI706/Core1293/public41 PASS. Fmt/check/workspace strict Clippy/MSRV,
66 feature checks/four groups, 7+7 doctests/three downstream snippets, C ABI,
Python 3.12 wheel/sdist, 53 tooling/118 help/38 raw/five host/15 CLI/three backup
contracts PASS. Remaining disk approximately 14 GiB, no cargo clean/storage
failure. Logs use /tmp/ios-l01-afc-: check.log, narrow.log, narrow-final.log,
regression.log, minimum.log, clippy.log, fmt.log, reference.log, source.patch
and full.log.

CLI/C/Python source, root/error definitions/classifier/dependencies are unchanged.
Fixtures do not establish usbmux/pairing/HouseArrest startup or device filesystem/
handle cleanup acceptance. Twenty-two device rows are NOT_RUN; Chromium and
installed Python 3.9/3.13 were not rerun. Capability maturity remains unchanged.
L01/class 4 and all 31 tasks/subtasks stay active; H2/XPC, other service cleanup,
L02–L04/Q04 and platform/device conditions remain.

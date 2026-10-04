# Deadlines, cancellation and connection state

L01/class 4 is in progress. The DTX profile's accepted implementation is
`940c4c673e7fe023503b97197777354c7f44e253`; the FPS owner profile is
`dcec5a2032f2994ed45678cb838447607abee65b`; AFC request/file ownership is
`c4b471d33b1260736a295e8718d3c47ee6562165`; the internal H2 framer profile is
`ddd3620f058c6c23649a083d209dceb41f0eb472`; the XPC client profile is
`09ed113e2d19489259a43c3a35de9e00caacb1a6`; borrowed RSD bootstrap/read ownership is
`85b8621f069b164d7da251a9531faf43b3456691`. This contract records implemented profiles and
their limits; it does not certify every service or remote cleanup.

| Deadline role | Meaning |
| --- | --- |
| Connect | Transport, discovery/bootstrap or service setup budget; each documented entry owns its start point |
| Request | One send/reply operation, including protocol flow-control and unrelated messages/ACKs |
| Total operation | Shared absolute budget across explicitly participating stages; separate per-stage timers do not establish this |
| Idle | Waiting for a complete next event. Empty idle expiry can be an ordinary listener outcome; a partial frame is different |
| Cleanup | A separately bounded stop/close/join budget; failure must retain any earlier primary operation failure |

InstallCoordination's existing query profile shares send/receive budget and
invalidates an uncorrelated request after I/O begins. DTX now has a corresponding
local state profile; other protocols/owners still need their own implementation
and acceptance. Existing service defaults are not globally replaced here.

`ios_core::dtx::DtxConnection` adds request_channel_with_timeout,
method_call_with_timeout and recv_with_timeout. They use one absolute cooperative
Tokio deadline for the entire operation; notification/ACK traffic does not reset
it. Zero budget returns a source-free timeout without polling the operation,
changing identifiers or sending bytes. Unrepresentable budgets return typed
input errors. Actual timer expiry retains Tokio Elapsed in existing DtxError::Io.
Legacy entry points keep their existing unbounded timing profile.

Existing public DTX I/O operations own a guard and is_usable reports local state.
A completed transport/codec error makes the connection unusable. Cancellation
after any reported read/write bytes also invalidates it and clears pending
replies, live-message backlog, outstanding IDs, fragments and retained-byte
accounting. All later operations reject before I/O; reconnect with a new
transport. The transport stays owned until the client is dropped. No remote ACK,
shutdown completion, rollback or safe retry is implied.

A cancelled zero-byte wait keeps framing/correlation intact and remains usable.
This preserves normal idle polling. Dropping an unpolled future also leaves the
connection untouched. Successful complete operations remain reusable; a service
business error parsed after a complete DTX message is outside the codec guard.
The standalone read_dtx_frame helper has no connection guard; its caller owns
cancellation/reassembly policy.

CoreProfile's 500 ms optional initial notice wait still accepts an empty idle
slot. Accessibility's 300 ms initial slots and focus idle wait retain their
empty-idle outcomes. A partial frame timeout now fails immediately with original
Elapsed rather than returning success/None for an unusable connection. Channel
selection, event filtering, ordering and successful serialization stay.

Host fixtures and limits: [DTX lifecycle gate](validation/dtx-lifecycle-2026-10-04.md).
Compatibility: [public contract](public-contract.md); classification/effects:
[error contract](error-contract.md).

FPS capture owns the session/read future directly, without an event queue or
background reader. A sample tick preserves an in-flight partial read; operator
stop and an already due absolute window take priority over ready events.
Setup shares 30 seconds across service connect/CoreProfile connect/start after
the outer device resolver. stop has a separate five-second cleanup budget.
Timestamp retention is capped at 262,144 per window; the next record fails with
a CLI resource_limit decision. The additive iterator avoids another collection;
the legacy Vec parser remains uncapped and compatible.

Cleanup after success can fail; after a primary failure both original errors
remain owned and the primary drives the final category. Count/stop during a
partial read can invalidate DTX and make cleanup fail. Whole-future cancellation
drops transport without async stop or a detached reader. Successful stop is a
wire write, not an ACK. Synchronous output/decoding cannot be interrupted by a
Tokio timer. See [FPS host fixtures and migration](validation/fps-lifecycle-2026-10-04.md).

AfcClient has a shared send/flush/reply guard at every request site. Optional
set_request_timeout(Some(budget)) uses one absolute per-request budget; default
None retains legacy unbounded request timing. Zero/unrepresentable deadlines
reject before I/O, actual expiry retains Elapsed in IO.get_ref. Cancelled
zero-byte requests remain usable; completed IO/codec errors and cancellation
after bytes reject reuse. Complete ordinary Status errors preserve framing.
is_usable also reports automatic file-helper ownership; it is not a remote ACK.

Automatic reads/writes attempt file close after returned operation failure and
success, with an independent five-second budget (or a shorter configured request
budget). Failed close or transfer cancellation after open requires a fresh
transport, including cancellation between requests while waiting for a host
reader. Dual failure keeps original primary and cleanup objects; the primary
drives classification. Drop does not await close, manually opened handles remain
caller-owned, and borrowed raw transports must not bypass invalid state. Existing
packet-number acceptance and frame/whole-file limits remain. See
[AFC fixture boundaries and migration](validation/afc-lifecycle-2026-10-04.md).

H2Framer guards its frame I/O entries and the borrowed finish-connect stage.
Cancellation after reported bytes, transport errors and native EOF invalidate
local framing, clear retained H2 buffers/queues/accounting and reject further
framer I/O. The handle stays owned until Drop; borrowed transports must not
bypass this state. Zero-byte cancellation/expiry and unpolled futures remain
usable. Completed stream-local reset/EOF/GOAWAY keep their previous isolation.

Internal set_operation_timeout configures one absolute budget across a framer
operation's HEADERS/DATA/flush/flow-control reads. Default None and all current
production callers retain unbounded framer timing. This adds no public raw-H2
export or frontend option, connect budget, whole XPC-call deadline or async
cleanup. The held operation future is boxed before the timeout wrapper to keep
nested service/bootstrap futures within the existing CLI stack test. It is
still owned directly, without a background reader/task.

Those logical XPC boundaries cannot be inferred from the H2 byte guard. See
[H2 fixtures and limits](validation/h2-lifecycle-2026-10-04.md).

XpcClient now reports local is_usable and adds two call_with_timeout variants
(stream 3/stream 1) and three single-read timeout variants (stream 3/1/any).
Each owns one absolute budget; a call includes send/flow control, unmatched
replies and reception. Zero/unrepresentable budgets reject before I/O or IDs,
actual expiry retains Elapsed. Legacy timing stays unbounded. A single-read
budget is an explicit idle policy, not a lifetime deadline for a long stream.

The XPC operation guard tracks consumed wrapper headers and successful sends,
including headers already buffered by a previous H2 operation. Cancellation or
failure after that commitment, or lower H2 invalidation, clears pending replies/
budgets and rejects cached/wire reuse. Empty waits and unpolled futures remain
usable. Complete stream-local failure before message/request commitment can
preserve other streams; an abandoned committed call requires a fresh transport.
Message-ID exhaustion rejects without wrapping while leaving receive framing
usable. The getter does not prove peer liveness or every stream's availability.

Raw stream invocations retain ownership across yielded messages; dropping the
stream after send invalidates the client. Cancelling a next() poll alone keeps
the stream's held read future. The CoreDevice owner completes only after its
existing parser recognizes finishStreaming. Finite Pasteboard send/reply uses
one guarded uncorrelated operation, skips existing empty ACK forms, and parses
business errors after the completed boundary. Existing service timeout wrappers
remain; these normal completions preserve reuse.

Announced Cryptex file sends guard ownership between H2 writes, including host
reader work. Failure/cancellation after the preamble invalidates; successful FIN
preserves the response half. Default timing remains unbounded and there is no
new host-reader/whole-file budget or async cancel/FIN task. Transport stays
owned until Drop; no ACK, cleanup, rollback, safe retry or device acceptance is
implied. Public split send/recv and other multi-service totals require their
owner's contract. See
[XPC fixtures and migration](validation/xpc-lifecycle-2026-10-04.md).

Borrowed RSD initialization/queued bootstrap retains ownership after its first
complete send. Raw stream-1/stream-3 reads retain a consumed 24-byte wrapper
header, including one buffered by an earlier H2 operation. Discovery retains
completed messages while skipping non-handshake replies with its existing
six-message parser policy. Cancellation/failure after that commitment, or lower
H2 invalidation, invalidates the borrowed framer and clears its H2 state.
Empty idle/unpolled futures, successful completion and complete stream-local
failure before commitment preserve local framing as applicable. This does not
prove that every stream remains available.

Existing RSD dial/bootstrap/response/outer timers and fresh-connection fallback
order remain. There is no new deadline setter, task, async shutdown or cleanup
ACK. Helper futures are boxed before their await state and remain owned
directly. Separately completed queue/send/read stages are not combined into a
new request budget; owned discovery attempts already drop their transport on
failure/cancellation. The single-write host handshake retains its H2 guard.
Manual-pairing bootstrap uses the guarded initialization, while its later
protocol exchanges still need their own ownership policy. Transport stays
owned until Drop; borrowed raw transports must not bypass invalid state. See
[RSD fixtures and migration](validation/rsd-lifecycle-2026-10-04.md).

Remaining L01 work includes other service/protocol deadlines and cancellation
state, cleanup-primary retention and bounded stream/task ownership for other clients,
and late/fragmented/blocked-send cases for each owner. L02–L04/Q04 and platform/
device evidence remain separate tasks. A local category or is_usable value is
not evidence of device recovery or remote cleanup.

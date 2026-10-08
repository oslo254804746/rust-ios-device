# Deadlines, cancellation and connection state

L01/class 4 is in progress. The DTX profile's accepted implementation is
`940c4c673e7fe023503b97197777354c7f44e253`; the FPS owner profile is
`dcec5a2032f2994ed45678cb838447607abee65b`; AFC request/file ownership is
`c4b471d33b1260736a295e8718d3c47ee6562165`; the internal H2 framer profile is
`ddd3620f058c6c23649a083d209dceb41f0eb472`; the XPC client profile is
`09ed113e2d19489259a43c3a35de9e00caacb1a6`; borrowed RSD bootstrap/read ownership is
`85b8621f069b164d7da251a9531faf43b3456691`; Lockdown client ownership is
`79f750e78b4550909c440fc4697b592101878935`; DeviceLink message/handshake ownership is
`3db5c1835ad73bf881c1c6a334674bc566cfe641`; MobileBackup2 operation ownership is
`d2828b802e697ff4d1069744e89d7a1eb31214c7`; NotificationProxy framed-event ownership is
`db732437a945c75096d05f67b9e2063838756c97`; notification CLI cleanup ownership is
`9b2c488ec9052d2899de91a68622b00be207446c`; notification CLI discovery/setup,
send/total deadlines and operator signal ownership is
`e3ab42b8e1d47155edd9e0ffd08c668b5e3a41ec`; Companion shared request/listener/forwarding
ownership is `7a0ed63d99f04d99971004b177dd68e1cdf3b3bb`; Companion CLI ownership is
`c2fa6fb60c9bdd2f56514b7b4844fe39f596bc3d`; WebInspector client frame/start/event
ownership is `30fb63fe5d6a6530bb96270adbb59c27c96d1e27`.
This contract records implemented profiles and
their limits; it does not certify every service or remote cleanup.

The active roadmap scope excludes Q04 and all its subtasks as of2026-10-06.
Older profiles below retain their historical scope and unresolved findings.

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

Caller-bounded AFC reads retain that same request/file ownership policy. A fully
consumed byte-limit refusal plus successful close can retain reuse; an undrained
declared-length refusal or Drop after open requires a fresh transport. The new
InstallationProxy bounded Browse owns one total from first poll through final
parsing, preserving acquired native errors while suppressing late success.
Committed failure/cancellation invalidates all subsequent legacy client entries;
explicit Complete business errors can preserve reuse. Neither method truncates
a successful result. See
[bounded service acceptance](validation/bounded-service-read-2026-10-08.md).

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

LockdownClient guards complete query/value/service/activation send-reply
transactions. Optional set_request_timeout uses an absolute per-request budget
through encoding/send/flush/decoding; default None keeps unbounded requests.
Zero/unrepresentable request budgets reject before I/O, actual expiry retains
Elapsed. Synchronous work cannot be preempted. Constructor/setup timing remains
caller-owned. This is not a total budget across several separately completed
requests or post-reply convenience parsing.

Accepted read/write bytes include buffered TLS halves. Cancellation/failure
after progress, transport errors/native EOF and frame decoding failures reject
later requests before reading a cached/late reply. No pending queue is added;
opaque I/O halves and their buffers stay owned until client Drop. Empty write
wait/unpolled futures and local serialization failure before I/O can preserve
reuse. Complete business replies keep the existing response parser and reuse
policy, independently of their returned error category.

StopSession uses an independent five-second write/flush budget, clamped by a
shorter configured request budget. It retains the session identifier until a
successful write, then marks closed; repeated stop is a local no-op and other
requests reject. Failed/cancelled close after bytes requires a fresh transport;
an empty write expiry can preserve state for another attempt. There is no ACK,
automatic stop, async Drop or primary/cleanup aggregate; manual callers own
both results. Generic borrowed raw/session helpers are unchanged. Device-side
fresh sessions remain owned by their surrounding operation, without new setup/
service totals or remote release guarantees. See
[Lockdown fixtures and migration](validation/lockdown-lifecycle-2026-10-04.md).

DeviceLinkClient guards single framed sends/reads and the entire version
exchange. Optional set_operation_timeout shares an absolute budget through
encoding/send/flush/read/decoding; call_process_message owns its send and reply
together. Default None keeps unbounded timing, including idle reads. Separately
completed public sends/reads do not become one operation. Zero/unrepresentable
ordinary budgets reject before I/O and actual expiry retains Elapsed; synchronous
serialization/decoding remains cooperative.

Accepted bytes include caller-buffered reads/writes. Cancellation/failure after
progress, native transport failure/EOF and frame decoding errors invalidate local
reuse. Empty idle/write waits, unpolled futures and local pre-I/O errors can
preserve state. Complete process envelopes retain their existing validation and
reuse policy; failed version negotiation after progress invalidates. No pending
queue is added and the opaque stream/buffers remain owned until Drop.

Disconnect has an independent five-second write/flush cap, shortened by a smaller
configured budget. Success marks locally closed; all later tracked operations,
including another disconnect, reject. Interrupted close after bytes is uncertain
and requires a fresh transport. No ACK, asynchronous shutdown/Drop, automatic
disconnect or primary/cleanup aggregate is added. MobileBackup2 native EOF now
keeps the original error directly and attempts no cleanup write on invalid
DeviceLink; its valid-connection timeout-retention fixtures remain.

Public stream_mut and into_inner retain their signatures and transfer raw
framing/cancellation responsibility to the caller. They bypass tracking and do
not restore an unusable transport. MobileBackup2 now supplies the outer operation
owner described below. Screenshot and other raw plist helpers remain separate
profiles. See
[DeviceLink fixtures and migration](validation/device-link-lifecycle-2026-10-04.md).

Mobilebackup2Client guards the complete public operation across version/Hello,
command, host work, raw upload/download and cleanup. Optional operation timeout
is one absolute budget; default None retains ordinary unbounded timing, while
DeviceUnback/Extract keep their five-minute ceiling. Zero/overflow rejects before
I/O or host work. Actual expiry retains Elapsed, with typed operation evidence
for Unback/Extract. Boxing the directly owned future preserves CLI stack bounds.

Cancellation/failure after accepted bytes before terminal completion, native
transport failures and EOF invalidate. All nine operation paths then reject
before I/O; private filter, diagnostic and discarded-path state is released.
Empty idle cancellation, unpolled futures and local pre-I/O failures can preserve
reuse. A complete DLMessageProcessMessage marks terminal completion before its
business dictionary validation, retaining the existing disconnect attempt.
Cleanup shares the total deadline and DeviceLink's independent five-second cap;
expiry does not start another disconnect or interleave it with a raw transfer.
Expected terminal peer closure retains prior completion policy, with unusable
local transport. Other cleanup failure is returned, including after success.

SessionCleanupFailure retains both original errors under existing Io; its main
source/category follows the primary and cleanup_error borrows the separate
branch. The primary remains owned while cleanup awaits, including outer expiry
that drops that body. Lookup is bounded to 32 typed sources. No background task,
async Drop, remote cleanup ACK, retry or rollback guarantee is added. Host
spawn_blocking jobs can outlive a dropped awaiter; the controlled released-worker
fixture does not establish stop/join for arbitrary filesystem work. See
[MobileBackup2 fixtures and migration](validation/backup-lifecycle-2026-10-04.md).

NotificationProxyClient owns each complete frame with accepted-byte tracking,
including buffered reads. Optional operation budgets and explicit waits share
one absolute deadline; wait_for includes subscription writes and unrelated
complete events do not reset timing. Default None preserves unbounded sends/
idle receives. Zero/overflow rejects before I/O. Actual timer expiry, including
an already due timer between complete events, retains Elapsed.

Empty idle cancellation/expiry keeps completed subscriptions and valid late
notifications. One-way subscription completion is not an abandoned request/
reply. Partial frame cancellation, native IO/EOF and typed plist decoding failure
invalidate, clear subscription names and reject all seven public paths before
I/O. Complete business-shape errors remain outside the frame guard. ProxyDeath
returns its original event/error and closes local state. Shutdown has an
independent five-second write/flush cap with shorter configured clamp and safe
MAX handling; success closes reuse, including repeat shutdown. Empty blocked
shutdown may remain usable; partial aborted shutdown requires a new transport.
No queue/task/automatic cleanup/ACK/async Drop is added; opaque buffers stay until
Drop. CLI overall budgets and signal ownership are supplied by the owner below. See
[notification fixtures](validation/notification-lifecycle-2026-10-04.md).

Notification CLI now awaits the entire command body before deciding cleanup,
including returned output/operation errors. A still usable stream attempts the
core five-second Shutdown; an invalid or ProxyDeath stream is dropped without
unsafe control writes. Cleanup failure after success is returned. Dual failure
keeps both originals under a private CLI aggregate, with primary source/category
and static Display/category-only Debug. Boxing the command body preserves stack
bounds without a task/queue. Whole-future cancellation still drops transport
without asynchronous cleanup. That cleanup profile leaves setup, command timers
and signal handling unchanged. See [CLI cleanup fixtures](validation/notification-cli-cleanup-2026-10-04.md).

The subsequent notification CLI control owner validates finite zero/overflow
timeouts before any device selection or I/O. It shares a 30-second setup budget
across default usbmux discovery, device connect and service connect. Explicit
UDID and the original first-device selection use the same setup owner. One
pinned Ctrl+C future survives setup into capture; setup stop returns the existing
typed OperationCancelled. The device owner stays alive through cleanup.

Post sends and each command's complete subscription stage have a 30-second send
cap. Roundtrip/Wait/Springboard and Stream with --timeout share one absolute
user budget from before their first subscription write through reception/output;
setup precedes this budget and cleanup remains independently capped at five
seconds. Roundtrip includes observe and post; Stream includes all subscriptions.
Observe retains its per-complete-event idle timeout, including unrelated events;
it has no new total budget. Stream without --timeout waits indefinitely until
count, operator stop, failure or caller cancellation.

Ready operator stop and an already due total take priority over operation polls.
Stream/Observe operator stop and Stream total expiry are ordinary completion,
subject to cleanup failure. Finite command stop retains typed cancellation;
actual setup/send/finite-total expiry retains real Tokio Elapsed. Complete event
boundaries yield before output and the next read, so continuously ready stream
traffic cannot starve control. Interrupted partial frames remain invalid and
skip unsafe late Shutdown. No timer tick interrupts an arbitrary partial read.

These are cooperative budgets: synchronous serialization, decoding, output and
already running blocking workers cannot be preempted. Directly owned boxed
futures add no reader task/queue or asynchronous Drop. Host fixtures include
actual Linux SIGINT during an owned fake ListDevices wait; actual device signal,
service cleanup and other platforms remain pending. See
[CLI control fixtures and migration](validation/notification-cli-control-2026-10-04.md).

CompanionProxyClient's additive is_usable reports shared local admission across
the client, listener and forwarding handles. Requests keep the ten-second default
and share one checked absolute budget across mutex wait, send/flush and reply.
Zero rejects before polling with legacy Timeout(duration); overflow is existing
Io InvalidInput. Actual expiry retains the native Elapsed/duration wrapper.
Admission is checked again after acquiring the mutex, so an owner queued before
another failed operation cannot read its late reply.

Cancellation after accepted send/read bytes, a committed unanswered request,
native transport error/EOF or frame decoding failure invalidates all shared
owners. Empty blocked sends/lock waits, unpolled futures and pre-I/O local errors
can preserve state. Complete business replies are parsed outside the guard and
retain the original local reuse policy. Opaque transport/buffers stay until all
shared owners drop. A local query does not prove peer liveness or safe retry.

The listener keeps one directly owned pending read across cancelled next polls,
including a partial header. Actual idle timeout preserves the existing terminal
policy even for empty reads, now drops pending immediately and invalidates the
shared connection. Whole-listener Drop also invalidates, preventing another
shared owner from consuming a subscription event as a reply. Finished reads
return None; active zero/overflow admission does not finish or start a read.

All forwarding stop paths have an independent five-second cap, shortened by
their configured/explicit budget; MAX is safe after this clamp. Successful
explicit stop remains cached/idempotent even with later zero/MAX input. Complete
business failure may retain reuse; abandoned/native/codec failure requires a
new connection. Existing ForwardingHandle Drop still schedules a bounded
best-effort task when runtime/state permit; stopped/invalid/zero-configured state
skips it, and the task checks shared state after locking. It is not an awaited
join or remote stop guarantee. A business-failed borrowed stop can still trigger
the pre-existing Drop attempt. That core profile adds no reader task/queue,
protocol acknowledgement or cleanup aggregate. See
[Companion fixtures and migration](validation/companion-lifecycle-2026-10-04.md).

The subsequent Companion CLI owner checks finite zero/overflow before explicit
or default discovery. Setup caps at30s; a finite total starts before discovery
and spans connection/start/wait/output without restart. One pinned Ctrl+C future
serves every stage. Setup and finite request stages return typed cancellation;
started Listen/Forward keep graceful operator stop. List/Get retain core10s
requests, while unbounded listener next_event does not inherit a request idle
timer. Ready stop/due total precedes work, including the nominal-deadline timer
registration gap, and actual expiry retains Elapsed. Event delivery yields only
after complete reads/output. Synchronous work remains cooperative.

Additive consuming ForwardingHandle::close/close_with_timeout disarm Drop when
the future is created, even if unpolled. CLI creates that future immediately
after start, before output/wait, then awaits one independent5s Stop after a
returned primary or graceful stop, even past the total deadline. Invalid state
skips unsafe cleanup. Cleanup failure after success is returned; dual failure
retains both originals with primary source/category and safe static diagnostics.
Whole-command or close cancellation leaves no detached fallback. Legacy borrowed
stop/Drop policies remain; shared owners retain the stream. No remote release,
rollback, retry or guessed stop-listen command is claimed. See
[Companion CLI fixtures and migration](validation/companion-cli-control-2026-10-05.md).

WebInspectorClient adds is_usable and an optional operation budget, default None.
Startup/timed events use the shorter explicit/configured limit. Sends include
encoding/send/flush and event totals include decoding/routing/automatic listings.
Checked zero rejects before cache/IO as legacy Timeout; unrepresentable effective
budgets become Io InvalidInput. A shorter explicit limit can safely clamp MAX
configuration. Due nominal deadlines suppress ready work while the native timer
supplies actual Elapsed; synchronous codecs remain cooperative.

Frame owners track accepted bytes, including buffered input, native failure/EOF
and flush. Partial cancellation/expiry or codec failure invalidates and drops
owned metadata/pending collections; all later wire/cache-read operations reject
and restore attempts discard supplied events. Empty idle/send cancellation and
pre-IO local XML failure can preserve reuse. Opaque transport/buffers remain
until Drop. A decoded event stays owned through automatic listing writes, even
when cancellation occurs before those writes accept a byte.

Startup owns a committed report until CurrentState, preventing late state after
cancel/expiry from being reused. Legacy start still keeps blocked report outer
expiry as native Timeout and maps its former report/response failures to Disabled;
the preserving API keeps originals. Effective operation budget replaces varying
startup response remainder in the native getter. Page discovery quiescence now
requires empty usable idle; partial/routing and configured total expiry fail.
Raw helpers, protocol strings/models/16MiB framing and six public variants remain.
That client-only profile deferred Inspector/Automation correlated ownership,
retained cache limits and frontend/server setup/total/signal/join/output cleanup.
No task/queue, remote cleanup/ACK/retry or measured memory claim was added.
See [WebInspector client fixtures](validation/webinspector-client-lifecycle-2026-10-05.md).

Inspector/Automation correlated commands and attach now own one checked total
across send/flush and the expected response/target/page. A completed outgoing
request commits the owner: abandoned or failed waits invalidate the client even
after unrelated complete events followed by empty IO. Other sessions/clones on
that client reject a late complete reply. Frame failures still preserve original
sources. Matched business failures complete the wire obligation and retain their
original reuse/error policy. Automation page_id is assigned after full attach.
Send-only attach(false)/command/bridge APIs remain local write-completion owners;
session metadata is not transferable recovery when a new client is needed.

Pending/skipped events share1024/16MiB compact-JSON caps, counting existing pending
state before append/restoration. Complete owners restore original ordering;
committed cancellation discards skipped state and drops pending capacity. Cached
pop releases its byte accounting. Typed private local limits remain ResourceLimit
under Io; checked u64 counters admit MAX once and never wrap/reuse. Logical bytes
are not RSS/peak-memory evidence. That session profile deferred application/page
metadata to the subsequent cache profile below.

Finite scopes inherit their original absolute deadline through nested sends,
startup, timed socket/event reads and automatic listing writes. Scope Drop restores
the prior policy without an async task. Zero rejects before ready IO; effective
overflow is Io InvalidInput, native expiry keeps original effective total/Elapsed.
Due priority and cooperative codecs stay. No remote ACK/cleanup/retry or reader
task is added; frontend/server setup/total/signal/join/output/primary-cleanup and
multi-command workflow owners remain pending. See
[WebInspector session fixtures](validation/webinspector-session-lifecycle-2026-10-05.md).

Received metadata now has LOCAL caps:1024 applications,1024 page application
sets (including empty/orphan sets),4096 aggregate cached pages,4096 raw entries
per listing and1024 raw application-list entries. Combined compact JSON for
availability/applications/page sets includes keys, duplicate model IDs,
containers, escaping and UTF-8, capped at16MiB. A separate16MiB flattened
ApplicationPage projection prevents copying a large application for every page.
Staged maps and the duplicate-preserving listing Vec each have16MiB admission
before insert/clone/append; raw count admission precedes collection reservation.

Borrowed projections replace old values for accounting without cloning the full
cache/snapshot or allocating encoded JSON. Rejected updates do not partially
replace a snapshot. Successful connected snapshots remove absent page owners;
page replacement drops the old map; disconnect removes both owners and empty
tables drop capacity. Original listing-event duplicates/order and cached last
value/first ID order remain. Complete routing rejection is typed ResourceLimit
under existing Io and invalidates/drops owned client metadata/pending state.

These are local admission policies, not device limits, RSS, allocator/decoder
peak, recursion/object-reference expansion or performance evidence. Actual
binary plist shared-string fixtures fit the unchanged16MiB wire ceiling but
expand before model/cache admission; decoded allocation/CPU bounds remain
L01/Q04 work. Codecs/counting remain cooperative. No public API/error variant,
model/selector/dependency/ACK/retry/task is added. Frontend/server whole-flow
owners still remain. See
[WebInspector cache fixtures](validation/webinspector-cache-lifecycle-2026-10-05.md).

CLI application/page discovery now treats an issued timeout as ordinary idle
only while the supplied client remains usable. Partial header/body or blocked
automatic listing expiry returns its original native Elapsed/Timeout instead
of retrying on the invalid client and replacing the primary with Protocol.
No retry consumes a late reply. Empty usable idle still retries; successful
selection and cached matches preserve their previous behavior. Seven actual
supplied-stream fixtures cover both workflows, including EOF/plist/resource
sources and explicit page-key precedence. Existing helpers/deadlines/outputs
stay; this does not certify frontend setup/whole-flow/signal/join/cleanup or
decoder bounds. See
[WebInspector discovery fixtures](validation/webinspector-discovery-lifecycle-2026-10-05.md).

Finite WebInspector opened-tabs/eval/launch now own default-device selection
through final output under one checked absolute total and one pinned signal.
Every session stage caps the owned client's existing operation policy to the
remaining total. A500ms discovery idle period cannot reset the total. A due
timer suppresses ready work; a late successful cooperative poll is also rejected,
while an already returned native failure stays primary. Serialization and
synchronous output remain cooperative rather than forcibly preemptible.

The stream and real device guard remain owned until cleanup finishes. Only fully
attached usable sessions issue teardown: Inspector uses the existing forwarding
socket close; Automation first closes its owned browsing context, then its
forwarding socket. Both cleanup steps share an independent5s total and do not
poll the already completed Ctrl+C future. Invalid clients and incomplete attach
owners drop without issuing guessed recovery requests. Primary and cleanup
failures remain available, including two failures within automation cleanup.
Local write/flush completion is not a remote ACK. Normal formats stay; a cleanup
failure after output is now a returned failure. Twenty-one host tests and actual
owned CLI probes cover this profile. See
[WebInspector command fixtures](validation/webinspector-command-lifecycle-2026-10-05.md).

Remaining L01 work includes other service/protocol deadlines and cancellation
state, cleanup-primary retention and bounded stream/task ownership for other clients,
and late/fragmented/blocked-send cases for each owner. L02–L04/Q04 and platform/
device evidence remain separate tasks. A local category or is_usable value is
not evidence of device recovery or remote cleanup.

Interactive WebInspector js-shell now owns selection through metadata under one
setup total, then uses cancellable idle input and fresh per-expression totals
covering command, interpretation and output. One pinned signal spans all phases.
Ready blank input yields to cancellation. The client policy is refreshed before
each operation, so expired setup/request caps do not govern normal idle input
or the next expression. Continue only complete business responses; a native
wire/codec/deadline failure terminates and cannot consume a late reply.

Setup Ctrl+C is typed cancellation; active Ctrl+C is normal operator stop. Only
completed attachment and a usable client authorize the existing forwarding
socket close under separate5s cleanup. Device/stream ownership lasts through
cleanup, which never polls a completed signal. Primary, session cleanup and
explicit stdin-restoration failures retain their sources; successful stop can
still fail cleanup. No remote ACK, rollback, automatic retry or recovery claim.

Linux stdin pipe/terminal readiness is directly owned without a detached blocking
read. Modified shared file-description flags restore on explicit finish and
best-effort Drop, including failed readiness admission. Regular-file reads,
serialization and output remain cooperative. Other platforms retain Tokio
stdin and need platform-specific cancellation/shutdown work. The actual owned
Linux child sends its ready marker from the first input poll, then receives
SIGINT and exits after cleanup while the parent still holds the writer open.
This is host evidence, not Apple teardown or a resource/performance baseline.
See [WebInspector shell fixtures](validation/webinspector-shell-lifecycle-2026-10-05.md).

CDP/WebDriver/server totals, revocation and joined child ownership remain, along
with other L01 and L02–L04/Q04/classes5–9. Host shell acceptance does not complete
L01, class4 or the entire engineering goal.

The CDP page bridge continues an issued timeout only while its WebInspector
client remains usable. A partial header/body or blocked automatic listing write
can invalidate that client; the loop now returns the original native
Timeout/Elapsed and drops its stream without another request or teardown write.
Healthy empty idle still waits, forwards the next target message and uses the
existing close selector on normal WebSocket shutdown. Native EOF, plist and
legacy size errors retain their original families. Five actual WebSocket/owned
plist fixtures verify this narrow profile; see
[CDP page lifecycle](validation/cdp-page-idle-lifecycle-2026-10-05.md).

Page/browser setup totals, finite sends/output, primary-preserving independent
cleanup, server cancellation, stored-session revocation and joined child
ownership remain. The stored-session100ms and browser-page2s cleanup policies
were not changed. Host page-loop acceptance does not complete any of those
owners, other protocols, L02–L04/Q04 or classes5–9.

CDP full-page discovery, page/browser-page constructors and WebDriver runtime
construction now share one checked absolute setup total from before lock/source
contact through final attach/context creation. Every stage caps the client to
the remaining total. Exact CDP targets attach on an admitted match; only usable
idle can retry. Full-page and WebDriver discovery keep500ms quiet collection
inside the total. Successful constructors clear their cap for existing active
policies. Late success/caller cancellation follow the already accepted private
controller; no new async Drop or detached source worker is added.

Fifteen owned fixtures verify source, lock and writer expiry, stage remainder,
zero/overflow, actual Automation context creation/use and caller cancellation
after real attach/create frames. Stream Drop, peer EOF and page-lock reacquisition
are observed. Incomplete attach drops without guessing teardown. Constructor
primary cleanup after a fully attached/business-failed step, server signals,
HTTP publication/output, active request totals, revocation and joined child/
stored task ownership still need work. The existing100ms/2s cleanup policies
stay; the successful WebDriver fixture observes context stop and zero forwarding
teardowns. See [server setup evidence](validation/webinspector-server-setup-2026-10-05.md).

After a direct CDP page loop returns, its device guard, stream and page lock stay
owned through independent 2s cleanup. The existing session owner sends the
forwarding close only for an attached usable client; invalid clients drop without
recovery writes. A normal close retains cleanup failure, and a failed page retains
both primary and cleanup causes through the existing private wrapper. Actual
native flush faults, default expiry and an 80ms successful flush under a 50ms
active idle policy are covered. Write/flush completion is not an Apple ACK.
See [CDP page cleanup evidence](validation/cdp-page-cleanup-2026-10-05.md).

This returned-error path does not execute if outer principal/server cancellation
drops the handler. Constructor primary cleanup, finite sends/output, joined
browser children and stored-session revocation still need owner work. Existing
stored100ms and browser-page2s policies remain. Other L01, L02–L04/Q04 and
classes5–9 stay in the engineering scope.

WebDriver runtime stop now borrows its original AutomationSession and application
identity to close context then forwarding socket under one100ms stored cleanup
total. The existing Automation cleanup body is shared with finite/shell owners;
their5s policy and the direct page2s policy stay. Once stop is first polled, its
private flag disarms further cleanup after success, business failure or caller
cancellation. Both cleanup errors retain their native sources and original
primary classification. Invalid committed context close skips socket recovery.

Eight actual constructor/owned-stream fixtures cover normal close, native flush
failure, dual business/native errors, total remainder, context/flush expiry,
repeat stop and caller cancellation after the real close frame. Command IDs1/2
remain; a valid owner sends one forwarding close. Caller cancellation invalidates
the client; explicit runtime Drop releases the stream. This is not async Drop,
remote ACK or registry-task join proof. HTTP DELETE result swallowing, outer
timeouts, lock admission, active Arc races, constructor primary cleanup and
joined revocation/server ownership remain. See
[WebDriver cleanup evidence](validation/webdriver-cleanup-2026-10-05.md).

Direct CDP page ownership now includes one pinned principal.revoked future across
setup and active work. Checked setup drives source/start/discovery/attach beside
that signal; active work runs behind biased lease control. Each message turn
yields while the existing inner traffic selection stays unbiased. Returned
revocation has typed cancellation and awaits the existing independent2s session
cleanup without polling the completed signal again. Device/client/page lock
remain owned through cleanup. Invalid partial-read clients skip recovery writes.

Six actual admitted page fixtures use synthetic policy expiry/reload and owned
streams; a private observer verifies Cancelled/native cleanup dual causes.
Source cancellation, observed read bytes, close identity, delayed/default-expiring
flush, stream Drop/EOF and actual lock-key reacquisition are covered. Existing
safe diagnostic output now runs on revocation. Local flush is not a remote ACK;
cooperative timers cannot preempt synchronous work. See
[CDP page lease evidence](validation/cdp-page-lease-2026-10-05.md).

This supersedes the earlier direct-page principal-cancellation gap. Generic
caller/server Drop still has no async cleanup guarantee. Constructor primary
cleanup, active request/output totals, joined browser children, stored-session
revocation, other L01, L02–L04/Q04 and classes5–9 remain.

CDP browser children now pin their local stop signal outside active traffic and
yield each turn. Usable empty idle resumes; a timeout that invalidates the client
returns its original error. Each child records its primary before independent
2s forwarding cleanup, then records cleanup before the existing Closed event.
An explicit supervisor stop retains that primary even if it must abort cleanup.
Detach, Closed and a failed input sender explicitly stop and join the removed
child; browser loop exit cancels all remaining children under one2s total,
aborts all unfinished tasks, then joins every handle before releasing owners.
Unexpected native JoinError and a completed child missing its cleanup outcome
are errors. One child failure cannot skip the other joins.

Eleven fixtures cover actual child constructors and owned plist streams, native
errors, partial-read cancellation, delayed/default-expiring flush, full output,
idle resumption, shared group expiry and retained primary after abort. Two
standalone actors within one fixture test supervisor panic/missing-result
admission only. The existing terminal try_send can miss a full queue; release
and join are proved independently, without claiming terminal delivery. See
[browser task evidence](validation/cdp-browser-tasks-2026-10-05.md).

Browser outer Principal.run still drops the whole handler on revocation; Drop
only aborts. Revocation during a removed child's cleanup also needs an owner
outside the raced future. This profile proves explicit returned stops, not
async Drop, browser lease or server joins. Constructor and active totals,
registry ownership, other L01, L02–L04/Q04 and classes5–9 continue.

Browser ownership now includes one pinned principal.revoked future through
initial discovery and active work. Biased outer control and a yield per turn
preserve the inner unbiased traffic choice. Returned revocation has typed
cancellation, then performs common independent2s child cleanup without polling
the completed lease again. Removed children enter a retiring owner outside the
raced active future before any cleanup await. Dropping its borrowed stop future
retains owners, original deadline, completed join results and an already-issued
wait error. Resuming never polls a completed JoinHandle again. Outcomes are
taken together after all joins, with no further await during their transfer.

On parent exit, retiring and remaining children join one common2s cleanup group;
any previously issued retiring error is preserved alongside the final result.
Eight actual admitted browser fixtures cover expiry/reload, delayed/default
cleanup, observed partial frame, detach in progress and source cancellation
before return. A private observer checks Cancelled plus original cleanup IO.
Three additional child fixtures verify interrupted cleanup progress/errors.
See [browser lease evidence](validation/cdp-browser-lease-2026-10-05.md).

This covers the earlier browser principal/removed-owner gap. Generic caller or
server Drop still only aborts; no product server supervisor join is certified.
Constructor primary cleanup, active request/output totals, stored registry,
terminal event retention, other L01, L02–L04/Q04 and classes5–9 continue.

### WebDriver DELETE handler total (2026-10-05)

DELETE creates one checked100ms Budget at handler entry. Map removal, runtime
mutex admission, context close and forwarding close consume that same absolute
deadline. Removal precedes protocol I/O. Cleanup disarms its once-only flag
when it starts and skips an unusable original client; it borrows the original
Automation session and command IDs. Existing native context/socket cleanup
body is shared unchanged, with private visibility only. Its combined result
has no competing outer timer that could discard an already observed primary.

Six actual protected POST/DELETE host fixtures verify success/repeat, native
flush IO, real default100ms expiry, business primary after expiry, mutex wait
with zero cleanup requests and85ms mutex time leaving insufficient forwarding
flush time. Three private owned-result fixtures retain original business+IO,
business+Elapsed and mutex Elapsed. These observers make no HTTP/admission
claim. The routed fixtures use synthetic authorization and actual owned plist
streams; listener/peer joins are fixture supervision, not product registry join.

Accepted runtime.stop, unit revocation callback, constructor/active methods and
eight prior cleanup assertion functions remain. Explicitly held fixture Arcs
release after removal; DELETE does not thereby cancel all in-flight request
Arcs. Stored revocation tasks still lack explicit joins. Generic caller/server
Drop, constructor primary cleanup, active request/output totals, terminal event
retention and remaining L01/L02–L04/Q04/classes5–9 continue. No async Drop,
Apple ACK/rollback/retry, platform or performance acceptance. See
[DELETE lifecycle evidence](validation/webdriver-delete-2026-10-05.md).

### Stored session retirement and joins (2026-10-05)

The store owns active entries and retired watcher owners. Explicit removal or
replacement publishes the retired owner and issues abort before awaiting its
join mutex. Cancelling that wait cannot lose the owner or abort request. Join
completion/native errors persist; repeat calls skip a completed handle.
Insert/get/principal/remove reap only finished retired actors and try_lock
their join state, avoiding waits on active actors. No global table lock spans
protocol cleanup or task join. No extra actor, supervisor or background reaper
is introduced; idle completed handles remain until the next registry API/Drop.

On lease revocation the actor checks original weak resource identity before
removing the entry, transfers its own handle into retirement and releases the
strong store reference/global lock before the existing100ms unit callback.
It never aborts or joins itself. Replacement keeps the new resource protected
from a stale actor. Store Drop remains fallback abort without async join.

Seven owned Tokio actor tests cover normal/repeated/abort/native panic/external
cancel/resumable join/nonblocking finished reap/fallback release. Six actual
protected loopback registry fixtures use generic owned host resources: explicit
remove join, cancelled wait plus another actual HTTP insertion,80ms reload
cleanup, real100ms callback expiry, replacement and an actually pending actor
at store Drop with weak-store release. They are not Automation protocol/device
fixtures. Nine accepted actual WebDriver DELETE cases and both original scope/
Drop tests pass. Original middleware/assertions/other owners remain.
See [session ownership evidence](validation/session-store-2026-10-05.md).

Legacy callback native cleanup errors/primary retention, active request Arcs,
idle handle reclamation without API calls, caller/server shutdown, constructor
primary cleanup, active totals, terminal retention and resource caps remain.
Other L01/L02–L04/Q04/classes5–9 continue; no async Drop/Apple ACK/retry/platform/
performance/RSS/decoder peak acceptance.

### WebDriver typed revocation callback (2026-10-05)

The production registration now supplies stop_selenium_runtime_until directly
to insert_with_budget. After original entry retirement, the actor creates one
checked100ms cleanup Budget and invokes that Result callback without a competing
outer timer. Existing mutex admission, original Automation context and forwarding
cleanup use the same deadline. Native JoinHandle<Result<()>> keeps callback
failure through normal task exit, and its existing owner retains/transfers that
result on join/reap. Native task normal/abort/unexpected classifications remain;
normal exit does not imply cleanup success. No extra task/source/protocol action.

The old unit log adapter is removed; legacy unit insert is now a cfg(test)
Budget.drive adapter for existing host fixtures. Runtime.stop and the private
duration cleanup wrapper become cfg(test) with bodies intact. Actual DELETE,
stop_until, shared protocol cleanup, constructors/active work, retirement and
all earlier assertions stay. Eight new fixtures create actual protected product
Selenium sessions, seven reload through the actual HTTP endpoint and one awaits
actual grant expiry. Private native-result observation checks original errors,
default expiry, unchanged IDs and owned release. Fixture Arc release and listener
joins do not certify cancellation of all requests or product server supervision.
See [typed revocation evidence](validation/webdriver-revocation-2026-10-05.md).

This covers the earlier unit callback/outer-timer error gap. Map wait before the
callback total, known primary before callback return if store aborts, active
request Arcs, idle completed handles, caller/server Drop, constructor primary
cleanup, active totals, terminal retention and resource caps remain. Other
L01/L02–L04/Q04/classes5–9 continue; no device/Apple ACK/async Drop/server join/
retry/platform/performance/RSS/decoder peak acceptance.

## CLI tunnel manager device coordination

[L02 device isolation](validation/tunnel-isolation-2026-10-06.md) accepts
`e0f52671396e6408fc38921f78f73a256a45ce65`: actual CLI local query/stop avoids
network lifecycle locking, same-device requests serialize, four permits bound
simultaneous source connections. One15s manual total after ticket acquisition
includes queue/permit/native connection/existing-trust fallback and publication.
A stopped generation cannot publish after removal; active and queued requests
retain OperationCancelled. Clear invalidates old tickets. Native source options
and wire paths stay. The weak gate table's4096-entry cap is not a measured RSS
bound. Private boxed native devices add allocation.

Scans use a separate serial scan lock, capture candidates before waiting and
fence stopped discovery/old publication snapshots. Backoff remains30/60/120/
240/300s. Automatic candidate connects are still sequential. In-flight
detach/replug identity, scan discovery/whole total, /shutdown supervision,
native worker joins and long-run resource evidence remain. A later scan can
recreate a stopped present device. Actual A-stalled/B combined HTTP query/stop
is below1s in the owned host fixture; this is not L04 latency/throughput/RSS
measurement or device acceptance.15/min15,tunnel56,full2524Rust/13Python PASS;
22device NOT_RUN. Parent9635 handle missing/exit unknown; all14 subprocess
exits0 and frozen source/binary audit prove the scoped gate. L01/L02/class4/
all31 tasks and later classes remain active.

## Bounded automatic scan futures

The [parallel scanner profile](validation/tunnel-parallel-2026-10-06.md),
`66bf7311020763b8a29b175a117cdf1b70d4e22f`, supersedes the preceding
sequential candidate connection condition. USB and known paired mobdev2
candidates enter one deduplicated window, directly owned by the parent scanner,
with at most four futures and the existing shared permits/serial/generation
guards. Parent cancellation drops all active candidates; stopping A leaves
B's generation running. Original per-candidate15s/backoff/source options stay.

Paired discovery now precedes connections, so that setup can delay USB startup.
No discovery/whole-scan total or task/RSS bound is certified. Native discovery
errors return after USB candidate outcomes; known error retention under generic
parent Drop remains. Eight new actual manager fixtures and23/min23,tunnel64,
full2532Rust/13Python pass;22device NOT_RUN. Attachment/replug identity, stale
snapshots, native worker/server joins, metadata/resources/L04, other L01/
L03–L04/Q04/classes5–9 and the full31-task goal remain.

## Observed native attachment IDs

The [L02 identity gate](validation/tunnel-identity-2026-10-06.md),
`1eb45c25145f5306a5831df6a0c29d494a94ca96`, retains returned usbmux IDs/USB
flags and prunes only original usbmux-route publications absent from the
observed(UDID,ID) set. Another attached transport and concurrent new publication
survive. Candidate first-USB/last-other selection matches Core; returned ID
mismatch releases its owned result with local cancellation and no backoff.
Private route family replaces ID0 inference, without wire-validity evidence.
Existing device gates/watch/permits/budgets/window stay.

Seven new actual manager fixtures,30/min30/tunnel71,full2539Rust/13Python pass;
22device NOT_RUN. A matching old ID after unobserved detach/reuse/restart or
snapshot change can still be admitted; manual expected ID is None and no
post-connect query/listener exists. Physical freshness, native worker/server
joins, discovery/wholeScan deadlines,metadata/resources/L04 and otherL01/
L03–L04/Q04/classes5–9 remain. This profile does not complete L02 or the goal.

## Completed device ownership during publication query

[L02 publication query](validation/tunnel-publication-2026-10-06.md),
`86fecfe3331e9efa74dc76fbc48b47ad5c3df8f1`, owns a completed USB device
through an additional current(UDID,ID) query before the global table lock.
Original15s/serial/permit/watch cover it; failure/cancellation releases that
owned result and preserves native IO/Elapsed/local cancellation. No global
lock covers network wait. Initial expected-ID rejection and Wi-Fi/mobdev2
skip the extra query. Six actual manager fixtures,36/min36,tunnel77 and
full2545Rust/13Python pass;22device NOT_RUN.

Observation at query completion leaves later changes/reused IDs/daemon restart/
manual epochs/nativealivefalse/worker-server joins unresolved. Failed attempts
still key only UDID, so prior-generation backoff may suppress a new ID. Native
tasks/allocation/retained metadata/discovery-wholeScan budgets/L04 and otherL01/
L03–L04/Q04/classes5–9 remain. The extra query is not a performance or RSS proof.

## Failure waits for one known connection identity

The [L02 backoff profile](validation/tunnel-backoff-2026-10-06.md),
`d0b04bed79ecec5c3f724ff003122f66b1159216`, associates each recorded failure
with its known Route/expected native ID. Only identical attempts inherit a wait
or increment its counter; a known ID/paired host/route change can retry and a
new failure starts at1. Time calculation and same-identity30/60/120/240/300s
stay. Successful publication removes failure as before. Native connections,
trust, guards/queries/serial/watch/permits stay; Debug hides retained hosts.
Five actual/security fixtures,41/min41,tunnel82,full2550Rust/13Python pass;
22device NOT_RUN.

Failure-map entries across UDIDs remain unbounded. Unobserved same-ID/host
reuse/restart, after-query changes/manualepochs/nativeLive/worker-server joins,
metadata bounds/discovery-wholeScan/L03 configurable reconnection/L04/Q04 and
all otherL01/L02/classes5–9 remain. Do not reset every expired same-device
failure to1 or infer resource stability from this identity correction.

## Known local inactive state

The [liveness profile](validation/tunnel-liveness-2026-10-06.md),
`7a77000cdbd0a8e6d2fbaaf6ac5bd630af47d6c5`, checks existing device.is_alive
after record validation before query and again after publication lock/epoch/
policy before insertion. Known false releases result/localcancellation/no
backoff across all routes. Four actual manager fixtures,45/min45,tunnel86,
full2554Rust/13Python pass;22device NOT_RUN.

Kernel true means a cancel receiver remains; userspace true means every task
handle is unfinished. SDK bodies stay and this does not imply physical
transport/ACK/native task joins; receiver clones/after-check exit remain.
Resource/retention/snapshot setup-wholeScan/native joins/L03-L04/Q04/otherL01/
L02/classes5–9 continue. Six added production lines do not close these conditions.

## One scan discovery setup deadline

The [discovery setup profile](validation/tunnel-discovery-2026-10-06.md),
`a47648442d5b7707ad41beeb58a6ab1ad764ea9a`, captures a scan epoch before
queueing and shares checked15s across scan mutex/USB snapshot/paired snapshot.
Seen-version capture avoids spurious fresh-generation cancellation. Real
Elapsed/IO remain; due-zero never polls real work and overflow rejects before
source/mutation. Stop/clear quietly cancels old phase/queued futures without
marking ready. Nine actual manager fixtures,54/min54,tunnel95/full2563Rust/
13Python pass;22device NOT_RUN.

This deadline ends at paired reply; existing candidate15s/window/guards/
liveness/backoff remain. Whole-scan/metadataCPU/handoff budget and native
blocking pair-record worker/server joins are unproven. Fake probe release
certifies source future ownership only. Retained metadata/lastcheck-races/
reuse-restart/manualepochs/L03-L04/Q04/otherL01/classes5–9 continue.

## Failure rows and reserved capacity

The [retention profile](validation/tunnel-retention-2026-10-06.md),
`26b89951b52dd690bcc0e994541007b5426fba93`, reconciles rows against current
authorized candidates. Complete absence releases rows; failed paired discovery
retains unknown mobdev2. Present expired identity keeps30/60/120/240/300s.

4096 owned slots cover records and pending attempts. Existing retries share a
lease; success/cancel releases local reservations. Failure recording checks
generation again under its write lock, preventing clear/queued-write resurrection.
Ten actual fixtures and64/min64/tunnel105/full2573Rust/13Python pass.
Capacity prefill invokes real private reserve/record, not4095 native failures;
fake release is not Core worker cancellation/join. Bytes, candidate metadata,
sockets/RSS, whole-scan/CPU/server joins and physical evidence remain.
L01/L02/L03–L04/Q04/classes5–9 stay active;22device NOT_RUN.

## Owned tunnel scanner and returned shutdown

The [server profile](validation/tunnel-server-2026-10-06.md),
`5120fa90e06a92f4734603382b98779b69f1ca38`, retains a bool shutdown state for late subscribers. The
parent owns its scanner future and every active candidate. HTTP shutdown,
a pinned signal outcome or server completion stops that scanner before cleanup.
One independent2s budget covers existing table clear and graceful HTTP drain;
native primary/cleanup sources remain separately inspectable.

Eight fixtures/72 minimum+full/113 tunnel/full2581Rust/13Python pass, including
a real Linux SIGINT child exiting its Tokio runtime with a fake pending scanner.
Caller Drop signals shutdown/generations and clears unlocked tables only.
Held-table and HTTP-drain30ms cases prove native Elapsed; the fixtures explicitly
release their remaining held resources. Timeout does not force-join Axum
delegated tasks or Core workers. Startup totals, later admission/check races,
generic caller-drop primary reporting, L03–L04/Q04 and classes5–9 remain.
22device NOT_RUN; no physical ACK/native join/RSS/performance certification.

## Terminal tunnel connection admission

The [shutdown admission profile](validation/tunnel-shutdown-admission-2026-10-06.md),
`0d025dd3404082fb76adab20758b2e278b2d9464`, distinguishes reusable clear epochs
from terminal server shutdown. Notification closes admission before async cleanup;
old connection tickets observe cancellation and fresh tickets/scans are rejected.
Clear cannot reopen a closed state. A ticket queued at the coordinator captures
its original epoch before waiting and cannot adopt a later clear epoch.

An actual incomplete HTTP body finishes during graceful drain after table clear:
the production start handler returns502/cancelled without source contact or
publication. Caller Drop also leaves retained state clones closed. Nine new
full/minimum host tests,122 tunnel tests, the original five CLI stack checks,
strict checks and full2590Rust/13Python pass. There is no forced delegate/Core
join: coordinator lock wait totals/cancellation, post-check races, startup and
whole-scan budgets, forwarding owners and other L01/L02 work remain. L03–L04,
classes5–9 and all other active30-task acceptance conditions continue.

## Cancelled tunnel coordinator lock waits

The [coordinator cancellation profile](validation/tunnel-coordinator-cancel-2026-10-06.md),
`a4beec7fa01b605933b1966b79ef3ee738ae41fd`, replaces the uninterruptible
coordinator mutex await with a directly owned biased clear/shutdown wait.
Both reusable clear and terminal close return OperationCancelled while that
mutex remains held; no coordinator row is created. The original post-lock
epoch check stays, ordinary fresh admission after clear works, and no actor or
network request is added. One new test runs both paths; full/minimum,tunnel123,
stack5,strict checks and full2591Rust/13Python pass. Acquisition deadlines,
post-check races and the other pending L01/L02 scope remain.

## Manual tunnel connection absolute deadline

The [manual deadline profile](validation/tunnel-manual-deadline-2026-10-06.md),
`d3ed1c33c14b2dcfdf83ef6c678cc1f451813176`, starts the existing 15-second total
before coordinator admission. One checked absolute deadline drives admission
and the existing serial/cache/permit/connect/paired fallback/attachment query/
publication body. Clear/shutdown cancellation and original source errors stay
typed. Overflow is InvalidInput before source work; zero retains native Elapsed
without polling work. A due nominal deadline suppresses the next work poll,
so a late ready coordinator, permit, paired reply, native connection or
publication lock cannot advance that operation. Caller Drop releases directly
owned queued futures, fake devices and permits.

Thirteen actual manager tests include held coordinator/permit expiry before
release, cumulative admission and fallback budgets, late ready boundaries,
completed-device release at publication lock, all transport choices at zero,
overflow, cancellation, caller Drop and actual HTTP 502/timeout. Full/minimum
13/13, tunnel136/1 existing ignored, stack5, strict checks and section11
2604 Rust/13 Python pass; project .venv tooling73 pass. A read-only parallel
review ran no builds or edits; all nine gates ran serially under the primary.

This bounds cooperative work polling, not CPU preemption within a poll,
physical attachment freshness, the shutdown/publication linearization gap,
native worker joins or whole-scan/metadata/handoff budgets. Scanned candidate
deadlines and direct `ios tunnel start` remain their separate profiles.
22device NOT_RUN; other L01/L02, L03–L04 and classes5–9 stay active. All30 tasks
and subtasks exclude Q04; its known UID finding stays unfixed and other tasks'
original acceptance remains.

## Tunnel scan setup internal waits

The [scan setup profile](validation/tunnel-scan-setup-2026-10-06.md),
`57d57999fddbe0234b502686b65b8500d7d50cfd`, extends the existing checked15s
discovery clock to initial/post-USB table locks, USB coordinator admission and the no-authorized
failure/ready locks. These waits observe clear/shutdown while held. The biased
native expiry and per-poll nominal guard prevent late unlocks from pruning,
marking ready or inserting coordinator gates. Native USB errors already
returned survive same-poll cancellation. ListError cleanup skips stale writes
on cancellation and retains both primary IO and cleanup Elapsed on expiry;
source remains primary, wrapper/CLI rendering stays safe.

Sixteen actual manager cases cover held/late locks, gate-table observation,
cumulative discovery budget, cancellation/fresh scans and error retention.
Old-body control74607 exited101:0PASS/16FAIL; frozen91653 exited0/all nine
serial gates. Full/minimum16/16,tunnel152/1existing ignored,stack5,strict checks,
2620Rust/13Python and project .venv tooling73 pass. A delegated implementation
and independent review ran no Rust tools; product sources stayed frozen.

The clock ends at paired reply; candidate connection totals and post-paired
handoff/reconciliation/final readiness keep their original behavior. CPU
within one poll,shutdown/publication linearization,startup/whole-scan/metadata,
native construction/worker/server joins,physical freshness/reuse/restart/manual
epochs and other L01/L02/L03–L04/classes5–9 continue.22device NOT_RUN; active30
tasks/subtasks exclude Q04,whose UID risk remains unfixed; other acceptance
conditions remain. No maturity,ACK,async Drop,resource or performance claim.

## Userspace tunnel construction ownership

The [native construction profile](validation/userspace-construction-2026-10-06.md),
`19315169dda049d6ab1acd68414bf4976b147b47`, binds the listener and queries its
port before creating channels, splitting the stream or spawning workers. Initialization
failure now releases the stream without detached handles. Native IO source,
IPv6/MTU validation and the successful four-task/abort-on-Drop behavior remain.

One Linux exact current-thread child uses actual descriptor exhaustion and
public start:three EMFILE attempts retain aggregate/source errno,spawn no
tasks,Drop stream once and yield peer EOF. Restored limits allow a real start
with four workers,reader poll and observed abort completion/stream release. Old
direct-child97374 exited101:alive3/baseline0/drop0 at first attempt; later old
assertions were not reached. Frozen62936 exited0/all11 serial gates;new parent/
direct child/minimum1/1/1,Coretunnel46,originalstack5,strict/minimum checks,
full2621Rust/13Python and project .venv tooling73 PASS. Original assertions and
other product sources stay. RAII unwind/local_addr failure are structural
review,not separately executed fault-injection evidence.

This observes one host lifecycle without proxy clients,not explicit joins,device ACK,
RSS or long-run performance. Success/proxy/kernel/blocking/HTTP owners and
other L01/L02/L03–L04/classes5–9 continue;22device NOT_RUN. Active30 tasks/all
subtasks exclude Q04,UID risk unfixed,other acceptance unchanged. No maturity
or platform certification. All Rust commands primary-only and serial.

## Tunnel shutdown state changes

The [shutdown state profile](validation/tunnel-shutdown-fences-2026-10-07.md),
`7dc589bf4f26ead3597d991992f251d0ec29831e`, retains ticket generation/shutdown watch guards
through synchronous publication and failure-row mutations. Stale actions are
skipped with native cancellation; accepted actions finish before lifecycle
writers complete. Guards do not cross await or reenter watches. Original
early checks, policy/liveness order, row/count/slot rules and timing remain.
Scan capture rechecks terminal state after subscribing, retaining preclosed/
checked-deadline priority and reusable clear. Test callbacks release their
mutex before invocation and exist only in test builds.

Ten cases exercise actual manager publication on four routes, close/commit
ordering, stale stop/clear actions, scan source suppression and capture
priority, stale failure insert/update and reservation release. Old callsite
control9611 exited101:6PASS/4FAIL at first Usbmux/stop branches; later old routes/
clear were not negative-tested, and new-helper positive tests retained it.
Frozen68894 exited0/all9 serial gates,new full/minimum10/10,tunnel162/1existing
ignored,minCLIalltargets,stack5,strict/fmt,full2631Rust/13Python and tooling73.
Old assertions/other product sources remain; all Rust tools primary-only.

An accepted response can finish after close, and close can wait before the
separate server cleanup timer starts. No complete hard shutdown/CPU preemption
claim. Policy/physical snapshots, same-poll fallback, other scan writes,
startup/whole-scan/metadata/native joins/physical freshness/reuse/restart/manual
epochs and other L01/L02/L03-L04/classes5-9 continue. Existing proxy JoinSet
ownership remains distinct from explicit whole-owner joins.22device NOT_RUN;
active30 tasks/subtasks exclude Q04,UID unfixed,other acceptance unchanged.

## Packet forwarding reader cancellation

The [reader ownership profile](validation/forward-reader-owner-2026-10-07.md),
`a455d6788168b61b3bae49fa0a996b3fa2b5824e`, owns its one packet reader in a local JoinSet.
Caller future drop, parent abort or unwind requests reader abortion; active
framing/select/queue/MTU behavior and the normal outcome remain unchanged.
Seven cases observe actual duplex stream Drop/EOF and isolated task-baseline
recovery. Capacity1 write is actual IO; flush/TUN waits are controlled. Old
control40930 exited101:1PASS/6FAIL after fixture recovery. Frozen96089 exited0:
all10 serial gates,new full/minimum7/7,Coretunnel53,full2638Rust/13Python.

This is scheduled abort/eventual release, not normal reader join, reader panic
propagation, cleanup deadline or outer kernel/whole-owner join. Cooperative CPU
and other L01/L02/L03-L04/classes5-9 conditions remain;22device NOT_RUN.
Active30 tasks/subtasks exclude Q04;9complete21unfinished,UID unfixed.

## Packet forwarding watch cancellation

The [watch cancellation profile](validation/forward-watch-shutdown-2026-10-07.md),
`a4fc6aa6ccf455773009c3a1a38a9d0e8ca67958`, checks its one watch receiver before every
iteration and covers the selected branch's write/flush await. Private step
results preserve acquired native IO and execute existing reader abort_all.
Cancellation can abandon partial writes and close the connection. Active
framing/MTU/queue/reader ownership and all prior assertions remain.

Six new full/minimum6/6,Coretunnel59 and all10 frozen serial gates PASS;
full2644Rust/13Python,0FAIL/2existing ignored. Oldcontrol71973:2PASS/4FAIL;
wrong whole-loop control90513:0PASS/1FAIL. Fixture recovery precedes assertions.
Ready cancellation may skip unconsumed worker errors; no single-poll CPU/ACK/
normal reader join/panic propagation/cleanup deadline/outer-owner guarantee.
L01-L04/classes5-9 and22device NOT_RUN remain;active30/Q04excluded/UIDunfixed.

## Core native tunnel task ownership

The [native owner profile](validation/native-tunnel-owner-2026-10-07.md), `f00ff25395a8432087daf578c9dd23975b09be5a`, adds retained original
parent handles and child sets for library-created kernel/userspace runtimes.
shutdown captures its checked absolute clock and synchronous stop before future
poll; join-lock waits share it. Zero requests stop and returns native expiry;
overflow does not request stop. Timeout/caller-future Drop retains unfinished
joins while the owner remains held; retry uses a new budget. Outcomes are saved
before pending accounting changes. Final Drop is cancellation fallback only.

Normal raw forwarding aborts and joins its reader without an independent budget.
Kernel parent panic leaves its original reader set retained for explicit drain.
Userspace drains four parents and proxy children. Ten fixed cause slots plus
saturating omitted-observation count preserve primary and cleanup native causes.
joins_complete requires zero pending and valid accounting; it is not error-free,
deadline or device acknowledgement. Direct same-ledger work reports are recognized
by private identity; foreign/full/wrapped reports remain native work errors.

35 new tests;14 frozen serial gates/full2679Rust13Python0FAIL2existing ignored,
project .venv tools73 PASS. Five failing negative controls are recorded. CLI
retirement/server/Source pre-return ownership, other L01-L04/classes5-9 and22device
NOT_RUN remain. Active30/Q04allchildren excluded/9complete21unfinished/UIDunfixed.

## CLI native tunnel retention and ownership before RSD

The [CLI native retirement profile](validation/cli-tunnel-native-retirement-2026-10-07.md),
`6debaf9b6d78c10b4be1cfd55e2fbcbb69da8756`, reserves before Source polling and retains exact original runtime
Arcs before the initial RSD probe and all metadata/query/table waits. Each Core
strategy target has its own reservation; an abandoned earlier target remains
retired while the next must acquire fresh capacity. One4096-slot registry covers
all reserved/inflight/unpublished/active/retired ownership, including late attach.
Every managed-record Drop retires; joins never await under tables/watch borrows.

Shutdown seals/stops independently of table availability, drops the owned scanner
and drains native owners concurrently with metadata/HTTP inside one checked2s
absolute clock. Phase outcomes survive late Ready. Original native shutdown Ok or
the first outer report's valid-accounting joins_complete alone releases a slot;
failed fully joined owners retain bounded history. Readonly shutdown_status and
has_retained_outcomes capture acquired causes across outer-future cancellation.
Caller Drop requests stop, retains joins while the owner is held and cannot await.
Standalone start uses the same Core bridge and independent native terminal drain.

39 new tests/all16 frozen serial gates/full2718Rust13Python0FAIL2existing ignored,
three failing controls/tools73 PASS. This certifies the native host slice, not
generic HTTP/upgrades/blocking/startup/whole-scan/metadata/physical freshness/
reuse/restart/manual epoch or other L01-L04/classes5-9.22device NOT_RUN; active30
excluding Q04/all children,9complete21unfinished,UID untouched.

## Manual tunnel deadline crossing within one poll

The [manual same-poll profile](validation/tunnel-manual-poll-crossing-2026-10-07.md),
`a963679ab0f853b1a38bfcee9ac59471d4920641`, reuses the original checked15s deadline at Source admission,
native handoff, metadata/query and synchronous publication. A late insertion
removes the unobserved new row and restores the exact original row while table
and epoch fences stay held; the new lease retires after both fences release.
Late successful Ready becomes the independent timer's actual Elapsed. An acquired
native IO remains primary and default fallback stops before another contact.
Timers remain cooperative through synchronous work/Drop. Automatic candidate,
whole-scan, HTTP/upgrades/blocking/startup/metadata/device and other original
conditions remain.7newtests,201tunnel/1ignored,all7 serial gates/full2725
Rust13Python0FAIL2existingignored/tools73 PASS; active30 excludingQ04,
9complete21unfinished,22device NOT_RUN,UID untouched.

## Automatic tunnel candidate absolute deadline

Implementation `c92503ded8a60515a272f4afb1dba99085b130dc`; [automatic candidate evidence](validation/tunnel-automatic-deadline-2026-10-07.md).
Each admitted automatic candidate keeps its established15s budget under one
checked absolute clock across serial/cache/backoff/permit/Source and the shared
native-handoff/metadata/query/publication fences. Late real insertion removes its
unobserved row; after table/epoch guards release the exact original native Arc
retires with joins retained. Zero/unrepresentable internal budgets reject Source.
Actual Elapsed/scanned context and acquired native IO remain. The candidate
window/admission policy stays; synchronous Source/CPU/Drop remain cooperative.
Whole-scan post-paired handoff/reconciliation/readiness and out-of-clock failure bookkeeping,metadata byte budgets,HTTP/upgrades/blocking/startup/physical freshness/reuse/restart/manualepochs and all other original conditions remain.
7newtests/208tunnel1ignored/all7serial gates/full2732Rust13Python0FAIL2ignored/
tools73 PASS. Active30/allchildren excludingQ04,9complete21unfinished,
22device NOT_RUN;UID untouched,no main-task completion.

## Paired discovery acquired IO across clear and close

Implementation `1ab05e3bcefd3926a402dc645f55c72d16aad6f4`; [paired error retention evidence](validation/tunnel-paired-error-retention-2026-10-07.md).
A stale paired Source success is still discarded before candidate handoff/ready.
If clear/close completed inside the Source before its original error reply,
the scanner now returns it unchanged. Actual loopback partial
EOF and completed manager clear/close fixtures recover the initial exact native
userspace Arc/joins/status,DuplexEOF/tasks before regression assertions.
4newtests/212tunnel1ignored/all7serial gates/full2736Rust13Python0FAIL2ignored/
tools73 PASS;existing clocks and per-device isolation remain. Whole-scan post-paired totals/cancellation,out-of-clock failure bookkeeping,metadata bytes,HTTP/upgrades/blocking/startup/physical freshness/reuse/restart/manualepochs and all other original L01-L04/classes5-9 requirements remain.
Active30/allchildren excludingQ04,9complete21unfinished,22device NOT_RUN;UID untouched.

This paired profile covers cancellation completed inside the Source reply.
The later paired epoch fence still needs error-retention regression and repair.

## Paired reply final epoch fence

Implementation `aa3c3a46c19dd19175c879bcb44f5856d24259ec`; [final paired fence evidence](validation/tunnel-paired-final-fence-2026-10-07.md).
Clear/close after the first paired check no longer turns an acquired paired error into success. Nonempty clear-success is discarded before connection/ready;nonempty close can return Cancelled at ticket admission,so universal stale-success Ok is not promised.
Whole-scan total/cancellation/paired primary across caller drop,out-of-clock failure bookkeeping,metadata admission and all other original L01-L04/classes5-9 requirements remain.

## Managed snapshot and candidate admission

Implementation `327b0ef38e43576f2313c51c597b5c16cb7abc01`; [snapshot admission evidence](validation/tunnel-snapshot-admission-2026-10-08.md).
Raw USB/paired and complete borrowed candidate plans now have private row/field/UTF-8 byte/count limits. USB rejects before attachment eviction;paired refuses the entire batch as incomplete while legal USB candidates proceed and unknown Mobdev2 backoff stays. Source errors remain outside Complete/Incomplete admission state;stale Source success stays quiet. Tickets/clocks/four-window/individual cancellation stay.
Source allocation/RSS,global gate/failure/cache/native/list/federation bytes,whole-scan total/cancel/paired primary across caller drop/out-of-clock failure writes,L03-L04/classes5-9 and all other original acceptance remain.

## Deferred failure writes

Implementation `4415be9a65921a346208f912dbe59e399e471591`; [failure bookkeeping evidence](validation/tunnel-bookkeeping-2026-10-08.md).
Scans finish candidate connection futures before queueing writers for failed outcomes. Admission-bounded intents retain original tickets/permits;one checked absolute1s flush clock covers write-lock wait and the entire batch. StopA skips only A. Late row rollback retains exact old backoff/permit;timely prefix commits stay. This does not certify immediate stop while write-lock admission is pending.
Whole-scan total/global cancellation/durable paired or List primary across callerDrop,reconciliation/readiness/global metadata,successful-candidate writer fairness,L03-L04/classes5-9 and all other original acceptance remain.

## Whole-scan control and retained discovery ownership

The manager now owns a checked 30-second scan total and the clear/close epoch across all discovery, admission, connection, reconciliation, failure-write and readiness waits. Per-device stop remains isolated after discovery. Whole-clock checks participate in candidate publication/failure rollback and global reconciliation; whole expiry adds no failed-attempt backoff or new readiness.

A scan session retains the original acquired paired/List error before any subsequent wait. Caller Drop records it in one of eight fixed slots before releasing the scan lock. Empty/normal sessions release their slot; queued scans reserve none. Clear retains history and terminal closure seals admission. The server snapshots history after dropping its scanner and reports original errors independently from native joins. Details, evidence and remaining limitations are in [scan control evidence](validation/tunnel-scan-control-2026-10-08.md).

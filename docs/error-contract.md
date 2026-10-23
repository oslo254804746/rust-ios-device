# Error and machine-output contract

A02's current Linux host profiles are delivered. Additive Rust classification, opt-in final CLI machine
errors, Python factory/proxy metadata, caller-thread C categories, checked C
projections, protected router HTTP headers and initial RSD probe observations
are implemented. The current scoped acceptance and compatibility/platform/device
limits are recorded below; historical pending notes in individual slices
describe their acceptance date. R04's accepted
[stdout/encoding migration](plist-json-contract.md) remains independent.

## Current acceptance boundaries

| A02 requirement | Current host evidence / remaining boundary |
| --- | --- |
| Typed categories and retained sources | ErrorClass and native-source migrations below; preserving Lockdown APIs retain response codes and replace backup/debugserver text decisions. The [thirteen identified owner guards](validation/cli-owner-guards-2026-10-04.md) have typed outcomes; source-reviewed unreachable guards and opaque legacy Unknown profiles remain explicit |
| CLI output and partial failure | Checked stdout/stderr, final safe v1 errors, syslog/apps/install/CPU and the current [stream inventory](validation/cli-stream-contract-2026-10-04.md). Accessibility empty/partial traversal semantics and issued timeout/input guards accepted; stronger cleanup/reuse/queue contracts belong to L01 |
| Service and probe evidence | InitialRsdProbe distinguishes not_attempted/available/failed; RsdFeatureEvidence distinguishes missing service and unavailable metadata from advertised/not_advertised. NotAdvertised is not explicit unsupported |
| Python mapped errors | Native factories/runtime and proxy/mode preconditions have stable attributes; argument extraction keeps Python's built-in typed exceptions. Metadata delivery can itself fail with a safe cause |
| C mapped errors | Calling-thread category plus checked list/runtime/port/consuming cleanup; legacy void/zero and ALLOC ambiguity remain explicit compatibility profiles. ALLOC alone stays unknown, not inferred memory exhaustion |
| Protected HTTP errors | Existing statuses/body plus safe machine headers within protected routers; framework/opaque outcomes stay unknown. Protocol-native CDP/WebSocket messages after upgrade retain their separate protocol fields |

This inventory records the delivered Linux host profiles, not platform or device acceptance.
Explicitly unsupported typed outcomes are separate from skipped local probes,
absent capability metadata and a missing entry in an obtained RSD snapshot.
Legacy C cached zero and permissive supports_feature do not provide that
distinction; migrate to the observations/checked profiles described here.

## Rust classification

`ios_core::{classify_error, ErrorClass}` and `CoreError::class()` inspect typed
errors and their original sources. Existing public error enum definitions and
fields remain unchanged, including exhaustive CoreError matches. The explicit
migrations below describe returned-variant, Display and source-chain changes
where adapters previously discarded typed failures. ErrorClass is a new
non-exhaustive enum, serializes to a snake_case
string, and exposes the same stable code through `as_str()`. Callers must keep
a fallback for future categories. Human diagnostic text is not a classifier.

| Stable code | Typed evidence |
| --- | --- |
| `timeout` | IO TimedOut, Tokio Elapsed, or an existing service Timeout variant |
| `cancelled` | Caller-observed OperationCancelled marker or a cancelled Tokio JoinError; neither acknowledges device cleanup |
| `disconnected` | Named IO connection-reset/aborted/not-connected/broken-pipe categories or explicit closed/dead-connection variants |
| `service_missing` | XPC ServiceNotFound or the RemotePairing ServiceNotFound variant |
| `permission_denied` | IO PermissionDenied, AFC PermDenied, or typed pairing rejection |
| `protocol` | Existing Protocol/typed codec shape failures or invalid data; a codec's underlying IO failure takes precedence over its generic fallback |
| `invalid_input` | IO InvalidInput, credential/GPX parse, known non-IO ZIP package failure, or typed test-plan input validation |
| `unsupported` | Existing Unsupported/LegacyUnsupported/FeatureMissing/deprecated-version/backend variants |
| `device_not_found` | Existing Core/usbmux DeviceNotFound |
| `not_found` | IO NotFound, missing pair record or AFC ObjectNotFound; it is not inferred to mean a missing service |
| `resource_limit` | Explicit codec nesting/resource-limit, clipboard Limit or named AFC resource statuses |
| `conflict` | Typed media-in-use, DDI conflict or AFC existing/busy object |
| `precondition` | Typed passcode-set or legacy WebInspector Disabled; Disabled does not prove a permission rejection |
| `service_failure` | An explicit service/device/status failure without a more specific known category; unknown wire status codes stay here |
| `io` | Other IO kinds, including EINTR and UnexpectedEof where the underlying object could be a host file |
| `unknown` | Unrecognized typed errors, opaque legacy failures or lost source information |

Classification reads at most 32 sources and never formats or matches an error
message. The first precise typed category wins. Generic codec/IO fallback lets
an underlying writer/reader failure retain its category; custom IO payloads are
visited directly. Cycles and deeper chains stop at the same bound. A category
alone never implies retry safety, rollback, absence of side effects or successful
protocol cleanup. Keep the original error available when caller-authorized
details are needed; automatic diagnostics still use the redaction boundary.

## Known information-loss boundaries

WebInspector client guards preserve original IO/EOF, plist/JSON, typed PID
conversion and routing errors while invalidating owned metadata/cache state.
Later guarded paths reject static Protocol before IO/cache removal; empty idle
or pre-IO local XML failure can remain usable. Six variants and native timeout
wrapper/getter remain. Zero is source-free Timeout, effective overflow is Io
InvalidInput, and real expiry retains native Elapsed without synthesis. Startup
reports its effective budget rather than a changing response remainder; legacy
send/response Disabled mappings and the preserving API stay. Discovery idle
quiescence requires usable empty input; partial/routing or configured total
expiry cannot return a successful snapshot. Existing correlated business parsing
after a returned event was untouched by that client-only profile; correlated
ownership is covered by the subsequent profile below. Metadata cache limits
and frontend primary/cleanup owners remain separate work.
Local admission never proves device settings, cleanup or retry safety. See
[WebInspector client error evidence](validation/webinspector-client-lifecycle-2026-10-05.md).

Correlated Inspector/Automation now preserve the primary native/JSON/Protocol
failure while invalidating an incomplete committed client. Matched Inspector
business replies remain response objects; matched Automation failures keep the
original Protocol message and do not invalidate solely for that business error.
Skipped restoration keeps order on complete responses; a restoration failure
cannot replace an existing primary error. Incomplete/cancelled owners discard
their events with the invalid client, preventing late reply reuse.

Private WebInspectorLimit classifies ResourceLimit for local retained-event
count/compact-JSON byte and exhausted-ID admission under Io, using a feature-
gated typed rule. Generic IO OutOfMemory and unrelated decoder errors are not
repurposed. The six variants remain exhaustive. Session zero is source-free
Timeout, effective overflow migrates from internal Protocol to Io InvalidInput;
actual expiry keeps original effective total/Elapsed without synthesis. Nested
finite deadline scopes avoid independently overflowing configured MAX, and
scope exit restores the previous policy. Existing matching/selection/business
rules, startup Disabled mapping and safe diagnostics stay. Local categories or
queue release do not certify remote cleanup, retry or measured memory. See
[WebInspector session error evidence](validation/webinspector-session-lifecycle-2026-10-05.md).

WebInspector metadata/snapshot/staging count and compact-JSON byte rejection
also uses the existing private WebInspectorLimit typed ResourceLimit under Io
Other. The six variants remain exhaustive; no string matching, OutOfMemory or
unrelated decoder source is substituted. Raw list counts are checked before
parsing individual entries, so an oversized list rejects ResourceLimit even if
its entries would otherwise be malformed. Smaller malformed input retains the
original parser source. Admission precedes cache mutation; complete receiving
routing failures invalidate/drop owned state, and discovery cannot turn them
into successful idle snapshots. Later guarded IO rejects local invalid reuse.
No application identifier/title/URL or metadata payload is included in local
limit diagnostics. Logical count/byte admission does not certify decoder peak
or retry/remote cleanup. See
[WebInspector cache error evidence](validation/webinspector-cache-lifecycle-2026-10-05.md).

CLI wait_for_application/wait_for_inspectable_page now return a terminal issued
Timeout directly when the client was invalidated by partial receive or automatic
listing writes. Original native Elapsed/duration and Timeout classification
survive; another request can no longer mask the primary with local Protocol.
Empty usable idle keeps its retry policy. Native EOF, plist and ResourceLimit
still retain their original families/sources; selection and diagnostics stay.
This narrow adapter change does not certify other frontend whole-flow/cleanup
owners. See
[WebInspector discovery error evidence](validation/webinspector-discovery-lifecycle-2026-10-05.md).

Finite WebInspector CLI commands now reject zero as the existing source-free
Timeout before default discovery. Existing invalid-input guards and native
duration conversion failures keep their sources. The absolute frontend timer
retains actual Elapsed when selected; a successful Ctrl+C wait uses
OperationCancelled, and a failed wait keeps its original IO. Ready work cannot
win an already due total, nor can a successful cooperative poll finish after
the total and escape it. An already returned native failure remains primary.

Native startup/codec/command/output failures retain their sources through session
cleanup. A private dual-failure wrapper exposes the primary as its source,
retains the cleanup error separately and uses class-only Debug/static Display.
Automation stop plus socket teardown also preserve the first failure when the
second step expires. Successful output followed by cleanup failure now exits1;
no rollback, retry or remote cleanup ACK is implied. Core error variants and
final machine envelope stay. See
[WebInspector command error evidence](validation/webinspector-command-lifecycle-2026-10-05.md).

Interactive js-shell now uses the same checked admission and native timer/signal
sources. Setup cancellation is typed Cancelled and exits1. After setup, a
successful directly owned Ctrl+C signal is a normal stop; failed signal
registration remains a native error. Completed JavaScript or remote-response
business errors may produce the existing error records and continue. Native
wire, deadline, codec, input or output failures stop even when continue-on-error
is enabled, retaining the first source instead of attempting an invalid client.

Session teardown and explicit stdin flag restoration both preserve a primary
failure and their own cleanup failures. Cleanup failure after an otherwise
successful stop returns failure; prior stdout records remain emitted. Native
EOF retains its observed IO source/class rather than being relabeled from prose.
The final machine envelope and error variants stay. See
[WebInspector shell error evidence](validation/webinspector-shell-lifecycle-2026-10-05.md).

Companion's shared operation guard preserves the original native/frame error
while invalidating local client/listener/forwarding reuse. Later guarded paths
return static Protocol before I/O, including owners already queued on the mutex.
Complete device business replies are parsed after the guard and keep original
sources/categories and reuse; is_usable does not assert retry safety.

Request and active-idle zero admission uses source-free legacy Timeout(duration).
Overflow is existing Io InvalidInput, without fabricated Elapsed. Actual request,
idle and stop expiry retains the existing native Elapsed/duration wrapper; stop
reports its effective min(supplied budget,5s). Terminal idle expiry now drops the
owned read and invalidates shared state. Successful cached stop/finished listener
outcomes remain local no-ops independent of later input budgets. Existing Drop
cleanup errors remain best-effort and unreported on the legacy handle path. The
four variants, safe diagnostics and automatic v1 fields stay. See
[Companion lifecycle evidence](validation/companion-lifecycle-2026-10-04.md).

The subsequent consuming close API schedules no Drop fallback, even on an
unpolled/cancelled future or complete business error. Companion CLI rejects
finite zero/overflow as InvalidInput before discovery. Actual setup/total expiry
retains native Elapsed and signal cancellation uses OperationCancelled; native
signal IO retains its original kind. Finite expiry's previous synthetic
TimedOut payload is deliberately migrated. Started listener/forwarding operator
stop remains graceful; a forwarding cleanup failure after success is returned.
On dual failure a private aggregate owns both originals but source/category
follows the primary, with static Display and category-only Debug. Native cleanup
UnexpectedEof remains Io with its original object, while business failure keeps
Protocol and issued cleanup expiry has its own native duration/Elapsed. An
invalid session skips unsafe cleanup without inventing another error. Neither
category nor successful local close proves remote release or safe retry. See
[Companion CLI error evidence](validation/companion-cli-control-2026-10-05.md).

NotificationProxy actual expiry retains Elapsed through existing Io/retained
context, including budget exhaustion between complete unrelated events: an
already due timer is polled before reading another frame. Zero admission uses
source-free existing Timeout; ordinary deadline overflow is Io InvalidInput.
Native/frame decoding errors keep their original family/cause and invalidate;
complete business-shape errors keep reuse. ProxyDeath preserves its event/error
and disconnected category while closing state; later invalid/closed operations
return Protocol. Successful Shutdown is a bounded write/flush without ACK.
No core cleanup aggregate or automatic close is added. The subsequent CLI owner
below supplies returned-error cleanup and primary retention. See
[notification lifecycle evidence](validation/notification-lifecycle-2026-10-04.md).

Notification CLI now retains primary and cleanup in a private aggregate whose
source/category follows the primary. Cleanup keeps native error or actual
Elapsed separately; it cannot override earlier permission/protocol/output
failure. Single failure retains its original object. Unexpected cleanup error
after success is returned, including actual default five-second expiry. Display
is static and Debug exposes only categories; safe v1 output retains its fields.
Already invalid/ProxyDeath transport skips cleanup rather than manufacturing a
secondary error or sending late control bytes. Drop closes owned transport;
the subsequent setup/total/signal owner is described below; remote cleanup remains
a separate condition. See
[notification CLI cleanup evidence](validation/notification-cli-cleanup-2026-10-04.md).

Notification CLI finite zero/overflow admission is existing InvalidInput before
device selection/I/O, without fabricated Elapsed. Setup, send and finite total
expiry retain a real issued Tokio Elapsed with fixed phase context. The same
pinned signal survives default discovery/setup into capture. Setup/finite-command
operator stop uses existing OperationCancelled; Stream/Observe stop and Stream
total expiry return ordinary completion subject to cleanup. Native signal-wait
failure retains its original IO cause. Output errors and any later cleanup
failure retain the primary category/source through the existing CLI aggregate.

Actual CLI v1 admission errors remain class invalid_input, phase notify, exit1
and conservative effects unknown; no new field or cause formatting is added.
Human diagnostics retain the existing io/InvalidInput label. Linux host SIGINT
during a fake default ListDevices wait yields cancelled/notify/exit1 and owned
peer EOF. No physical service is contacted. Injected write_line_to permission
faults exercise actual command consumption, without claiming OS stdout failure
or device cleanup acceptance. See
[notification CLI control evidence](validation/notification-cli-control-2026-10-04.md).

MobileBackup2's current operation owner retains both failures through
Mobilebackup2Error::cleanup_failure() and its opaque SessionCleanupFailure.
operation_error()/cleanup_error() borrow the original objects. Existing Io wraps
dual failure; its source chain and category follow the primary, including when
outer expiry drops a pending cleanup body. Cleanup expiry after an earlier
protocol error therefore remains protocol, with issued Elapsed in the separately
borrowed cleanup branch. Single failures keep their original object; unexpected
cleanup failure after success is reported. Lookup inspects at most 32 typed
sources, including custom IO payloads. The four error variants remain.
Zero/overflow admission precedes I/O; native failure or interrupted progress
rejects reuse. Total expiry starts no late disconnect. Expected peer closure
after a complete terminal reply preserves the existing completion policy while
local reuse becomes unavailable. No remote cleanup/retry claim follows. See
[current backup lifecycle evidence](validation/backup-lifecycle-2026-10-04.md).

DeviceLink operation expiry retains actual Elapsed in existing Io.get_ref;
zero budget is source-free TimedOut and unrepresentable ordinary budgets are
InvalidInput. Disconnect clamps its configured budget to five seconds, including
Duration::MAX. Original native/codec causes remain intact; later invalid/closed
reuse rejects with existing Protocol. Complete process-envelope errors keep
framing reusable, while an aborted version handshake after progress invalidates.
MobileBackup2 native EOF now returns its original DeviceLink::Io directly rather
than issuing a later cleanup timeout: invalid DeviceLink rejects disconnect
before another write. Existing cleanup-timeout retention on valid business/
success/pending paths was retained in that DeviceLink increment. The subsequent
MobileBackup2 owner above supplies raw/session ownership and primary retention. See
[DeviceLink lifecycle evidence](validation/device-link-lifecycle-2026-10-04.md).

LockdownClient request/close expiry retains actual Elapsed in existing
LockdownError::Io.get_ref. Zero is a source-free TimedOut decision; unrepresentable
request budgets are InvalidInput. Original transport/codec causes and complete
business reply errors retain their objects/categories. Later invalid/closed
client requests reject with existing Protocol. A local pre-I/O serialization
error and an empty cancelled/expired write wait can preserve reuse; transport
failure/native EOF invalidates even without accepted application bytes.
Successful StopSession means a complete write/flush, not a remote cleanup ACK.
It marks closed and repeated stop is a local no-op. Interrupted close keeps the
private session identifier uncertain; it does not establish that the remote
session remains active. Manual cleanup returns its own result; retaining an
earlier primary error alongside it remains caller-owned. No error enum, raw
borrowed helper policy or frontend schema changes. See
[Lockdown lifecycle evidence](validation/lockdown-lifecycle-2026-10-04.md).

Borrowed RSD guards return original operation errors and leave existing stage
expiry's native Elapsed/context intact. Later invalid-framer reuse returns the
existing H2-to-XPC Protocol family, separately from that original failure.
Commitment includes a complete initialization send, a consumed wrapper header
already buffered by H2, or a completed message skipped during discovery.
Cancellation/failure after commitment invalidates H2 and clears its retained
state; there is no upper XpcConnection pending map in these bootstrap helpers.
Empty idle and normal completion retain reuse; a complete stream-local error
before commitment can preserve other streams. No new RSD timer/zero-budget API,
remote cleanup or retry guarantee is added. See
[RSD lifecycle evidence](validation/rsd-lifecycle-2026-10-04.md).

XPC configured expiry retains original Elapsed in existing XpcError::Io.get_ref;
zero is source-free TimedOut, unrepresentable deadlines are InvalidInput.
Cancelled committed operations and lower H2 failure clear pending replies and
reject reuse through the existing typed H2-to-XPC adapter. Empty idle expiry is
distinct from cancellation after a wrapper header or sent request. Complete
service business errors parsed outside the transport guard keep their original
objects/categories and reuse rules. Raw stream Drop/announced-file failure do
not await remote cleanup; split send/recv remain caller-owned operations. See
[XPC lifecycle boundaries](validation/xpc-lifecycle-2026-10-04.md).

H2 native transport errors keep their existing Io objects and kinds. Internal
configured expiry retains actual Elapsed in H2Error::Io's get_ref; zero budget
is source-free TimedOut and an unrepresentable budget is InvalidInput. Existing
H2-to-XPC adapters retain these typed causes. Rejected reuse returns the existing
Protocol family, separately from the original operation's error. Complete
stream-local errors do not globally invalidate H2. This profile does not yet
clear upper XPC pending replies or guard an entire call on its own; see
[H2 lifecycle evidence](validation/h2-lifecycle-2026-10-04.md).

AFC request/cleanup expiry retains original Elapsed in AfcError::Io's get_ref
payload; zero budget and oversized deadline are issued source-free Timeout/
InvalidInput decisions. Unusable client rejection is Protocol, separate from
the original failure. Ordinary complete device Status errors keep reuse, while
failed automatic-file cleanup or cancellation after open prevents it. Dual
transfer/close failure wraps both original objects in existing Io with the
primary as source and a safe fixed Display/Debug; a primary Status may change
outer variant on this path but keeps its precise category. Cleanup-only failure
returns its original object. No extra public error variant or retry/rollback/
device cleanup claim. See [AFC migration](validation/afc-lifecycle-2026-10-04.md).

FPS capture retains native setup/read/signal/output failures and actual cleanup
Elapsed. If both capture and stop fail, a private wrapper keeps the primary in
its source chain and owns cleanup separately; successful count with failed stop
now fails. A source-free private resource decision for the timestamp cap maps to
resource_limit only in the CLI's existing 32-source scan, without confirmation;
general Core classification for it remains Unknown. Deeper/opaque errors retain
their previous fallback. Final schema v1 and effects unknown remain. See the
[FPS lifecycle migration](validation/fps-lifecycle-2026-10-04.md); a failed or
completed stop write does not establish device cleanup or safe retry.

DTX shared-budget expiry retains actual Elapsed in existing DtxError::Io;
zero budget is an issued source-free timeout. The new is_usable getter reports
local framing/correlation state after failure or cancellation. An unusable
connection's existing Protocol error is separate from the original operation's
retained native failure; neither permits guessing retry/rollback/remote cleanup.
Empty idle reads remain usable, partial frames fail immediately at affected
service entry points. See [L01 lifecycle](lifecycle-contract.md) and the
[DTX host gate](validation/dtx-lifecycle-2026-10-04.md).

`LockdownResponseError` observes device-issued Error strings through the additive
`LockdownClient::get_value_preserving_errors`, `start_service_preserving_errors`
and ConnectedDevice preserving value-query methods. Private fields expose fixed
operation, exact code and class getters; in_error_chain examines at most 32 typed
sources including IO custom payloads. Debug/Display omit raw codes. GetValue's
exact MissingValue is not_found; StartService's exact InvalidService is
service_missing; other/wrong-operation codes are service_failure. This compares
machine fields, never error prose. No OS/parser/timer cause is invented for a
device response outcome. Existing exhaustive errors wrap the observation in Io.

Legacy get_value/start_service retain their Protocol strings, Display and shape
policy; existing C value queries still use that profile. Their lost response
codes cannot be reconstructed from words. Preserving GetValue explicitly rejects
non-string Error fields even when Value exists; legacy still accepts its Value.
Missing Value without Error remains malformed protocol, distinct from an explicit
MissingValue. Native IO/plist errors and valid request/value/port/SSL results stay.

Backup's three WillEncrypt calls now opt in. Only observed GetValue/MissingValue
defaults false; generic host NotFound, legacy prose and similar strings propagate
their native source. Invalid received bool shapes classify protocol. Debugserver
keeps its two-name order, using only observed StartService/InvalidService for
fallback, and preserves the final error after exact misses. Category/Display/
source/correlation migrate, while success/exit/v1/effects and existing lifecycle
policy stay. See the [response-code host evidence](validation/lockdown-response-codes-2026-10-04.md)
for actual owned TLS/duplex versus private/default/ConnectedDevice scope.

Existing Protocol strings cannot be reclassified by words such as "timeout"
or "permission denied". Existing Other strings remain Unknown. Several older
callers flatten error types into strings, and WebInspector startup maps some
timeout/closed-stream failures to Disabled. Those sources require an additive
API or an explicit migration; this classifier does not restore information
that was already discarded. IO UnexpectedEof and EINTR alone do not prove a
device disconnect or a cancelled operation.

`OperationCancelled::new()` is an additive error marker with private fields,
also available through Default. A caller uses it only after observing local
cancellation; it is not inferred from an Interrupted IO kind or prose. Existing
CoreError variants stay unchanged. WebInspector launch records this marker under
its existing cancellation context when Ctrl+C completes successfully; failed
signal registration retains its original IO error, and the outer timeout keeps
Elapsed. The opt-in final CLI class consequently becomes `cancelled` instead
of the former string error's `unknown`. Exit 1, safe human text, success output,
unbiased selection, timeout and caller-owned future scope remain unchanged.
Host ready-signal/owned-pending tests prove these source and drop boundaries;
they do not validate actual OS Ctrl+C against a device or orderly Apple cleanup.
An additional actual CLI fixture sends SIGINT to its owned process while its
sole owned usbmux peer withholds a ListDevices reply. It observes exit 1,
`cancelled`/unknown effects and peer EOF, before any pairing/device service.
This validates the host signal/error path and local socket release only.

The CLI connection, application-list, launch and Safari-launch deadline wrappers
retain Tokio Elapsed through anyhow context. An expired shared operation budget
retains a typed WebInspector Timeout under its existing human context. These
changes preserve timeout values and exits. They do not establish
cancellation/cleanup acknowledgement.

`WebInspectorClient::start_preserving_errors(timeout)` shares the existing
identifier-report/event loop and one deadline with `start(timeout)`, but returns
original IO errors and native Elapsed under the existing Io variant. The additive
`WebInspectorError::timeout_duration()` reports the budget formerly carried by
Timeout(Duration): response/event stages can report their remaining phase budget,
while a blocked report write reports the original startup budget. The legacy
Timeout variant remains for created budget markers and existing callers. Legacy
`start` still intentionally maps response own-timeout/closed-stream errors to
Disabled, discarding that source; a blocked report-write timer is returned
directly. The preserving method keeps the original startup source without
inventing a device setting or unsupported reply from timeout or EOF.

WebInspector, Companion and NotificationProxy now retain 22 original deadline/
integer-parser/conversion causes under their existing Io variants. Native timers
classify timeout, malformed received integers remain protocol and local frame
length conversion becomes invalid_input. Public error enum definitions stay
exhaustive; CompanionError also adds `timeout_duration()`. Both budget getters
inspect at most 32 typed sources for a private service-issued deadline or their
own legacy Timeout variant, without formatting. A generic IO TimedOut or standalone
Elapsed is not proof of that issuer and returns None, even when its class is
timeout. None does not prove absence of a timeout. WebInspector quiescence and
CLI/CDP idle loops continue to consume only their own deadlines; other IO timeout
still propagates. Companion listener expiry remains terminal and forwarding stop
state still follows its existing successful response. Created zero-budget
markers, protocol limits, wire/success, budgets and cleanup behavior stay. See
the [discovery source migration and host evidence](validation/discovery-native-sources-2026-10-03.md)
for returned-variant/Display/source access changes and partial-operation limits.

CrashReport and OS Trace retain 21 native service causes, plus OS Trace CLI's
native PID conversion, through existing Io/private source contexts. Timers
classify timeout, received report/trace decoding remains protocol, and local
request/PID/regex errors become invalid_input. Archive native IO kinds/OS chains
survive instead of Protocol prose; a truncated host archive can classify io.
Existing public error families and successful wire/state/limits stay. CrashReport
watch uses a bounded typed parser marker to preserve its three parse retries;
generic protocol/IO class alone never identifies that retry scope. Checked-add
deadline and parser shape guards retain their source-free outcomes.

`MessageFilterOptions::compile_preserving_errors()` adds typed filter compilation;
Core next_filtered_options and CLI use it before reading/connecting. The old
compile method keeps its String return/formatting and deliberately loses the
native regex type. Term count/length guards on the new method are source-free
InvalidInput; actual regex errors keep their original cause. Filter predicates,
budgets, output/envelope shapes and failure exits stay. A private CLI PID wrapper
also preserves standard source traversal through IO custom payload forwarding.
The [diagnostic source migration](validation/diagnostic-native-sources-2026-10-03.md)
records returned variants/Display, local AFC peer cleanup, archive OS faults and
the limits of partial-read/write/device evidence.

Cryptex, MobileActivation, InstallCoordination and Pasteboard now retain 21
native causes. Their timers use existing Io or Xpc(Io), retaining actual Elapsed
and the former phase/Duration/whole-seconds value. Additive getters are
`CryptexError::timeout_phase()`, `MobileActivationError::timeout_duration()`,
`InstallCoordinationError::timeout_duration()` and
`PasteboardError::timeout_seconds()`. Each recognizes only its own private
issuer or legacy Timeout value within 32 typed sources. Generic IO timeout or
unissued Elapsed returns None; None does not exclude a timeout. The seconds
getter preserves as_secs truncation, including Some(0). Recognition establishes
neither safe retry nor completed remote cleanup.

Received pasteboard integer/UUID errors stay protocol. Cryptex HTTP timeouts and
builder faults now classify timeout/invalid_input; native cache publication IO
and InstallCoordination CoreError keep their original kinds/classes/sources
instead of Protocol strings. Returned variants/Display/immediate payloads and
safe diagnostic correlation can change; native source access follows redaction
policy. Public exhaustive families, production deadlines, wire and existing
owner/fail-closed/fallback behavior stay. Created activation/InstallCoordination
deadline markers remain source-free. One promised-size guard now compares the
full nonnegative integer against its limit, closing a 32-bit conversion bypass;
32-bit behavior is source-reviewed. The [service deadline evidence](validation/service-deadline-native-sources-2026-10-03.md)
records actual local faults, helper/device-entry scope and migration limits.

ImageMounter download/cache/extraction and TSS now preserve 34 original native
boundaries under existing Io with fixed private context. IO/OS and nested ZIP
IO kinds/sources survive; received ZIP/UTF-8/plist errors classify protocol,
local TSS serialization and HTTP builder faults invalid_input, HTTP timeout
timeout, and actual host JoinError cancellation cancelled (panic stays unknown).
Other native HTTP categories stay, including unknown. Affected Download/Tss
variants, Display, immediate payloads, kinds and safe correlation can change;
use typed class/source inspection. Source-free status/ticket/size/cache/build/
HTML/magic/missing-entry guards keep their existing outcomes. Public six-variant
family, signatures, wire, proxy precedence/policy, budgets and partial-file/
blocking-work behavior stay. Source retention does not add bounds, rollback or
cleanup guarantees. The [DDI/TSS evidence](validation/image-assets-native-sources-2026-10-03.md)
records native/local-injected faults, corrected baseline fixtures and limits.

Historical A02 MobileBackup2 profile (superseded for lifecycle/dual-failure
precedence by the current owner above): Unback/Extract retain actual operation and disconnect `Elapsed`
under the existing Io variant, changing their former Protocol class to timeout.
`Mobilebackup2Error::device_operation_timeout()` inspects at most 32 typed
sources for issued `DeviceOperationTimeout` evidence. Its `operation()`,
`phase()` and `duration()` report the typed operation, awaiting phase and total
shared budget. Generic IO timeouts, foreign Elapsed and protocol prose return
None; None does not prove absence of a timeout. New operation/phase enums are
non-exhaustive; the evidence has private fields and no public constructor.

If disconnect times out after an operation failure, the main source chain
follows that final Elapsed and classification is timeout. The evidence's
`operation_error()` separately borrows the earlier Mobilebackup2Error with its
original sources, including an earlier operation Elapsed. These are two branches,
not two causes in one linear source chain. A successful content value still is
not returned when disconnect times out. Native received path UTF-8 and transfer
count conversions now retain their causes under Io InvalidData, keeping protocol
classification; the count failure is source-reviewed on this 64-bit host.

Four existing error variants, five-minute shared absolute deadline, requests,
validation order, transfer bounds, state reset and best-effort disconnect policy
stay. Affected variants/Display/immediate payloads and diagnostic correlation
can change; explicit native sources and the earlier-error branch follow the
redaction policy. Blocking work may outlive a dropped awaiter, partial transfers
may remain, and client reuse or remote cleanup is not certified. See the
[backup deadline evidence](validation/backup-deadline-native-sources-2026-10-03.md)
for actual owned timers, primary-loss negative controls and public/minimum scope.

Restore retains 25 original H2/XPC, IPSW and TSS error objects under existing
Io with fixed private context. Native IO, including nested ZIP/H2 IO, keeps its
kind/category/OS cause; H2 protocol and received TSS UTF-8/plist stay protocol.
XPC keeps its original category, including typed input and opaque unknown.
Local IPSW ZIP format/password/manifest and TSS serialization faults become
invalid_input; HTTP builder faults become invalid_input and HTTP expiry timeout.
Other HTTP errors keep their own category, including unknown. These failures
formerly flattened to Protocol, so returned variant, Display, immediate payload,
class and safe diagnostic correlation can change. Follow typed class/source
traversal and the redaction policy for explicit native data.

The three-variant public family, signatures/fields/features/classifier, H2/XPC
wire/state and TSS/IPSW validation/limits stay. Request/bootstrap encoding and
firmware size conversion include source-reviewed failure branches; host checks
do not manufacture giant inputs. Direct status/shape/size guards retain existing
Protocol outcomes. The legacy TSS batch/per-updater API intentionally consumes
errors into failed updater names; this source-retention migration alone does not
fix that partial-result contract or certify retry, connection reuse, restore
execution or Apple signing. See the
[restore source evidence](validation/restore-native-sources-2026-10-03.md).

TSS callers can now select `prefetch_tss_batch_preserving_errors` to retain
failures from `BuildCombinedRequest`, `SignCombinedRequest` and
`SignIndividualRequest`. `TssBatchFailureKind::Error` exposes the original
RestoreError and its source chain/class without reclassification by text.
`MissingTicket` means a successful signer response lacked a nonempty data ticket;
its error and class are None. A whole combined failure has no rider index;
per-rider observations carry the input index, including duplicate updater names.
These are independent attempts, not successive links in one Error::source chain.

Final tickets/failed names retain the legacy semantics. Intermediate missing
tickets and recovered combined errors remain in the ordered observations; a
nonempty list alone does not mean final failure. Invalid riders still return Err
before any signing, and empty input invokes no signer. `into_parts`/`into_error`
preserve native ownership; `into_result` deliberately discards observations.
New Debug implementations show typed metadata/counts only. Explicit original
sources may carry private input and follow the existing redaction policy.
The new API adds no timeout, retry, cancellation, allocation cap on caller-owned
error payloads, rollback or cleanup guarantee. Wire/bounds/fallback and legacy
output stay; no CLI/HTTP/binding currently consumes this new result. See
[batch observation evidence](validation/tss-batch-observations-2026-10-03.md).

Application/test process identifiers, fixed application plist encoding, five IPA
integer conversions and GPX XML/paired timestamp parsing now retain eleven
original native error objects. AppService uses its existing `Xpc(Io)` family;
Testmanager's DtxError, ZipConduitError and SimLocationError use existing Io with
InvalidInput and fixed private context. Local PID/ZIP failures migrate from
protocol to invalid_input; GPX keeps invalid_input while moving from GpxParse
text to native Io. The plist boundary is a local encoder: the private writer
fault fixture retains its nested IO kind/OS code while the wrapper is input.

Public families/signatures/fields/features/classifier stay. Returned variants,
Display, immediate source payload and diagnostic correlation can change; inspect
typed classes and bounded sources instead of matching text. Existing validation
order, feature admission, ZIP metadata arithmetic guards/headers/CRC/transfer
order and GPX coordinates/timing/stream bytes stay. A ZIP input error may follow
earlier install writes; the private header zero-write fixture does not prove
whole-install preflight. GPX/test PID no-write observations are scoped to their
owned local streams. No cleanup/retry/rollback or new parser/transfer limits.
Explicit sources remain private data under the redaction policy. See
[input source evidence](validation/service-input-native-sources-2026-10-03.md)
for private injected encoding versus public consumer and unreachable 64-bit
conversion branches.

CLI retains 105 native objects previously formatted/discarded: 81 service,
plan and clock errors, 13 Elapsed, six local input errors, three received
hex/integer errors and two file IO errors. Service objects retain their original
typed category, including Unknown for opaque or unclassified sources. Actual
Elapsed is Timeout. Local input IO wrappers use InvalidInput, received-data
wrappers InvalidData/Protocol, with fixed private Debug and original source.
These wrapper kinds express owner provenance and are not OS fault evidence.

Four upstream input Strings remain owned opaque data with no native source;
four unit-returning URL adapters and five input/received decisions have no
fabricated cause. Three cancellation paths use existing OperationCancelled.
HID distinguishes successful signal cancellation from native signal-registration
failure. Its budget and select policy stay; no new cleanup/reuse guarantee.
Machine schemas/exit policies and finite successful output stay, but classes,
Display/immediate source and diagnostic correlation can change. GPX/P12/coordinate
contexts omit raw input/path text; explicit sources/opaque Display remain private
data. Migrate string/Unknown/cancellation assumptions to typed categories. See
[CLI native evidence](validation/cli-native-sources-2026-10-04.md).

CLI's sixty direct status/diagnostic eprintln sites use the checked stderr
writer: fifty-four return native IO failures, six retain a prior primary failure
when its diagnostic also fails. Status failure now reaches normal exit 1 rather
than panic; final reporting remains best effort on unavailable stderr. Existing
line arguments/redaction and stdout/stderr roles stay. Prior output/effects can
exist before a status write; no general no-effects or rollback guarantee follows.

Sysmontap CPU read failure now returns its original DtxError after the existing
stop attempt, preserving prior records and the primary over reporting failure.
Normal count/deadline/Ok(None) completion and ignored stop errors stay. Finite
record formatting stays; non-finite CPU JSON is rejected before that record as
source-free InvalidData/Protocol. Existing successful human formatting stays.
Unix directory source/destination CString errors retain native NulError and
owned path bytes with fixed safe context, existing InvalidInput and no rename.
Explicit source access is private data. Root/binding schemas stay; migrate
panic/success assumptions to typed error/exit handling. See
[CLI diagnostic evidence](validation/cli-diagnostic-sources-2026-10-04.md).

Fifteen discovery/connection adapters retain original mDNS, integer, JSON,
returned TunnelError and Elapsed objects through existing Io with fixed private
context. Opaque mdns-sd errors remain Unknown, including its Msg strings; the
backend's own erased OS causes cannot be reconstructed. Local MTU rejection is
InvalidInput, with an original TryFromIntError only for the upper overflow;
the minimum-MTU decision has no invented native cause. Received port/MTU/JSON
errors remain Protocol through InvalidData, including an originally returned
local MTU error wrapped with received-data provenance.

CDTunnel's actual Elapsed now classifies Timeout instead of textual Protocol.
RSD check-in and lockdown dial keep Timeout and their original Elapsed, fixed
phase and configured Duration. Context Debug hides native text and addresses;
explicit getters/sources remain private data. Returned variants, Display and
immediate-source/diagnostic correlation can change. Existing three TunnelError
variants, root paths, binding schemas and success output remain; migrate text
matches to typed class and bounded source traversal. No new deadline, retry,
invalidation, daemon-close acknowledgement or safe-reuse rule. See
[connection source evidence](validation/connection-native-sources-2026-10-04.md)
for actual owned timer/duplex/loopback versus private backend-object fixtures,
unreachable fixed encoding adapters and source-reviewed TCP dial expiry.

Eight guarded AFC/DTX/FileService/symbol conversion adapters retain their native
objects through existing Io with private fixed context, optional full-width
declared u64 and InvalidData. Classification stays Protocol: the container
records received-data provenance, not an OS fault. AFC maps the original
zerocopy cast source into owned Box<[u8]> only on failure; successful borrowed
decoding stays. Debug hides native text/bytes; explicit sources remain private.

Six adapters cannot fail after the current guards on any supported host,
including AFC this_len after this <= entire and successful entire conversion.
Only AFC entire_len and remote symbol count can overflow on narrower usize.
The 64-bit production tests reach existing frame/count ceilings; private
39-byte cast, actual u32 overflow and short-slice fixtures verify adapter
ownership, without claiming those production branches ran. Returned variant,
Display/immediate-source and diagnostic correlation can change; use typed
class/source traversal. Zerocopy's existing std feature is required for its
native StdError on MSRV, with unchanged version/lockfile. No new lifecycle,
limit or success behavior. See
[guarded source evidence](validation/guarded-native-sources-2026-10-03.md).

DProxy callers can select `StreamDecoder::push_preserving_errors`. Its opaque
`DecodeOutcome` keeps ordered events, independent `DecodeFailure` observations
and the original optional fatal DproxyError. It retains original plist/DTX/XPC
errors and the XPC length check's original IO error instead of just their text.
The latter can be an SDK guard error with no deeper source; retaining it does
not turn a limit decision into an OS failure. The two fixed XPC slice-conversion
errors are retained at their existing adapters but unreachable after the header
length check on this host.

Failures expose direction, typed phase/kind, optional stream ID and an index in
this push's events. Source-free resource limits, incomplete END_STREAM and the
existing missing-stream decision have no fabricated native error or ErrorClass.
Fatal guards have no corresponding event; fatal_error_class classifies the
unchanged DproxyError independently. A fatal push can retain earlier successful
events and nonfatal errors. Independent failures are not one source chain.
Debug shows metadata/counts only; explicit events/errors follow source privacy.

`into_parts()` owns all three branches; `into_error()` takes a failure's original
error. `into_result()` deliberately drops observations and, on fatal error,
preceding events, matching old `push`. Only input skipped because of a previous
DTX failure sets `input_ignored`; partial/Binary input with no events does not.
Existing event JSON/summary/guard text, parsing limits, clear/stop behavior and
forwarding remain. The legacy path allocates no observation storage.
No new retry, reset, rollback or cleanup promise. See
[decoder observation evidence](validation/dproxy-decoder-observations-2026-10-03.md).

`ProxyRecorder::record_chunk_with_observer` and
`proxy_bidirectional_with_observer` now deliver owned DecodeFailure values to
a synchronous callback. Raw writing precedes decoding; a raw-write failure
skips decoding/callbacks and can still leave partial raw bytes. Observations
arrive before event writing or a fatal decoder return, so a later serializer,
newline, diagnostic or flush failure does not erase already delivered native
objects. Those failures are independent of the original primary Result error,
not its source chain. Decoder fatal still skips all events from that push;
event indices refer to decoded events in the push, not persisted NDJSON lines
or global session indices. Use the decoder outcome directly for partial events.

The proxy serializes both directions' callbacks under the existing recording
mutex before forwarding. It holds no session failure list; callers must bound
their own retention and keep callbacks quick. Blocking callbacks stall recording;
callback panics are not intercepted. Existing record_chunk/proxy_bidirectional
retain their unobserved mode. CLI selects the observer mode and emits only safe
metadata at DEBUG (`-vv`), with no native Display/source/raw event content.
Ordinary nonfatal decode errors keep their existing successful-recording policy.

Core event diagnostics and two CLI status lines now check stderr writes and
return the original IO error instead of panic. A diagnostic failure can follow
raw/complete NDJSON writes, before explicit event flush and before forwarding
the current chunk; no disk-durability/rollback guarantee follows. Existing
primary serde/raw/newline/flush/transport variants, message text, stdout/JSON
and failure dispatch stay. Dispatcher maps a returned operation failure to exit
1 and best-effort final reporting; physical DProxy status placement is reviewed,
not a device invocation. Tracing itself remains the existing logging facility.
See [recording and stderr acceptance](validation/dproxy-recording-observations-2026-10-03.md).

CLI WebInspector tasks, CDP discovery/page/browser startup and WebDriver runtime
construction now use start_preserving_errors. A closed stream can yield an IO
category in existing safe diagnostics instead of opaque Disabled correlation.
Existing failure exits/HTTP statuses/error body shapes, successful output and
wire frames stay unchanged. Migrate Rust callers by selecting the new method
and inspecting WebInspectorError or classify_error; callers needing the legacy
mapping can keep start. Neither method promises retry safety after a partial
write, a cleanup acknowledgement or that AutomationAvailability::NotAvailable
means Web Inspector itself is disabled.

`RsdFeatureEvidence` and the additive `feature_evidence` methods on
ServiceDescriptor/RsdHandshake describe a successful directory snapshot:
Advertised, NotAdvertised, MetadataUnavailable or ServiceMissing. Exact and
canonical-then-shim queries preserve the existing resolution order. An empty
legacy feature vector gives MetadataUnavailable; it cannot distinguish an
omitted list from a deliberately empty list. The old `supports_feature` keeps
its permissive behavior. NotAdvertised is not an explicit device-side
unsupported response; Unsupported errors remain a separate typed outcome.

`ConnectedDevice::initial_rsd_probe()` now borrows the connection owner's initial
observation without IO or retry. Its extensible state is `not_attempted`,
`available` or `failed`. Unattempted probes distinguish `tunnel_skipped` from
`direct_probe_feature_unavailable`; failures retain the CoreError reaching the
owner and offer an optional typed error class. Debug prints only fixed state,
skip reason and class, while explicit Rust callers can inspect the original
error/source. Diagnostic reporting must still follow the redaction policy.

`rsd()`/`into_rsd()` retain their legacy optional initial directory. A successful
directory can independently report ServiceMissing or MetadataUnavailable;
neither a skipped probe nor a failed request is an explicit device-side
unsupported response. Empty feature lists retain their permissive legacy
behavior. The added observation is historical: later on-demand retries do not
update it or the old cache, and it cannot certify current reachability or device
cleanup. Later service calls now retain their retry failure independently, as
described below; this getter does not become a mutable latest-probe report.

The private userspace-proxy dial adapter now retains native IO instead of
flattening it to XpcError::Tls text. RSD/H2 bootstrap and read/write adapters
also retain their typed sources as described below; the observation cannot
recover information already discarded by other adapters.
Existing public error enum definitions, retry counts, deadlines, bootstrap
frames and permissive feature rules remain unchanged. Read the initial state before interpreting a missing directory;
`rsd() == None` alone continues to conflate skipped and failed initial discovery.

On-demand RSD service calls keep the existing five attempts with 500 ms between
attempts, inner bootstrap budgets and fresh transports. When all completed
probes fail, the owner returns the final original CoreError/source instead of
the former `Unsupported("RSD handshake failed after retries")`. Earlier failures
are released; this is not an aggregate history. Owner preflight failures still
return immediately, and success stops retries. A skipped direct probe without
the local mdns feature returns Unsupported with that local prerequisite; it
does not prove a device-side rejection. No-tunnel behavior remains unchanged.

A missing canonical and shim service in a successfully obtained directory now
returns `CoreError::Xpc(XpcError::ServiceNotFound(requested_name))`, with class
`service_missing`, instead of Unsupported. Missing feature metadata and
unadvertised features keep their separate evidence semantics. The class concerns
that directory snapshot, without proving the service is absent for all routes
or currently unreachable. Both cached and newly discovered directories follow
the same rule; no service socket is opened for a lookup miss.

Migrate exhaustive callers that treated every former Unsupported as a device
capability decision: inspect the native class/source, keep a future fallback,
and do not retry side effects merely because discovery failed. Enum definitions,
signatures, successful frames/output, initial observations and cache behavior
are unchanged; returned failure variants and safe diagnostic correlation can
change. CLI screenshot fallback accepts the same specific missing service names
through the typed variant, retaining its legacy compatibility cases. XPC service
reconnector dials also retain native IO instead of a TLS string. Stronger
lifecycle/reconnection guarantees remain work.

RSD and RemoteXPC H2 setup, bootstrap, header/body reads, message writes and
cryptex transfer writes now wrap the original H2Error in the existing
`XpcError::Io` variant instead of formatting it into `Tls(String)`. A private
error payload preserves the fixed operation context and original source chain.
Native IO kinds remain intact; H2 protocol failures use InvalidData and retain
their typed protocol source. The wrapper's Debug exposes only fixed context;
explicit Display/source access still requires the diagnostic redaction policy.

Migrate callers matching every H2 failure as Tls or parsing Display: inspect
class/source and retain a fallback. Native timeout, permission and disconnect
failures can now report their actual category instead of protocol; Display gains
the existing IO variant's prefix, and safe diagnostic correlation can change.
Protocol/input strings that have no original source remain their existing Tls
variant. Public enums/signatures, wire frames, retry counts/budgets and pending
memory limits are unchanged. Host faults cover setup, bootstrap/write, header
and partial-body reads through all three connection receive methods, plus a
public XpcClient consumer. They establish local transport drop and retained
sources, without certifying safe reuse/retry after a partial operation or device
cleanup. Other lossy adapters and stream/binding boundaries remain pending.

## Machine output and partial results

Current CLI success formats and failure exits remain as recorded by R04.
`ios --error-format=json <command>` opts into one compact, newline-terminated
final failure object on stderr. `human` is the default and retains existing
safe diagnostic text. The option is global; put it before invalid arguments
so Clap can recognize the selection before its error. Unrecognized/invalid
format selection safely retains human errors. Recovery uses the same parser
without help/version early display; it never scans positional values or error
text to choose a format. Successful commands keep their existing stdout and
produce no final failure object. Help/version use normal Clap success output.

| JSON v1 field | Meaning |
| --- | --- |
| `schema_version` | Integer `1`; consumers should retain a version/fallback branch and ignore additional fields in this new extensible schema |
| `error.class` | Stable ErrorClass code from typed sources; typed CLI confirmation/state guards map to `precondition`, opaque strings remain `unknown` |
| `error.phase` | Parser's canonical root command or fixed `argument_parsing` / `help/version output`; no user command value is copied here |
| `error.diagnostic` | Opaque safe diagnostic/correlation; do not parse it for a category or assume correlation persists across processes |
| `error.exit_code` | Integer `1` for operation/help-output failure or `2` for argument failure; matches the original process status |
| `error.confirmation_required` | Boolean from a typed confirmation guard observed within 32 sources; false means no such guard was observed in that bound, and grants no confirmation or retry |
| `error.effects` | `not_started` for argument parsing before dispatch, `unknown` for operation/output failures; failure alone does not establish rollback, retry safety or completed cleanup |

Argument failures carry `invalid_input` (or an underlying IO class for a typed
Clap IO failure). IO on a closed result/help pipe may classify as `disconnected`;
that code alone does not identify a device as the disconnected object. Stderr
reporting remains best effort after failure: a closed stderr retains the original
nonzero status and may prevent delivery of the envelope. Ordinary progress,
tracing and stream callbacks may still emit other stderr records; this option
does not yet make the entire stderr stream JSON. Partial stdout may already
exist. Existing HTTP and C statuses retain their shapes. Additive HTTP headers
and C/Python category queries are described below; post-upgrade frames and
remaining adapters still need their own acceptance.

Both the human confirmation hint and machine field now inspect at most 32
sources, matching the classifier's bounded interpretation. Normal guard errors
and 24-layer contexts retain the hint. A guard hidden beyond the bound yields
no hint and Unknown classification; callers must not infer permission from its
absence. Previously the hint scan could continue indefinitely through a cyclic
source. This bounded diagnostic change does not alter the operation's actual
confirmation check or authorize any device action.
Existing safe diagnostic messages remain useful for correlation but must not
be parsed as stable natural-language categories.

The current [stream framing inventory](validation/cli-stream-contract-2026-10-04.md)
records ordinary stderr progress/callbacks and partial-operation semantics.
Eleven invalid timeout inputs classify invalid_input; empty accessibility
traversal expiry is a source-free timeout. A private CLI state marker maps to
precondition within the same 32-source walk as confirmation, with no force hint
or confirmation_required flag. The general Core classifier remains Unknown for
that CLI-only type. The current missing-crash-deadline branch is unreachable by
construction; its mapping is an adapter fixture/source review, not a device
fault. See [owner-guard acceptance](validation/cli-owner-guards-2026-10-04.md).
A partial stream or an output error can follow completed operations;
no rollback, automatic retry or device cleanup acknowledgement is promised.
L01 retains the stronger lifecycle/deadline acceptance.

Pcap/FPS native read failures, XCTest/notification/CrashReport Elapsed and
OS Trace/crash/power signal errors now retain typed sources through the CLI.
Notification ProxyDeath events use the existing typed closed-proxy error.
Companion/OS Trace issued deadline decisions classify timeout with no invented
Elapsed or OS cause. Existing successful operator stops and count/capture-window
policy remain. JSON WebInspector shell suppresses interactive prompts and its
EOF presentation newline. Returned category/Display/source/correlation migrate;
failure exits and v1 schema/effects stay. The
[stream acceptance and framing inventory](validation/cli-stream-contract-2026-10-04.md)
distinguishes NDJSON, successive pretty documents, aggregate summaries and
per-record partial failure. It records source-reviewed guards separately from
owned reader/channel/timer/signal tests; no device cleanup/retry is certified.

`ios syslog` success stdout is newline-delimited JSON by default: one compact
object per matching relay message, retaining the existing parsed/raw fields.
Embedded newlines and quotes are escaped inside that object. `--no-json` retains
raw bytes; `--parse` retains its formatted line and added newline, including
existing extra newlines when the raw message already ends with one. Each emitted
record now checks both write and flush. An output failure can leave a partial
final record, so consumers must validate framing and inspect the process status.

Relay read errors, including EOF before a chosen stopping condition, now return
through the normal final CLI failure path with exit 1, rather than printing a
stream diagnostic and returning exit 0. Native IO/source/class are preserved;
UnexpectedEof remains `io` under the classifier's existing rule. Unterminated
relay messages are not emitted, and previously emitted records remain stdout.
The final failure uses safe human text or the opt-in JSON stderr envelope with
phase `syslog` and effects `unknown`. The existing streaming progress line stays
on stderr; its write/flush now returns IO failure instead of panicking.

The existing `--count` matching-record cap and user-selected `--timeout` capture
window still end successfully. That window starts after service setup; its
expiry is a chosen collection stop, distinct from a native reader TimedOut.
An unbounded listener gains no new idle timeout. Controlled host readers/outputs
exercise partial JSON, EOF, native failures, count/filter stops and output/flush
failure; an owned idle duplex observes capture expiry and peer EOF. This proves
local stream drop, without certifying physical relay/signal behavior, connection
reuse, rollback or device cleanup. Migrate scripts that relied on relay failure
as exit 0 to preserve valid prior records and handle the nonzero result. Other
streams and binding/lifecycle boundaries remain pending.

The relay's 1 MiB limit applies to message bytes before the NUL delimiter and
before vis/UTF-8 decoding. The cumulative check now also covers the final chunk
containing NUL, before copying it into the message buffer. An exact-limit record
is accepted; one extra byte is InvalidData/`protocol`, whether or not the final
chunk contains its terminator. Previously a terminated final chunk could bypass
the check. CLI syslog returns this error with prior NDJSON retained and no
oversized record emitted. Discard the reader after a size error; this fix does
not establish error/cancellation-safe reuse or a limit on the serialized JSON's
size after decoding/escaping. L01 retains connection invalidation/reuse work.

`apps install`, `upgrade`, `archive`, `restore`, `kill`, `signal`, `launch` and
`uninstall` now honor global JSON mode instead of printing human status lines.
Each emits one compact newline-terminated result after its existing service and,
where present, staging-cleanup checks. New records contain integer
`schema_version: 1`, an `operation` tag and the fields below. Preserve a version
and unknown-operation fallback, and ignore future extra fields. `--no-json`
retains the prior human bytes. Existing list/show/install-record/processes/roots/
spawn/icons/monitor JSON shapes stay unchanged.

| Operation | New result fields |
| --- | --- |
| `install` | `path` and boolean `streaming` |
| `upgrade` | `path` |
| `archive`, `restore`, `uninstall` | `bundle_id` |
| `kill` | unsigned `pid` |
| `signal` | unsigned `pid`, signed `signal` |
| `launch` | `bundle_id`, `pid` (null if the existing service result supplies no PID) |
| `pkill` | `pattern`, `signal`, `matched_count`, `completed_count`, `complete`; per-process records additionally contain `process: {pid, name}` |

`apps pkill` JSON is NDJSON. After each successful signal call it emits a record
with `complete: false`, that process and the number of calls completed so far.
After all matching calls succeed it emits one summary with `complete: true` and
no process field. No matches yields only the zero-count complete summary and no
signal calls. Counts describe successful signal requests, not observed process
termination. ASCII case-insensitive matching, process order and per-process
connections are unchanged. The human summary keeps its old wording, including
"Killed" for a selected signal other than SIGKILL.

A request failure preserves prior records and emits no complete summary. An
output failure also stops before the next process and may leave a partial final
record. Final errors retain their native source, exit 1 and unknown effects;
there is no rollback or automatic retry guarantee. Even a valid result can be
followed by a flush error. Result data intentionally includes requested paths,
bundle identifiers and process names; it is distinct from safe diagnostics.
Install/upgrade can succeed on the device before later cleanup/output failure,
so absence of a success record does not establish absence of device effects.
Streaming-install stderr uses the checked observer contract below.

Application fallback accepts the existing typed ServiceNotFound only for the
canonical appservice and its shim, retaining old Unsupported compatibility
cases. Native timeout/permission/refusal/broken-pipe or another missing service
does not select fallback. Ten DTX adapters now retain sources through fixed
anyhow contexts; install-record/monitor wrappers retain Tokio Elapsed, and
signal prerequisites keep the original routing error under context. Error enum
definitions, deadlines, successful wire calls and SIGKILL-only Instruments
fallback are unchanged. Returned error source/class, Display context and safe
correlation can change. The output/ordered-call fixtures use owned callbacks;
they do not certify physical mutation, DTX fallback or complete CLI service
startup workflows. Other JSON commands, callbacks and binding/lifecycle boundaries
remain pending.

`apps::install_ipa_with_checked_progress` adds an optional
`CheckedProgressCallback` returning `io::Result<()>`. A callback error immediately
returns the existing `ZipConduitError::Io` with the original IO source and native
class, without reading another reply or reporting completion. The old
`install_ipa` and void `zipconduit::ProgressCallback` keep their signatures and
callback/completion behavior through the same transfer implementation. Error
enums, requests, progress fields, remote-error precedence and successful
transfer bytes are unchanged. Completion replies do not invoke either observer.
This API accepts returned observer errors; it does not catch callback panics.

CLI streaming install now checks the existing startup stderr line and each
progress write/flush. Progress stays stderr text with the same diagnostic ID
projection; global JSON success output keeps its versioned install result on
stdout. Startup output failure returns before payload transfer but after service
connection. Progress observer failure occurs after payload transfer and can
leave earlier lines or a partial last line; even a full line can precede a flush
error. No success stdout result is emitted on these failures. Final errors follow
the safe CLI exit-1 path, phase apps, with the native class and unknown effects.
Final stderr reporting remains best effort if stderr itself is unavailable.
A BrokenPipe/disconnected category for output does not identify a disconnected
device and supplies no retry/rollback authority.

After observer failure, discard the service connection. The core API borrows
its stream and does not close/join it; CLI dispatch returns through its existing
owner-drop path. Neither path certifies cancellation completion or Apple cleanup.
Owned tiny-IPAs/services drive the entire public install function, showing no
later completion read after callback failure, unchanged legacy/checked/no-observer
transfer bytes and completion behavior, and original remote/plist/IO failures
after progress. CLI writer fixtures exercise the shared checked writer and actual
safe error renderer; an external consumer keeps exhaustive error matching and
old/new callback signatures. These are host source/output/transfer fixtures,
not a paired-TLS CLI process or physical installation/teardown. Other callbacks,
JSON commands, bindings, other source adapters and L01/L03 remain pending.

Streaming install's local archive-open, entry-open and entry-read adapters now
retain the original `zip::result::ZipError` or `io::Error` under the existing
`ZipConduitError::Io`. Previously they flattened every failure into `Zip(String)`.
No enum variant or function signature changes; manually constructed legacy
`Zip(String)` still classifies as invalid input. Consumers matching `Zip` for
decoder failures must migrate to `Io` and typed cause inspection, or use
`classify_error`. Returned Display prefixes and safe diagnostic correlations
can change. Class selection uses typed variants/kinds, never error words.

| Original decoder cause | Returned kind / stable class |
| --- | --- |
| InvalidArchive, UnsupportedArchive, FileNotFound, InvalidPassword | InvalidInput / invalid_input; these describe the local package, without device support or service-missing inference |
| ZipError::Io or entry-read IO | Original native kind/class, including Timeout, PermissionDenied and disconnected; InvalidData becomes protocol, UnexpectedEof/Interrupted/Other remain io |
| Future non-exhaustive ZIP reason | Other / generic io fallback, with the original typed cause retained |

Fixed owner context is `open IPA ZIP archive`, `open IPA ZIP entry` or
`read IPA ZIP entry`. Owner/new wrapper Debug and CLI machine diagnostics keep
private data hidden; native Rust cause access intentionally retains original
payloads. Entry names are no longer inserted into read-failure context. A real
stored-entry checksum mismatch emits native InvalidData and now classifies as
protocol instead of the former flattened invalid_input. Locked zip 2.4.2 checks
local headers during archive construction, so a damaged header is an archive
phase error; a password-required metadata entry reaches the entry phase.

Both public install entries finish local extraction before writing InitTransfer
or payload, flushing, reading replies or invoking observers. Owned fixtures
verify these boundaries on malformed archive/header, password-required metadata
and actual CRC failures. CLI rendering of a real local decode failure keeps
phase apps, exit field 1 and unknown effects; CLI service setup happens before
local decoding, so this envelope does not claim the whole command never started.
Valid/empty archive entries retain names, order, contents and directory flags;
successful transfer bytes/keys and whole-file/in-memory extraction are unchanged.
These host faults do not prove physical installation, encrypted-payload support,
cleanup, stream reuse or new decompression/resource budgets. Other A02 source,
JSON/callback/binding work and L01/L03/L04/Q04 conditions remain pending.

## Connection and pairing native sources

Twenty connection/pairing adapters now retain original native objects instead
of flattened Other or Protocol strings, under existing CoreError::Io and a
private fixed-context payload. Four task-join paths retain JoinError, including
observed cancellation or blocking-worker panic. Two verification and two PSK
paths retain the original PairingError/PskTlsError, native IO kinds and precise
source categories. A PSK backend-unsupported error also retains its OpenSSL
stack as a standard source. Three local credential adapters retain typed host
identity/hex failures with InvalidInput. Two remote Base64 decoders, two received
public-key slice conversions and tunnel IPv6 parsing retain their native objects
with InvalidData/protocol. Four listener AEAD adapters retain the original AEAD
error: encryption preserves its unknown category and response authentication
failure is explicitly protocol, without guessing a permission rejection.

Private context construction takes a bounded category snapshot from the native
source. Classification uses that snapshot after any precise outer IO kind,
including Unknown; adding an IO container alone does not invent IO evidence
for an opaque crypto/backend/panic cause. Standard source inspection retains
the original object, even when that object's own older fields are already
strings. Construction reads at most the existing 32-source bound and does not
format text; subsequent classification of the context reads the cached value.
Wrapper Debug exposes fixed phase/category only, while original Rust Display
and source access can reveal private material. The native AEAD source requires
the existing dependency's std feature, now enabled with unchanged version and
crypto parameters; this workspace already requires std/Tokio.

Three other adapters retain original Tokio Elapsed under TimedOut: PSK dial,
RemotePairing control-channel dial and tunnel endpoint dial budgets. Values,
clock semantics, success and failure schedule remain. Fixed contexts replace
host/port-bearing timer text, and native elapsed text follows the context.
Returned Other/Protocol become Io on the specified native paths; existing
exhaustive enums and public signatures remain. Category can change from unknown
to native timeout/disconnected/unsupported/cancelled, local invalid_input or
malformed remote protocol. Opaque crypto/backend/panic stays unknown. Safe
correlations/Display prefixes change, while existing machine fields/statuses/
exit codes and success output remain.

Owned host fixtures cover original IO/opaque categories, bounded cyclic sources,
real async JoinError cancellation and blocking JoinError panic, actual PSK dial
refusal and closed-stream handshake failure, local in-memory credential loaders,
both Base64 extraction functions, received key-length conversion and the actual
RemotePairing listener request/response over loopback TCP. The listener fixture
tests malformed Base64, failed authentication and valid encrypted port success;
it joins its peer and drops the local control channel. Existing endpoint tests
now verify original address/Elapsed objects rather than only text. No worker
fault is injected into an entire discovery/device process, no actual blocking
task is claimed cancellable after it starts, and the two other dial timers are
source-reviewed rather than network timeout-injected. Not every listener stage
is independently fault-injected. No real credential directory, pairing/device
operation, trust-policy change or Apple request occurs.

Seven inner pairing adapters now use the same retained-cause context through
existing PairingError::Io: private-seed slice conversion, HKDF expansion, both
cipher key initializations, payload encryption/decryption and device-info OPACK
encoding. Seed/key/output length rejection is InvalidInput; received payload
authentication failure is InvalidData/protocol. Opaque encryption remains
unknown, and OPACK preserves its typed category. Explicit source inspection
reaches the original TryFromSliceError, hkdf::InvalidLength, cipher InvalidLength,
AEAD Error or OpackError through IO; wrapper Debug omits private payloads.
Nested credential loaders retain both the PairingError and its native cause.
The public manual-pairing transport now retains these four PairingError paths,
including M6 verification, through the outer contract below. This preserves
already-retained native chains; it does not reconstruct earlier legacy strings
or establish a successful full device pairing workflow.

Affected Crypto(String)/Protocol(String) failures now return Io, with changed
Display prefixes/correlation and more precise categories. Match class/source
rather than old variants or message text for these internal failures. PairingError
is inside a crate-private module; its three variants remain, as do external APIs.
The legacy Protocol variant has a targeted dead-code allowance after its sole
OPACK producer now retains Io, with other warnings still checked. HKDF enables
its existing dependency's std feature so InvalidLength implements standard Error; versions,
lockfile and service feature names/defaults/groups stay unchanged. Successful
derivation lengths, salts/info, nonces, AEAD, signing, wire fields and optional
M6 information handling are unchanged. No extra verification or trust behavior
is introduced. PairingError's classifier stays tunnel-gated because the pairing
module/type itself only exists with tunnel; default builds have no such type.
Ed25519 signing is infallible here, and SRP proof verification returns bool,
so neither supplies a discarded native signature-validation error.

Owned actual constructor, M6 authentication and device-info encoding faults
verify sources, safe Debug and no partial success. Fixed HKDF values independently
computed with Python hmac and authenticated M5 contents/signature check preserved
success. Native HKDF/key-length errors and bounded in-place AEAD errors pass
through the production context adapter; these do not inject failure into the
fixed 32-byte production HKDF/key initialization or Vec encryption call.

Failure does not authorize retry or prove remote cleanup. Dropping an awaiting
caller still does not stop already-running blocking work. Other legacy string
outcomes, CLI/HTTP/stream/binding boundaries and L01/L03/device conditions remain
pending; the new source adapters do not add invalidation or reuse rules.

## Manual-pairing transport sources

Sixteen adapters retain original RSD/bootstrap/codec XpcError, H2Error,
PairingError, header TryFromSliceError, AEAD Error and serde_json::Error objects
through existing PairingTransportError::Io and the bounded fixed-context
snapshot. Native IO kinds remain, H2 protocol maps to InvalidData, received
authentication/JSON/header faults are protocol, and opaque crypto/encryption
stays unknown. Named XPC service misses/closed states retain service_missing/
disconnected rather than becoming generic protocol text. Four inner crypto
stages retain PairingError and its own source, including the inner M6 native
authentication error. Pre-existing string-only fields remain string-only.

The service dial wrapper also retains original Tokio Elapsed under TimedOut
with fixed context instead of address-bearing text; the existing timeout value
and schedule remain. A short private header-length input now yields the native
slice error under InvalidData rather than panicking on indexing. The actual
H2 reader still requests 24 header bytes. Existing body-length limit diagnostics
remain PairingTransportError::Xpc: the shared checked-length helper currently
returns String, not a native Error. No cause is fabricated for it or for legacy
created failure/proof strings; those typed guard/lifecycle details remain work.

Affected Xpc/Rsd/Crypto failures migrate to existing Io. All nine exhaustive
public variants, root function signature/future and credential fields remain.
Class/source inspection replaces affected variant or Display parsing; safe
correlations/Display prefixes can change. Requests, sequence numbers/nonces,
salts/derivation, signing, stream selection/bootstrap, body limits, trust, retry
schedule, remote-error handling and optional/successful unlock-key results stay.
No dependency/lockfile/service-feature or binding/CLI-schema changes.

Owned H2 streams test actual closed write, header/body read, decoder and
bootstrap faults, and the actual unlock request/response with AEAD/JSON failures
and both returned key/optional-empty success. Peers bound reads, verify request
ID/sequence/plaintext, shutdown and join; the local framer is dropped. A real
RSD refusal uses an owned non-listening IPv6 socket; real Elapsed uses an expired
pending future. Typed adapters cover native kinds, named XPC and opaque sources,
and actual inner M6/seed/SRP errors. The root API future is externally type-checked
without polling. This does not inject every stage of the public full SRP flow,
the existing 15-second network dial timeout, huge encoder-size or Vec encryption failure;
the encryption adapter uses a typed native-error fixture. No device, new trust,
real credential directory or Apple request is involved. Local drop/join does not
prove remote cleanup, rollback, invalidation/reuse or safe retry.

## Default lockdown native sources

Default builds now retain request/response plist, ProductVersion semver,
certificate/key PEM, rustls client-auth, DNS-name and TLS-handshake errors through
existing LockdownError::Io. The common private source/context adapter is always
compiled; cached Unknown and the 32-source construction bound remain. Local
serialization/PEM/name failures are InvalidInput; received plist/version faults
are InvalidData/protocol. Opaque rustls configuration errors remain unknown.
TLS preserves original IO kind and backend payload. These categories express
typed causes or the explicit input/response role, without matching prose.

PairRecord::load/load_from_path now return existing PairRecordError::Read for
missing files and malformed local plists. Missing-file failures retain native
NotFound IO/OS code; malformed plists retain plist::Error under InvalidInput.
Both keep their previous `not_found`/`invalid_input` classes. The Read path field
and native sources remain explicitly inspectable; Display/Debug still omit
credential paths/data. All three variants of both public error enums remain.
Migrate affected NotFound/Parse/Protocol matches to class/source observations;
Display prefixes and safe correlations can change. Original source/Display
access remains private data, while context Debug omits payloads.

The explicit path remains authoritative and the identifier argument remains
accepted. The file bound/Zeroizing key and buffer, frame budget, ProductVersion
normalization, selectors and successful output remain. TLS provider/verifier,
server names and parameters stay; no timeout, retry or trust policy is added.
Created validation failures (empty certificate, missing key/fields, disabled SSL,
read-ahead guard) retain their existing Protocol outcomes without invented causes.

Owned host fixtures cover actual malformed/valid serializers and files, native
PEM/DER/name errors, closed TLS write and malformed peer/backend, successful
duplex TLS/application IO, product-version normalization and root API consumers.
The internally constructed version client and unpolled root constructor do not
prove full lockdown session/device pairing. Shared-context default tests cover
native kinds/Unknown, bounded cyclic sources and actual Join flags. Native
classification does not authorize retry or certify running-worker cancellation,
remote cleanup or stream reuse. Supervised OpenSSL/record-save, other codec/
created outcome/stream/binding and L01/L03/A03 conditions remain. Evidence:
[default lockdown source validation](validation/default-lockdown-sources-2026-10-02.md).

## Supervised pairing native sources

With supervised-pair, 51 OpenSSL adapters, record plist serialization and file
writing retain their original native sources through existing LockdownError::Io
and bounded fixed context. Opaque backend/key/certificate/signing stacks stay
unknown, local PKCS12 parsing/unlocking is invalid_input, malformed received
device public keys are protocol, and file errors preserve native IO kinds.
Original ErrorStack codes/entries survive. No permission or unsupported class
is inferred from backend text. Missing certificate/key and created device
response validation retain their existing Protocol outcomes.

An interior NUL password now returns InvalidInput with the original NulError
instead of panicking inside the dependency's CString unwrap. Validation follows
DER parsing and precedes parse2; valid passwords keep their existing path. The
NulError object owns original password bytes, so source/Debug access is sensitive.
Wrapper Debug omits these bytes; raw Rust Display/source access remains private
data. Affected Protocol-to-Io, category/Display/correlation migrations are
explicit; the exhaustive error enum, root functions and FullPairRecord fields
stay. Consumers should use class/source rather than old message/variant parsing.

Certificate generation parameters, PKCS7 BINARY, Pair request fields/order,
private-key stripping, results, file format and owner-only publication/replacement
policy remain. No timeout, retry, trust or lifecycle policy is added. Owned
fixtures verify actual P12/NUL/public-key/backend/signing/file failures and a
complete root exchange with generated certificate/signature checks and signed
binary challenge. Test-only PKCS7 NOVERIFY/NOINTERN uses a known synthetic signer
and verifies the signature without establishing chain trust. No physical
supervised device/enrollment/Trust or remote cleanup/reuse is certified; not
every backend or save stage is fault-injected. MCInstall's analogous P12 boundary
has subsequent coverage below; other sources/streams/bindings and L01/L03/A03
remain. Evidence:
[supervised pairing source validation](validation/supervised-pairing-sources-2026-10-02.md).

## Archive and backup metadata sources

The private archive decoder retains plist::Error under ArchiveError::Plist.
ArchiveError has no root export or public signature exposure. All 18 anonymous
Manifest adapters and the SQLite/crypto/MBDB helpers retain their original
objects through existing public Mobilebackup2Error::Io and private fixed context.
The four public error variants, root APIs/result fields and feature definitions
stay. Metadata integer/path/validation/UTF-8 errors remain protocol; archive depth
now survives as resource_limit. Crypto IO preserves native kind/source/OS code,
while the opaque Crypto marker now classifies as unknown. Subsequent inner
crypto/MBDB retention is described below; previously discarded objects cannot be
reconstructed from older string errors.

With backup2-manifest, native SQLite codes supply permission/authorization denied,
busy/locked/constraint conflict, memory/disk/size resource_limit, read-only
precondition, corrupt/not-a-database protocol and system io categories. Unknown
codes remain unknown; CannotOpen does not infer NotFound/permission and interrupt
does not acknowledge cancellation. Column type/range/UTF-8 metadata errors remain
protocol; other source chains are traversable. No code or message authorizes retry.

Migrate affected Protocol matches to class/source; fixed phase labels replace
adapter-created path/identifier prose. Display prefixes, categories and safe
correlations can change. Native sources may retain SQL/path metadata and original
FromUtf8Error bytes; raw source/Debug access is private data. Context/outer Debug
and machine output remain safe. SQL, limits/depth, format selection, paths,
encryption/Zeroizing, transactions/staging/rollback and successful output stay.
Owned public list/extract and actual SQLite backend fixtures establish host
evidence; test-only authorizer/BLOB limits do not change production policy. Physical
devices, existing backups, every backend/architecture stage and remote cleanup
remain unverified. Evidence:
[backup metadata source validation](validation/backup-metadata-sources-2026-10-02.md).

## Encrypted backup and legacy index inner sources

Twelve private crypto adapters retain original AES-KW/cipher/slice/integer errors
under the existing CryptoError::Io and fixed context. Malformed keybag/class fields
and failed key unwrap use protocol; opaque initialization/count errors retain
unknown. Native CBC read/write kinds survive. AES-KW integrity failure does not
distinguish an incorrect password from corrupted wrapped data. Enabling the
resolved optional aes-kw 0.2.1 std feature makes its Error source available; dependency
version, lockfile, public features and algorithms remain.

Three private MBDB conversions retain FromUtf8Error/TryFromIntError. MbdbError is
now a private Clone enum with native sources and safe outer Debug; its unused
private Eq/PartialEq traits are removed. Public Mobilebackup2Error's four variants,
root signatures and results stay. Native UTF-8 errors retain private metadata bytes;
explicit raw source/Debug access must follow the redaction boundary.

Affected Display/correlations change; callers inspect class/source rather than
password or field prose. Existing validation-created guards have no invented
backend cause. Bounds, sentinel/null encoding, parser/serializer order, encryption
parameters/Zeroizing and successful operations stay. Owned legacy and modern
encrypted fixtures establish host rejection/source and success behavior, not
physical backup, every cipher/allocator/architecture fault or remote cleanup.
Evidence: [inner backup source validation](validation/backup-inner-sources-2026-10-02.md).

## Backup filesystem and archive integer sources

Five host path adapters retain native StripPrefixError under existing
Mobilebackup2Error::Io with InvalidData/protocol. Missing required backup metadata
files now retain native IO as NotFound instead of Protocol prose, including the
nested OS code. This is host not_found, not device_not_found or service_missing.
Inspect Error::source for the original IO; the context's raw_os_error is not the
original code. Blocking-task JoinError uses fixed context and its native category
snapshot: panic is unknown, cancelled Join remains cancelled. Caller cancellation
or that category does not certify termination of running blocking work.

The private ArchiveError adds native Integer(TryFromIntError); unsigned archive
values above i64::MAX remain protocol, with an original source. Its other variants
and public root errors/APIs/features stay. Migrate affected Protocol/immediate
get_ref matches to class/source traversal. Fixed phases, Display/correlations and
missing-file/panic categories can change. Context/outer Debug omit private native
payloads; explicit raw access remains private, and runtime panic hooks are unchanged.
Path/symlink/platform aliases, caps, incremental/staging/copy policy, scheduling,
decoding/depth and optional/raw fallbacks stay. Actual owned filesystem/worker/
binary-archive/public reader fixtures establish host evidence, not every OS or
architecture, running-worker termination, physical backup or remote cleanup.
Evidence: [backup host source validation](validation/backup-host-sources-2026-10-02.md).

## OPACK and XPC native codec, identity and deadline sources

Thirty-two native UTF-8/integer/slice/Elapsed/environment adapters retain original
errors. Public XpcError's four and H2Error's two variants stay; affected failures
use existing Io with fixed context. Private OPACK InvalidUtf8 gains its Utf8Error
payload, preserving protocol. The private body-length helper returns IO, retaining
declared u64 and native conversion source; existing admission limits/order stay.
Manual pairing retains that nested IO as protocol. DProxy still projects it to
String at its existing diagnostic boundary; structured observations remain pending.

Local XPC encoding conversions now classify invalid_input rather than protocol;
received conversions/UTF-8 remain protocol. Non-Unix hostname NotPresent keeps
not_found/native VarError, while NotUnicode changes from not_found to protocol.
Empty-value guards and Unix hostname policy stay. Migrate affected Tls/Protocol/
Xpc and immediate get_ref matches to class/source traversal. Fixed phases replace
endpoint/field prose; Display/correlations may change. Outer/context Debug stay
safe; explicit native source/Display/Debug, including OsString, remains private.

Fatal H2 count conversion still poisons the connection before state writes;
the returned error retains its native cause, while later cached protocol guards
do not reconstruct it. Other windows/frames/buffering/stream behavior stay.
Shared timeout mapping retains real Elapsed with the same existing stage budgets,
polling, reconnect ownership and command cleanup. Local future Drop is not remote
cancellation or Apple teardown. No new L01 budget/retry/cleanup contract.

Owned codecs/private flow helpers, short-stage timers, hostname/OS-value and
external root consumers establish host evidence. Actual Windows/macOS identity,
production fifteen-second TCP deadlines, narrow-host conversions and physical
devices remain unverified. Wire/limits/success/features/root exports and optional
M6/UID/DTX/instruments compatibility fallbacks stay. Evidence:
[XPC native source validation](validation/xpc-native-sources-2026-10-02.md).

## Media, HID and packet capture native sources

Seventeen native address/integer/Elapsed/UTF-8 adapters retain original sources.
HID close uses existing Xpc(Io), changing protocol to timeout. Display's native
timers use Io rather than its unit Timeout; the source-free remaining-zero marker
stays. Local media IP parsing becomes invalid_input, received configuration
integers remain protocol. Pcap find-IP expiry becomes timeout, metadata UTF-8
remains protocol with owned original bytes, and pcap/pcapng/plist encoding lengths
become invalid_input. Bluetooth host block-add overflow is an explicit input
guard without an invented native cause.

Public error families/signatures/exports/features stay. Migrate affected Protocol/
Timeout/get_ref matches to class/source; fixed phase/Display/correlations can
change. Native metadata/raw source access stays private, outer/context Debug safe.
HID state still clears after send success, without waiting for a reply. Display
still marks its channel before I/O and consumes service/requests task abort before
stop awaits; a subsequent no-op is not remote cleanup proof. Existing deadlines,
wire/limits/RTP/optional fields/stop EOF compatibility and success stay.

Actual owned XPC bootstrap with writes gated before request bytes, UDP timers,
metadata frames and private conversion helpers establish host evidence. Controlled
gate reopening proves local state/send success, not partial-send reuse or device
delivery. Physical media/capture, task joins and every allocator/architecture/
platform stage remain unverified; L01/streams and other A02 work remain. Evidence:
[media and capture source validation](validation/media-capture-sources-2026-10-03.md).

## usbmux native codec, connection and deadline sources

Request serialization and received plist XML/schema faults retain original
plist::Error through existing MuxError::Io and private fixed context. Local
encoding is invalid_input; received codec failures remain protocol. TCP
connector failure retains native IO kind/object/source OS code; deadline expiry
retains the actual Tokio Elapsed as timeout. ConnectionRefused remains generic
io, and UnexpectedEof is not inferred to be device disconnect. CoreError::Mux
keeps the nested typed chain. All three MuxError variants and root client APIs stay.

Affected codec Protocol matches become Io; request encoding category changes
to invalid_input. TCP phase text is fixed, replacing interpolated endpoint/
duration messages. Display prefixes and safe correlations can change; use
kind/class/source observations. Context/outer Debug omit private payloads;
explicit native Display/source access remains private data. Wire framing,
receive bounds, created length guards, tags, request fields, flush, ten-second
TCP timeout and Unix policy remain. Owned codec/TCP/Elapsed and public BUID
fault/success fixtures establish host evidence only; external root connect is
type-checked without polling. They do not certify physical devices, production
network black holes, cleanup/reuse or safe retry. Archive/integer/guard/stream/
binding and lifecycle work remain. Evidence:
[usbmux source validation](validation/usbmux-sources-2026-10-02.md).

## MCInstall native sources and explicit P12 outcomes

Five OpenSSL adapters retain original ErrorStack objects under existing
McInstallError::Io and bounded private fixed context. Local PKCS12 DER/password
failures are invalid_input; certificate DER, signing-stack allocation and PKCS7
signing/encoding failures remain unknown. The remove-profile identifier encoder
propagates original plist::Error as existing Plist instead of Protocol(String).
Its class remains protocol. No backend text is used for classification.

Interior NUL passwords return invalid_input with the original NulError after
DER parsing and before the dependency's parse2 unwrap. This native object owns
password bytes; explicit source/Debug access is private data. Context Debug
omits payloads. Missing parsed certificate/key predicates return InvalidInput
IO without inventing an underlying backend cause. Without supervised-pair,
both public P12 entry points retain an explicit CoreError::Unsupported feature
decision through Io and perform no writes. Legacy manually constructed Crypto
remains unknown; PasscodeSet remains precondition.

All five public error variants, root exports/signatures and feature definitions
stay. Affected Crypto-to-Io and Protocol-to-Plist matches should migrate to
class/source observations; Display prefixes, correlations and categories can
change. Existing PKCS7 BINARY, request keys/order, challenge handling, zeroization,
ACK gates and best-effort profile-list fallback remain. No timeout, retry, trust
or lifecycle policy is added. Owned fixtures cover actual faults, each ACK gate
and a complete public silent-install exchange with known-signer verification;
test-only NOVERIFY/NOINTERN does not establish chain trust. Physical supervision,
profile installation, remote cleanup/reuse, every backend/allocator/encoder stage
and memory-lifecycle guarantees remain unverified. Existing profile metadata
integer conversion and other codec/guard/stream/binding work remain. Evidence:
[MCInstall source validation](validation/mcinstall-sources-2026-10-02.md).

## Deactivation fallback and activation HTTP sources

`ConnectedDevice::deactivate` attempts mobileactivationd and, on failure, one
legacy lockdown fallback as before. Both failed attempts now return existing
CoreError::Io with additive private-field `DeactivationFallbackError`, exported
at root and device with feature mobileactivation. Borrow the observation through
the IO payload's `get_ref().downcast_ref::<DeactivationFallbackError>()`.
`service_error()` and `fallback_error()` retain their original CoreError objects;
`service_class()` and `fallback_class()` are bounded classifications taken before
wrapping. Its standard source is the final fallback error. The final category
controls classification even when unknown; an earlier precise failure cannot
override it or turn opaque Other text into a guessed category. Safe Debug shows
categories, not payloads. Explicit Display/source access can expose private data.

Modern service errors use the existing source-preserving activation adapter.
Combined failure previously became Other(String); a failure establishing the
fallback connection previously dropped the earlier cause. Both now retain the
two observations. Primary success still skips fallback construction, successful
fallback still returns success without an earlier-failure observation, and no
additional retry is introduced. Returned variants/Display and safe correlation
can change. No success or category certifies rollback, stream reuse or cleanup.

ActivationHttpClient preserves original reqwest errors from build/send/chunk
through private fixed-phase IO payloads. Native timeout/builder flags and typed
underlying sources participate in classification; otherwise the IO fallback
applies. Endpoint parse errors keep their original source; HTTPS, credentials
and missing custom-endpoint opt-in remain typed local InvalidInput rejections.
These seven former Http-string paths migrate to MobileActivationError::Io:
native request timeout changes service_failure to timeout, endpoint rejection
changes service_failure to invalid_input, and other HTTP causes use their typed
chain. Status and request/response body-budget rejections remain Http/service_failure.
Client Debug hashes both URLs; original Rust cause access can reveal private
queries. Existing HTTPS verification, exact official endpoints, explicit custom
opt-in, redirect policy, timeout values, body limits, wire headers and success
response fields remain unchanged.

Owned loopback fixtures cover refusal, stalled TLS handshake, verified HTTPS
body truncation/timeout, complete response/status rejection, and rejection of
an untrusted test certificate. Positive TLS fixtures install an ephemeral root
only in the private test client, with verification enabled; they do not add a
production custom-CA interface. Interrupted response reads return no successful
ActivationHttpResponse; a request may already have been sent and remote effects
remain unknown. Partial read-buffer zeroization, borrowed-client invalidation,
ordered cleanup and real Apple/device acceptance remain separate A02/L01/L03
conditions. The production CLI renderer retains exit 1, activation phase and
unknown effects, and hides private fixture values. Host tests do not execute
paired ConnectedDevice routing, TLS lockdown setup or physical deactivation.

## Symbols, activation and pasteboard sources

The following adapters preserve original typed causes instead of formatting
them into strings. Public exhaustive enums and function signatures remain
unchanged; affected returned variants, Display prefixes and safe correlations
can change. Consumers should use typed causes or classify_error instead of
matching the former flattened variant or diagnostic words.

| Boundary | Returned migration / retained source |
| --- | --- |
| RemoteFetchSymbolsClient H2 connect/bootstrap/catalog/file operations and XPC encoding/buffering/decoding | FetchSymbolsError::Protocol(String) becomes existing Io, with fixed phase and original H2Error/XpcError; native IO retains kind/class, genuine protocol failures stay protocol |
| CLI remote symbols connection | Opaque anyhow string becomes original FetchSymbolsError chain under fixed connect remote symbols context |
| ConnectedDevice modern activation record application | CoreError::Other(String) becomes existing Io with fixed mobile activation context and original MobileActivationError; native kinds and typed Timeout/Http/Protocol/Plist causes survive |
| Pasteboard notification snapshot connection | PasteboardError::Protocol(String) becomes existing Xpc/Io with fixed connect pasteboard XPC context and original CoreError; precise native, named service-missing and explicit unsupported sources remain distinct |

Modern activation's legacy-wrapper routing, service command and later lockdown
acknowledgement are unchanged. The CLI passes its original CoreError through the
existing absolute-deadline wrapper. Actual expiry now retains Tokio Elapsed;
observed Ctrl-C failure retains OperationCancelled, while signal-registration
failure keeps native IO under context. Timeout/cancellation Display summaries
are retained; registration-error context and safe correlations can change.
Plain Interrupted is still IO. Deadline/signal selection and durations remain
unchanged; caller-observed cancellation and local future drop do not certify
rollback, Apple cleanup or safe retry. Deactivation's separate combined fallback
and activation HTTP/source adapters remain pending.

CLI symbols, activation and pasteboard keep success output, command/flag shapes
and final error envelope fields. Retained sources can change formerly unknown
or protocol categories to timeout, cancelled, disconnected, permission_denied,
service_missing or explicit unsupported. Final errors still use exit 1 and
unknown effects. A host-output/native category does not identify device state.
Private native payloads remain accessible to explicit Rust cause inspection;
owner/new wrapper Debug and safe CLI JSON keep them hidden.

A failed symbols download can leave completed local chunks; a later incomplete
H2 read does not deliver its partial bytes to the sink. No success size/result
is returned after failure. Discard the failed remote client; these changes do
not automatically invalidate, close/join or certify reuse of its borrowed stream.
The bounded host fixture retains a complete 10 MiB chunk before a second-chunk
EOF, and a successful limited download retains exact requested bytes. H2 frame,
buffer and file budgets are unchanged. Pasteboard registration can precede a
baseline connection error; the host fixture observes one baseline attempt,
no monitor result, registration bytes and local notification-stream EOF.
Neither fixture is device cleanup acknowledgement.

Host evidence covers real public remote-symbols transport/codec faults, a real
modern service write fault through the shared CoreError mapper, typed mapping
fixtures, the production CLI deadline/signal helper over controlled futures,
the actual pasteboard monitor baseline with the shared connection/snapshot path,
and actual safe error rendering. It does not establish paired-TLS ConnectedDevice
activation/acknowledgement, actual OS Ctrl-C registration/delivery, remote-symbol
device workflows or physical pasteboard state. Other source/JSON/callback/binding,
L01/L03/A03 and platform/device conditions remain pending.

## Python exceptions

The existing RuntimeError from `ios_rs.list_devices()` and `start_tunnel()`
retains its safe single-string `args`, with category selected from the native
typed error before converting to Python. Invalid tunnel mode keeps ValueError
and its exact message; `Tunnel.asyncio_proxy()` without a userspace proxy keeps
its existing RuntimeError. These errors gain four additive attributes:

| Attribute | Meaning |
| --- | --- |
| `ios_error_schema_version` | Integer `1`; allow future versions and additional attributes |
| `ios_error_class` | Stable ErrorClass code; consumers need a fallback for unknown/future codes |
| `ios_error_phase` | Fixed `list devices`, `start tunnel`, `initialize runtime`, `tunnel mode` or `asyncio proxy`, without user values |
| `ios_error_effects` | `unknown` for native operations; `not_started` for rejected mode or unavailable proxy before that operation |

No public factory/method parameter is added. Existing `except RuntimeError` and
`except ValueError` callers keep working; callers can select a category by
attribute instead of parsing diagnostic text. Invalid mode fails before runtime
initialization or device discovery. A proxy precondition failure does not retain
an asyncio patch. Unknown effects never grant retry, rollback or cleanup safety.
Original Rust sources remain available to native callers before this conversion;
they are not exported as unsafe Python causes or diagnostic payloads.

Ordinary Tokio runtime-initialization IO errors now return this safe RuntimeError
profile with phase `initialize runtime` and unknown effects, before device work.
The original IO error is cached instead of triggering the former expect panic
and poisoning later initialization attempts. Initialization and device blocking
work still execute inside allow_threads; invalid mode still fails first. This
changes the exceptional initialization path from a panic boundary to the mapped
built-in exception. Success and existing operation-error args/signatures remain.
Controlled host tests inject a typed failed initialization result; they do not
induce real thread/memory exhaustion or certify runtime cleanup under that failure.

If metadata allocation/assignment fails, that Python exception is returned with
the original safe Python exception as its cause. The operation remains failed;
complete metadata delivery is not promised under allocation failure. Python
argument extraction errors, interpreter/allocation errors, callback exceptions
and other runtime-internal panics are outside this mapped
profile and still need their own review. Absence of attributes does not mean
success or an unknown side effect was rolled back. The host tests and installed
owned ListDevices peer certify this boundary, not physical tunnel startup or
Apple teardown.

## C status categories

`ios_last_error_class(void)` is an additive symbol returning the stable ErrorClass
code of the calling OS thread's last function returning an int status. The
borrowed NUL-terminated string is static while the library remains loaded;
never modify it or pass it to `ios_free_string`. Read it on the same OS thread
as the operation, before another int-status call replaces the observation.
Each such call resets its prior category before work, records the native typed
source on failure, and clears it to `none` on success. `none` also describes a
thread before its first int-status call. Runtime initialization, void cleanup,
uint16_t port queries and the getter itself do not replace it. The getter does
not initialize or require the Tokio runtime.

The existing fourteen symbols/signatures, statuses 0–8, mode discriminants and
IosDevice layout remain unchanged. Core/serialization errors are classified
before status conversion. Runtime initialization retains its original IO error
instead of a String and still returns status 6 on failure. NULL/UTF-8 rejection
gives `invalid_input`, unavailable/poisoned handle state gives `precondition`,
and a too-small caller buffer gives `resource_limit`. The existing ALLOC status
also represents an embedded NUL, so it does not prove exhausted memory and
falls back to `unknown`. Captured int-call panic status 7 replaces any earlier
category with `unknown`; no panic message is returned by this getter. Remaining
opaque sources stay unknown. Consumers must retain a future-code fallback.

This query adds no interruptible C call, concurrent handle-mutation guarantee,
automatic retry or Apple cleanup acknowledgement. A `disconnected` IO class
does not identify which object disconnected. `none` describes a successful
numeric status, not a certificate of device cleanup or absence of partial
effects. Legacy void calls still report no outcome; additive checked entries
are described below. The owned linked-C malformed ListDevices fixture validates a real
library call, not device connection/query/tunnel acceptance. Native source
details are not exported as an unsafe C string.

`ios_list_devices_checked` is an additive int-status entry using the existing
IosDevice array layout and `ios_free_devices` ownership. If any received UDID
or connection type contains an embedded NUL, it returns `IOS_ERR_CORE` with
class `protocol`, clears outputs to NULL/0, and exposes no partial array. Native
IO/discovery errors retain their existing status/class mapping. Valid and empty
lists succeed and clear the caller's category to `none`.

The original `ios_list_devices` retains its historical behavior of skipping
unencodable records and returning the remaining rows, potentially an empty
success. Migrate to the checked symbol to detect these projection failures;
existing callers need no signature/layout change. Checking C encoding does not
prove completeness of the upstream discovery result or current reachability.
Controlled binary-plist peers exercise both real C entries and free successful
arrays. No device service, pairing, or Apple cleanup is certified by this check.

`ios_tunnel_rsd_port_checked` and `ios_tunnel_userspace_port_checked` add
int-status queries over the handle's cached port fields. Each non-null output
is reset to zero before the handle check. NULL input returns `IOS_ERR_NULL` with
`invalid_input`; a zero cached value returns `IOS_ERR_STATE` with `precondition`;
success writes a nonzero unsigned 16-bit port and resets the category to `none`.
They do not require a runtime, allocate a resource, send a request or update a
probe observation. A zero cache entry alone cannot distinguish kernel mode,
missing metadata, skipped/failed discovery or explicit device unsupported.
Neither zero nor nonzero establishes current disconnect or reachability.

The existing uint16_t queries preserve their signatures, zero fallback and
non-updating category behavior. Do not classify a legacy zero from the previous
int call's category. Migrate consumers requiring NULL/missing/success status to
the checked symbols and read the category on the same OS thread. Their library
must provide these additive symbols; old binaries retain the legacy API.
Outputs must not overlap the live handle, and concurrent close/use or freed
handle reuse remain unsupported. Rust-owned metadata tests exercise both actual
exported functions for missing/nonzero values, NULL output and category/thread
transitions; strict linked C verifies signatures and NULL clearing. This proves
the projection boundary, not a physical tunnel/proxy/RSD probe or Apple cleanup.
Other binding/source/lifecycle review remains pending.

`ios_runtime_init_checked` returns an int status for shared runtime initialization:
success clears the category, a cached ordinary native IO error returns
`IOS_ERR_RUNTIME` with its typed category, and a caught unwind returns
`IOS_ERR_PANIC`/`unknown`. The original IO result remains cached and is classified
without exporting its private Display text. Repeating initialization does not
retry a failed result. No device is opened. The legacy void initializer retains
its ordinary result/category behavior and now catches unwinds before the C
boundary. Catching an unwind does not suppress a process panic hook or recover
from abort/invalid memory. The global initializer panic path is source-reviewed,
not established by an actual process-level poisoned runtime fixture.

`ios_device_close_checked`, `ios_tunnel_close_checked` and
`ios_free_string_checked` take writable pointer-to-pointer owner storage.
A NULL owner pointer returns `IOS_ERR_NULL`/`invalid_input`; a valid owner whose
value is NULL succeeds as a no-op. A non-null owned pointer is cleared **before**
release. Handle state poisoning returns `IOS_ERR_STATE`/`precondition` after
consumption and local drop; a caught unwind returns `IOS_ERR_PANIC`/`unknown`.
The owner stays cleared on these failures. A consumed pointer or retained alias
must never be retried, used or freed. No API validates arbitrary/dangling pointers.

`ios_free_devices_checked` requires both pointer and count owner storage.
If either is NULL, neither is modified and the result is `IOS_ERR_NULL`/
`invalid_input`. Once both are accepted, the list pointer and count are cleared
to NULL/0 before release. A NULL list succeeds as a no-op and clears the count;
a non-null list requires its exact returned count, including a non-null allocated
empty list. The standard allocator/string ownership rules remain unchanged.
Owner storage must be writable, distinct where there are two outputs, outside
the allocation being released, and unavailable to concurrent use/close.

Every checked call uses the same caller-thread category replacement rule.
Read/save an operation's original status/category **before** checked cleanup;
even a successful no-op cleanup resets the observation to `none`. The borrowed
static code pointer remains valid after later int calls while the library is
loaded. Inspect release failures separately. Original void cleanup preserves
the category and signatures; its shared release routines retain prior behavior.
The checked symbols require a library providing these additive entries.

Success reports local release/no-op status. It does not acknowledge Apple
teardown, task joins, cancellation completion or absence of partial effects;
other owners can retain a ConnectedDevice. Failure after consumption does not
promise all resources were released or supply rollback/retry authority. Repeated
release is safe only through the now-NULL owner storage, never through a saved
non-null alias. Owned tests cover native initialization projection, actual
runtime success, allocated lists/strings, no-device handles and poisoned state;
the shared consuming helper's counted destructor fixture verifies clearing,
single drop and caught unwind. Strict linked C exercises actual ListDevices
allocations and checked freeing. These do not certify physical service/tunnel
teardown or actual global runtime resource exhaustion. Stronger L01/A03 contracts,
other sources/bindings and stream/callback boundaries remain pending.

## Protected HTTP responses

Manager, CDP and WebDriver router errors gain four fixed response headers,
without changing their existing HTTP statuses, JSON body shapes or safe
messages. Successful/informational responses do not gain these fields.

| Header | Meaning |
| --- | --- |
| `X-Ios-Error-Schema-Version` | String `1`; retain a future-version/fallback branch |
| `X-Ios-Error-Class` | Stable ErrorClass code from a native typed source or an explicit admission/issuer outcome; opaque/unobserved sources stay `unknown` |
| `X-Ios-Error-Phase` | Enum-selected `http_request`, `http_access`, `browser_ticket`, `webinspector`, `tunnel_manager` or `policy_reload`; never a submitted URL, selector or header |
| `X-Ios-Error-Effects` | String `unknown`; failure alone does not certify rollback, safe retry or completed cleanup |

WebInspector conversions and manager operation failures inspect the original
typed error before constructing the existing safe body. Both manager dial
timeout wrappers use the actual Elapsed class while preserving their original
message-only response. Access/Host/Origin refusal is PermissionDenied; ticket
capacity is ResourceLimit, issuer input rejection InvalidInput and unavailable
entropy Unknown. These are local named outcomes, not a classifier of arbitrary
HTTP status numbers. Named manager hop/upstream limits are ResourceLimit,
zero/missing connect input is InvalidInput, registry/tunnel lookup misses are
NotFound, and unsupported local transport/legacy GET routes are Unsupported.
Those local outcomes do not prove a missing daemon service or an explicit
device-side capability rejection. The transport guard is currently dormant:
every constructible StartTunnelConnectionType returns no unsupported message;
its header mapping is source-reviewed, with no fabricated runtime/device proof.
Other opaque failures and framework rejections
without a retained native source stay Unknown even when their prose/status
mentions timeout or a missing target.

The reviewed-response marker stays in server-only extensions. An incoming or
unreviewed outgoing header cannot bypass body sanitization or select a class.
Framework error bodies/headers are discarded as before, with status/Allow,
no-store and empty HEAD bodies retained. HEAD can still deliver machine headers.
Approved Origin responses expose exactly these four error headers to browser
fetch. Existing Host/Origin/credential and preflight admission remain unchanged;
unapproved origins gain no CORS access. Existing JSON/body consumers require no
migration; new callers read headers and allow future codes/versions.

This profile covers responses reaching the protected routers. TLS/HTTP parser
failures before routing, WebSocket frames after upgrade, remote client error
sources, remaining opaque decisions and stream/partial semantics remain
separate work. Missing headers are not proof of success. Native failures are
classified before boundary conversion; private original Rust chains are not
exported as unsafe HTTP data. Host/Chromium evidence does not certify a device
workflow or Apple teardown.

The CDP page WebSocket bridge now returns the original native Timeout/Elapsed
when a partial header/body or automatic listing write invalidates its client.
Previously it ignored that timeout, then returned a new local Protocol error
on the next loop. Empty usable idle still resumes. Native EOF remains IO,
malformed plist retains its native source and existing IO classification, and
the legacy oversize-frame rejection remains Protocol. This change applies to
the private page loop after upgrade; it adds no error headers or unsafe error
data to WebSocket frames. Setup totals, cleanup errors, browser child results
and server cancellation remain separate work. See
[CDP page fixtures](validation/cdp-page-idle-lifecycle-2026-10-05.md).

CDP discovery/page/browser-page and WebDriver runtime constructors now share
one absolute setup deadline from before lock/source admission through their
final setup step. Expiry retains native Elapsed; private zero/clock overflow
rejects before source contact under the existing typed families. Source/native
failures stay primary. Exact target discovery waits within the total and retries
only reusable idle; a missing target can now time out rather than fail after
one snapshot. Full-list/automation discovery keeps a separate500ms quiet window.
Returned setup failure or caller cancellation drops the owned stream/lock,
without guessed incomplete-attach recovery. Existing active, cleanup, HTTP
publication and server signal error handling remain outside this profile.
See [server setup fixtures](validation/webinspector-server-setup-2026-10-05.md).

Direct CDP page shutdown now returns a failed forwarding close instead of
discarding it. Cleanup receives an independent 2s total, rather than inheriting
the active page timeout. Native IO/Elapsed sources remain. If the page already
failed, the existing private cleanup wrapper owns both errors and exposes the
original primary through its source chain and classification. Diagnostic output
continues to use the existing safe formatter; no new error frames are sent.
Invalid clients skip recovery writes. Five actual WebSocket/owned-stream fixtures
cover this returned-error profile. Principal/server cancellation still drops the
handler and is not covered by this cleanup path. See
[CDP page cleanup fixtures](validation/cdp-page-cleanup-2026-10-05.md).

WebDriver runtime stop now retains context-close and forwarding-close failures
through the existing private cleanup wrapper. Both steps share the stored100ms
total; native IO/Elapsed and the original primary classification survive. An
invalid client skips socket recovery, and starting stop disarms another attempt.
These are runtime-result fixtures: HTTP DELETE still ignores its cleanup result,
and outer revocation timeouts retain their existing handling. Request/registry
ownership and routed error projection remain separate work. See
[WebDriver cleanup fixtures](validation/webdriver-cleanup-2026-10-05.md).

Direct CDP page lease expiry/reload now becomes a returned typed
OperationCancelled inside the owner, followed by independent cleanup. If cleanup
also fails, the existing wrapper retains Cancelled as primary and the native
cleanup cause. Previously outer Principal.run discarded ScopeDenied and dropped
the handler before cleanup. Revocation now reaches the existing safe diagnostic
formatter; it sends no new error frames or headers and exposes no credentials.
This supersedes the earlier principal-cancellation limitation for direct pages.
Generic caller/server cancellation and browser/stored task joins remain separate.
See [CDP page lease fixtures](validation/cdp-page-lease-2026-10-05.md).

CDP browser child tasks retain original send, decode, receive and cleanup
failures instead of breaking silently. The primary is recorded before cleanup;
supervisor abort cannot erase that observed cause. Explicit stops combine
child outcomes, unexpected native JoinError and an actually issued cleanup
expiry through the existing private wrapper. Every other child is joined even
after a failure. Known causes remain owned; a cleanup result interrupted before
completion is not invented. A normally completed task without its cleanup
outcome is a local Protocol error.

Explicit detach/Closed handling uses the existing safe diagnostic formatter
while keeping successful detach replies and healthy browser traffic. Browser
loop exit combines its primary with group cleanup. No raw causes, new headers
or error frames are sent. Browser lease/caller/server cancellation still uses
Drop and needs separate ownership work. See
[browser child errors](validation/cdp-browser-tasks-2026-10-05.md).

CDP browser lease expiry/reload now returns typed OperationCancelled inside the
owner. Existing cleanup then retains native child and waiter failures under
that parent primary. Removed-child owners, completed joins, native primary and
cleanup outcomes, and an already issued Elapsed survive cancellation of a
borrowed cleanup future. Resuming/reparenting does not invent errors or poll
completed handles again. Private observation verifies Cancelled primary plus
original PermissionDenied IO; other fixtures retain native JSON/IO/Elapsed
through interrupted cleanup and all joins.

Browser revocation now reaches the existing safe diagnostic formatter instead
of silently discarding outer ScopeDenied. No raw causes, new error headers or
WebSocket error frames are sent. Child detach replies and healthy traffic stay.
This supersedes the earlier browser principal/retiring-owner limitation; generic
caller/server Drop and stored registry remain. See
[browser lease errors](validation/cdp-browser-lease-2026-10-05.md).

### WebDriver DELETE cleanup errors (2026-10-05)

DELETE now returns an actual cleanup failure through the existing AppError:
HTTP500, unchanged safe WebDriver unknown-error body and schema1 machine
headers with WebInspector phase/unknown effects. A native forwarding flush IO
retains PermissionDenied; actual mutex or cleanup expiry retains native Elapsed
and Timeout. An already received business-close Protocol error stays primary
if forwarding cleanup subsequently fails or expires. Both original causes stay
owned in the private result until the existing HTTP projection classifies it.

Previously DELETE ignored its outer timeout and the unit cleanup callback
logged failures, so these cases returned200/null. Success and repeat deletion
still return200/null without error headers. The new private Result helper
shares the entry deadline and avoids an outer competing cleanup timer. Legacy
revocation callback, registry join and active request Arcs remain separate.
This pre-1.0 error/timing migration needs release review and device evidence;
see [DELETE errors](validation/webdriver-delete-2026-10-05.md).

### Stored session watcher join errors (2026-10-05)

The private watcher owner keeps original Tokio JoinError after an unexpected
panic or external cancellation, retaining Unknown or Cancelled classification.
Only an explicitly issued abort's cancellation is neutral. Completed normal,
requested-abort and unexpected outcomes persist across cancelled join waits;
completed handles are not polled again. Native error transfer has no subsequent
await before the existing safe diagnostic formatter receives it. Registry
operations report join failures through safe tracing rather than new HTTP
fields or raw panic payloads.

This covers task joins only. The unchanged unit revocation callback/outer100ms
timer still needs native protocol cleanup primary/secondary retention. Idle
completed handles await the next registry call or Drop; generic Drop aborts
without an async join. See [watcher errors](validation/session-store-2026-10-05.md).

### WebDriver revocation cleanup Result (2026-10-05)

The actual WebDriver registry callback now returns Result through its native
JoinHandle<Result<()>>. Normal native task exit can contain cleanup failure;
it is not a success claim. The watcher owner retains that original result until
join/reap transfers it without another await. Native PermissionDenied flush IO,
issued mutex/context/socket Elapsed and business Protocol primary plus original
secondary IO/Elapsed remain downcastable. No outer timer races the combined
cleanup result. Previously a unit adapter logged the error and its outer timer
could discard a known business primary while forwarding close was pending.

Registry reports use existing safe error_message and the broader tracing label
automation session closed with error; unexpected native join flags/categories
stay. No new HTTP fields/status/body/exit schema is introduced. The private
actor result observer tests real product creation/reload/expiry with owned plist
streams; it does not certify captured production logging or device cleanup.
Map wait before callback budget, known errors before callback return during
store abort and generic Drop/server supervision remain. See
[revocation errors](validation/webdriver-revocation-2026-10-05.md).

## Tunnel manager connection generations

Local target lookup now prepares an independent actual connection Ticket root after the original record copy and before cloning publication Life. Complete global, published, and retained foreign-origin grants precede projection and allocation. The connection keeps its original device/global epochs while omitting the desired fence and its unused origin. The same paid root moves into the existing upgrade callback and concrete future; it does not delay row close or native joins. Lookup refusal uses the original record-copy 503 path before TCP dial. Other keepers and the physical marker arena remain open. See [LocalTarget Ticket evidence](validation/tunnel-local-target-ticket-root-2026-10-23.md).

Managed publication rows and their displaced or retired values now retain an independent actual Ticket allocation. Its complete global, original published, and captured foreign-origin grants are admitted before Source creation and survive until the root is physically freed. Published-origin deduplication requires an actual held permit for exactly the complete root size. The original publication fences, metadata/native ownership, rollback, and deadlines are preserved. LocalTarget connection Ticket captures and the physical marker arena remain open work. See [publication Ticket evidence](validation/tunnel-publication-ticket-root-2026-10-23.md).

Retry episodes and receipts now share an actual preadmitted Ticket allocation, with complete destination and captured-origin grants. The root contains only Ticket, so retaining an episode does not retain an old receipt or IO payload. Removed::into_inner still frees its original outer Box, while the same Ticket root and canonical key remain alive and paid until the final actual owner releases them. Same-episode retries retain their original first Ticket and clocks. This adds no logical placeholder fee and does not prove other arbitrary Ticket escapes or the physical marker arena. See [episode Ticket root evidence](validation/tunnel-episode-ticket-root-2026-10-23.md).

Failure receipts now admit their complete actual shared allocation in each foreign producer bank captured by the original Ticket global/device/optional desired identity fields. The original strong/weak allocator stores these grants outside the value, so last-strong Ticket destruction cannot refund origin credit before the final actual Weak deallocation. Same-bank and duplicate origins are not double charged; quota refusal precedes selectors/Source and rolls back all temporary grants. Original IO/history/completion and retry clocks remain. This does not cover the separate inline Episode/Removed escape lifetime or the physical marker arena. See [receipt origin evidence](validation/tunnel-receipt-marker-origin-2026-10-23.md).

Private control identities now retain their actual producer bank from the actual Arc birth, without an existing-Arc relabelling or raw escape API. New foreign native Entry keepers admit origin credit for the whole original destination slots backing, retained through its actual final free; funded foreign adapters remain valid and existing-origin reuse/retirement acquires no late credit. Historical-origin catalogue backing is paid in the destination with staged rollback and no slot-count origin cap. Independent foreign RefreshProofs retain their own logical origin grants after stamp destruction, preserving the original registry/key/stamp/handle cleanup order. This closes those known admission edges, not the underlying Arc arena or every retainer. See [native marker origin evidence](validation/tunnel-native-marker-origin-2026-10-23.md).

The failure-record count semaphore now has an independent complete Shared<Semaphore> root paid in the original manager byte bank. A private one-count owner retains only that root, returns the original Tokio count before releasing it, and never retains State or native resources. Original count-before-body refusal priority, exact retained-Slot reuse, capacity and cleanup order remain. Slot and Counts allocations retain separate actual-layout fees through final physical release, including when receipts or count observers outlive State. Exact fixture budgets include the new root; independent count/body test pools remain explicit. See [failure-count root evidence](validation/tunnel-failure-count-root-2026-10-23.md).

A finite cross-bank census now executes the supported independent Lifecycle-A/Registry-B boundary: four retired B entries retain eight distinct old raw marker Arcs at strong count one after the closed/zero A producer is dropped and its current known-storage fees fully return. This is evidence against assuming every historical keeper pays A, not a production infinite trace or an arena capacity. Future arena integration must keep the entire actual backing paid through its final marker/weak keeper, rather than preserve the current missing-marker-fee refund. Production behavior is unchanged by this test-only item. See [cross-bank census evidence](validation/tunnel-marker-cross-bank-census-2026-10-22.md).

Standalone tunnel start now admits the complete concrete connection/report/signal/cleanup future in its original registry byte bank before first polling Ownership or Source. The original options/log prefix, 30-second connect-only timeout, synchronous output/flush before signal polling, lease-before-device destruction, Ownership early return, and terminal close/drain/primary-plus-cleanup handling remain. Actual SDK futures are constructor-tested only; local duplex fixtures exercise cancellation, joins, strict EOF, original error pointers and closed-bank refunds. This pays the concrete F allocation, not every heap allocation reachable from it. See [standalone frame evidence](validation/tunnel-standalone-frame-2026-10-22.md).

Foreground scope construction now admits and prepares separate complete Inner and connection-Semaphore shared allocations in the original registry bank before constructing Inner. The original Tokio four-permit try-acquisition and release rules remain; each private owned permit retains only the independent Slots root, so a live connection cannot delay the last Scope Inner native-stop request. A refused second empty destination leaves an independently retained registry open; preadmitted destinations can be placed after bank closure. Each actual shared allocation stays paid through physical free. See [foreground Slots root evidence](validation/tunnel-foreground-slots-root-2026-10-22.md).

Foreground NativeScope shared ownership is admitted as the complete declared Shared<Inner> allocation in its original registry bank, before constructing Inner. Scope clones and retained cleanup failures keep that same fee until the actual last shared body free and original Inner destruction. Refused fresh root admission leaves an independently retained registry open and empty; connection, native retirement, cleanup and original error-source bodies are unchanged. The separate connection semaphore and other opaque/SDK/marker scopes remain open. See [foreground Scope root evidence](validation/tunnel-foreground-scope-root-2026-10-22.md).

Foreground NativeScope connections retain their original selector and four-connection admission prefix, then admit the complete concrete connect future in the original registry byte bank before creating standalone stamp/reservation ownership or invoking the SDK. Fresh refusal is typed resource_limit; an admitted call retains the original connector options, acquired error source, exact runtime lease and device-before-lease-before-permit destruction. Actual native future checks are constructor-only, while local runtime cancellation/closed-zero/strict EOF and original IO pointers are tested through the same concrete owner path. See [foreground connect frame evidence](validation/tunnel-foreground-connect-frame-2026-10-22.md).

Manager native attempts prepare the exact first Attempt Box destination in the original registry bank before Source creation, and admit each additional target inside its original Ticket fence before reserving/starting that target. Refused fresh admission remains a typed resource_limit; acquired native causes and original handle/join ownership remain intact. Core carries the original reservation Box and its independent grant in an inline TunnelReservationOwner, destroying the Box before refund. Existing public reserve callbacks retain their default behavior; manager legacy reserve retains the same paid fee root in both the scope and every original Box, so a forgotten Box keeps its grant after scope destruction. See [native Attempt Box evidence](validation/tunnel-attempt-box-owner-2026-10-22.md).

Fresh HTTP DELETE and epoch requests admit the concrete control future only after their original permission, diagnostic and success-wire preparation. A refused allocation returns 503/resource_limit before Ticket/lock/mutation work. DELETE keeps the original Principal cancellation guard and ScopeDenied response, while epoch keeps its original unguarded core clear body. Internal core stop/clear/notify and terminal Drop receive no new byte admission and retain their existing closed-zero control rules. See [HTTP control future evidence](validation/tunnel-http-control-frames-2026-10-22.md).

The [L02 manager host profile](validation/tunnel-isolation-2026-10-06.md),
`e0f52671396e6408fc38921f78f73a256a45ce65`, preserves native IO, Tokio Elapsed
and OperationCancelled through the private source/actual HTTP error adapter.
Manual creation shares one15s queue/connect/fallback total after ticket
acquisition. An unmatched mobdev2 fallback retains original usbmux IO instead
of rebuilding a formatted error. Stop cancels old pending/queued starts;
their existing502 now carries cancelled. Stop itself retains200/404, including
404 when there was no record but a pending start was cancelled. Safe bodies,
schema1 and authorization stay. Timing/category corrections need release review.
Cancellation is local ownership; no native worker join, Apple ACK, retry safety
or persistent device exclusion follows. Scan discovery/whole-flow totals and
in-flight detach/replug remain; L01/L02/L03–L04/Q04 and later classes continue.

## Parallel scanner outcomes

[L02 bounded scans](validation/tunnel-parallel-2026-10-06.md) tests
`66bf7311020763b8a29b175a117cdf1b70d4e22f`. USB and existing-trust mobdev2
candidates share one deduplicated four-future window. Original candidate
connection/error/backoff code stays. A paired discovery error is retained until
USB candidates finish, then returned with its native source; successful USB
records stay and ready is not newly set. Candidate cancellation does not
manufacture connection-failure backoff. Discovery now precedes USB attempts;
whole-discovery/scan deadlines and stronger known-error retention during
arbitrary parent Drop remain. No machine schema/status/auth change, safe retry
or remote cleanup claim. L01/L02/L03–L04/Q04 and later classes continue.

## Observed attachment cancellation

The [L02 identity snapshot gate](validation/tunnel-identity-2026-10-06.md),
`1eb45c25145f5306a5831df6a0c29d494a94ca96`, rejects a scanned candidate
whose completed native ID differs from the observed expected ID. Its owned
result drops and local OperationCancelled identifies admission cancellation;
this is not an original device IO failure. Such cancellation adds no failure
backoff. Original IO/backoff remains. Snapshot equality is not physical
freshness, native worker join or safe retry proof. Manual expected ID is absent,
and post-connect queries/listeners are not implemented. Original HTTP/schema/
safe body/public SDK stay. L02/L01/L03–L04/Q04 and later classes continue.

## Query errors before tunnel publication

The [publication query gate](validation/tunnel-publication-2026-10-06.md)
accepts `86fecfe3331e9efa74dc76fbc48b47ad5c3df8f1`. An extra current
attachment query before USB publication retains original Connect/List IO and
original Elapsed under the existing total; absence is local cancellation with
owned result release. It is not an original device failure or remote ACK.
Safe context/body/schema/auth stay. Query completion is a point-in-time view;
after-query changes, reused IDs, daemon restart, native liveness/joins and
generation-specific backoff remain. L01/L02/L03–L04/Q04/classes5–9 continue.

## Known identity and failure backoff

[L02 backoff identity](validation/tunnel-backoff-2026-10-06.md) accepts
`d0b04bed79ecec5c3f724ff003122f66b1159216`. Ordinary native failure now
records its known route/expected ID; waiting and count increment apply only
to that identity. Known changes can retry immediately; new identity failure
starts at1. Original source IO/cancel/reporting/category and same-identity
schedule stay. Debug exposes no retained host. No safe retry or side-effect/
new-pairing claim follows. Same ID/host after reuse/restart remains unresolved;
retention/native resources/L03/L04 and otherL01/L02/Q04/classes5–9 continue.

## Known inactive publication result

[L02 local liveness](validation/tunnel-liveness-2026-10-06.md) accepts
`7a77000cdbd0a8e6d2fbaaf6ac5bd630af47d6c5`. Existing is_alive false
before query or after publication-lock wait cancels local admission and
releases the owned result; no synthetic original IO/native failure is inferred.
Original IO/Elapsed, schemas/auth/old guards stay when locally active. True
means kernel receiver exists or userspace tasks unfinished, not connectivity/
ACK/join proof. Last-check races/false positives/resources/platform/device/
L01/L02/L03–L04/Q04 and later classes remain.

## Scan setup expiry and quiet cancellation

[L02 discovery setup](validation/tunnel-discovery-2026-10-06.md) accepts
`a47648442d5b7707ad41beeb58a6ab1ad764ea9a`. A checked15s deadline spans
queue/USB/paired waits; native Elapsed remains. Due/zero does not poll real
work, overflow uses invalid_input before source or mutation. Stop/clear of an
old scan epoch quietly drops its phase future and returns Ok without failure
backoff/ready. Original source IO and candidate outcomes stay; whole-scan/
blocking-worker joins/known primary under generic parent Drop and otherL01/
L02/L03–L04/Q04/classes5–9 conditions remain.

## Scanner failure record capacity

[L02 retention](validation/tunnel-retention-2026-10-06.md), tested at
`26b89951b52dd690bcc0e994541007b5426fba93`, uses the existing source-free
resource_limit decision when a fresh scanned identity cannot reserve capacity.
It precedes native connection work and does not invent an IO cause or evict
active backoff. Existing identities reuse their slot. Original native IO,
Elapsed and paired-discovery reporting stay. A ticket recheck under the failure
write lock rejects old queued outcomes after clear/stop. Other L01/L02,
L03–L04/Q04/classes5–9 and native worker/device conditions remain.

## Tunnel server outcomes and cleanup

Private server startup has a synchronous fallible preparation step which admits the complete concrete generic future from the original global metadata bank before its Source work. Lifecycle Owner starts only on the returned future's first poll; failed preparation or unpolled Drop does not newly shut down retained state. Already acquired native/scan outcomes must not require a new frame grant during terminal reporting: callers retain the originally prepared future through quota exhaustion and consume that same instance. Repeated zero-quota reports preserve the same prepaid history and deep native cause; actual frame-free credit remains an independent refund. The original execution/cleanup body and started Owner Drop rules remain. See [prepared server evidence](validation/tunnel-server-frame-owner-2026-10-22.md).

The [server gate](validation/tunnel-server-2026-10-06.md), `5120fa90e06a92f4734603382b98779b69f1ca38`,
retains signal/server IO and actual cleanup Elapsed. A dual failure stores both
outcomes with the primary as its source; BrokenPipe still classifies as
Disconnected. Wrapper Debug and CLI diagnostic output stay safe. Raw Anyhow
Debug follows the deliberately retained native chain and is not the diagnostic
boundary. Shutdown now retains its signal for late waiters. The returned server
uses an independent2s cleanup total; expiry does not certify delegated HTTP or
Core worker termination. Caller Drop cannot await or clear a locked table.
Other L01/L02/L03–L04/Q04/classes5–9 and platform/device conditions remain.

## Native tunnel shutdown evidence

The [Core native owner profile](validation/native-tunnel-owner-2026-10-07.md), `f00ff25395a8432087daf578c9dd23975b09be5a`, retains original
work IO and Tokio JoinError/Elapsed objects through existing TunnelError::Io
retained context. Opaque TunnelShutdownFailure follows primary for Error::source
and classification; cleanup exposes other retained failures/current expiry.
Ten fixed role slots and a saturating omitted-observation count bound retention.
That counter counts observations, not distinct faults. Snapshot retry does not
mutate old evidence. joins_complete requires zero pending joins and valid count
accounting, and can be true on an error report after recovery; it does not certify
the timed attempt succeeded. An outer report certifies only its runtime's joins.

Requested aborts are quiet while preserving native cancellation. Already-finished
foreign cancellation remains unexpected; competing unfinished aborts have no
exclusive Tokio provenance. Direct work reports use private ledger identity to
avoid re-recording; foreign/full/extra-wrapped native faults remain ordinary work
causes. ForwardReaderCleanup preserves simultaneous raw-forward work and reader
join errors with primary source. Both opaque payloads use fixed/category Debug
and Display; original causes remain inspectable. Existing error enums are unchanged.

## Native CLI cleanup causes and immutable observation

The [CLI retirement profile](validation/cli-tunnel-native-retirement-2026-10-07.md),
`6debaf9b6d78c10b4be1cfd55e2fbcbb69da8756`, adds readonly Core shutdown_status and opaque report flag
has_retained_outcomes. Observation records no new attempt expiry and does not
stop/join workers; incomplete accounting alone is not an acquired native fault.
The CLI captures already acquired original IO if its outer clock cancels inner
shutdown before return. First outer joins_complete alone can certify that exact
runtime, including false; a nested foreign true report cannot release capacity.

Two fixed history causes retain first pure Core expiry and first non-expiry native
report. Later native faults outrank earlier expiry, including same-owner retries;
current attempt Elapsed stays separate. Fully joined Err releases capacity while
its original cause remains available to server terminal cleanup. Bounded counters
describe other failed owners; Core omitted observations keep their existing meaning.
Server phase outcomes are saved before Ready; held-table expiry can coexist with
complete native joins. Metadata-only direct Elapsed and original signal/server
primary contracts remain. Opaque wrappers render fixed context/categories and
preserve native sources; no error-enum, protocol, safe-retry or remote-ACK change.

## Late manual connection Ready and acquired IO

Explicit tunnel start preserves selector bounds, the manual exact-identifier check and original ConnectionDeadline construction before admitting its concrete future from the same global metadata bank. A refused frame returns resource_limit before Ticket capture, coordinator/serial waits or Source/fallback work, including a prepared zero deadline or terminal lifecycle. An admitted frame retains the original deadline.run/Ticket ordering, acquired-error priorities, absolute clock and route/fallback guards. See [explicit-start frame evidence](validation/tunnel-ensure-frame-owner-2026-10-22.md).

Implementation `a963679ab0f853b1a38bfcee9ac59471d4920641`; [original error evidence](validation/tunnel-manual-poll-crossing-2026-10-07.md).
Payload-free private deadline decisions return control to the independently
polled Tokio timer; callers still receive actual Elapsed classified Timeout.
Late successful Ready is rejected. An already acquired Source/metadata native
error remains inspectable with its original category even after a long poll;
default fallback cannot open another discovery/transport after the total expires.
The private decision adds no public error enum or safe-retry/remote rollback rule.
Automatic candidate and other timing/cleanup requirements remain.

## Automatic candidate native expiry and original IO

Implementation `c92503ded8a60515a272f4afb1dba99085b130dc`; [automatic candidate evidence](validation/tunnel-automatic-deadline-2026-10-07.md).
The checked candidate clock preserves actual Tokio Elapsed with the prior scanned
timeout context and the shared wrapper's already acquired original IO precedence.
Zero never polls work; unrepresentable internal duration is InvalidInput before
Source without panic. Automatic workflow fixtures prove admission/publication
and retained original joins; two direct scanned-wrapper cases prove categories.
Failure identity/backoff and cancellation recording policy stay. Whole-scan post-paired handoff/reconciliation/readiness and out-of-clock failure bookkeeping,metadata byte budgets,HTTP/upgrades/blocking/startup/physical freshness/reuse/restart/manualepochs and all other original conditions remain.

## Paired discovery native cause before epoch cancellation

Implementation `1ab05e3bcefd3926a402dc645f55c72d16aad6f4`; [paired error retention evidence](validation/tunnel-paired-error-retention-2026-10-07.md).
The stale paired-result branch preserves its acquired original Err and maps only
successful stale discovery to Ok. The fixture's actual TCP partial UnexpectedEof
remains directly downcastable and retains the established Io category;EOF is not
automatically classified as device disconnect. Core classification/error enums
and all preceding assertions remain. No new wrapper or synthetic cause is added.
Returned error instead of swallowed success needs pre-1.0 review. Whole-scan post-paired totals/cancellation,out-of-clock failure bookkeeping,metadata bytes,HTTP/upgrades/blocking/startup/physical freshness/reuse/restart/manualepochs and all other original L01-L04/classes5-9 requirements remain.

The paired fixtures cover clear/close completed before Source returns. A later
epoch change between the first and second paired checks can still discard that
original error; this profile does not certify that second fence.

## Original paired error at both stale fences

Implementation `aa3c3a46c19dd19175c879bcb44f5856d24259ec`; [final paired fence evidence](validation/tunnel-paired-final-fence-2026-10-07.md).
Both paired stale branches now map only successful replies to Ok and return their acquired original Err unchanged. The second fence handles an epoch change after the first check passed. Native EOF kind/Io classification remain;no wrapper/error-enum change.
Whole-scan total/cancellation/paired primary across caller drop,out-of-clock failure bookkeeping,metadata admission and all other original L01-L04/classes5-9 requirements remain.

## Managed snapshot resource decisions

Implementation `327b0ef38e43576f2313c51c597b5c16cb7abc01`; [snapshot admission evidence](validation/tunnel-snapshot-admission-2026-10-08.md).
Oversized snapshots or combined candidate plans now return the existing CLI typed resource_limit. Both USB matching-row and manual paired matching-target paths validate the full snapshot first. Original Source Err is preserved;Source Ok admission refusal becomes private Incomplete until the current scan finishes its legal USB candidates. Stale Source success is still discarded without converting a new admission error into native primary.
Source allocation/RSS,global gate/failure/cache/native/list/federation bytes,whole-scan total/cancel/paired primary across caller drop/out-of-clock failure writes,L03-L04/classes5-9 and all other original acceptance remain.

## Failure bookkeeping outcomes

Implementation `4415be9a65921a346208f912dbe59e399e471591`; [failure bookkeeping evidence](validation/tunnel-bookkeeping-2026-10-08.md).
Incomplete bookkeeping now returns its original timer Elapsed and skips new readiness. An acquired paired Source Err or admission Incomplete stays primary through original.context(FailureContext);direct IO downcast/CLI resource classification remain and independent actualElapsed secondary is inspectable. CallerDrop persistence is separate.
Whole-scan total/global cancellation/durable paired or List primary across callerDrop,reconciliation/readiness/global metadata,successful-candidate writer fairness,L03-L04/classes5-9 and all other original acceptance remain.

## Interrupted scans and abandoned discovery errors

The original whole-deadline construction, terminal check and discovery-deadline overflow check precede admission of the actual scan future. The same global metadata bank must pay that concrete frame before generation capture, scan-lock waits or Source work. A refused frame returns resource_limit, including a prepared zero/expired whole deadline; no timer or Source result has been acquired. Admitted scans retain both original absolute clocks and the existing acquired-error/cancellation rules. See [scan frame evidence](validation/tunnel-scan-frame-owner-2026-10-22.md).

A whole scan timeout returns native Elapsed. If discovery already failed, its original anyhow error remains primary and a private redacted context retains the secondary elapsed/cancellation; normal returns preserve direct IO downcast. Global cancellation without an acquired primary remains quiet. A whole timeout creates no device-failure backoff.

Caller Drop preserves the original error in immutable bounded history, exposed through the server terminal report's error source and private original-error getter. The wrapper does not promise transparent direct IO downcast. Full eight-slot history returns the existing CLI resource_limit before Source polling. Native and server primary priorities remain. See [scan control evidence](validation/tunnel-scan-control-2026-10-08.md).

Caller-bounded AFC reads and checked InstallationProxy Browse retain their
existing public error variants. Private typed limit observations inside Io map
to resource_limit; Browse's owned explicit remote business error maps to
service_failure without echoing its code/description. Malformed checked
metadata maps to protocol. Actual Browse elapsed/native errors retain original
sources and late native errors keep priority. AFC primary/cleanup classification
continues to follow the original primary error; callers must inspect reuse and
cleanup separately. See [bounded service evidence](validation/bounded-service-read-2026-10-08.md).

## Checked existing-route RSD and Syslog terminal errors

`ExistingRsdProbeError::class()` exposes a safe typed class under `tunnel` while
native IO/Elapsed remain downcastable in the source chain. Invalid route selectors
fail before dial; checked retained-metadata limits classify ResourceLimit. Missing
fields remain unknown; malformed handshake metadata is Protocol and stops
compatibility fallback. Doctor target mismatch is Precondition, and selected
service absence is ServiceMissing only for a complete directory. Per-check classes
remain distinct; an earlier failed existing-trust check can stay overall primary.

Syslog preserves its first original EOF/native IO/limit error, then marks framing
terminal. Later `next_message` calls return local InvalidData/Protocol without
reading a cached tail or polling transport. Cancellation alone retains the prefix
and permits resumption. Debug/output do not include remote content.
[Host tests and limits](validation/doctor-rsd-syslog-2026-10-09.md).

## Finite SDK operation failures

Private SessionFailure preserves typed ResourceLimit, Conflict, Precondition,
Cancelled and local total Timeout without inventing native Elapsed. Checked
existing-trust replies validate Error before metadata and enforce request identity,
nonempty SessionID, explicit SSL boolean and nonzero u16 service port. Owning
StartSession permission codes classify PermissionDenied; legacy GetValue and
StartService classifications stay unchanged. Original native IO/Elapsed/rustls
causes remain inspectable. A crossing native error wins; late Ok is suppressed.
The IO fence retains an actual native error across an upper Pending until a
terminal result, another allowed native poll or explicit handled-error acknowledgement.
Arbitrary future causality is not inferred. Failed partially consumed service
operations drop their private stream and reject reuse. Diagnostic text excludes
remote error codes and trust contents. [Acceptance](validation/sdk-owned-session-2026-10-09.md).

Finite SDK queries retain nonzero usbmux Result as public session::MuxResponseError
and original numeric code under ServiceFailure, without guessing device meanings.
Native IO/plist/TLS/Elapsed causes remain inspectable; checked response Error wins
over selected Value. Missing Value, missing field and empty string are observations;
explicit remote MissingValue remains NotFound. [Query contract](owned-device-query.md).
